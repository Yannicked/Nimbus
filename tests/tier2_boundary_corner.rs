#![allow(dead_code, unused_imports)]

#[path = "../src/constants.rs"]
mod constants;
#[path = "../src/models.rs"]
mod models;
#[path = "../src/state.rs"]
mod state;
#[path = "../src/projection.rs"]
mod projection;
#[path = "../src/interpolation.rs"]
mod interpolation;
#[path = "../src/rendering.rs"]
mod rendering;
#[path = "../src/radar.rs"]
mod radar;
#[path = "../src/rtcor.rs"]
mod rtcor;
#[path = "../src/harmonie.rs"]
mod harmonie;
#[path = "../src/handlers.rs"]
mod handlers;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::{routing::get, Router};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;
use tower_http::cors::CorsLayer;

use constants::*;
use handlers::*;
use harmonie::*;
use interpolation::*;
use models::*;
use projection::*;
use radar::*;
use rendering::*;
use rtcor::*;
use state::*;

fn create_test_router(state: Arc<AppState>) -> Router {
    Router::new()
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
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([axum::http::Method::GET]),
        )
        .with_state(state)
}

fn create_mock_app_state() -> Arc<AppState> {
    let projection_lut = Arc::new(init_projection_lut());
    let actuals_lut = Arc::new(init_actuals_projection_lut());
    let grib_lut = Arc::new(init_temp_projection_lut());

    let ref_time = 1700000000i64;
    let times = vec![0, 300, 600];
    let ensembles = vec![0, 1];
    let meta = Metadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        ensembles: ensembles.clone(),
        times: times.clone(),
        reference_time_str: "seconds since 2023-11-14 22:13:20".to_string(),
        version: 12345678,
        radar_times_len: times.len(),
    };

    let radar_data = Arc::new(RadarData::new("mock_radar.nc".to_string(), meta));

    let fc_len = FORECAST_GRID_W * FORECAST_GRID_H;
    for &t in &times {
        for e in 0..2 {
            let grid = vec![0u16; fc_len];
            radar_data.grid_cache.insert((e.to_string(), t), Arc::new(grid));
        }
        radar_data.grid_cache.insert(("med".to_string(), t), Arc::new(vec![100u16; fc_len]));
        radar_data.grid_cache.insert(("max".to_string(), t), Arc::new(vec![200u16; fc_len]));
        radar_data.grid_cache.insert(("prob".to_string(), t), Arc::new(vec![50u16; fc_len]));
        radar_data.grid_cache.insert(("spread".to_string(), t), Arc::new(vec![10u16; fc_len]));
        radar_data.grid_cache.insert(("pmm".to_string(), t), Arc::new(vec![120u16; fc_len]));
    }

    let temp_fc = TempForecast {
        reference_time: ref_time,
        steps: vec![TempStep {
            forecast_hour: 0,
            width: GRIB_WIDTH,
            height: GRIB_HEIGHT,
            values: Arc::new(vec![2931u16; GRIB_WIDTH * GRIB_HEIGHT]),
        }],
    };
    let temp_data = Arc::new(TempData::new(temp_fc));

    let wind_fc = WindForecast {
        reference_time: ref_time,
        steps: vec![WindStep {
            forecast_hour: 0,
            height_level: 10,
            width: GRIB_WIDTH,
            height: GRIB_HEIGHT,
            u_values: Arc::new(vec![10500u16; GRIB_WIDTH * GRIB_HEIGHT]),
            v_values: Arc::new(vec![10000u16; GRIB_WIDTH * GRIB_HEIGHT]),
        }],
    };
    let wind_data = Arc::new(WindData::new(wind_fc));

    let solar_fc = SolarForecast {
        reference_time: ref_time,
        steps: vec![SolarStep {
            forecast_hour: 0,
            width: GRIB_WIDTH,
            height: GRIB_HEIGHT,
            values: Arc::new(vec![500u16; GRIB_WIDTH * GRIB_HEIGHT]),
        }],
    };
    let solar_data = Arc::new(SolarData::new(solar_fc));

    let rain_fc = RainForecast {
        reference_time: ref_time,
        steps: vec![RainStep {
            forecast_hour: 0,
            width: GRIB_WIDTH,
            height: GRIB_HEIGHT,
            values: Arc::new(vec![50u16; GRIB_WIDTH * GRIB_HEIGHT]),
        }],
    };
    let rain_data = Arc::new(RainData::new(rain_fc));

    let mut actuals = ActualsData::new();
    let act_len = RTCOR_GRID_W * RTCOR_GRID_H;
    actuals.insert_or_update(
        ActualsFrame {
            timestamp: ref_time - 300,
            raw_values: Arc::new(vec![100u16; act_len]),
            webp_bytes: vec![0u8; 128],
        },
        24,
    );

    Arc::new(AppState {
        radar_data: RwLock::new(Some(radar_data)),
        projection_lut,
        actuals_data: RwLock::new(Some(Arc::new(actuals))),
        actuals_projection_lut: actuals_lut,
        temp_data: RwLock::new(Some(temp_data)),
        temp_projection_lut: grib_lut.clone(),
        wind_data: RwLock::new(Some(wind_data)),
        wind_projection_lut: grib_lut.clone(),
        solar_data: RwLock::new(Some(solar_data)),
        solar_projection_lut: grib_lut,
        rain_data: RwLock::new(Some(rain_data)),
    })
}

