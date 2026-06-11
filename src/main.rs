//! Real-time precipitation radar tile server using KNMI ensemble forecast data.
//!
//! This service ingests NetCDF files from the KNMI seamless precipitation ensemble
//! forecast, projects the Polar Stereographic grid onto Web Mercator tiles, and
//! serves them as WebP images via an HTTP API. It also exposes point-query and
//! time-series endpoints for individual grid cells.

mod projection;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use dashmap::DashMap;
use notify::{EventKind, RecursiveMode, Watcher};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Sentinel value used in the u16 grid to indicate missing / no-data pixels.
const NODATA: u16 = 65535;

/// Conversion factor from raw u16 grid values to mm/h.
const SCALE_FACTOR: f64 = 0.01;

/// Raw value threshold: members with `val >= RAIN_THRESHOLD` count as "raining"
/// when computing probability.
const RAIN_THRESHOLD: u16 = 10;

/// KNMI Open Data dataset identifier.
const KNMI_DATASET: &str = "seamless_precipitation_ensemble_forecast_members";

/// NetCDF variable name for precipitation intensity.
const PRECIP_VAR: &str = "precip_intensity";

// Target Web Mercator grid dimensions and bounds
const GRID_W: u32 = 700;
const GRID_H: u32 = 765;
const MERCATOR_LEFT: f64 = 0.0;
const MERCATOR_RIGHT: f64 = 1210000.0;
const MERCATOR_BOTTOM: f64 = 6250000.0;
const MERCATOR_TOP: f64 = 7560000.0;

// KNMI grid parameters
const KNMI_DX: f64 = 1000.0026129808;
const KNMI_DY: f64 = -1000.0050704712;
const KNMI_X0: f64 = 500.00130649042126;
const KNMI_Y0: f64 = -3650495.413595936;
const KNMI_GRID_W: usize = 700;
const KNMI_GRID_H: usize = 765;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Metadata describing the current NetCDF dataset geometry and contents.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Metadata {
    left: f64,
    right: f64,
    bottom: f64,
    top: f64,
    width: u32,
    height: u32,
    ensembles: Vec<i32>,
    times: Vec<i64>,
    reference_time_str: String,
    version: u64,
}

/// Shared application state accessible from all request handlers.
struct AppState {
    file_path: RwLock<String>,
    empty_tile: Vec<u8>,
    /// Key: (ens, time, z, x, y)
    tile_cache: DashMap<(String, i64, u32, u32, u32), Vec<u8>>,
    /// Key: (ens, time), value: raw grid slice
    grid_cache: DashMap<(String, i64), Arc<Vec<u16>>>,
    metadata: RwLock<Option<Metadata>>,
}

/// Query parameters for the `/api/value` endpoint.
#[derive(Deserialize)]
struct ValueQuery {
    ens: String,
    time: i64,
    lat: f64,
    lon: f64,
}

/// JSON response returned by the `/api/value` endpoint.
#[derive(Serialize)]
struct ValueResponse {
    status: String,
    value: Option<f64>,
}

/// Query parameters for the `/api/timeseries` endpoint.
#[derive(Deserialize)]
struct TimeseriesQuery {
    ens: String,
    lat: f64,
    lon: f64,
}

/// JSON response returned by the `/api/timeseries` endpoint.
#[derive(Serialize)]
struct TimeseriesResponse {
    status: String,
    lat: f64,
    lon: f64,
    ens: String,
    times: Vec<i64>,
    values: Vec<f64>,
}

/// Deserialized response from the KNMI Open Data download-URL endpoint.
#[derive(Deserialize)]
struct FileUrlResponse {
    #[serde(rename = "temporaryDownloadUrl")]
    temporary_download_url: String,
}

// ---------------------------------------------------------------------------
// Ensemble statistics helpers
// ---------------------------------------------------------------------------

/// The statistical reduction to apply across ensemble members.
enum EnsembleStat {
    /// Take the median member value.
    Median,
    /// Take the maximum member value.
    Maximum,
    /// Compute the percentage of members exceeding [`RAIN_THRESHOLD`].
    Probability,
}

impl EnsembleStat {
    /// Parse a short string identifier into an [`EnsembleStat`].
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "med" => Some(Self::Median),
            "max" => Some(Self::Maximum),
            "prob" => Some(Self::Probability),
            _ => None,
        }
    }
}

