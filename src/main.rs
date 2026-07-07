//! Real-time precipitation radar tile server using KNMI ensemble forecast data.
//!
//! This service ingests NetCDF files from the KNMI seamless precipitation ensemble
//! forecast, projects the Polar Stereographic grid onto Web Mercator tiles, and
//! serves them as WebP images via an HTTP API. It also exposes point-query and
//! time-series endpoints for individual grid cells.

mod constants;
mod handlers;
mod harmonie;
mod interpolation;
mod models;
mod mqtt;
mod projection;
mod radar;
mod rendering;
mod state;

use axum::{routing::get, Router};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use handlers::*;
use harmonie::{
    cleanup_tar_files, load_or_fetch_combined_forecast, precalculate_rain_data,
    precalculate_solar_data, precalculate_temp_data, precalculate_wind_data,
};
use interpolation::{init_projection_lut, init_temp_projection_lut};
use mqtt::{start_knmi_harmonie_mqtt_listener, start_knmi_mqtt_listener};
use radar::{fetch_latest_nc_file, find_latest_nc_file, load_metadata, precalculate_all_data};
use state::AppState;

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();
    println!("Starting Weather Radar service...");

    let open_data_api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .expect("KNMI_OPEN_DATA_API_KEY environment variable not set!");

    // Create cache directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(constants::CACHE_DIR) {
        eprintln!(
            "Failed to create cache directory '{}': {:?}",
            constants::CACHE_DIR,
            e
        );
    }

    // Clean up leftover tar files on startup
    cleanup_tar_files();

    // Load or fetch temperature, wind, solar, and rain forecasts (combined)
    let (temp_fc, wind_fc, solar_fc, rain_fc) =
        load_or_fetch_combined_forecast(&open_data_api_key).await;

    // 1. Find the latest netcdf file in the cache directory, or download it if none exists
    let initial_file = match find_latest_nc_file(constants::CACHE_DIR) {
        Some(f) => f,
        None => {
            println!("No NetCDF (.nc) files found in cache directory. Fetching latest file from KNMI Open Data API...");
            match fetch_latest_nc_file(constants::CACHE_DIR).await {
                Ok(f) => f,
                Err(e) => {
                    panic!("Failed to download initial NetCDF file on startup: {}", e);
                }
            }
        }
    };
    println!("Found initial NetCDF file: {}", initial_file);

    // 2. Load initial metadata
    let metadata_val = match load_metadata(&initial_file).await {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("Error loading metadata from {}: {}", initial_file, e);
            None
        }
    };

    let state = Arc::new(AppState {
        file_path: tokio::sync::RwLock::new(initial_file.clone()),
        grid_cache: dashmap::DashMap::new(),
        data_cache: dashmap::DashMap::new(),
        metadata: tokio::sync::RwLock::new(metadata_val.clone()),
        projection_lut: init_projection_lut(),

        temp_forecast: tokio::sync::RwLock::new(Some(temp_fc)),
        temp_projection_lut: init_temp_projection_lut(),
        temp_data_cache: dashmap::DashMap::new(),

        wind_forecast: tokio::sync::RwLock::new(Some(wind_fc)),
        wind_projection_lut: init_temp_projection_lut(),
        wind_data_cache: dashmap::DashMap::new(),

        solar_forecast: tokio::sync::RwLock::new(Some(solar_fc)),
        solar_projection_lut: init_temp_projection_lut(),
        solar_data_cache: dashmap::DashMap::new(),

        rain_forecast: tokio::sync::RwLock::new(Some(rain_fc)),
        timeseries_cache: dashmap::DashMap::new(),
    });

    if let Some(ref meta) = metadata_val {
        let state_clone = state.clone();
        let meta_clone = meta.clone();
        tokio::spawn(async move {
            precalculate_all_data(state_clone, meta_clone).await;
        });
    }

    // Precalculate temperature PNGs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_temp_data(state_clone).await;
        });
    }

    // Precalculate wind PNGs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_wind_data(state_clone).await;
        });
    }

    // Precalculate solar PNGs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_solar_data(state_clone).await;
        });
    }

    // Precalculate rain PNGs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_rain_data(state_clone).await;
        });
    }

    // Spawn MQTT client to listen for radar updates from KNMI
    let state_clone_mqtt = state.clone();
    tokio::spawn(async move {
        start_knmi_mqtt_listener(state_clone_mqtt).await;
    });

    // Spawn MQTT client to listen for HARMONIE updates from KNMI (combined temp and wind)
    let state_clone_harmonie_mqtt = state.clone();
    tokio::spawn(async move {
        start_knmi_harmonie_mqtt_listener(state_clone_harmonie_mqtt).await;
    });

    // Note: State reloads are now triggered directly in-memory upon successful download completion
    // in download_and_update_nc_file.

    // 5. Configure Router
    let app = Router::new()
        .route("/favicon.ico", get(favicon))
        .route("/api/metadata", get(get_metadata))
        .route("/api/data/{ens}/{time}", get(get_data_image))
        .route("/api/value", get(get_value))
        .route("/api/timeseries", get(get_timeseries))
        .route("/api/metadata/temp", get(get_temp_metadata))
        .route("/api/data/temp/{time}", get(get_temp_data_image))
        .route("/api/value/temp", get(get_temp_value))
        .route("/api/timeseries/temp", get(get_temp_timeseries))
        .route("/api/metadata/wind", get(get_wind_metadata))
        .route("/api/data/wind/{time}", get(get_wind_data_image_legacy))
        .route("/api/data/wind/{height}/{time}", get(get_wind_data_image))
        .route("/api/value/wind", get(get_wind_value))
        .route("/api/timeseries/wind", get(get_wind_timeseries))
        .route("/api/metadata/solar", get(get_solar_metadata))
        .route("/api/data/solar/{time}", get(get_solar_data_image))
        .route("/api/value/solar", get(get_solar_value))
        .route("/api/timeseries/solar", get(get_solar_timeseries))
        .fallback_service(ServeDir::new("static"))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([axum::http::Method::GET]),
        )
        .with_state(state);

    // 6. Start Server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Webservice running on http://localhost:8080");
    axum::serve(listener, app).await.unwrap();
}