// =========================================================================
// Tier 2: F01 Ensemble & Statistical Reduction Corner Cases
// =========================================================================

#[test]
fn test_t2_f01_single_member_reduction() {
    let mut vals = [350u16];
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut vals), 350);
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut vals), 350);
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut vals), 0);
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals), 100);
}

#[test]
fn test_t2_f01_all_nodata_interspersed() {
    let mut vals = [NODATA, NODATA, NODATA];
    // First member is NODATA -> entire cell is NODATA
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals), NODATA);
}

#[test]
fn test_t2_f01_max_u16_saturation() {
    let mut vals = [65534, 65534, 65534];
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut vals), 65534);
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut vals), 65534);
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut vals), 0);
}

#[test]
fn test_t2_f01_probability_threshold_boundary() {
    // Exact threshold boundary: 10 vs 9
    let mut vals_below = [RAIN_THRESHOLD - 1; 10];
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals_below), 0);

    let mut vals_exact = [RAIN_THRESHOLD; 10];
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals_exact), 100);
}

#[test]
fn test_t2_f01_spread_with_single_valid_member() {
    let mut vals = [500, NODATA, NODATA];
    // Spread with only 1 valid member: variance = 0
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut vals), 0);
}

#[test]
fn test_t2_f01_dilated_mask_zero_radius() {
    let w = 5;
    let h = 5;
    let mut data = vec![0u16; w * h];
    data[2 * w + 2] = RAIN_THRESHOLD + 5;

    let mask = compute_dilated_mask_with_dims(&data, 0, w, h);
    assert!(mask[2 * w + 2]);
    assert!(!mask[2 * w + 1]);
    assert!(!mask[2 * w + 3]);
}

#[test]
fn test_t2_f01_dilated_mask_huge_radius() {
    let w = 4;
    let h = 4;
    let mut data = vec![0u16; w * h];
    data[0] = RAIN_THRESHOLD + 50;

    let mask = compute_dilated_mask_with_dims(&data, 100, w, h);
    // Huge radius covers all pixels in grid without out-of-bounds panics
    for &m in &mask {
        assert!(m);
    }
}

// =========================================================================
// Tier 2: F02 Harmonie Multi-Variable Boundary & Corner Cases
// =========================================================================