/// Reduces a set of ensemble member values into a single statistic.
///
/// If the first member is [`NODATA`] the entire cell is considered missing and
/// [`NODATA`] is returned. For probability mode the result is a percentage
/// (0–100) rather than a raw precipitation value.
fn reduce_ensemble(stat: &EnsembleStat, member_vals: &mut [u16]) -> u16 {
    if member_vals.is_empty() || member_vals[0] == NODATA {
        return NODATA;
    }
    match stat {
        EnsembleStat::Maximum => {
            member_vals
                .iter()
                .copied()
                .filter(|&v| v != NODATA)
                .max()
                .unwrap_or(0)
        }
        EnsembleStat::Probability => {
            let count = member_vals
                .iter()
                .copied()
                .filter(|&v| v != NODATA && v >= RAIN_THRESHOLD)
                .count();
            ((count * 100) / member_vals.len()) as u16
        }
        EnsembleStat::Median => {
            member_vals.sort_unstable();
            member_vals[member_vals.len() / 2]
        }
    }
}

/// Converts a raw u16 grid value to a floating-point value in mm/h.
///
/// [`NODATA`] is mapped to `0.0`.
fn raw_to_value(raw: u16) -> f64 {
    if raw == NODATA {
        0.0
    } else {
        raw as f64 * SCALE_FACTOR
    }
}

// ---------------------------------------------------------------------------
// Color mapping helpers
// ---------------------------------------------------------------------------

/// An anchor point in a piecewise-linear colour ramp.
struct ColorAnchor {
    value: f64,
    color: [u8; 4],
}

/// Linearly interpolates an RGBA colour from a sorted list of anchor points.
fn interpolate_color(value: f64, anchors: &[ColorAnchor]) -> [u8; 4] {
    if value <= anchors[0].value {
        return anchors[0].color;
    }
    if value >= anchors[anchors.len() - 1].value {
        return anchors[anchors.len() - 1].color;
    }
    for i in 0..anchors.len() - 1 {
        let a1 = &anchors[i];
        let a2 = &anchors[i + 1];
        if value >= a1.value && value <= a2.value {
            let t = (value - a1.value) / (a2.value - a1.value);
            return [
                (a1.color[0] as f64 + t * (a2.color[0] as f64 - a1.color[0] as f64)) as u8,
                (a1.color[1] as f64 + t * (a2.color[1] as f64 - a1.color[1] as f64)) as u8,
                (a1.color[2] as f64 + t * (a2.color[2] as f64 - a1.color[2] as f64)) as u8,
                (a1.color[3] as f64 + t * (a2.color[3] as f64 - a1.color[3] as f64)) as u8,
            ];
        }
    }
    [0, 0, 0, 0]
}

/// Maps a raw u16 precipitation value to an interpolated RGBA colour.
fn get_color(val_raw: u16) -> [u8; 4] {
    if val_raw == NODATA {
        return [0, 0, 0, 0];
    }
    let val = val_raw as f64 * SCALE_FACTOR;
    if val < 0.05 {
        return [0, 0, 0, 0]; // Ignore extremely light values (noise)
    }

    let anchors = [
        ColorAnchor { value: 0.05,  color: [120, 200, 255, 120] }, // Very light blue
        ColorAnchor { value: 0.2,   color: [0, 100, 255, 170] },   // Blue
        ColorAnchor { value: 1.0,   color: [0, 200, 0, 190] },     // Green
        ColorAnchor { value: 5.0,   color: [255, 230, 0, 210] },   // Yellow
        ColorAnchor { value: 15.0,  color: [255, 120, 0, 230] },   // Orange
        ColorAnchor { value: 30.0,  color: [255, 0, 0, 240] },     // Red
        ColorAnchor { value: 100.0, color: [200, 0, 200, 255] },   // Purple
        ColorAnchor { value: 250.0, color: [255, 255, 255, 255] }, // White
    ];

    interpolate_color(val, &anchors)
}

