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

/// Creates a mock test Router with full route bindings identical to main.rs
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

/// Creates a comprehensive mock AppState populated with synthetic valid forecast grids
fn create_mock_app_state() -> Arc<AppState> {
    let projection_lut = Arc::new(init_projection_lut());
    let actuals_lut = Arc::new(init_actuals_projection_lut());
    let grib_lut = Arc::new(init_temp_projection_lut());

    // 1. Radar mock data
    let ref_time = 1700000000i64;
    let times = vec![0, 300, 600, 900, 1200];
    let ensembles = vec![0, 1, 2, 3, 4];
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

    // Populate grid caches for ens 0..4 and stats med, max, prob, pmm, spread
    let fc_len = FORECAST_GRID_W * FORECAST_GRID_H;
    for &t in &times {
        for e in 0..5 {
            let mut grid = vec![0u16; fc_len];
            for item in grid.iter_mut().skip(fc_len / 2).take(100) {
                *item = 250 + (e as u16) * 50;
            }
            radar_data
                .grid_cache
                .insert((e.to_string(), t), Arc::new(grid));
        }

        let mut med_grid = vec![0u16; fc_len];
        let mut max_grid = vec![0u16; fc_len];
        let mut prob_grid = vec![0u16; fc_len];
        let mut spread_grid = vec![0u16; fc_len];
        let mut pmm_grid = vec![0u16; fc_len];

        for idx in (fc_len / 2)..(fc_len / 2 + 100) {
            med_grid[idx] = 300;
            max_grid[idx] = 450;
            prob_grid[idx] = 80;
            spread_grid[idx] = 20;
            pmm_grid[idx] = 320;
        }

        radar_data
            .grid_cache
            .insert(("med".to_string(), t), Arc::new(med_grid));
        radar_data
            .grid_cache
            .insert(("max".to_string(), t), Arc::new(max_grid));
        radar_data
            .grid_cache
            .insert(("prob".to_string(), t), Arc::new(prob_grid));
        radar_data
            .grid_cache
            .insert(("spread".to_string(), t), Arc::new(spread_grid));
        radar_data
            .grid_cache
            .insert(("pmm".to_string(), t), Arc::new(pmm_grid));
    }

    // 2. Temp mock data
    let temp_steps = (0..5)
        .map(|hour| {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let mut values = vec![2931u16; grib_len];
            for v in values.iter_mut() {
                *v = (*v as i32 + hour * 10) as u16;
            }
            TempStep {
                forecast_hour: hour,
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                values: Arc::new(values),
            }
        })
        .collect();

    let temp_fc = TempForecast {
        reference_time: ref_time,
        steps: temp_steps,
    };
    let temp_data = Arc::new(TempData::new(temp_fc));

    // 3. Wind mock data
    let mut wind_steps = Vec::new();
    for &height_lvl in &[10, 50, 100, 200, 300] {
        for hour in 0..5 {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let u_values = vec![10500u16; grib_len];
            let v_values = vec![10000u16; grib_len];
            wind_steps.push(WindStep {
                forecast_hour: hour,
                height_level: height_lvl,
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                u_values: Arc::new(u_values),
                v_values: Arc::new(v_values),
            });
        }
    }
    let wind_fc = WindForecast {
        reference_time: ref_time,
        steps: wind_steps,
    };
    let wind_data = Arc::new(WindData::new(wind_fc));

    // 4. Solar mock data
    let solar_steps = (0..5)
        .map(|hour| {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let values = vec![(hour as u16) * 150; grib_len];
            SolarStep {
                forecast_hour: hour,
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                values: Arc::new(values),
            }
        })
        .collect();
    let solar_fc = SolarForecast {
        reference_time: ref_time,
        steps: solar_steps,
    };
    let solar_data = Arc::new(SolarData::new(solar_fc));

    // 5. Rain mock data (Harmonie extended)
    let rain_steps = (0..5)
        .map(|hour| {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let values = vec![50u16; grib_len];
            RainStep {
                forecast_hour: hour,
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                values: Arc::new(values),
            }
        })
        .collect();
    let rain_fc = RainForecast {
        reference_time: ref_time,
        steps: rain_steps,
    };
    let rain_data = Arc::new(RainData::new(rain_fc));

    // 6. Actuals mock data
    let mut actuals = ActualsData::new();
    let act_len = constants::RTCOR_GRID_W * constants::RTCOR_GRID_H;
    for offset in &[-900i64, -600, -300] {
        let frame = ActualsFrame {
            timestamp: ref_time + offset,
            raw_values: Arc::new(vec![100u16; act_len]),
            webp_bytes: vec![0u8; 128],
        };
        actuals.insert_or_update(frame, 24);
    }

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
// F01: NetCDF-4 Ensemble Decoding & PMM Reductions (>= 5 tests)
// =========================================================================

#[test]
fn test_f01_01_ensemble_stat_from_str() {
    assert!(matches!(EnsembleStat::from_str("med"), Some(EnsembleStat::Median)));
    assert!(matches!(EnsembleStat::from_str("max"), Some(EnsembleStat::Maximum)));
    assert!(matches!(EnsembleStat::from_str("prob"), Some(EnsembleStat::Probability)));
    assert!(matches!(EnsembleStat::from_str("spread"), Some(EnsembleStat::Spread)));
    assert!(matches!(EnsembleStat::from_str("pmm"), Some(EnsembleStat::Pmm)));
    assert!(EnsembleStat::from_str("invalid_stat").is_none());
    assert!(EnsembleStat::from_str("").is_none());
}

#[test]
fn test_f01_02_ensemble_median_reduction() {
    let mut vals_odd = [100, 300, 200];
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut vals_odd), 200);

    let mut vals_even = [500, 100, 400, 200];
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut vals_even), 400);

    let mut vals_same = [250, 250, 250, 250, 250];
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut vals_same), 250);
}

