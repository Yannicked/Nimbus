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
mod rtcor;
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
use mqtt::{
    start_knmi_harmonie_mqtt_listener, start_knmi_mqtt_listener, start_knmi_rtcor_mqtt_listener,
};
use radar::{fetch_latest_nc_file, find_latest_nc_file, load_metadata, precalculate_all_data};
use state::AppState;

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();
    println!("Starting Weather Radar service...");

    let open_data_api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .expect("KNMI_OPEN_DATA_API_KEY environment variable not set!");
    let mqtt_password = std::env::var("KNMI_MQTT_PASSWORD")
        .expect("KNMI_MQTT_PASSWORD environment variable not set!");
    drop(mqtt_password);

    // Create cache directory if it doesn't exist
    if let Err(e) = tokio::fs::create_dir_all(constants::CACHE_DIR).await {
        eprintln!(
            "Failed to create cache directory '{}': {:?}",
            constants::CACHE_DIR,
            e
        );
    }

    // Clean up leftover tar files on startup
    cleanup_tar_files().await;

    // Load or fetch temperature, wind, solar, and rain forecasts (combined)
    let (temp_fc, wind_fc, solar_fc, rain_fc) =
        load_or_fetch_combined_forecast(&open_data_api_key).await;

    // 1. Find the latest netcdf file in the cache directory, or download it if none exists
    let initial_file = match find_latest_nc_file(constants::CACHE_DIR).await {
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

    let initial_radar_data =
        metadata_val.map(|m| Arc::new(state::RadarData::new(initial_file.clone(), m)));
    let initial_temp_data = Arc::new(state::TempData::new(temp_fc));
    let initial_wind_data = Arc::new(state::WindData::new(wind_fc));
    let initial_solar_data = Arc::new(state::SolarData::new(solar_fc));
    let initial_rain_data = Arc::new(state::RainData::new(rain_fc));

    let projection_lut = Arc::new(init_projection_lut());
    let grib_lut = Arc::new(init_temp_projection_lut());

    let state = Arc::new(AppState {
        radar_data: tokio::sync::RwLock::new(initial_radar_data.clone()),
        projection_lut: projection_lut.clone(),

        actuals_data: tokio::sync::RwLock::new(None),

        temp_data: tokio::sync::RwLock::new(Some(initial_temp_data.clone())),
        temp_projection_lut: grib_lut.clone(),

        wind_data: tokio::sync::RwLock::new(Some(initial_wind_data.clone())),
        wind_projection_lut: grib_lut.clone(),

        solar_data: tokio::sync::RwLock::new(Some(initial_solar_data.clone())),
        solar_projection_lut: grib_lut,

        rain_data: tokio::sync::RwLock::new(Some(initial_rain_data.clone())),
    });

    if let Some(radar_data) = initial_radar_data {
        let lut_arc = projection_lut.clone();
        tokio::spawn(async move {
            precalculate_all_data(radar_data, lut_arc, None).await;
        });
    }

    // Backfill recent 5-minute radar actuals on startup
    {
        let state_clone = state.clone();
        let api_key_clone = open_data_api_key.clone();
        tokio::spawn(async move {
            rtcor::backfill_recent_rtcor_frames(state_clone, &api_key_clone).await;
        });
    }

    // Precalculate temperature WebPs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_temp_data(state_clone).await;
        });
    }

    // Precalculate wind WebPs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_wind_data(state_clone).await;
        });
    }

    // Precalculate solar WebPs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_solar_data(state_clone).await;
        });
    }

    // Precalculate rain WebPs in background
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            precalculate_rain_data(state_clone).await;
        });
    }

    // Spawn MQTT client to listen for radar forecast updates from KNMI
    let state_clone_mqtt = state.clone();
    tokio::spawn(async move {
        start_knmi_mqtt_listener(state_clone_mqtt).await;
    });

    // Spawn MQTT client to listen for real-time radar actuals from KNMI (RTCOR)
    let state_clone_rtcor_mqtt = state.clone();
    tokio::spawn(async move {
        start_knmi_rtcor_mqtt_listener(state_clone_rtcor_mqtt).await;
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
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let bind_addr = format!("{}:{}", host, port);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("Failed to bind server to {}: {}", bind_addr, e));
    let display_host = if host == "0.0.0.0" {
        "localhost"
    } else {
        &host
    };
    println!("Webservice running on http://{}:{}", display_host, port);
    axum::serve(listener, app).await.unwrap();
}