/// Maps a probability percentage (0–100) to an interpolated RGBA colour.
fn get_probability_color(p: u16) -> [u8; 4] {
    if p == NODATA || p < 10 {
        return [0, 0, 0, 0];
    }

    let anchors = [
        ColorAnchor { value: 10.0,  color: [180, 200, 220, 80] },  // Very light grey-blue
        ColorAnchor { value: 30.0,  color: [100, 160, 255, 120] }, // Light blue
        ColorAnchor { value: 50.0,  color: [0, 100, 255, 160] },   // Blue
        ColorAnchor { value: 70.0,  color: [0, 200, 100, 180] },   // Teal
        ColorAnchor { value: 90.0,  color: [220, 0, 220, 220] },   // Magenta-purple
        ColorAnchor { value: 100.0, color: [255, 255, 255, 240] }, // White
    ];

    interpolate_color(p as f64, &anchors)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();
    println!("Starting Weather Radar service...");

    // 1. Find the latest netcdf file in the current directory
    let initial_file =
        find_latest_nc_file(".").expect("No NetCDF (.nc) files found in workspace root!");
    println!("Found initial NetCDF file: {}", initial_file);

    // 2. Generate the empty transparent tile
    let empty_tile = generate_empty_tile();

    // 3. Load initial metadata
    let metadata_val = match load_metadata(&initial_file) {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("Error loading metadata from {}: {}", initial_file, e);
            None
        }
    };

    let state = Arc::new(AppState {
        file_path: RwLock::new(initial_file.clone()),
        empty_tile,
        tile_cache: DashMap::new(),
        grid_cache: DashMap::new(),
        metadata: RwLock::new(metadata_val.clone()),
    });

    // Spawn MQTT client to listen for updates from KNMI
    let state_clone_mqtt = state.clone();
    tokio::spawn(async move {
        start_knmi_mqtt_listener(state_clone_mqtt).await;
    });

    // 4. Set up directory watcher to monitor file updates
    let state_clone = state.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    let _ = tx.blocking_send(());
                }
            }
        })
        .expect("Failed to create file watcher");

    watcher
        .watch(std::path::Path::new("."), RecursiveMode::NonRecursive)
        .expect("Failed to watch current directory");

    tokio::spawn(async move {
        // Keep watcher reference alive in task
        let _watcher = watcher;
        while rx.recv().await.is_some() {
            // Wait for file write to complete
            tokio::time::sleep(Duration::from_millis(1000)).await;

            if let Some(new_file) = find_latest_nc_file(".") {
                let current_file = state_clone.file_path.read().await.clone();
                if current_file != new_file {
                    println!("Detected new NetCDF file: {}", new_file);
                    match load_metadata(&new_file) {
                        Ok(meta) => {
                            let mut file_write = state_clone.file_path.write().await;
                            *file_write = new_file;

                            let mut meta_write = state_clone.metadata.write().await;
                            *meta_write = Some(meta.clone());

                            state_clone.tile_cache.clear();
                            state_clone.grid_cache.clear();
                            println!("Successfully reloaded metadata and cleared caches.");
                        }
                        Err(e) => {
                            eprintln!("Failed to load new NetCDF metadata: {}", e);
                        }
                    }
                }
            }
        }
    });

    // 5. Configure Router
    let app = Router::new()
        .route("/api/metadata", get(get_metadata))
        .route("/api/map/:ens/:time/:z/:x/:y", get(get_tile))
        .route("/api/value", get(get_value))
        .route("/api/timeseries", get(get_timeseries))
        .nest_service("/", ServeDir::new("static"))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // 6. Start Server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .unwrap();
    println!("Webservice running on http://localhost:8080");
    axum::serve(listener, app).await.unwrap();
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Scans a directory for the most-recently-modified `.nc` file and returns its path.
fn find_latest_nc_file(dir: &str) -> Option<String> {
    let mut latest: Option<(std::path::PathBuf, std::time::SystemTime)> = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "nc") {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if latest.is_none() || modified > latest.as_ref().unwrap().1 {
                            latest = Some((path, modified));
                        }
                    }
                }
            }
        }
    }
    latest.map(|(path, _)| path.to_string_lossy().to_string())
}

/// Loads dimension sizes and coordinate variables from a NetCDF file and
/// returns a [`Metadata`] struct suitable for JSON serialisation.
fn load_metadata(file_path: &str) -> Result<Metadata, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let ens_var = file
        .variable("ens_number")
        .ok_or("ens_number variable not found")?;
    let time_var = file.variable("time").ok_or("time variable not found")?;

    let ensembles = ens_var.get_values::<i32, _>(..)?;
    let times = time_var.get_values::<i64, _>(..)?;

    let time_units = match time_var
        .attribute("units")
        .ok_or("time units attribute not found")?
        .value()?
    {
        netcdf::AttributeValue::Str(s) => s,
        val => return Err(format!("Unexpected time units type: {:?}", val).into()),
    };

    // Use file modified time as the version number for client-side cache invalidation
    let metadata_fs = std::fs::metadata(file_path)?;
    let modified = metadata_fs.modified()?;
    let version = modified
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(Metadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        ensembles,
        times,
        reference_time_str: time_units,
        version,
    })
}