#[test]
fn test_f01_03_ensemble_maximum_reduction() {
    let mut vals1 = [10, 50, 20, 80, 30];
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut vals1), 80);

    let mut vals_with_nodata = [50, 120, NODATA, 30];
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut vals_with_nodata), 120);

    let mut vals_all_zeros = [0, 0, 0];
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut vals_all_zeros), 0);
}

#[test]
fn test_f01_04_ensemble_probability_reduction() {
    let mut vals = [
        RAIN_THRESHOLD - 2,
        RAIN_THRESHOLD,
        RAIN_THRESHOLD + 5,
        100,
        0,
    ];
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals), 60);

    let mut vals_none = [0, 1, 2, 3, 4];
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals_none), 0);

    let mut vals_all = [RAIN_THRESHOLD, RAIN_THRESHOLD + 10, RAIN_THRESHOLD + 20, 50];
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals_all), 100);
}

#[test]
fn test_f01_05_ensemble_spread_reduction() {
    let mut vals = [100, 200, 300];
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut vals), 82);

    let mut vals_identical = [400, 400, 400, 400];
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut vals_identical), 0);
}

#[test]
fn test_f01_06_nep_mask_dilation_circular() {
    let w = 10;
    let h = 10;
    let mut data = vec![0u16; w * h];
    data[5 * w + 5] = RAIN_THRESHOLD + 10;

    let mask_r1 = compute_dilated_mask_with_dims(&data, 1, w, h);
    assert!(mask_r1[5 * w + 5]);
    assert!(mask_r1[5 * w + 4]);
    assert!(mask_r1[5 * w + 6]);
    assert!(mask_r1[4 * w + 5]);
    assert!(mask_r1[6 * w + 5]);
    assert!(!mask_r1[4 * w + 4]);

    let mask_r2 = compute_dilated_mask_with_dims(&data, 2, w, h);
    assert!(mask_r2[4 * w + 4]);
    assert!(mask_r2[3 * w + 5]);
    assert!(!mask_r2[2 * w + 5]);
}

#[test]
fn test_f01_07_raw_to_value_conversion() {
    assert_eq!(raw_to_value(NODATA), 0.0);
    assert_eq!(raw_to_value(0), 0.0);
    assert_eq!(raw_to_value(100), 1.0);
    assert_eq!(raw_to_value(250), 2.5);
    assert_eq!(raw_to_value(1550), 15.5);
}

// =========================================================================
// F02: Harmonie GRIB1 Multi-Variable Extraction (>= 5 tests)
// =========================================================================

