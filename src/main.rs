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
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use rumqttc::{AsyncClient, MqttOptions, Transport, TlsConfiguration, Event, Packet, QoS};

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

struct AppState {
    file_path: RwLock<String>,
    empty_tile: Vec<u8>,
    tile_cache: DashMap<(String, i64, u32, u32, u32), Vec<u8>>, // Key: (ens, time, z, x, y)
    grid_cache: DashMap<(String, i64), Arc<Vec<u16>>>, // Key: (ens, time), value: raw grid slice
    metadata: RwLock<Option<Metadata>>,
}

#[derive(Deserialize)]
struct ValueQuery {
    ens: String,
    time: i64,
    lat: f64,
    lon: f64,
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();
    println!("Starting Weather Radar service...");

    // 1. Find the latest netcdf file in the current directory
    let initial_file = find_latest_nc_file(".").expect("No NetCDF (.nc) files found in workspace root!");
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
    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
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
                // Check if file is actually different or modified
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
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Webservice running on http://localhost:8080");
    axum::serve(listener, app).await.unwrap();
}

/// Scans the directory for the latest modified .nc file
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

/// Load dimensions and attributes from the NetCDF file
fn load_metadata(file_path: &str) -> Result<Metadata, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let ens_var = file.variable("ens_number").ok_or("ens_number variable not found")?;
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

/// Generates a 256x256 transparent WebP to serve for empty/out-of-bounds tiles
fn generate_empty_tile() -> Vec<u8> {
    use image::{ImageBuffer, ImageFormat};
    use std::io::Cursor;
    let img = ImageBuffer::from_pixel(256, 256, image::Rgba([0_u8, 0_u8, 0_u8, 0_u8]));
    let mut webp_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut webp_bytes);
    img.write_to(&mut cursor, ImageFormat::WebP).unwrap();
    webp_bytes
}

// API: Metadata
async fn get_metadata(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let meta = state.metadata.read().await.clone();
    match meta {
        Some(m) => Ok(axum::Json(m)),
        None => Err((StatusCode::INTERNAL_SERVER_ERROR, "Metadata not loaded".to_string())),
    }
}

/// Bilinear interpolation helper for raw data values
fn interpolate_bilinear(fx: f64, fy: f64, raw_slice: &[u16]) -> u16 {
    let ix1 = fx.floor() as i32;
    let iy1 = fy.floor() as i32;
    let ix2 = ix1 + 1;
    let iy2 = iy1 + 1;

    if ix1 < -1 || ix1 >= KNMI_GRID_W as i32 || iy1 < -1 || iy1 >= KNMI_GRID_H as i32 {
        return 65535;
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
            if val != 65535 {
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
        65535
    }
}

/// Helper to compute or load the raw grid slice (doing ensembles statistics if necessary)
fn compute_raw_slice(
    file_path: &str,
    meta: &Metadata,
    ens_str: &str,
    time: i64,
) -> Result<Vec<u16>, (StatusCode, String)> {
    let mut raw_slice = vec![65535_u16; KNMI_GRID_H * KNMI_GRID_W];

    if ens_str == "med" || ens_str == "max" || ens_str == "prob" {
        // Read all 20 members
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
        for i in 0..grid_size {
            if member_slices[0][i] == 65535 {
                raw_slice[i] = 65535;
                continue;
            }

            if ens_str == "max" {
                let mut max_val = 0;
                for slice in &member_slices {
                    let val = slice[i];
                    if val != 65535 && val > max_val {
                        max_val = val;
                    }
                }
                raw_slice[i] = max_val;
            } else if ens_str == "prob" {
                let mut count = 0;
                for slice in &member_slices {
                    let val = slice[i];
                    if val != 65535 && val >= 10 {
                        count += 1;
                    }
                }
                raw_slice[i] = ((count * 100) / member_slices.len()) as u16;
            } else { // "med"
                let mut vals = vec![0; member_slices.len()];
                for e in 0..member_slices.len() {
                    vals[e] = member_slices[e][i];
                }
                vals.sort_unstable();
                raw_slice[i] = vals[vals.len() / 2];
            }
        }
    } else {
        // Individual member
        let ens_num: i32 = ens_str.parse().map_err(|_| {
            (StatusCode::BAD_REQUEST, format!("Invalid ensemble parameter: {}", ens_str))
        })?;

        let ens_idx = meta
            .ensembles
            .iter()
            .position(|&e| e == ens_num)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid ensemble number: {}", ens_num)))?;

        let time_idx = meta
            .times
            .iter()
            .position(|&t| t == time)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", time)))?;

        raw_slice = read_netcdf_slice(file_path, ens_idx, time_idx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error reading slice: {}", e),
            )
        })?;
    }

    Ok(raw_slice)
}