/// Generates a 256×256 fully-transparent WebP image used for empty / out-of-bounds tiles.
fn generate_empty_tile() -> Vec<u8> {
    use image::{ImageBuffer, ImageFormat};
    use std::io::Cursor;
    let img = ImageBuffer::from_pixel(256, 256, image::Rgba([0_u8, 0_u8, 0_u8, 0_u8]));
    let mut webp_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut webp_bytes);
    img.write_to(&mut cursor, ImageFormat::WebP).unwrap();
    webp_bytes
}

/// Reads a 2D slice `(y, x)` for a given ensemble and time index from a NetCDF file.
fn read_netcdf_slice(
    file_path: &str,
    ens_idx: usize,
    time_idx: usize,
) -> Result<Vec<u16>, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let var = file
        .variable(PRECIP_VAR)
        .ok_or("precip_intensity variable not found")?;

    let slice = var.get_values::<u16, _>((
        &[ens_idx, time_idx, 0, 0][..],
        &[1, 1, KNMI_GRID_H, KNMI_GRID_W][..],
    ))?;
    Ok(slice)
}

/// Bilinear interpolation of a raw u16 grid value at fractional grid coordinates.
///
/// Returns [`NODATA`] when the query point falls entirely outside the grid or
/// when no valid neighbours are found.
fn interpolate_bilinear(fx: f64, fy: f64, raw_slice: &[u16]) -> u16 {
    let ix1 = fx.floor() as i32;
    let iy1 = fy.floor() as i32;
    let ix2 = ix1 + 1;
    let iy2 = iy1 + 1;

    if ix1 < -1 || ix1 >= KNMI_GRID_W as i32 || iy1 < -1 || iy1 >= KNMI_GRID_H as i32 {
        return NODATA;
    }

    let wx = (fx - ix1 as f64) as f32;
    let wy = (fy - iy1 as f64) as f32;

    let w00 = (1.0 - wx) * (1.0 - wy);
    let w10 = wx * (1.0 - wy);
    let w01 = (1.0 - wx) * wy;
    let w11 = wx * wy;

    let get_val = |x: i32, y: i32| -> Option<(u16, f32)> {
        if x >= 0 && x < KNMI_GRID_W as i32 && y >= 0 && y < KNMI_GRID_H as i32 {
            let val = raw_slice[(y * KNMI_GRID_W as i32 + x) as usize];
            if val != NODATA {
                Some((val, 1.0))
            } else {
                None
            }
        } else {
            None
        }
    };

    let mut sum_val = 0.0;
    let mut sum_weight = 0.0;

    let neighbors = [
        (get_val(ix1, iy1), w00),
        (get_val(ix2, iy1), w10),
        (get_val(ix1, iy2), w01),
        (get_val(ix2, iy2), w11),
    ];

    for (opt, w) in neighbors {
        if let Some((val, _)) = opt {
            sum_val += (val as f64) * (w as f64);
            sum_weight += w as f64;
        }
    }

    if sum_weight > 0.001 {
        (sum_val / sum_weight).round() as u16
    } else {
        NODATA
    }
}

