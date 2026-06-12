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
    /// Key: (ens, time), value: raw grid slice
    grid_cache: DashMap<(String, i64), Arc<Vec<u16>>>,
    /// Key: (ens, time), value: PNG data image bytes
    data_cache: DashMap<(String, i64), Vec<u8>>,
    metadata: RwLock<Option<Metadata>>,
    projection_lut: Vec<(f32, f32)>,
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
/// Initializes the coordinate projection lookup table.
fn init_projection_lut() -> Vec<(f32, f32)> {
    let mut lut = Vec::with_capacity((GRID_W * GRID_H) as usize);
    for row in 0..GRID_H {
        for col in 0..GRID_W {
            let col_frac = (col as f64 + 0.5) / GRID_W as f64;
            let row_frac = (row as f64 + 0.5) / GRID_H as f64;
            
            let x_merc = MERCATOR_LEFT + col_frac * (MERCATOR_RIGHT - MERCATOR_LEFT);
            let y_merc = MERCATOR_TOP - row_frac * (MERCATOR_TOP - MERCATOR_BOTTOM);

            let (lon, lat) = projection::mercator_to_lonlat(x_merc, y_merc);
            let (px, py) = projection::lonlat_to_polar_stereographic(lon, lat);

            let fx = ((px - KNMI_X0) / KNMI_DX) as f32;
            let fy = ((py - KNMI_Y0) / KNMI_DY) as f32;
            lut.push((fx, fy));
        }
    }
    lut
}