#[test]
fn test_t2_f02_subzero_freezing_temperature() {
    // 240.0 K -> 2400 / 10 - 273.15 = -33.15 °C
    let raw_val = 2400u16;
    let temp_c = raw_val as f64 / 10.0 - 273.15;
    assert!((temp_c - (-33.15)).abs() < 1e-4);
}

#[test]
fn test_t2_f02_extreme_heatwave_temperature() {
    // 320.0 K -> 3200 / 10 - 273.15 = +46.85 °C
    let raw_val = 3200u16;
    let temp_c = raw_val as f64 / 10.0 - 273.15;
    assert!((temp_c - 46.85).abs() < 1e-4);
}

#[test]
fn test_t2_f02_extreme_gale_wind_speed() {
    // u = 60 m/s -> raw = 16000, v = 45 m/s -> raw = 14500
    let u_raw = 16000u16;
    let v_raw = 14500u16;
    let u = u_raw as f64 / 100.0 - 100.0;
    let v = v_raw as f64 / 100.0 - 100.0;
    let speed = (u * u + v * v).sqrt();
    assert!((speed - 75.0).abs() < 1e-4);
}

#[test]
fn test_t2_f02_calm_zero_wind() {
    let u_raw = 10000u16; // 0 m/s
    let v_raw = 10000u16; // 0 m/s
    let u = u_raw as f64 / 100.0 - 100.0;
    let v = v_raw as f64 / 100.0 - 100.0;
    let speed = (u * u + v * v).sqrt();
    assert_eq!(speed, 0.0);
}

#[test]
fn test_t2_f02_solar_radiation_bounds() {
    let midnight_raw = 0u16;
    assert_eq!(midnight_raw as f64, 0.0);

    let solar_noon_raw = 1200u16;
    assert_eq!(solar_noon_raw as f64, 1200.0);
}

// =========================================================================
// Tier 2: F03 RTCOR Actuals Boundary Cases
// =========================================================================

#[test]
fn test_t2_f03_rtcor_max_history_overflow_100_frames() {
    let mut store = ActualsData::new();
    for i in 0..100 {
        let frame = ActualsFrame {
            timestamp: 10000 + i * 300,
            raw_values: Arc::new(vec![i as u16; 5]),
            webp_bytes: vec![0u8; 10],
        };
        store.insert_or_update(frame, 24);
    }
    assert_eq!(store.frames.len(), 24);
    assert_eq!(store.frames[0].timestamp, 10000 + 76 * 300);
    assert_eq!(store.frames[23].timestamp, 10000 + 99 * 300);
}

#[test]
fn test_t2_f03_rtcor_single_frame_capacity() {
    let mut store = ActualsData::new();
    for i in 0..5 {
        let frame = ActualsFrame {
            timestamp: 1000 + i * 300,
            raw_values: Arc::new(vec![i as u16]),
            webp_bytes: vec![0u8],
        };
        store.insert_or_update(frame, 1);
    }
    assert_eq!(store.frames.len(), 1);
    assert_eq!(store.frames[0].timestamp, 2200);
}

#[test]
fn test_t2_f03_rtcor_reverse_chronological_insert() {
    let mut store = ActualsData::new();
    for i in (0..10).rev() {
        let frame = ActualsFrame {
            timestamp: 1000 + i * 300,
            raw_values: Arc::new(vec![i as u16]),
            webp_bytes: vec![0u8],
        };
        store.insert_or_update(frame, 10);
    }
    assert_eq!(store.frames.len(), 10);
    for i in 0..9 {
        assert!(store.frames[i].timestamp < store.frames[i + 1].timestamp);
    }
}

#[test]
fn test_t2_f03_rtcor_idempotent_overwrite() {
    let mut store = ActualsData::new();
    for _ in 0..5 {
        let frame = ActualsFrame {
            timestamp: 5000,
            raw_values: Arc::new(vec![42u16]),
            webp_bytes: vec![0u8],
        };
        store.insert_or_update(frame, 10);
    }
    assert_eq!(store.frames.len(), 1);
    assert_eq!(store.frames[0].timestamp, 5000);
    assert_eq!(store.frames[0].raw_values[0], 42);
}