/// Computes (or reads from cache) the raw u16 grid for a given ensemble selector
/// and time step, applying ensemble statistics when `ens_str` is `"med"`, `"max"`,
/// or `"prob"`.
fn compute_raw_slice(
    file_path: &str,
    meta: &Metadata,
    ens_str: &str,
    time: i64,
) -> Result<Vec<u16>, (StatusCode, String)> {
    if let Some(stat) = EnsembleStat::from_str(ens_str) {
        // Read all ensemble members
        let mut member_slices = Vec::with_capacity(meta.ensembles.len());
        for &ens_val in &meta.ensembles {
            let ens_idx = meta.ensembles.iter().position(|&e| e == ens_val).unwrap();
            let time_idx = meta.times.iter().position(|&t| t == time).ok_or((
                StatusCode::BAD_REQUEST,
                format!("Invalid time: {}", time),
            ))?;
            let slice = read_netcdf_slice(file_path, ens_idx, time_idx).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Error reading slice for E{}: {}", ens_val, e),
                )
            })?;
            member_slices.push(slice);
        }

        // Compute statistics for each cell
        let grid_size = KNMI_GRID_H * KNMI_GRID_W;
        let mut raw_slice = vec![NODATA; grid_size];
        for i in 0..grid_size {
            let mut vals: Vec<u16> = member_slices.iter().map(|s| s[i]).collect();
            raw_slice[i] = reduce_ensemble(&stat, &mut vals);
        }
        Ok(raw_slice)
    } else {
        // Individual member
        let ens_num: i32 = ens_str.parse().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid ensemble parameter: {}", ens_str),
            )
        })?;

        let ens_idx = meta
            .ensembles
            .iter()
            .position(|&e| e == ens_num)
            .ok_or((
                StatusCode::BAD_REQUEST,
                format!("Invalid ensemble number: {}", ens_num),
            ))?;

        let time_idx = meta
            .times
            .iter()
            .position(|&t| t == time)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", time)))?;

        read_netcdf_slice(file_path, ens_idx, time_idx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error reading slice: {}", e),
            )
        })
    }
}

/// Renders a 256×256 WebP tile by projecting each pixel from Web Mercator into
/// the KNMI Polar Stereographic grid and applying the appropriate colour ramp.
fn render_tile_webp_bytes(
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    raw_slice: &[u16],
    is_prob: bool,
) -> Vec<u8> {
    use image::{ImageBuffer, ImageFormat};
    use std::io::Cursor;
    let mut img = ImageBuffer::new(256, 256);

    for row in 0..256 {
        for col in 0..256 {
            let col_frac = (col as f64 + 0.5) / 256.0;
            let row_frac = (row as f64 + 0.5) / 256.0;
            let x_merc = left + col_frac * (right - left);
            let y_merc = top - row_frac * (top - bottom);

            let (lon, lat) = projection::mercator_to_lonlat(x_merc, y_merc);
            let (px, py) = projection::lonlat_to_polar_stereographic(lon, lat);

            let fx = (px - KNMI_X0) / KNMI_DX;
            let fy = (py - KNMI_Y0) / KNMI_DY;

            let val_raw = interpolate_bilinear(fx, fy, raw_slice);

            let color = if is_prob {
                get_probability_color(val_raw)
            } else {
                get_color(val_raw)
            };

            img.put_pixel(col, row, image::Rgba(color));
        }
    }

    let mut webp_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut webp_bytes);
    img.write_to(&mut cursor, ImageFormat::WebP).unwrap();
    webp_bytes
}

// ---------------------------------------------------------------------------
// API handlers
// ---------------------------------------------------------------------------

/// Returns the current dataset metadata as JSON.
async fn get_metadata(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let meta = state.metadata.read().await.clone();
    match meta {
        Some(m) => Ok(axum::Json(m)),
        None => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Metadata not loaded".to_string(),
        )),
    }
}