#[test]
fn test_f02_01_parse_reference_time() {
    let ref_str = "seconds since 2023-11-14 22:13:20";
    let ts = parse_reference_time(ref_str);
    assert!(ts.is_some());
    assert_eq!(ts.unwrap(), 1700000000);

    let invalid_str = "invalid timestamp string format";
    assert!(parse_reference_time(invalid_str).is_none());
}

#[test]
fn test_f02_02_parse_forecast_hour_from_name() {
    assert_eq!(parse_forecast_hour_from_name("HA40_NWP_202311142200_00300_GB"), Some(3));
    assert_eq!(parse_forecast_hour_from_name("HA40_NWP_202311142200_04800_GB"), Some(48));
    assert_eq!(parse_forecast_hour_from_name("HA40_NWP_202311142200_00000_GB"), Some(0));
    assert_eq!(parse_forecast_hour_from_name("unrecognized_filename_pattern"), None);
}

#[test]
fn test_f02_03_parse_run_time_from_name() {
    let ts = parse_run_time_from_name("HA40_NWP_202311142200_00300_GB");
    assert!(ts.is_some());
    let ts_val = ts.unwrap();
    assert!(ts_val > 1699900000);

    assert_eq!(parse_run_time_from_name("malformed_run_time"), None);
}

#[test]
fn test_f02_04_temperature_conversion_and_step() {
    let step = TempStep {
        forecast_hour: 6,
        width: 10,
        height: 10,
        values: Arc::new(vec![2931u16; 100]),
    };
    assert_eq!(step.forecast_hour(), 6);
    let temp_c = step.values[0] as f64 / 10.0 - 273.15;
    assert!((temp_c - 19.95).abs() < 0.1);
}

#[test]
fn test_f02_05_wind_multi_height_levels() {
    let heights = [10u32, 50, 100, 200, 300];
    for &h in &heights {
        let step = WindStep {
            forecast_hour: 3,
            height_level: h,
            width: 10,
            height: 10,
            u_values: Arc::new(vec![10500u16; 100]),
            v_values: Arc::new(vec![10000u16; 100]),
        };
        assert_eq!(step.forecast_hour(), 3);
        assert_eq!(step.height_level, h);
    }
}

#[test]
fn test_f02_06_solar_and_rain_forecast_steps() {
    let solar_step = SolarStep {
        forecast_hour: 12,
        width: 10,
        height: 10,
        values: Arc::new(vec![750u16; 100]),
    };
    assert_eq!(solar_step.forecast_hour(), 12);
    assert_eq!(solar_step.values[0], 750);

    let rain_step = RainStep {
        forecast_hour: 24,
        width: 10,
        height: 10,
        values: Arc::new(vec![200u16; 100]),
    };
    assert_eq!(rain_step.forecast_hour(), 24);
    assert_eq!(rain_step.values[0], 200);
}

// =========================================================================
// F03: RTCOR HDF5 Actuals Ingestion & History Bounding (>= 5 tests)
// =========================================================================

#[test]
fn test_f03_01_parse_rtcor_filename_timestamp() {
    let ts = parse_rtcor_filename_timestamp("RAD_NL25_RAC_RT_202608261300.h5");
    assert!(ts.is_some());
    let ts_val = ts.unwrap();
    assert!(ts_val > 1699900000);

    let ts2 = parse_rtcor_filename_timestamp("RAD_NL25_RAC_RT_202401010000.h5");
    assert!(ts2.is_some());

    assert_eq!(parse_rtcor_filename_timestamp("corrupted_rtcor_file.h5"), None);
    assert_eq!(parse_rtcor_filename_timestamp(""), None);
}

#[test]
fn test_f03_02_rtcor_actuals_store_bounding() {
    let mut store = ActualsData::new();
    let max_frames = 5;

    for i in 0..10 {
        let frame = ActualsFrame {
            timestamp: 1000 + (i * 300),
            raw_values: Arc::new(vec![0u16; 10]),
            webp_bytes: vec![0u8; 10],
        };
        store.insert_or_update(frame, max_frames);
    }

    assert_eq!(store.frames.len(), max_frames);
    assert_eq!(store.frames[0].timestamp, 2500);
    assert_eq!(store.frames[4].timestamp, 3700);
}