/// Precalculates all packed PNG data in the background.
async fn precalculate_all_data(state: Arc<AppState>, meta: Metadata) {
    let target_version = meta.version;
    let file_path = state.file_path.read().await.clone();
    let num_times = meta.times.len();
    let num_ensembles = meta.ensembles.len();

    println!(
        "Starting background precalculation for NetCDF version {} ({} times, {} ensembles)...",
        target_version, num_times, num_ensembles
    );

    // Limit concurrency of rendering tasks to the number of CPU cores (min 2)
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cpus));

    // Loop over time steps
    for (time_idx, &time_val) in meta.times.iter().enumerate() {
        // Check for cancellation
        {
            let current_meta = state.metadata.read().await;
            if current_meta.as_ref().map(|m| m.version) != Some(target_version) {
                println!(
                    "Precalculation for version {} cancelled.",
                    target_version
                );
                return;
            }
        }

        if time_idx % 10 == 0 || time_idx == num_times - 1 {
            println!(
                "Precalculating version {}... {}% done ({}/{})",
                target_version,
                (time_idx * 100) / num_times,
                time_idx + 1,
                num_times
            );
        }

        // Read all ensemble member slices for this time step in a single sequential I/O read call
        let all_members_data = match read_netcdf_all_ensembles(&file_path, time_idx, num_ensembles) {
            Ok(data) => data,
            Err(e) => {
                eprintln!(
                    "Error reading all ensemble slices for time index {}: {}",
                    time_idx, e
                );
                continue;
            }
        };

        // Compute stats: med, max, prob
        let grid_size = KNMI_GRID_H * KNMI_GRID_W;
        let mut med_slice = vec![NODATA; grid_size];
        let mut max_slice = vec![NODATA; grid_size];
        let mut prob_slice = vec![NODATA; grid_size];

        let mut vals_buf = vec![0u16; num_ensembles];
        let mut vals_med = vec![0u16; num_ensembles];
        let mut vals_max = vec![0u16; num_ensembles];
        let mut vals_prob = vec![0u16; num_ensembles];

        for i in 0..grid_size {
            for ens_idx in 0..num_ensembles {
                vals_buf[ens_idx] = all_members_data[ens_idx * grid_size + i];
            }

            vals_med.copy_from_slice(&vals_buf);
            med_slice[i] = reduce_ensemble(&EnsembleStat::Median, &mut vals_med);

            vals_max.copy_from_slice(&vals_buf);
            max_slice[i] = reduce_ensemble(&EnsembleStat::Maximum, &mut vals_max);

            vals_prob.copy_from_slice(&vals_buf);
            prob_slice[i] = reduce_ensemble(&EnsembleStat::Probability, &mut vals_prob);
        }

        // Insert stats into grid_cache
        let arc_med = Arc::new(med_slice);
        let arc_max = Arc::new(max_slice);
        let arc_prob = Arc::new(prob_slice);

        state.grid_cache.insert(("med".to_string(), time_val), arc_med.clone());
        state.grid_cache.insert(("max".to_string(), time_val), arc_max.clone());
        state.grid_cache.insert(("prob".to_string(), time_val), arc_prob.clone());

        // Insert individual member slices into grid_cache
        for (ens_idx, &ens_num) in meta.ensembles.iter().enumerate() {
            let start = ens_idx * grid_size;
            let end = start + grid_size;
            let slice = all_members_data[start..end].to_vec();
            state.grid_cache.insert((ens_num.to_string(), time_val), Arc::new(slice));
        }

        // Render PNGs for stats (med, max, prob)
        let render_items = vec![
            ("med".to_string(), arc_med),
            ("max".to_string(), arc_max),
            ("prob".to_string(), arc_prob),
        ];

        for (ens_str, slice) in render_items {
            let state_clone = state.clone();
            let sem = semaphore.clone();
            let ens_str_clone = ens_str.clone();
            let time_val_clone = time_val;
            
            tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let png_bytes = render_data_png_bytes(&slice, &state_clone.projection_lut);
                state_clone.data_cache.insert((ens_str_clone, time_val_clone), png_bytes);
            });
        }


        // Yield control back to executor to let other tasks run
        tokio::task::yield_now().await;
    }

    println!(
        "Background precalculation completed for NetCDF version {}.",
        target_version
    );
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();
    println!("Starting Weather Radar service...");

    // 1. Find the latest netcdf file in the current directory, or download it if none exists
    let initial_file = match find_latest_nc_file(".") {
        Some(f) => f,
        None => {
            println!("No NetCDF (.nc) files found in workspace root. Fetching latest file from KNMI Open Data API...");
            match fetch_latest_nc_file(".").await {
                Ok(f) => f,
                Err(e) => {
                    panic!("Failed to download initial NetCDF file on startup: {}", e);
                }
            }
        }
    };
    println!("Found initial NetCDF file: {}", initial_file);

    // 2. Load initial metadata
    let metadata_val = match load_metadata(&initial_file) {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("Error loading metadata from {}: {}", initial_file, e);
            None
        }
    };

    let state = Arc::new(AppState {
        file_path: RwLock::new(initial_file.clone()),
        grid_cache: DashMap::new(),
        data_cache: DashMap::new(),
        metadata: RwLock::new(metadata_val.clone()),
        projection_lut: init_projection_lut(),
    });

    if let Some(ref meta) = metadata_val {
        let state_clone = state.clone();
        let meta_clone = meta.clone();
        tokio::spawn(async move {
            precalculate_all_data(state_clone, meta_clone).await;
        });
    }

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

                            state_clone.grid_cache.clear();
                            state_clone.data_cache.clear();
                            println!("Successfully reloaded metadata and cleared caches.");

                            let state_clone2 = state_clone.clone();
                            let meta_clone = meta.clone();
                            tokio::spawn(async move {
                                precalculate_all_data(state_clone2, meta_clone).await;
                            });
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
        .route("/api/data/:ens/:time", get(get_data_image))
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

/// Reads all ensemble slices for a given time index from a NetCDF file in a single I/O call.
fn read_netcdf_all_ensembles(
    file_path: &str,
    time_idx: usize,
    num_ensembles: usize,
) -> Result<Vec<u16>, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let var = file
        .variable(PRECIP_VAR)
        .ok_or("precip_intensity variable not found")?;

    let values = var.get_values::<u16, _>((
        &[0, time_idx, 0, 0][..],
        &[num_ensembles, 1, KNMI_GRID_H, KNMI_GRID_W][..],
    ))?;
    Ok(values)
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


/// Renders the entire KNMI radar grid for a timeframe as a 700x765 Web Mercator projected
/// lossless PNG using a coordinate lookup table (LUT). The u16 raw values are packed into the Red (high byte) and Green (low byte) channels.
fn render_data_png_bytes(raw_slice: &[u16], lut: &[(f32, f32)]) -> Vec<u8> {
    use image::{ImageBuffer, ImageFormat};
    use std::io::Cursor;
    
    let mut img = ImageBuffer::new(GRID_W, GRID_H);

    for row in 0..GRID_H {
        for col in 0..GRID_W {
            let idx = (row * GRID_W + col) as usize;
            let (fx, fy) = lut[idx];

            let val_raw = interpolate_bilinear(fx as f64, fy as f64, raw_slice);
            let packed_val = if val_raw == NODATA { 0 } else { val_raw };

            // Pack the u16 value into the Red (high byte) and Green (low byte) channels
            let r = (packed_val >> 8) as u8;
            let g = (packed_val & 0xFF) as u8;

            img.put_pixel(col, row, image::Rgba([r, g, 0, 255]));
        }
    }

    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, ImageFormat::Png).unwrap();
    png_bytes
}