/// Serves a Web Mercator projected, colour-mapped WebP overlay tile.
async fn get_tile(
    Path((ens_str, time, z, x, y)): Path<(String, i64, u32, u32, u32)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check cache
    if let Some(cached_tile) = state.tile_cache.get(&(ens_str.clone(), time, z, x, y)) {
        return Ok(Response::builder()
            .header("Content-Type", "image/webp")
            .header("Cache-Control", "public, max-age=300")
            .body(axum::body::Body::from(cached_tile.value().clone()))
            .unwrap());
    }

    // 1. Calculate the Web Mercator bounds of the tile
    const MAP_LIMIT: f64 = std::f64::consts::PI * 6378137.0; // 20037508.342789244
    let n = 2.0_f64.powi(z as i32);
    let tile_size = (2.0 * MAP_LIMIT) / n;
    let left = -MAP_LIMIT + (x as f64) * tile_size;
    let right = left + tile_size;
    let top = MAP_LIMIT - (y as f64) * tile_size;
    let bottom = top - tile_size;

    // 2. Convert the 4 corners to Polar Stereographic to verify overlap
    let corners = [
        (left, top),
        (right, top),
        (left, bottom),
        (right, bottom),
    ];
    let mut ps_coords = Vec::with_capacity(4);
    for (cx, cy) in corners {
        let (lon, lat) = projection::mercator_to_lonlat(cx, cy);
        let (px, py) = projection::lonlat_to_polar_stereographic(lon, lat);
        ps_coords.push((px, py));
    }

    let min_px = ps_coords.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    let max_px = ps_coords
        .iter()
        .map(|c| c.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_py = ps_coords.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
    let max_py = ps_coords
        .iter()
        .map(|c| c.1)
        .fold(f64::NEG_INFINITY, f64::max);

    // Bounding Box of KNMI grid in Polar Stereographic
    const KNMI_X_MIN: f64 = 500.0 - 50000.0;
    const KNMI_X_MAX: f64 = 700500.0 + 50000.0;
    const KNMI_Y_MIN: f64 = -4415495.4 - 50000.0;
    const KNMI_Y_MAX: f64 = -3650495.4 + 50000.0;

    let overlap = max_px >= KNMI_X_MIN
        && min_px <= KNMI_X_MAX
        && max_py >= KNMI_Y_MIN
        && min_py <= KNMI_Y_MAX;

    if !overlap {
        return Ok(Response::builder()
            .header("Content-Type", "image/webp")
            .header("Cache-Control", "public, max-age=300")
            .body(axum::body::Body::from(state.empty_tile.clone()))
            .unwrap());
    }

    // Get current file path and metadata
    let file_path = state.file_path.read().await.clone();
    let meta = state.metadata.read().await.clone().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    let is_prob = ens_str == "prob";

    // 3. Retrieve or compute raw slice (cached in memory)
    let raw_slice = if let Some(cached) = state.grid_cache.get(&(ens_str.clone(), time)) {
        cached.value().clone()
    } else {
        let computed = compute_raw_slice(&file_path, &meta, &ens_str, time)?;
        let arc = Arc::new(computed);
        state
            .grid_cache
            .insert((ens_str.clone(), time), arc.clone());
        arc
    };

    // Render tile image dynamically using WebP
    let webp_bytes = render_tile_webp_bytes(left, right, top, bottom, &raw_slice, is_prob);

    // Cache results
    state
        .tile_cache
        .insert((ens_str, time, z, x, y), webp_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "public, max-age=300")
        .body(axum::body::Body::from(webp_bytes))
        .unwrap())
}

/// Returns the precipitation value (or ensemble statistic) at a single
/// geographic point as JSON.
async fn get_value(
    Query(q): Query<ValueQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let file_path = state.file_path.read().await.clone();
    let meta = state.metadata.read().await.clone().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    // Convert GPS coordinates to Polar Stereographic
    let (px, py) = projection::lonlat_to_polar_stereographic(q.lon, q.lat);

    // Get grid cell index
    let ix = ((px - KNMI_X0) / KNMI_DX).round() as i32;
    let iy = ((py - KNMI_Y0) / KNMI_DY).round() as i32;

    if ix < 0 || ix >= KNMI_GRID_W as i32 || iy < 0 || iy >= KNMI_GRID_H as i32 {
        return Ok(axum::Json(ValueResponse {
            status: "out_of_bounds".to_string(),
            value: None,
        }));
    }

    // Read value based on query type
    let file = netcdf::open(&file_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let var = file.variable(PRECIP_VAR).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "precip_intensity variable missing".to_string(),
    ))?;

    let (status_out, value_out) = if let Some(stat) = EnsembleStat::from_str(&q.ens) {
        // Read value at target cell across all members
        let time_idx = meta.times.iter().position(|&t| t == q.time).ok_or((
            StatusCode::BAD_REQUEST,
            format!("Invalid time: {}", q.time),
        ))?;
        let mut vals = Vec::with_capacity(meta.ensembles.len());
        for (ens_idx, _) in meta.ensembles.iter().enumerate() {
            let val_raw: u16 = var
                .get_value((ens_idx, time_idx, iy as usize, ix as usize))
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            vals.push(val_raw);
        }

        let reduced = reduce_ensemble(&stat, &mut vals);

        match stat {
            EnsembleStat::Probability => ("probability".to_string(), reduced as f64),
            _ => {
                let val_mmh = raw_to_value(reduced);
                let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
                (status.to_string(), val_mmh)
            }
        }
    } else {
        // Individual member
        let ens_num: i32 = q.ens.parse().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid ensemble parameter: {}", q.ens),
            )
        })?;

        let ens_idx = meta
            .ensembles
            .iter()
            .position(|&e| e == ens_num)
            .ok_or((
                StatusCode::BAD_REQUEST,
                format!("Invalid ensemble: {}", ens_num),
            ))?;

        let time_idx = meta
            .times
            .iter()
            .position(|&t| t == q.time)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", q.time)))?;

        let val_raw: u16 = var
            .get_value((ens_idx, time_idx, iy as usize, ix as usize))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let val_mmh = raw_to_value(val_raw);
        let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
        (status.to_string(), val_mmh)
    };

    Ok(axum::Json(ValueResponse {
        status: status_out,
        value: Some(value_out),
    }))
}