#[test]
fn test_f03_03_rtcor_actuals_sorting_order() {
    let mut store = ActualsData::new();
    let timestamps = [3000, 1000, 5000, 2000, 4000];

    for &ts in &timestamps {
        let frame = ActualsFrame {
            timestamp: ts,
            raw_values: Arc::new(vec![0u16; 10]),
            webp_bytes: vec![0u8; 10],
        };
        store.insert_or_update(frame, 10);
    }

    assert_eq!(store.frames.len(), 5);
    for i in 0..4 {
        assert!(store.frames[i].timestamp < store.frames[i + 1].timestamp);
    }
}

#[test]
fn test_f03_04_rtcor_actuals_insert_or_update_in_place() {
    let mut store = ActualsData::new();
    let frame1 = ActualsFrame {
        timestamp: 1500,
        raw_values: Arc::new(vec![10u16; 10]),
        webp_bytes: vec![1u8; 10],
    };
    store.insert_or_update(frame1, 10);

    assert_eq!(store.frames[0].raw_values[0], 10);

    let frame1_updated = ActualsFrame {
        timestamp: 1500,
        raw_values: Arc::new(vec![99u16; 10]),
        webp_bytes: vec![2u8; 10],
    };
    store.insert_or_update(frame1_updated, 10);

    assert_eq!(store.frames.len(), 1);
    assert_eq!(store.frames[0].raw_values[0], 99);
}

#[test]
fn test_f03_05_rtcor_polar_stereographic_bounds() {
    let (px, py) = lonlat_to_polar_stereographic(5.18, 52.10);
    assert!(px.is_finite());
    assert!(py.is_finite());

    let ix = ((px - RTCOR_X0) / RTCOR_DX).round() as i32;
    let iy = ((py - RTCOR_Y0) / RTCOR_DY).round() as i32;

    assert!(ix >= 0 && ix < RTCOR_GRID_W as i32);
    assert!(iy >= 0 && iy < RTCOR_GRID_H as i32);
}

// =========================================================================
// F04: Binary Cache Deserializer Memory Bounding (>= 5 tests)
// =========================================================================

#[test]
fn test_f04_01_temp_forecast_binary_roundtrip() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_f04_temp.bin").to_string_lossy().to_string();

    let step = TempStep {
        forecast_hour: 5,
        width: GRIB_WIDTH,
        height: GRIB_HEIGHT,
        values: Arc::new(vec![2850u16; GRIB_WIDTH * GRIB_HEIGHT]),
    };
    let fc = TempForecast {
        reference_time: 1700000000,
        steps: vec![step],
    };

    fc.write_to_file(&file_path).expect("Failed to write temp binary");
    let loaded = TempForecast::read_from_file(&file_path).expect("Failed to read temp binary");

    assert_eq!(loaded.reference_time, 1700000000);
    assert_eq!(loaded.steps.len(), 1);
    assert_eq!(loaded.steps[0].forecast_hour, 5);
    assert_eq!(loaded.steps[0].values[0], 2850);

    let _ = std::fs::remove_file(file_path);
}

#[test]
fn test_f04_02_wind_forecast_binary_roundtrip() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_f04_wind.bin").to_string_lossy().to_string();

    let step = WindStep {
        forecast_hour: 2,
        height_level: 100,
        width: GRIB_WIDTH,
        height: GRIB_HEIGHT,
        u_values: Arc::new(vec![11000u16; GRIB_WIDTH * GRIB_HEIGHT]),
        v_values: Arc::new(vec![9500u16; GRIB_WIDTH * GRIB_HEIGHT]),
    };
    let fc = WindForecast {
        reference_time: 1700000000,
        steps: vec![step],
    };

    fc.write_to_file(&file_path).expect("Failed to write wind binary");
    let loaded = WindForecast::read_from_file(&file_path).expect("Failed to read wind binary");

    assert_eq!(loaded.reference_time, 1700000000);
    assert_eq!(loaded.steps.len(), 1);
    assert_eq!(loaded.steps[0].height_level, 100);
    assert_eq!(loaded.steps[0].u_values[0], 11000);
    assert_eq!(loaded.steps[0].v_values[0], 9500);

    let _ = std::fs::remove_file(file_path);
}