/// Serves the lossless R/G packed raw radar data PNG for a timeframe.
async fn get_data_image(
    Path((ens_str, time)): Path<(String, i64)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check cache
    if let Some(cached_data) = state.data_cache.get(&(ens_str.clone(), time)) {
        return Ok(Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "public, max-age=300")
            .body(axum::body::Body::from(cached_data.value().clone()))
            .unwrap());
    }

    // Get current file path and metadata
    let file_path = state.file_path.read().await.clone();
    let meta = state.metadata.read().await.clone().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    // Retrieve or compute raw slice
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

    // Render data png bytes using LUT
    let png_bytes = render_data_png_bytes(&raw_slice, &state.projection_lut);

    // Cache results
    state
        .data_cache
        .insert((ens_str, time), png_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/png")
        .header("Cache-Control", "public, max-age=300")
        .body(axum::body::Body::from(png_bytes))
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

/// Fetches the latest NetCDF filename from the KNMI listing endpoint and downloads it.
async fn fetch_latest_nc_file(dest_dir: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .map_err(|_| "KNMI_OPEN_DATA_API_KEY environment variable missing")?;

    let client = reqwest::Client::new();
    
    // 1. Query the list endpoint to find the latest file
    let list_url = "https://api.dataplatform.knmi.nl/open-data/v1/datasets/seamless_precipitation_ensemble_forecast_members/versions/1.0/files?maxKeys=1&sorting=desc";
    let list_res = client
        .get(list_url)
        .header("Authorization", &api_key)
        .send()
        .await
        .map_err(|e| format!("Failed to send file listing request: {}", e))?;

    if !list_res.status().is_success() {
        return Err(format!("Failed to list files, HTTP status: {}", list_res.status()).into());
    }

    #[derive(Deserialize)]
    struct KnmiFileEntry {
        filename: String,
    }

    #[derive(Deserialize)]
    struct KnmiListResponse {
        files: Vec<KnmiFileEntry>,
    }

    let list_data: KnmiListResponse = list_res
        .json()
        .await
        .map_err(|e| format!("Failed to parse file listing JSON: {}", e))?;
    let entry = list_data.files.first().ok_or("No files returned by KNMI API")?;
    let filename = &entry.filename;

    println!("Latest file on KNMI API: {}", filename);

    // 2. Request download URL for this file
    let url_endpoint = format!(
        "https://api.dataplatform.knmi.nl/open-data/v1/datasets/{}/versions/1.0/files/{}/url",
        KNMI_DATASET, filename
    );
    let url_res = client
        .get(&url_endpoint)
        .header("Authorization", &api_key)
        .send()
        .await
        .map_err(|e| format!("Failed to request download URL: {}", e))?;

    if !url_res.status().is_success() {
        return Err(format!("Failed to get download URL, HTTP status: {}", url_res.status()).into());
    }

    #[derive(Deserialize)]
    struct FileUrlResponse {
        #[serde(rename = "temporaryDownloadUrl")]
        temporary_download_url: String,
    }

    let url_resp: FileUrlResponse = url_res
        .json()
        .await
        .map_err(|e| format!("Failed to parse download URL JSON: {}", e))?;
    let download_url = url_resp.temporary_download_url;

    // 3. Download and save the file
    println!("Downloading file: {}...", filename);
    let file_res = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to send download request: {}", e))?;
    if !file_res.status().is_success() {
        return Err(format!("Failed to download file content, HTTP status: {}", file_res.status()).into());
    }

    let bytes = file_res
        .bytes()
        .await
        .map_err(|e| format!("Failed to read file bytes: {}", e))?;
    let temp_path = format!("{}/temp_knmi_download.nc", dest_dir);
    tokio::fs::write(&temp_path, &bytes)
        .await
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    let final_path = format!("{}/{}", dest_dir, filename);
    tokio::fs::rename(&temp_path, &final_path)
        .await
        .map_err(|e| format!("Failed to rename final file: {}", e))?;
    println!("Successfully downloaded and saved initial file: {}", final_path);

    Ok(final_path)
}