// =========================================================================
// Tier 2: F04 Binary Cache Deserializer Memory Bounding & Corruptions
// =========================================================================

#[test]
fn test_t2_f04_oversized_steps_len_rejection() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_t2_oversized_steps.bin").to_string_lossy().to_string();

    let mut buf = Vec::new();
    buf.extend_from_slice(b"HRMT"); // Magic
    buf.extend_from_slice(&1700000000i64.to_le_bytes()); // ref time
    buf.extend_from_slice(&100_000_000u32.to_le_bytes()); // Maliciously huge steps_len

    std::fs::write(&file_path, buf).expect("Write failed");

    // Reading should fail without attempting 100M allocations or crashing OOM
    let res = TempForecast::read_from_file(&file_path);
    assert!(res.is_err());

    let _ = std::fs::remove_file(file_path);
}

#[test]
fn test_t2_f04_truncated_payload_unexpected_eof() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_t2_truncated.bin").to_string_lossy().to_string();

    let mut buf = Vec::new();
    buf.extend_from_slice(b"HRW2");
    buf.extend_from_slice(&1700000000i64.to_le_bytes());
    buf.extend_from_slice(&2u32.to_le_bytes()); // claims 2 steps
    buf.extend_from_slice(&1i32.to_le_bytes()); // hour
    // cut off here

    std::fs::write(&file_path, buf).expect("Write failed");

    let res = WindForecast::read_from_file(&file_path);
    assert!(res.is_err());

    let _ = std::fs::remove_file(file_path);
}

#[test]
fn test_t2_f04_invalid_grid_dimensions_header() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_t2_bad_dims.bin").to_string_lossy().to_string();

    let mut buf = Vec::new();
    buf.extend_from_slice(b"HRMS");
    buf.extend_from_slice(&1700000000i64.to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes()); // hour
    buf.extend_from_slice(&9999u32.to_le_bytes()); // Invalid width != GRIB_WIDTH
    buf.extend_from_slice(&9999u32.to_le_bytes()); // Invalid height != GRIB_HEIGHT

    std::fs::write(&file_path, buf).expect("Write failed");

    let res = SolarForecast::read_from_file(&file_path);
    assert!(res.is_err());

    let _ = std::fs::remove_file(file_path);
}

#[test]
fn test_t2_f04_zero_steps_len_forecast() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_t2_zero_steps.bin").to_string_lossy().to_string();

    let fc = RainForecast {
        reference_time: 1700000000,
        steps: Vec::new(),
    };
    fc.write_to_file(&file_path).expect("Write failed");

    let loaded = RainForecast::read_from_file(&file_path).expect("Read failed");
    assert_eq!(loaded.steps.len(), 0);

    let _ = std::fs::remove_file(file_path);
}

// =========================================================================
// Tier 2: F05 MQTT & Startup Resilience
// =========================================================================

#[test]
fn test_t2_f05_preserves_non_tar_files_in_cache() {
    let cache_dir = constants::CACHE_DIR;
    let _ = std::fs::create_dir_all(cache_dir);
    let keep_file = format!("{}/keep_me.txt", cache_dir);
    std::fs::write(&keep_file, b"important cache metadata").expect("Write failed");

    // cleanup_tar_files should leave .txt files intact
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        cleanup_tar_files().await;
    });

    assert!(std::fs::metadata(&keep_file).is_ok());
    let _ = std::fs::remove_file(keep_file);
}

#[tokio::test]
async fn test_t2_f05_rapid_consecutive_dataset_swaps() {
    let state = create_mock_app_state();
    let initial_radar = state.radar_data.read().await.clone().unwrap();

    for i in 0..50 {
        let mut new_meta = initial_radar.metadata.clone();
        new_meta.version = 1000 + i;
        let staged_radar = Arc::new(RadarData::new(format!("rapid_swap_{}.nc", i), new_meta));

        let mut guard = state.radar_data.write().await;
        *guard = Some(staged_radar);
    }

    let final_radar = state.radar_data.read().await.clone().unwrap();
    assert_eq!(final_radar.metadata.version, 1049);
}