// API: Serves Web Mercator projected colored WebP overlay tile dynamically
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
    let max_px = ps_coords.iter().map(|c| c.0).fold(f64::NEG_INFINITY, f64::max);
    let min_py = ps_coords.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
    let max_py = ps_coords.iter().map(|c| c.1).fold(f64::NEG_INFINITY, f64::max);

    // Bounding Box of KNMI grid in Polar Stereographic
    const KNMI_X_MIN: f64 = 500.0 - 50000.0;
    const KNMI_X_MAX: f64 = 700500.0 + 50000.0;
    const KNMI_Y_MIN: f64 = -4415495.4 - 50000.0;
    const KNMI_Y_MAX: f64 = -3650495.4 + 50000.0;

    let overlap = max_px >= KNMI_X_MIN && min_px <= KNMI_X_MAX &&
                  max_py >= KNMI_Y_MIN && min_py <= KNMI_Y_MAX;

    if !overlap {
        // Return transparent tile
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
        state.grid_cache.insert((ens_str.clone(), time), arc.clone());
        arc
    };

    // Render tile image dynamically using WebP
    let webp_bytes = render_tile_webp_bytes(left, right, top, bottom, &raw_slice, is_prob);

    // Cache results
    state.tile_cache.insert((ens_str, time, z, x, y), webp_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "public, max-age=300")
        .body(axum::body::Body::from(webp_bytes))
        .unwrap())
}