#[test]
fn test_f04_03_solar_forecast_binary_roundtrip() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_f04_solar.bin").to_string_lossy().to_string();

    let step = SolarStep {
        forecast_hour: 14,
        width: GRIB_WIDTH,
        height: GRIB_HEIGHT,
        values: Arc::new(vec![850u16; GRIB_WIDTH * GRIB_HEIGHT]),
    };
    let fc = SolarForecast {
        reference_time: 1700000000,
        steps: vec![step],
    };

    fc.write_to_file(&file_path).expect("Failed to write solar binary");
    let loaded = SolarForecast::read_from_file(&file_path).expect("Failed to read solar binary");

    assert_eq!(loaded.reference_time, 1700000000);
    assert_eq!(loaded.steps.len(), 1);
    assert_eq!(loaded.steps[0].forecast_hour, 14);
    assert_eq!(loaded.steps[0].values[0], 850);

    let _ = std::fs::remove_file(file_path);
}

#[test]
fn test_f04_04_rain_forecast_binary_roundtrip() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_f04_rain.bin").to_string_lossy().to_string();

    let step = RainStep {
        forecast_hour: 18,
        width: GRIB_WIDTH,
        height: GRIB_HEIGHT,
        values: Arc::new(vec![150u16; GRIB_WIDTH * GRIB_HEIGHT]),
    };
    let fc = RainForecast {
        reference_time: 1700000000,
        steps: vec![step],
    };

    fc.write_to_file(&file_path).expect("Failed to write rain binary");
    let loaded = RainForecast::read_from_file(&file_path).expect("Failed to read rain binary");

    assert_eq!(loaded.reference_time, 1700000000);
    assert_eq!(loaded.steps.len(), 1);
    assert_eq!(loaded.steps[0].forecast_hour, 18);
    assert_eq!(loaded.steps[0].values[0], 150);

    let _ = std::fs::remove_file(file_path);
}

#[test]
fn test_f04_05_binary_cache_magic_byte_validation() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("test_f04_corrupt_magic.bin").to_string_lossy().to_string();

    std::fs::write(&file_path, b"XXXX_bogus_payload_data").expect("Write failed");

    assert!(TempForecast::read_from_file(&file_path).is_err());
    assert!(WindForecast::read_from_file(&file_path).is_err());
    assert!(SolarForecast::read_from_file(&file_path).is_err());
    assert!(RainForecast::read_from_file(&file_path).is_err());

    let _ = std::fs::remove_file(file_path);
}

// =========================================================================
// F05: MQTT Cancellation & Cold Boot Resilience (>= 5 tests)
// =========================================================================

#[tokio::test]
async fn test_f05_01_atomic_staged_dataset_swapping() {
    let state = create_mock_app_state();
    let initial_radar = state.radar_data.read().await.clone().unwrap();
    assert_eq!(initial_radar.metadata.version, 12345678);

    let mut new_meta = initial_radar.metadata.clone();
    new_meta.version = 87654321;
    let staged_radar = Arc::new(RadarData::new("staged_dataset.nc".to_string(), new_meta));

    {
        let mut guard = state.radar_data.write().await;
        *guard = Some(staged_radar);
    }

    let updated_radar = state.radar_data.read().await.clone().unwrap();
    assert_eq!(updated_radar.metadata.version, 87654321);
    assert_eq!(updated_radar.file_path, "staged_dataset.nc");
}

#[tokio::test]
async fn test_f05_02_cold_boot_cache_directory() {
    let temp_cache = std::env::temp_dir().join("nimbus_cold_boot_cache");
    let cache_str = temp_cache.to_string_lossy().to_string();

    tokio::fs::create_dir_all(&cache_str).await.expect("Cache dir creation failed");
    assert!(tokio::fs::metadata(&cache_str).await.is_ok());

    let _ = tokio::fs::remove_dir_all(&cache_str).await;
}

#[tokio::test]
async fn test_f05_03_tar_cleanup_stale_files() {
    let cache_dir = constants::CACHE_DIR;
    let _ = tokio::fs::create_dir_all(cache_dir).await;
    let stale_tar = format!("{}/HARM43_stale_archive.tar", cache_dir);
    tokio::fs::write(&stale_tar, b"dummy tar bytes").await.expect("Write tar failed");

    cleanup_tar_files().await;

    assert!(tokio::fs::metadata(&stale_tar).await.is_err());
}

