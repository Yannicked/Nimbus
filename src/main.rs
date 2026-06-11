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
}

struct AppState {
    file_path: RwLock<String>,
    lookup_table: Vec<i32>,
    png_cache: DashMap<(String, i64), Vec<u8>>, // Key: (ensemble_or_stat, time_seconds)
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
    println!("Starting Weather Radar service...");

    // 1. Find the latest netcdf file in the current directory
    let initial_file = find_latest_nc_file(".").expect("No NetCDF (.nc) files found in workspace root!");
    println!("Found initial NetCDF file: {}", initial_file);

    // 2. Precompute the Web Mercator to NetCDF coordinate lookup table
    println!("Precomputing coordinate mapping table ({}x{})...", GRID_W, GRID_H);
    let lookup_table = precompute_lookup_table();
    println!("Coordinate mapping table initialized.");

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
        lookup_table,
        png_cache: DashMap::new(),
        metadata: RwLock::new(metadata_val),
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
                            *meta_write = Some(meta);

                            state_clone.png_cache.clear();
                            println!("Successfully reloaded metadata and cleared PNG cache.");
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
        .route("/api/map/:ens/:time", get(get_map))
        .route("/api/value", get(get_value))
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
    })
}

/// Precompute mapping from Web Mercator pixels to NetCDF grid array indices
fn precompute_lookup_table() -> Vec<i32> {
    let mut table = vec![-1; (GRID_W * GRID_H) as usize];

    for row in 0..GRID_H {
        for col in 0..GRID_W {
            // 1. Get Web Mercator coordinates for pixel center
            let x_merc = MERCATOR_LEFT + (col as f64 + 0.5) * (MERCATOR_RIGHT - MERCATOR_LEFT) / (GRID_W as f64);
            let y_merc = MERCATOR_TOP - (row as f64 + 0.5) * (MERCATOR_TOP - MERCATOR_BOTTOM) / (GRID_H as f64);

            // 2. Convert to GPS
            let (lon, lat) = projection::mercator_to_lonlat(x_merc, y_merc);

            // 3. Convert to Polar Stereographic
            let (px, py) = projection::lonlat_to_polar_stereographic(lon, lat);

            // 4. Find closest indices in the KNMI grid
            let ix = ((px - KNMI_X0) / KNMI_DX).round() as i32;
            let iy = ((py - KNMI_Y0) / KNMI_DY).round() as i32;

            if ix >= 0 && ix < KNMI_GRID_W as i32 && iy >= 0 && iy < KNMI_GRID_H as i32 {
                let index = iy * (KNMI_GRID_W as i32) + ix;
                table[(row * GRID_W + col) as usize] = index;
            }
        }
    }

    table
}

// API: Metadata
async fn get_metadata(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let meta = state.metadata.read().await.clone();
    match meta {
        Some(m) => Ok(axum::Json(m)),
        None => Err((StatusCode::INTERNAL_SERVER_ERROR, "Metadata not loaded".to_string())),
    }
}

// API: Serves Web Mercator projected colored PNG overlay
async fn get_map(
    Path((ens_str, time)): Path<(String, i64)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check cache
    if let Some(cached_png) = state.png_cache.get(&(ens_str.clone(), time)) {
        return Ok(Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "public, max-age=300")
            .body(axum::body::Body::from(cached_png.value().clone()))
            .unwrap());
    }

    // Get current file path and metadata
    let file_path = state.file_path.read().await.clone();
    let meta = state.metadata.read().await.clone().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    let mut raw_slice = vec![65535_u16; KNMI_GRID_H * KNMI_GRID_W];
    let is_prob = ens_str == "prob";

    if ens_str == "med" || ens_str == "max" || ens_str == "prob" {
        // Read all 20 members
        let mut member_slices = Vec::with_capacity(meta.ensembles.len());
        for &ens_val in &meta.ensembles {
            let ens_idx = meta.ensembles.iter().position(|&e| e == ens_val).unwrap();
            let time_idx = meta.times.iter().position(|&t| t == time).ok_or((
                StatusCode::BAD_REQUEST,
                format!("Invalid time: {}", time),
            ))?;
            let slice = read_netcdf_slice(&file_path, ens_idx, time_idx).map_err(|e| {
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
            // Check if cell is fill value (out of bounds)
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
                // Percentage of members with rain >= 0.1 mm/h (raw value >= 10)
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

        raw_slice = read_netcdf_slice(&file_path, ens_idx, time_idx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error reading slice: {}", e),
            )
        })?;
    }

    // Reproject to Web Mercator in-memory using the lookup table
    let mut projected_grid = vec![65535_u16; (GRID_W * GRID_H) as usize];
    for i in 0..projected_grid.len() {
        let src_idx = state.lookup_table[i];
        if src_idx >= 0 {
            projected_grid[i] = raw_slice[src_idx as usize];
        }
    }

    // Render PNG image
    let png_bytes = if is_prob {
        render_probability_png(&projected_grid)
    } else {
        render_png(&projected_grid)
    };

    // Cache results
    state.png_cache.insert((ens_str, time), png_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/png")
        .header("Cache-Control", "public, max-age=300")
        .body(axum::body::Body::from(png_bytes))
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

/// Renders raw uint16 grid to PNG bytes
fn render_png(grid: &[u16]) -> Vec<u8> {
    use image::{ImageBuffer, ImageFormat};
    use std::io::Cursor;

    let mut img = ImageBuffer::new(GRID_W, GRID_H);
    for (i, &val) in grid.iter().enumerate() {
        let x = (i as u32) % GRID_W;
        let y = (i as u32) / GRID_W;
        let color = get_color(val);
        img.put_pixel(x, y, image::Rgba(color));
    }

    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, ImageFormat::Png).unwrap();
    png_bytes
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

/// Renders probability percent grid to PNG bytes
fn render_probability_png(grid: &[u16]) -> Vec<u8> {
    use image::{ImageBuffer, ImageFormat};
    use std::io::Cursor;

    let mut img = ImageBuffer::new(GRID_W, GRID_H);
    for (i, &val) in grid.iter().enumerate() {
        let x = (i as u32) % GRID_W;
        let y = (i as u32) / GRID_W;
        let color = get_probability_color(val);
        img.put_pixel(x, y, image::Rgba(color));
    }

    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    img.write_to(&mut cursor, ImageFormat::Png).unwrap();
    png_bytes
}