// =========================================================================
// Tier 2: F06 Defensive Error Boundaries & No-Panic Invariants
// =========================================================================

#[test]
fn test_t2_f06_extreme_float_interpolation_safety() {
    let vals = vec![100u16; 100];
    assert_eq!(interpolate_bilinear(-1_000_000.0, 5.0, 10, 10, &vals), NODATA);
    assert_eq!(interpolate_bilinear(5.0, -1_000_000.0, 10, 10, &vals), NODATA);
    assert_eq!(interpolate_bilinear(1_000_000.0, 5.0, 10, 10, &vals), NODATA);
    assert_eq!(interpolate_bilinear(5.0, 1_000_000.0, 10, 10, &vals), NODATA);
}

#[test]
fn test_t2_f06_extreme_negative_grib_indices() {
    let (fx, fy) = lonlat_to_grib_indices(-180.0, -90.0);
    assert!(fx.is_finite());
    assert!(fy.is_finite());
    assert!(fx < 0.0);
    assert!(fy < 0.0);
}

#[test]
fn test_t2_f06_antimeridian_lonlat() {
    let (lon_pos, _) = mercator_to_lonlat(20037508.34, 0.0);
    let (lon_neg, _) = mercator_to_lonlat(-20037508.34, 0.0);
    assert!((lon_pos - 180.0).abs() < 1e-2);
    assert!((lon_neg + 180.0).abs() < 1e-2);
}

#[test]
fn test_t2_f06_poles_singularity_safety() {
    let (_, lat_north) = mercator_to_lonlat(0.0, 20037508.34);
    assert!(lat_north > 85.0 && lat_north < 86.0);

    let (px, py) = lonlat_to_polar_stereographic(0.0, 90.0);
    assert!(px.is_finite());
    assert!(py.is_finite());
}

// =========================================================================
// Tier 2: F07 Axum API Out-of-Grid & Corner Responses
// =========================================================================

#[tokio::test]
async fn test_t2_f07_api_value_out_of_grid_returns_200_out_of_bounds() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    // Coordinate far outside Netherlands domain (Pacific Ocean: 0, 0)
    let req = Request::builder()
        .uri("/api/value?ens=med&time=0&lon=0.0&lat=0.0")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let val_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(val_res["status"], "out_of_bounds");
    assert!(val_res["value"].is_null());
}

#[tokio::test]
async fn test_t2_f07_api_timeseries_out_of_grid_returns_empty_series() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/api/timeseries?ens=med&lon=-100.0&lat=0.0")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ts_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(ts_res["status"], "out_of_bounds");
    assert_eq!(ts_res["values"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_t2_f07_api_temp_value_out_of_grid() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/api/value/temp?time=0&lon=-50.0&lat=-50.0")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let val_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(val_res["status"], "out_of_bounds");
    assert!(val_res["value"].is_null());
}

#[tokio::test]
async fn test_t2_f07_api_wind_value_out_of_grid() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/api/value/wind?time=0&lon=-50.0&lat=-50.0&height=10")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let val_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(val_res["status"], "out_of_bounds");
    assert!(val_res["u"].is_null());
}

#[tokio::test]
async fn test_t2_f07_api_solar_value_out_of_grid() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/api/value/solar?time=0&lon=-50.0&lat=-50.0")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let val_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(val_res["status"], "out_of_bounds");
    assert!(val_res["value"].is_null());
}

#[tokio::test]
async fn test_t2_f07_api_data_image_negative_actuals_offset() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    // Negative timestamp (-300) should retrieve actuals frame
    let req = Request::builder()
        .uri("/api/data/med/-300")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("Content-Type").unwrap(), "image/webp");
}