/// Returns a time-series of precipitation values (or ensemble statistics) at a
/// single geographic point across all forecast time steps.
async fn get_timeseries(
    Query(q): Query<TimeseriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let file_path = state.file_path.read().await.clone();
    let meta = state.metadata.read().await.clone().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    // Convert GPS coordinates to Polar Stereographic
    let (px, py) = projection::lonlat_to_polar_stereographic(q.lon, q.lat);

    // Get grid cell index
    let ix = ((px - KNMI_X0) / KNMI_DX).round() as i32;
    let iy = ((py - KNMI_Y0) / KNMI_DY).round() as i32;

    if ix < 0 || ix >= KNMI_GRID_W as i32 || iy < 0 || iy >= KNMI_GRID_H as i32 {
        return Ok(axum::Json(TimeseriesResponse {
            status: "out_of_bounds".to_string(),
            lat: q.lat,
            lon: q.lon,
            ens: q.ens,
            times: Vec::new(),
            values: Vec::new(),
        }));
    }

    let file = netcdf::open(&file_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let var = file.variable(PRECIP_VAR).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "precip_intensity variable missing".to_string(),
    ))?;

    let num_times = meta.times.len();
    let num_ensembles = meta.ensembles.len();
    let mut values = Vec::with_capacity(num_times);

    if let Some(stat) = EnsembleStat::from_str(&q.ens) {
        // Read values for all ensembles and all times at the target pixel
        let raw_grid = var
            .get_values::<u16, _>((
                &[0, 0, iy as usize, ix as usize][..],
                &[num_ensembles, num_times, 1, 1][..],
            ))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        for t in 0..num_times {
            let mut member_vals: Vec<u16> = (0..num_ensembles)
                .map(|e| raw_grid[e * num_times + t])
                .collect();

            let reduced = reduce_ensemble(&stat, &mut member_vals);

            match stat {
                EnsembleStat::Probability => values.push(reduced as f64),
                _ => values.push(raw_to_value(reduced)),
            }
        }
    } else {
        // Individual member
        let ens_num: i32 = q.ens.parse().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid ensemble parameter: {}", q.ens),
            )
        })?;

        let ens_idx = meta
            .ensembles
            .iter()
            .position(|&e| e == ens_num)
            .ok_or((
                StatusCode::BAD_REQUEST,
                format!("Invalid ensemble: {}", ens_num),
            ))?;

        let raw_values = var
            .get_values::<u16, _>((
                &[ens_idx, 0, iy as usize, ix as usize][..],
                &[1, num_times, 1, 1][..],
            ))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        for val_raw in raw_values {
            values.push(raw_to_value(val_raw));
        }
    }

    Ok(axum::Json(TimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        ens: q.ens,
        times: meta.times.clone(),
        values,
    }))
}

// ---------------------------------------------------------------------------
// KNMI MQTT listener & file downloader
// ---------------------------------------------------------------------------