// API: Query value at point
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
        return Ok(axum::Json(HashMap::from([
            ("status".to_string(), serde_json::Value::String("out_of_bounds".to_string())),
            ("value".to_string(), serde_json::Value::Null),
        ])));
    }

    // Read value based on query type
    let file = netcdf::open(&file_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let var = file
        .variable("precip_intensity")
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "precip_intensity variable missing".to_string()))?;

    let val_out: serde_json::Value;
    let status_out: String;

    if q.ens == "med" || q.ens == "max" || q.ens == "prob" {
        // Read value at target cell across all members
        let mut vals = Vec::with_capacity(meta.ensembles.len());
        for (ens_idx, _) in meta.ensembles.iter().enumerate() {
            let time_idx = meta.times.iter().position(|&t| t == q.time).ok_or((
                StatusCode::BAD_REQUEST,
                format!("Invalid time: {}", q.time),
            ))?;
            let val_raw: u16 = var
                .get_value((ens_idx, time_idx, iy as usize, ix as usize))
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            vals.push(val_raw);
        }

        if vals[0] == 65535 {
            status_out = "no_rain".to_string();
            val_out = serde_json::Value::Number(serde_json::Number::from_f64(0.0).unwrap());
        } else if q.ens == "max" {
            let max_raw = vals.iter().cloned().filter(|&v| v != 65535).max().unwrap_or(0);
            let val_mmh = (max_raw as f64) * 0.01;
            status_out = if val_mmh > 0.0 { "ok".to_string() } else { "no_rain".to_string() };
            val_out = serde_json::Value::Number(serde_json::Number::from_f64(val_mmh).unwrap());
        } else if q.ens == "prob" {
            let count = vals.iter().cloned().filter(|&v| v != 65535 && v >= 10).count();
            let prob = (count * 100) / vals.len();
            status_out = "probability".to_string();
            val_out = serde_json::Value::Number(serde_json::Number::from_f64(prob as f64).unwrap());
        } else { // "med"
            vals.sort_unstable();
            let med_raw = vals[vals.len() / 2];
            let val_mmh = (med_raw as f64) * 0.01;
            status_out = if val_mmh > 0.0 { "ok".to_string() } else { "no_rain".to_string() };
            val_out = serde_json::Value::Number(serde_json::Number::from_f64(val_mmh).unwrap());
        }
    } else {
        // Individual member
        let ens_num: i32 = q.ens.parse().map_err(|_| {
            (StatusCode::BAD_REQUEST, format!("Invalid ensemble parameter: {}", q.ens))
        })?;

        let ens_idx = meta
            .ensembles
            .iter()
            .position(|&e| e == ens_num)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid ensemble: {}", ens_num)))?;

        let time_idx = meta
            .times
            .iter()
            .position(|&t| t == q.time)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", q.time)))?;

        let val_raw: u16 = var
            .get_value((ens_idx, time_idx, iy as usize, ix as usize))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if val_raw == 65535 {
            status_out = "no_rain".to_string();
            val_out = serde_json::Value::Number(serde_json::Number::from_f64(0.0).unwrap());
        } else {
            let value = (val_raw as f64) * 0.01;
            status_out = if value > 0.0 { "ok".to_string() } else { "no_rain".to_string() };
            val_out = serde_json::Value::Number(serde_json::Number::from_f64(value).unwrap());
        }
    }

    Ok(axum::Json(HashMap::from([
        ("status".to_string(), serde_json::Value::String(status_out)),
        ("value".to_string(), val_out),
    ])))
}

#[derive(Deserialize)]
struct TimeseriesQuery {
    ens: String,
    lat: f64,
    lon: f64,
}