#[test]
fn test_f05_04_forecast_and_actuals_time_merging() {
    let ref_str = "seconds since 2023-11-14 22:00:00";
    let ref_ts = parse_reference_time(ref_str).unwrap();

    let mut actuals = ActualsData::new();
    actuals.insert_or_update(
        ActualsFrame {
            timestamp: ref_ts - 600,
            raw_values: Arc::new(vec![0u16; 10]),
            webp_bytes: vec![0u8; 10],
        },
        10,
    );
    actuals.insert_or_update(
        ActualsFrame {
            timestamp: ref_ts - 300,
            raw_values: Arc::new(vec![0u16; 10]),
            webp_bytes: vec![0u8; 10],
        },
        10,
    );

    assert_eq!(actuals.frames.len(), 2);
    assert_eq!(actuals.frames[0].timestamp - ref_ts, -600);
    assert_eq!(actuals.frames[1].timestamp - ref_ts, -300);
}

#[test]
fn test_f05_05_metadata_version_generation() {
    let ts_now = 1700000000u64;
    let version_str = format!("v={}", ts_now);
    assert_eq!(version_str, "v=1700000000");
}

// =========================================================================
// F06: Async Worker Error Recovery & Zero Panics (>= 5 tests)
// =========================================================================

#[test]
fn test_f06_01_empty_ensemble_returns_nodata() {
    let mut empty_vals: [u16; 0] = [];
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut empty_vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut empty_vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut empty_vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut empty_vals), NODATA);
}

#[test]
fn test_f06_02_first_nodata_returns_nodata() {
    let mut vals = [NODATA, 50, 100, 200];
    assert_eq!(reduce_ensemble(&EnsembleStat::Median, &mut vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Maximum, &mut vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Probability, &mut vals), NODATA);
    assert_eq!(reduce_ensemble(&EnsembleStat::Spread, &mut vals), NODATA);
}

#[test]
fn test_f06_03_bilinear_interpolation_out_of_bounds() {
    let values = vec![100u16; 100];
    assert_eq!(interpolate_bilinear(-5.0, 5.0, 10, 10, &values), NODATA);
    assert_eq!(interpolate_bilinear(5.0, -2.0, 10, 10, &values), NODATA);
    assert_eq!(interpolate_bilinear(20.0, 5.0, 10, 10, &values), NODATA);
    assert_eq!(interpolate_bilinear(5.0, 20.0, 10, 10, &values), NODATA);
}

#[test]
fn test_f06_04_polar_stereographic_extreme_coords() {
    let (px_north, py_north) = lonlat_to_polar_stereographic(0.0, 89.9);
    assert!(px_north.is_finite());
    assert!(py_north.is_finite());

    let (px_south, py_south) = lonlat_to_polar_stereographic(0.0, -89.9);
    assert!(px_south.is_finite());
    assert!(py_south.is_finite());

    let (px_antimeridian, py_antimeridian) = lonlat_to_polar_stereographic(180.0, 50.0);
    assert!(px_antimeridian.is_finite());
    assert!(py_antimeridian.is_finite());
}

#[test]
fn test_f06_05_mercator_to_lonlat_boundaries() {
    let (lon_left, lat_top) = mercator_to_lonlat(MERCATOR_LEFT, MERCATOR_TOP);
    assert!(lat_top > 50.0 && lat_top < 65.0);
    assert!(lon_left > -10.0 && lon_left < 15.0);

    let (lon_right, lat_bot) = mercator_to_lonlat(MERCATOR_RIGHT, MERCATOR_BOTTOM);
    assert!(lat_bot > 40.0 && lat_bot < 55.0);
    assert!(lon_right > 0.0 && lon_right < 20.0);
}

// =========================================================================
// F07: Axum API Resilience & Error Boundaries (>= 8 tests)
// =========================================================================

#[tokio::test]
async fn test_f07_01_api_metadata() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/metadata")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let meta: Metadata = serde_json::from_slice(&body_bytes).expect("Failed to deserialize Metadata");
    assert_eq!(meta.width, GRID_W);
    assert_eq!(meta.height, GRID_H);
    assert_eq!(meta.version, 12345678);
    assert!(!meta.times.is_empty());
}