/// Connects to the KNMI MQTT broker and listens for new dataset notifications.
///
/// When a new NetCDF file is published, it is downloaded and written to the
/// current directory so the file watcher can pick it up.
async fn start_knmi_mqtt_listener(state: Arc<AppState>) {
    let broker = "wss://mqtt.dataplatform.knmi.nl";
    let port = 443;
    let mqtt_password =
        std::env::var("KNMI_MQTT_PASSWORD").expect("KNMI_MQTT_PASSWORD environment variable not set!");
    let open_data_api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .expect("KNMI_OPEN_DATA_API_KEY environment variable not set!");
    let topic = format!(
        "dataplatform/file/v1/{}/1.0/#",
        KNMI_DATASET
    );

    let client_id = format!(
        "weer-service-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );

    println!(
        "Initializing KNMI MQTT subscriber with Client ID: {}...",
        client_id
    );

    let mut mqttoptions = MqttOptions::new(client_id, broker, port);
    mqttoptions.set_keep_alive(Duration::from_secs(30));
    mqttoptions.set_credentials("token", mqtt_password);

    let tls_config = TlsConfiguration::default();
    mqttoptions.set_transport(Transport::wss_with_config(tls_config));

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 50);

    // Subscribe to topic
    if let Err(e) = client.subscribe(&topic, QoS::AtMostOnce).await {
        eprintln!("Failed to subscribe to KNMI MQTT topic: {:?}", e);
        return;
    }
    println!("Subscribed to KNMI topic: {}", topic);

    // Event loop
    loop {
        match eventloop.poll().await {
            Ok(notification) => {
                if let Event::Incoming(Packet::Publish(publish)) = notification {
                    let payload_str = match String::from_utf8(publish.payload.to_vec()) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    println!("Received KNMI MQTT notification: {}", payload_str);

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload_str) {
                        let data = json.get("data");
                        let file_name = data
                            .and_then(|d| {
                                d.get("filename")
                                    .or_else(|| d.get("fileName"))
                                    .or_else(|| d.get("file_name"))
                            })
                            .and_then(|v| v.as_str())
                            .or_else(|| {
                                json.get("fileName")
                                    .or_else(|| json.get("file_name"))
                                    .and_then(|v| v.as_str())
                            });

                        let file_url = data.and_then(|d| d.get("url")).and_then(|v| v.as_str());

                        if let Some(name) = file_name {
                            if name.ends_with(".nc") {
                                println!("New NetCDF file available: {}", name);
                                let state_clone = state.clone();
                                let name_clone = name.to_string();
                                let url_opt = file_url.map(|s| s.to_string());
                                let open_data_api_key_clone = open_data_api_key.to_string();
                                tokio::spawn(async move {
                                    if let Err(e) = download_and_update_nc_file(
                                        &name_clone,
                                        url_opt.as_deref(),
                                        &open_data_api_key_clone,
                                        state_clone,
                                    )
                                    .await
                                    {
                                        eprintln!(
                                            "Error processing file update for {}: {:?}",
                                            name_clone, e
                                        );
                                    }
                                });
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "MQTT Connection error: {:?}. Retrying in 10 seconds...",
                    e
                );
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

/// Downloads a new NetCDF file from the KNMI Open Data API, saves it to the
/// current directory, and removes stale files.
async fn download_and_update_nc_file(
    filename: &str,
    file_url: Option<&str>,
    api_key: &str,
    _state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Requesting download URL for {} from KNMI Open Data API...",
        filename
    );

    let url = match file_url {
        Some(u) => u.to_string(),
        None => format!(
            "https://api.dataplatform.knmi.nl/open-data/v1/datasets/{}/versions/1.0/files/{}/url",
            KNMI_DATASET, filename
        ),
    };

    let client = reqwest::Client::builder().build()?;

    let res = client
        .get(&url)
        .header("Authorization", api_key)
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(format!("Failed to get download URL, HTTP status: {}", res.status()).into());
    }

    let url_resp: FileUrlResponse = res.json().await?;
    let download_url = url_resp.temporary_download_url;

    println!("Downloading file from temporary URL: {}...", filename);

    let file_res = client.get(&download_url).send().await?;
    if !file_res.status().is_success() {
        return Err(
            format!("Failed to download file content, HTTP status: {}", file_res.status()).into(),
        );
    }

    let bytes = file_res.bytes().await?;

    let temp_path = "temp_knmi_download.nc";
    tokio::fs::write(temp_path, &bytes).await?;

    let final_path = format!("./{}", filename);
    tokio::fs::rename(temp_path, &final_path).await?;
    println!("Successfully downloaded and saved: {}", final_path);

    // Delete old NetCDF files to save space
    if let Ok(mut entries) = tokio::fs::read_dir(".").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "nc" {
                        if let Some(file_stem) = path.file_name().and_then(|n| n.to_str()) {
                            if file_stem != filename
                                && file_stem.starts_with("KNMI_PYSTEPS_BLEND_ENS_")
                            {
                                println!("Deleting old NetCDF file: {:?}", path);
                                let _ = tokio::fs::remove_file(path).await;
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