#[derive(Serialize)]
struct TimeseriesResponse {
    status: String,
    lat: f64,
    lon: f64,
    ens: String,
    times: Vec<i64>,
    values: Vec<f64>,
}

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

    let file = netcdf::open(&file_path).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let var = file
        .variable("precip_intensity")
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "precip_intensity variable missing".to_string()))?;

    let num_times = meta.times.len();
    let num_ensembles = meta.ensembles.len();
    let mut values = Vec::with_capacity(num_times);

    if q.ens == "med" || q.ens == "max" || q.ens == "prob" {
        // Read values for all ensembles and all times at the target pixel
        // slice starts at: [0, 0, iy, ix], count is: [num_ensembles, num_times, 1, 1]
        let raw_grid = var.get_values::<u16, _>((
            &[0, 0, iy as usize, ix as usize][..],
            &[num_ensembles, num_times, 1, 1][..]
        )).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        for t in 0..num_times {
            let mut member_vals = Vec::with_capacity(num_ensembles);
            for e in 0..num_ensembles {
                // Row-major index for shape [num_ensembles, num_times]
                let idx = e * num_times + t;
                member_vals.push(raw_grid[idx]);
            }

            if member_vals[0] == 65535 {
                values.push(0.0);
                continue;
            }

            if q.ens == "max" {
                let max_raw = member_vals.iter().cloned().filter(|&v| v != 65535).max().unwrap_or(0);
                values.push((max_raw as f64) * 0.01);
            } else if q.ens == "prob" {
                let count = member_vals.iter().cloned().filter(|&v| v != 65535 && v >= 10).count();
                let prob = ((count * 100) / num_ensembles) as f64;
                values.push(prob);
            } else { // "med"
                member_vals.sort_unstable();
                let med_raw = member_vals[num_ensembles / 2];
                if med_raw == 65535 {
                    values.push(0.0);
                } else {
                    values.push((med_raw as f64) * 0.01);
                }
            }
        }
    } else {
        // Individual member
        let ens_num: i32 = q.ens.parse().map_err(|_| {
            (StatusCode::BAD_REQUEST, format!("Invalid ensemble parameter: {}", q.ens))
        })?;

        let ens_idx = meta
            .ensembles
            .iter()
            .position(|&e| e == ens_num)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid ensemble: {}", ens_num)))?;

        // Slice starts at: [ens_idx, 0, iy, ix], count is: [1, num_times, 1, 1]
        let raw_values = var.get_values::<u16, _>((
            &[ens_idx, 0, iy as usize, ix as usize][..],
            &[1, num_times, 1, 1][..]
        )).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        for val_raw in raw_values {
            if val_raw == 65535 {
                values.push(0.0);
            } else {
                values.push((val_raw as f64) * 0.01);
            }
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

/// Reads a 2D slice (y, x) for a given ensemble and time index from NetCDF
fn read_netcdf_slice(
    file_path: &str,
    ens_idx: usize,
    time_idx: usize,
) -> Result<Vec<u16>, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let var = file
        .variable("precip_intensity")
        .ok_or("precip_intensity variable not found")?;

    // Slicing parameters: start = [ens_idx, time_idx, 0, 0], count = [1, 1, 765, 700]
    let slice = var.get_values::<u16, _>((
        &[ens_idx, time_idx, 0, 0][..],
        &[1, 1, KNMI_GRID_H, KNMI_GRID_W][..],
    ))?;
    Ok(slice)
}

struct ColorAnchor {
    value: f64,
    color: [u8; 4],
}

/// Maps raw uint16 to interpolated RGBA color
fn get_color(val_raw: u16) -> [u8; 4] {
    if val_raw == 65535 {
        return [0, 0, 0, 0]; // Transparent
    }
    let val = (val_raw as f64) * 0.01; // Scale factor 0.01 mm/h
    if val < 0.05 {
        return [0, 0, 0, 0]; // Ignore extremely light values (noise)
    }

    let anchors = [
        ColorAnchor { value: 0.05, color: [120, 200, 255, 120] }, // Very light blue
        ColorAnchor { value: 0.2,  color: [0, 100, 255, 170] },   // Blue
        ColorAnchor { value: 1.0,  color: [0, 200, 0, 190] },     // Green
        ColorAnchor { value: 5.0,  color: [255, 230, 0, 210] },   // Yellow
        ColorAnchor { value: 15.0, color: [255, 120, 0, 230] },   // Orange
        ColorAnchor { value: 30.0, color: [255, 0, 0, 240] },     // Red
        ColorAnchor { value: 100.0, color: [200, 0, 200, 255] },  // Purple
        ColorAnchor { value: 250.0, color: [255, 255, 255, 255] }, // White
    ];

    if val <= anchors[0].value {
        return anchors[0].color;
    }
    if val >= anchors[anchors.len() - 1].value {
        return anchors[anchors.len() - 1].color;
    }

    // Find the interpolation interval
    for i in 0..anchors.len() - 1 {
        let a1 = &anchors[i];
        let a2 = &anchors[i + 1];
        if val >= a1.value && val <= a2.value {
            let t = (val - a1.value) / (a2.value - a1.value);
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



/// Maps probability percent (0-100) to color
fn get_probability_color(p: u16) -> [u8; 4] {
    if p == 65535 || p < 10 {
        return [0, 0, 0, 0]; // Transparent
    }

    let anchors = [
        ColorAnchor { value: 10.0,  color: [180, 200, 220, 80] },   // Very light grey-blue (10% chance)
        ColorAnchor { value: 30.0,  color: [100, 160, 255, 120] },  // Light blue (30% chance)
        ColorAnchor { value: 50.0,  color: [0, 100, 255, 160] },    // Blue (50% chance)
        ColorAnchor { value: 70.0,  color: [0, 200, 100, 180] },    // Teal (70% chance)
        ColorAnchor { value: 90.0,  color: [220, 0, 220, 220] },    // Magenta-purple (90% chance)
        ColorAnchor { value: 100.0, color: [255, 255, 255, 240] },  // White (100% chance)
    ];

    if p <= anchors[0].value as u16 {
        return anchors[0].color;
    }
    if p >= anchors[anchors.len() - 1].value as u16 {
        return anchors[anchors.len() - 1].color;
    }

    // Find interpolation interval
    for i in 0..anchors.len() - 1 {
        let a1 = &anchors[i];
        let a2 = &anchors[i + 1];
        let p_f = p as f64;
        if p_f >= a1.value && p_f <= a2.value {
            let t = (p_f - a1.value) / (a2.value - a1.value);
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

/// Helper to render tile image bytes
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

async fn start_knmi_mqtt_listener(state: Arc<AppState>) {
    let broker = "wss://mqtt.dataplatform.knmi.nl";
    let port = 443;
    let mqtt_password = std::env::var("KNMI_MQTT_PASSWORD")
        .expect("KNMI_MQTT_PASSWORD environment variable not set!");
    let open_data_api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .expect("KNMI_OPEN_DATA_API_KEY environment variable not set!");
    let topic = "dataplatform/file/v1/seamless_precipitation_ensemble_forecast_members/1.0/#";

    let client_id = format!("weer-service-{}", std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs());

    println!("Initializing KNMI MQTT subscriber with Client ID: {}...", client_id);

    let mut mqttoptions = MqttOptions::new(client_id, broker, port);
    mqttoptions.set_keep_alive(Duration::from_secs(30));
    mqttoptions.set_credentials("token", mqtt_password);

    let tls_config = TlsConfiguration::default();
    mqttoptions.set_transport(Transport::wss_with_config(tls_config));

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 50);

    // Subscribe to topic
    if let Err(e) = client.subscribe(topic, QoS::AtMostOnce).await {
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
                            .and_then(|d| d.get("filename").or_else(|| d.get("fileName")).or_else(|| d.get("file_name")))
                            .and_then(|v| v.as_str())
                            .or_else(|| json.get("fileName").or_else(|| json.get("file_name")).and_then(|v| v.as_str()));

                        let file_url = data
                            .and_then(|d| d.get("url"))
                            .and_then(|v| v.as_str());

                        if let Some(name) = file_name {
                            if name.ends_with(".nc") {
                                println!("New NetCDF file available: {}", name);
                                let state_clone = state.clone();
                                let name_clone = name.to_string();
                                let url_opt = file_url.map(|s| s.to_string());
                                let open_data_api_key_clone = open_data_api_key.to_string();
                                tokio::spawn(async move {
                                    if let Err(e) = download_and_update_nc_file(&name_clone, url_opt.as_deref(), &open_data_api_key_clone, state_clone).await {
                                        eprintln!("Error processing file update for {}: {:?}", name_clone, e);
                                    }
                                });
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("MQTT Connection error: {:?}. Retrying in 10 seconds...", e);
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

#[derive(Deserialize)]
struct FileUrlResponse {
    #[serde(rename = "temporaryDownloadUrl")]
    temporary_download_url: String,
}

async fn download_and_update_nc_file(
    filename: &str,
    file_url: Option<&str>,
    api_key: &str,
    _state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Requesting download URL for {} from KNMI Open Data API...", filename);
    
    let url = match file_url {
        Some(u) => u.to_string(),
        None => format!(
            "https://api.dataplatform.knmi.nl/open-data/v1/datasets/seamless_precipitation_ensemble_forecast_members/versions/1.0/files/{}/url",
            filename
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
        return Err(format!("Failed to download file content, HTTP status: {}", file_res.status()).into());
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
                            if file_stem != filename && file_stem.starts_with("KNMI_PYSTEPS_BLEND_ENS_") {
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