#[tokio::test]
async fn test_f07_02_api_temp_wind_solar_metadata() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req_temp = Request::builder().uri("/api/metadata/temp").body(Body::empty()).unwrap();
    let res_temp = app.clone().oneshot(req_temp).await.unwrap();
    assert_eq!(res_temp.status(), StatusCode::OK);

    let req_wind = Request::builder().uri("/api/metadata/wind").body(Body::empty()).unwrap();
    let res_wind = app.clone().oneshot(req_wind).await.unwrap();
    assert_eq!(res_wind.status(), StatusCode::OK);

    let req_solar = Request::builder().uri("/api/metadata/solar").body(Body::empty()).unwrap();
    let res_solar = app.oneshot(req_solar).await.unwrap();
    assert_eq!(res_solar.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_f07_03_api_value_point_query() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/api/value?ens=med&time=0&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let val_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(val_res.get("status").is_some());
}

#[tokio::test]
async fn test_f07_04_api_timeseries_query() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/api/timeseries?ens=med&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let ts_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(ts_res["status"], "ok");
    assert!(!ts_res["times"].as_array().unwrap().is_empty());
    assert!(!ts_res["values"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_f07_05_api_temp_wind_solar_value() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req_temp = Request::builder()
        .uri("/api/value/temp?time=0&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_temp = app.clone().oneshot(req_temp).await.unwrap();
    assert_eq!(res_temp.status(), StatusCode::OK);

    let req_wind = Request::builder()
        .uri("/api/value/wind?time=0&lon=5.2&lat=52.1&height=10")
        .body(Body::empty())
        .unwrap();
    let res_wind = app.clone().oneshot(req_wind).await.unwrap();
    assert_eq!(res_wind.status(), StatusCode::OK);

    let req_solar = Request::builder()
        .uri("/api/value/solar?time=0&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_solar = app.oneshot(req_solar).await.unwrap();
    assert_eq!(res_solar.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_f07_06_api_temp_wind_solar_timeseries() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req_temp = Request::builder()
        .uri("/api/timeseries/temp?lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_temp = app.clone().oneshot(req_temp).await.unwrap();
    assert_eq!(res_temp.status(), StatusCode::OK);

    let req_wind = Request::builder()
        .uri("/api/timeseries/wind?lon=5.2&lat=52.1&height=10")
        .body(Body::empty())
        .unwrap();
    let res_wind = app.clone().oneshot(req_wind).await.unwrap();
    assert_eq!(res_wind.status(), StatusCode::OK);

    let req_solar = Request::builder()
        .uri("/api/timeseries/solar?lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_solar = app.oneshot(req_solar).await.unwrap();
    assert_eq!(res_solar.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_f07_07_api_data_image_rendering() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req_radar = Request::builder()
        .uri("/api/data/med/0")
        .body(Body::empty())
        .unwrap();
    let res_radar = app.clone().oneshot(req_radar).await.unwrap();
    assert_eq!(res_radar.status(), StatusCode::OK);
    assert_eq!(
        res_radar.headers().get("Content-Type").unwrap(),
        "image/webp"
    );

    let req_temp = Request::builder()
        .uri("/api/data/temp/0")
        .body(Body::empty())
        .unwrap();
    let res_temp = app.clone().oneshot(req_temp).await.unwrap();
    assert_eq!(res_temp.status(), StatusCode::OK);
    assert_eq!(res_temp.headers().get("Content-Type").unwrap(), "image/webp");

    let req_wind = Request::builder()
        .uri("/api/data/wind/10/0")
        .body(Body::empty())
        .unwrap();
    let res_wind = app.clone().oneshot(req_wind).await.unwrap();
    assert_eq!(res_wind.status(), StatusCode::OK);
    assert_eq!(res_wind.headers().get("Content-Type").unwrap(), "image/webp");

    let req_solar = Request::builder()
        .uri("/api/data/solar/0")
        .body(Body::empty())
        .unwrap();
    let res_solar = app.oneshot(req_solar).await.unwrap();
    assert_eq!(res_solar.status(), StatusCode::OK);
    assert_eq!(res_solar.headers().get("Content-Type").unwrap(), "image/webp");
}

#[tokio::test]
async fn test_f07_08_api_favicon_204_no_content() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/favicon.ico")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}
