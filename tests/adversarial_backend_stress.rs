#![allow(dead_code, unused_imports)]

#[path = "../src/constants.rs"]
mod constants;
#[path = "../src/handlers.rs"]
mod handlers;
#[path = "../src/harmonie.rs"]
mod harmonie;
#[path = "../src/interpolation.rs"]
mod interpolation;
#[path = "../src/models.rs"]
mod models;
#[path = "../src/projection.rs"]
mod projection;
#[path = "../src/radar.rs"]
mod radar;
#[path = "../src/rendering.rs"]
mod rendering;
#[path = "../src/rtcor.rs"]
mod rtcor;
#[path = "../src/state.rs"]
mod state;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::{routing::get, Router};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Creates a mock test Router matching main.rs
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

/// Creates a populated mock AppState with correct grid dimensions
fn create_mock_app_state() -> Arc<AppState> {
    let projection_lut = Arc::new(init_projection_lut());
    let actuals_lut = Arc::new(init_actuals_projection_lut());
    let grib_lut = Arc::new(init_temp_projection_lut());

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

    let fc_len = FORECAST_GRID_W * FORECAST_GRID_H;
    for &t in &times {
        let med_grid = Arc::new(vec![100u16; fc_len]);
        let max_grid = Arc::new(vec![250u16; fc_len]);
        let prob_grid = Arc::new(vec![50u16; fc_len]);
        let spread_grid = Arc::new(vec![15u16; fc_len]);
        let pmm_grid = Arc::new(vec![120u16; fc_len]);

        radar_data
            .grid_cache
            .insert(("med".to_string(), t), med_grid.clone());
        radar_data
            .grid_cache
            .insert(("max".to_string(), t), max_grid.clone());
        radar_data
            .grid_cache
            .insert(("prob".to_string(), t), prob_grid.clone());
        radar_data
            .grid_cache
            .insert(("spread".to_string(), t), spread_grid.clone());
        radar_data
            .grid_cache
            .insert(("pmm".to_string(), t), pmm_grid.clone());

        for e in 0..5 {
            let ens_grid = Arc::new(vec![100u16 + (e as u16) * 20; fc_len]);
            radar_data
                .grid_cache
                .insert((e.to_string(), t), ens_grid.clone());
            let webp_ens = render_data_webp_bytes(&ens_grid, &projection_lut);
            radar_data.data_cache.insert((e.to_string(), t), webp_ens);
        }

        let webp_med = render_data_webp_bytes(&med_grid, &projection_lut);
        let webp_max = render_data_webp_bytes(&max_grid, &projection_lut);
        let webp_prob = render_data_webp_bytes(&prob_grid, &projection_lut);
        let webp_spread = render_data_webp_bytes(&spread_grid, &projection_lut);
        let webp_pmm = render_data_webp_bytes(&pmm_grid, &projection_lut);

        radar_data
            .data_cache
            .insert(("med".to_string(), t), webp_med);
        radar_data
            .data_cache
            .insert(("max".to_string(), t), webp_max);
        radar_data
            .data_cache
            .insert(("prob".to_string(), t), webp_prob);
        radar_data
            .data_cache
            .insert(("spread".to_string(), t), webp_spread);
        radar_data
            .data_cache
            .insert(("pmm".to_string(), t), webp_pmm);
    }

    let temp_steps = (0..5)
        .map(|hour| {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let values = vec![2931u16 + (hour as u16) * 10; grib_len];
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

    // Precalculate temp WebPs
    for step in &temp_data.forecast.steps {
        let t = (step.forecast_hour as i64) * 3600;
        let webp = render_temp_webp_bytes(&step.values, &grib_lut);
        temp_data.data_cache.insert(t, webp);
    }

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

    // Precalculate wind WebPs
    for step in &wind_data.forecast.steps {
        let t = (step.forecast_hour as i64) * 3600;
        let webp = render_wind_webp_bytes(&step.u_values, &step.v_values, &grib_lut);
        wind_data.data_cache.insert((step.height_level, t), webp);
    }

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

    // Precalculate solar WebPs
    for step in &solar_data.forecast.steps {
        let t = (step.forecast_hour as i64) * 3600;
        let webp = render_solar_webp_bytes(&step.values, &grib_lut);
        solar_data.data_cache.insert(t, webp);
    }

    let rain_steps = (0..5)
        .map(|hour| {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let values = vec![(hour as u16) * 20; grib_len];
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

    Arc::new(AppState {
        radar_data: RwLock::new(Some(radar_data)),
        projection_lut,
        actuals_data: RwLock::new(None),
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

/// Creates an empty AppState (Cold boot with zero initialized datasets)
fn create_empty_app_state() -> Arc<AppState> {
    Arc::new(AppState {
        radar_data: RwLock::new(None),
        projection_lut: Arc::new(init_projection_lut()),
        actuals_data: RwLock::new(None),
        actuals_projection_lut: Arc::new(init_actuals_projection_lut()),
        temp_data: RwLock::new(None),
        temp_projection_lut: Arc::new(init_temp_projection_lut()),
        wind_data: RwLock::new(None),
        wind_projection_lut: Arc::new(init_temp_projection_lut()),
        solar_data: RwLock::new(None),
        solar_projection_lut: Arc::new(init_temp_projection_lut()),
        rain_data: RwLock::new(None),
    })
}

// ==============================================================================
// 1. BINARY CACHE STRESS TESTS
// ==============================================================================

#[test]
fn test_stress_binary_cache_steps_len_boundaries_temp() {
    let temp_dir = std::env::temp_dir();

    // 1. steps_len = 0 (valid empty forecast)
    let path_0 = temp_dir.join("stress_temp_0.bin");
    {
        let mut f = std::fs::File::create(&path_0).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap();
    }
    let res_0 = TempForecast::read_from_file(path_0.to_str().unwrap());
    assert!(res_0.is_ok(), "steps_len=0 must succeed cleanly");
    assert_eq!(res_0.unwrap().steps.len(), 0);
    let _ = std::fs::remove_file(path_0);

    // 2. steps_len = 1000 (boundary MAX_BINARY_CACHE_STEPS) without payload -> UnexpectedEof, not panic!
    let path_1000 = temp_dir.join("stress_temp_1000.bin");
    {
        let mut f = std::fs::File::create(&path_1000).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1000u32.to_le_bytes()).unwrap();
    }
    let res_1000 = TempForecast::read_from_file(path_1000.to_str().unwrap());
    assert!(
        res_1000.is_err(),
        "steps_len=1000 with truncated payload must return Err"
    );
    let _ = std::fs::remove_file(path_1000);

    // 3. steps_len = 1001 (MAX_BINARY_CACHE_STEPS + 1) -> must reject immediately with bounded limit error
    let path_1001 = temp_dir.join("stress_temp_1001.bin");
    {
        let mut f = std::fs::File::create(&path_1001).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1001u32.to_le_bytes()).unwrap();
    }
    let res_1001 = TempForecast::read_from_file(path_1001.to_str().unwrap());
    assert!(res_1001.is_err());
    assert!(res_1001
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_1001);

    // 4. steps_len = 2^31 - 1 (0x7FFFFFFF) -> must reject immediately without OOM
    let path_2_31 = temp_dir.join("stress_temp_2_31.bin");
    {
        let mut f = std::fs::File::create(&path_2_31).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0x7FFFFFFFu32.to_le_bytes()).unwrap();
    }
    let res_2_31 = TempForecast::read_from_file(path_2_31.to_str().unwrap());
    assert!(res_2_31.is_err());
    assert!(res_2_31
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_2_31);

    // 5. steps_len = 2^32 - 1 (0xFFFFFFFF) -> must reject immediately without OOM
    let path_2_32 = temp_dir.join("stress_temp_2_32.bin");
    {
        let mut f = std::fs::File::create(&path_2_32).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0xFFFFFFFFu32.to_le_bytes()).unwrap();
    }
    let res_2_32 = TempForecast::read_from_file(path_2_32.to_str().unwrap());
    assert!(res_2_32.is_err());
    assert!(res_2_32
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_2_32);
}

#[test]
fn test_stress_binary_cache_steps_len_boundaries_wind() {
    let temp_dir = std::env::temp_dir();

    // 1. steps_len = 0
    let path_0 = temp_dir.join("stress_wind_0.bin");
    {
        let mut f = std::fs::File::create(&path_0).unwrap();
        f.write_all(b"HRW2").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap();
    }
    let res_0 = WindForecast::read_from_file(path_0.to_str().unwrap());
    assert!(res_0.is_ok());
    assert_eq!(res_0.unwrap().steps.len(), 0);
    let _ = std::fs::remove_file(path_0);

    // 2. steps_len = 1000
    let path_1000 = temp_dir.join("stress_wind_1000.bin");
    {
        let mut f = std::fs::File::create(&path_1000).unwrap();
        f.write_all(b"HRW2").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1000u32.to_le_bytes()).unwrap();
    }
    let res_1000 = WindForecast::read_from_file(path_1000.to_str().unwrap());
    assert!(res_1000.is_err());
    let _ = std::fs::remove_file(path_1000);

    // 3. steps_len = 1001
    let path_1001 = temp_dir.join("stress_wind_1001.bin");
    {
        let mut f = std::fs::File::create(&path_1001).unwrap();
        f.write_all(b"HRW2").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1001u32.to_le_bytes()).unwrap();
    }
    let res_1001 = WindForecast::read_from_file(path_1001.to_str().unwrap());
    assert!(res_1001.is_err());
    assert!(res_1001
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_1001);

    // 4. steps_len = 2^31 - 1
    let path_2_31 = temp_dir.join("stress_wind_2_31.bin");
    {
        let mut f = std::fs::File::create(&path_2_31).unwrap();
        f.write_all(b"HRW2").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0x7FFFFFFFu32.to_le_bytes()).unwrap();
    }
    let res_2_31 = WindForecast::read_from_file(path_2_31.to_str().unwrap());
    assert!(res_2_31.is_err());
    assert!(res_2_31
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_2_31);

    // 5. steps_len = 2^32 - 1
    let path_2_32 = temp_dir.join("stress_wind_2_32.bin");
    {
        let mut f = std::fs::File::create(&path_2_32).unwrap();
        f.write_all(b"HRW2").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0xFFFFFFFFu32.to_le_bytes()).unwrap();
    }
    let res_2_32 = WindForecast::read_from_file(path_2_32.to_str().unwrap());
    assert!(res_2_32.is_err());
    assert!(res_2_32
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_2_32);
}

#[test]
fn test_stress_binary_cache_steps_len_boundaries_solar_and_rain() {
    let temp_dir = std::env::temp_dir();

    // Solar 0 and 1001
    let path_solar_0 = temp_dir.join("stress_solar_0.bin");
    {
        let mut f = std::fs::File::create(&path_solar_0).unwrap();
        f.write_all(b"HRMS").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap();
    }
    assert!(SolarForecast::read_from_file(path_solar_0.to_str().unwrap()).is_ok());
    let _ = std::fs::remove_file(path_solar_0);

    let path_solar_1001 = temp_dir.join("stress_solar_1001.bin");
    {
        let mut f = std::fs::File::create(&path_solar_1001).unwrap();
        f.write_all(b"HRMS").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1001u32.to_le_bytes()).unwrap();
    }
    let res_solar_1001 = SolarForecast::read_from_file(path_solar_1001.to_str().unwrap());
    assert!(res_solar_1001.is_err());
    assert!(res_solar_1001
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_solar_1001);

    // Rain 0 and 0xFFFFFFFF
    let path_rain_0 = temp_dir.join("stress_rain_0.bin");
    {
        let mut f = std::fs::File::create(&path_rain_0).unwrap();
        f.write_all(b"HRMR").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap();
    }
    assert!(RainForecast::read_from_file(path_rain_0.to_str().unwrap()).is_ok());
    let _ = std::fs::remove_file(path_rain_0);

    let path_rain_huge = temp_dir.join("stress_rain_huge.bin");
    {
        let mut f = std::fs::File::create(&path_rain_huge).unwrap();
        f.write_all(b"HRMR").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&0xFFFFFFFFu32.to_le_bytes()).unwrap();
    }
    let res_rain_huge = RainForecast::read_from_file(path_rain_huge.to_str().unwrap());
    assert!(res_rain_huge.is_err());
    assert!(res_rain_huge
        .unwrap_err()
        .to_string()
        .contains("exceeds maximum limit"));
    let _ = std::fs::remove_file(path_rain_huge);
}

#[test]
fn test_stress_binary_cache_corrupted_magic_and_truncated_files() {
    let temp_dir = std::env::temp_dir();

    // 1. Zero-byte file
    let path_empty = temp_dir.join("stress_empty.bin");
    {
        let _ = std::fs::File::create(&path_empty).unwrap();
    }
    assert!(TempForecast::read_from_file(path_empty.to_str().unwrap()).is_err());
    assert!(WindForecast::read_from_file(path_empty.to_str().unwrap()).is_err());
    assert!(SolarForecast::read_from_file(path_empty.to_str().unwrap()).is_err());
    assert!(RainForecast::read_from_file(path_empty.to_str().unwrap()).is_err());
    let _ = std::fs::remove_file(path_empty);

    // 2. Partial magic bytes (1 to 3 bytes)
    for len in 1..=3 {
        let path_partial = temp_dir.join(format!("stress_partial_{}.bin", len));
        {
            let mut f = std::fs::File::create(&path_partial).unwrap();
            f.write_all(&vec![b'H'; len]).unwrap();
        }
        assert!(TempForecast::read_from_file(path_partial.to_str().unwrap()).is_err());
        let _ = std::fs::remove_file(path_partial);
    }

    // 3. Invalid magic bytes ("BADD", "NULL", "1234")
    let path_bad_magic = temp_dir.join("stress_bad_magic.bin");
    {
        let mut f = std::fs::File::create(&path_bad_magic).unwrap();
        f.write_all(b"BADD").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap();
    }
    let err = TempForecast::read_from_file(path_bad_magic.to_str().unwrap()).unwrap_err();
    assert!(err.to_string().contains("Invalid magic bytes"));
    let _ = std::fs::remove_file(path_bad_magic);

    // 4. Truncated step header: magic + ref_time + steps_len=1, but file ends immediately
    let path_trunc_header = temp_dir.join("stress_trunc_header.bin");
    {
        let mut f = std::fs::File::create(&path_trunc_header).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap();
    }
    assert!(TempForecast::read_from_file(path_trunc_header.to_str().unwrap()).is_err());
    let _ = std::fs::remove_file(path_trunc_header);

    // 5. Corrupted grid dimensions (width=999, height=999 != GRIB_WIDTH, GRIB_HEIGHT)
    let path_bad_dim = temp_dir.join("stress_bad_dim.bin");
    {
        let mut f = std::fs::File::create(&path_bad_dim).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap();
        f.write_all(&0i32.to_le_bytes()).unwrap(); // forecast_hour
        f.write_all(&999u32.to_le_bytes()).unwrap(); // width != 390
        f.write_all(&999u32.to_le_bytes()).unwrap(); // height != 390
    }
    let dim_err = TempForecast::read_from_file(path_bad_dim.to_str().unwrap()).unwrap_err();
    assert!(dim_err.to_string().contains("Invalid grid dimensions"));
    let _ = std::fs::remove_file(path_bad_dim);

    // 6. Truncated pixel buffer: step header valid (width=390, height=390), but only 100 bytes written
    let path_trunc_buf = temp_dir.join("stress_trunc_buf.bin");
    {
        let mut f = std::fs::File::create(&path_trunc_buf).unwrap();
        f.write_all(b"HRMT").unwrap();
        f.write_all(&1700000000i64.to_le_bytes()).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap();
        f.write_all(&0i32.to_le_bytes()).unwrap();
        f.write_all(&(GRIB_WIDTH as u32).to_le_bytes()).unwrap();
        f.write_all(&(GRIB_HEIGHT as u32).to_le_bytes()).unwrap();
        f.write_all(&[0u8; 100]).unwrap(); // Incomplete pixel buffer!
    }
    assert!(TempForecast::read_from_file(path_trunc_buf.to_str().unwrap()).is_err());
    let _ = std::fs::remove_file(path_trunc_buf);
}

// ==============================================================================
// 2. PMM REDUCTION & ENSEMBLE STAT STRESS TESTS
// ==============================================================================

#[test]
fn test_stress_pmm_reduction_member_counts_and_extremes() {
    // 1. Zero members
    let mut empty_vals: [u16; 0] = [];
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut empty_vals), NODATA);

    // 2. Single member: valid values
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut [0]), 0);
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut [5000]), 5000);
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut [65534]), 65534);

    // 3. Single member: NODATA
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut [NODATA]), NODATA);

    // 4. 20 members: all NODATA
    let mut all_nodata = [NODATA; 20];
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut all_nodata), NODATA);

    // 5. 20 members: first member NODATA
    let mut first_nodata = [NODATA; 20];
    first_nodata[1] = 500;
    first_nodata[2] = 1000;
    assert_eq!(
        reduce_ensemble(&EnsembleStat::Pmm, &mut first_nodata),
        NODATA
    );

    // 6. 20 members: interspersed NODATA (defensive mean of valid members)
    let mut mixed = [NODATA; 20];
    mixed[0] = 100;
    mixed[2] = 200;
    mixed[4] = 300;
    // 3 valid values: 100, 200, 300 -> mean = 200
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut mixed), 200);

    // 7. Extreme maximum values (checking 64-bit sum accumulator safety)
    let mut extreme_vals = [65534u16; 20];
    assert_eq!(
        reduce_ensemble(&EnsembleStat::Pmm, &mut extreme_vals),
        65534
    );

    // 8. Alternating 0 and 65534
    let mut alt_vals = [0u16; 20];
    for i in (0..20).step_by(2) {
        alt_vals[i] = 65534;
    }
    // 10 values of 65534, 10 values of 0 -> mean = 32767
    assert_eq!(reduce_ensemble(&EnsembleStat::Pmm, &mut alt_vals), 32767);
}

#[test]
fn test_stress_all_ensemble_stats_with_adversarial_slices() {
    let stats = [
        EnsembleStat::Median,
        EnsembleStat::Maximum,
        EnsembleStat::Probability,
        EnsembleStat::Spread,
        EnsembleStat::Pmm,
    ];

    for stat in &stats {
        // Zero length
        let mut empty: [u16; 0] = [];
        assert_eq!(reduce_ensemble(stat, &mut empty), NODATA);

        // Single valid value
        let mut single_valid = [1500u16];
        let res_single = reduce_ensemble(stat, &mut single_valid);
        assert_ne!(res_single, NODATA);

        // Single NODATA value
        let mut single_nodata = [NODATA];
        assert_eq!(reduce_ensemble(stat, &mut single_nodata), NODATA);

        // All NODATA
        let mut all_nd = [NODATA; 20];
        assert_eq!(reduce_ensemble(stat, &mut all_nd), NODATA);

        // First NODATA
        let mut first_nd = [NODATA, 50, 100, 150];
        assert_eq!(reduce_ensemble(stat, &mut first_nd), NODATA);

        // Maximum values
        let mut max_vals = [65534u16; 20];
        let _ = reduce_ensemble(stat, &mut max_vals);
    }
}

// ==============================================================================
// 3. COLD BOOT & CONCURRENT MQTT NOTIFICATION STRESS TESTS
// ==============================================================================

#[tokio::test]
async fn test_stress_cold_boot_state_uninitialized_endpoints() {
    let empty_state = create_empty_app_state();
    let app = create_test_router(empty_state);

    // 1. Radar metadata -> 500 (Subsystem not loaded yet during cold boot)
    let req = Request::builder()
        .uri("/api/metadata")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // 2. Point inspection -> 500 (Radar metadata not loaded)
    let req = Request::builder()
        .uri("/api/value?ens=med&lat=52.0&lon=5.0&time=0")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // 3. Temp, Wind, Solar metadata -> 500 (Harmonie forecasts not loaded)
    for var in &["temp", "wind", "solar"] {
        let req = Request::builder()
            .uri(format!("/api/metadata/{}", var))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // 4. Temp, Wind, Solar value queries -> 500 (Harmonie forecasts not loaded)
    for var in &["temp", "wind", "solar"] {
        let req = Request::builder()
            .uri(format!("/api/value/{}?lat=52.0&lon=5.0&time=0", var))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // 5. Timeseries queries -> 500 (Forecasts not loaded)
    let req = Request::builder()
        .uri("/api/timeseries?ens=med&lat=52.0&lon=5.0")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

    for var in &["temp", "wind", "solar"] {
        let req = Request::builder()
            .uri(format!("/api/timeseries/{}?lat=52.0&lon=5.0", var))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}

#[tokio::test]
async fn test_stress_concurrent_rapid_mqtt_burst_and_atomic_swapping() {
    let state = create_mock_app_state();
    let tracker = Arc::new(AtomicU64::new(0));

    // Simulate 50 rapid sequential/concurrent MQTT dataset arrival notifications
    let mut handles = Vec::new();

    for version in 1..=50u64 {
        let state_clone = state.clone();
        let tracker_clone = tracker.clone();

        let handle = tokio::spawn(async move {
            // Update atomic target version
            tracker_clone.store(version, Ordering::Relaxed);

            // Construct new staged RadarData
            let times = vec![0, 300, 600];
            let ensembles = vec![0, 1, 2];
            let meta = Metadata {
                left: MERCATOR_LEFT,
                right: MERCATOR_RIGHT,
                bottom: MERCATOR_BOTTOM,
                top: MERCATOR_TOP,
                width: GRID_W,
                height: GRID_H,
                ensembles,
                times: times.clone(),
                reference_time_str: format!("seconds since 2026-09-01 00:00:{:02}", version % 60),
                version,
                radar_times_len: times.len(),
            };

            let staged = Arc::new(RadarData::new(format!("radar_v{}.nc", version), meta));

            // Populate some cache items with correct forecast grid dimensions (780 x 780)
            let dummy_grid = Arc::new(vec![version as u16 * 10; FORECAST_GRID_W * FORECAST_GRID_H]);
            staged
                .grid_cache
                .insert(("med".to_string(), 0), dummy_grid.clone());
            let webp = render_data_webp_bytes(&dummy_grid, &state_clone.projection_lut);
            staged.data_cache.insert(("med".to_string(), 0), webp);

            // Check if still latest before activating
            if tracker_clone.load(Ordering::Relaxed) == version {
                let mut guard = state_clone.radar_data.write().await;
                *guard = Some(staged);
            }
        });

        handles.push(handle);
    }

    // Simultaneously bombard the server with read requests during active dataset swaps
    let app = create_test_router(state.clone());
    let mut read_handles = Vec::new();
    for _ in 0..50 {
        let app_clone = app.clone();
        let read_handle = tokio::spawn(async move {
            let req = Request::builder()
                .uri("/api/metadata")
                .body(Body::empty())
                .unwrap();
            let res = app_clone.oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        });
        read_handles.push(read_handle);
    }

    for h in handles {
        h.await.unwrap();
    }
    for rh in read_handles {
        rh.await.unwrap();
    }

    // Verify final state is consistent and not poisoned
    let radar_guard = state.radar_data.read().await;
    assert!(radar_guard.is_some());
    let active_radar = radar_guard.as_ref().unwrap();
    assert!(active_radar.metadata.version > 0);
}

// ==============================================================================
// 4. AXUM API ENDPOINT STRESS TESTS
// ==============================================================================

#[tokio::test]
async fn test_stress_axum_missing_and_extreme_timestamps() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let bad_timestamps = [
        "-999999999999",
        "-10000",
        "-600", // not in actuals store
        "999999999999",
        "12345678",
    ];

    for &ts in &bad_timestamps {
        // 1. Radar tile -> 200 (empty transparent WebP image) or 404
        let req = Request::builder()
            .uri(format!("/api/data/med/{}", ts))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert!(res.status() == StatusCode::OK || res.status() == StatusCode::NOT_FOUND);
        if res.status() == StatusCode::OK {
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(!bytes.is_empty(), "WebP bytes must not be empty");
        }

        // 2. Temp tile -> 404 (for negative timestamps or nonexistent hours)
        let req = Request::builder()
            .uri(format!("/api/data/temp/{}", ts))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert!(res.status() == StatusCode::OK || res.status() == StatusCode::NOT_FOUND);

        // 3. Wind tile -> 404 or OK nearest
        let req = Request::builder()
            .uri(format!("/api/data/wind/10/{}", ts))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert!(res.status() == StatusCode::OK || res.status() == StatusCode::NOT_FOUND);

        // 4. Solar tile -> 404 or OK nearest
        let req = Request::builder()
            .uri(format!("/api/data/solar/{}", ts))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert!(res.status() == StatusCode::OK || res.status() == StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn test_stress_axum_extreme_and_special_coordinates() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let test_coords = [
        ("999.0", "999.0", "Extreme Out-of-Grid (+999)"),
        ("-999.0", "-999.0", "Extreme Out-of-Grid (-999)"),
        ("0.0", "0.0", "Null Island"),
        ("90.0", "0.0", "North Pole"),
        ("-90.0", "0.0", "South Pole"),
        ("85.05112878", "180.0", "Web Mercator North-East Bound"),
        ("-85.05112878", "-180.0", "Web Mercator South-West Bound"),
        ("NaN", "NaN", "NaN coordinates"),
        ("inf", "inf", "Positive infinity coordinates"),
        ("-inf", "-inf", "Negative infinity coordinates"),
        ("1e30", "1e30", "Large float coordinates (1e30)"),
        ("-1e30", "-1e30", "Large negative float coordinates (-1e30)"),
    ];

    for (lat, lon, desc) in &test_coords {
        // 1. /api/value
        let req = Request::builder()
            .uri(format!("/api/value?ens=med&lat={}&lon={}&time=0", lat, lon))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/value for {}",
            desc
        );
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            json_body["value"].is_null(),
            "Expected null value for out-of-grid coord: {}",
            desc
        );

        // 2. /api/value/temp
        let req = Request::builder()
            .uri(format!("/api/value/temp?lat={}&lon={}&time=0", lat, lon))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/value/temp for {}",
            desc
        );
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            json_body["value"].is_null(),
            "Expected null temp for {}",
            desc
        );

        // 3. /api/value/wind
        let req = Request::builder()
            .uri(format!(
                "/api/value/wind?lat={}&lon={}&time=0&height=10",
                lat, lon
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/value/wind for {}",
            desc
        );
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            json_body["speed"].is_null(),
            "Expected null wind speed for {}",
            desc
        );
        assert!(
            json_body["direction"].is_null(),
            "Expected null wind direction for {}",
            desc
        );

        // 4. /api/value/solar
        let req = Request::builder()
            .uri(format!("/api/value/solar?lat={}&lon={}&time=0", lat, lon))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/value/solar for {}",
            desc
        );
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            json_body["value"].is_null(),
            "Expected null solar for {}",
            desc
        );

        // 5. /api/timeseries
        let req = Request::builder()
            .uri(format!("/api/timeseries?ens=med&lat={}&lon={}", lat, lon))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/timeseries for {}",
            desc
        );

        // 6. /api/timeseries/temp
        let req = Request::builder()
            .uri(format!("/api/timeseries/temp?lat={}&lon={}", lat, lon))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/timeseries/temp for {}",
            desc
        );

        // 7. /api/timeseries/wind
        let req = Request::builder()
            .uri(format!(
                "/api/timeseries/wind?lat={}&lon={}&height=10",
                lat, lon
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/timeseries/wind for {}",
            desc
        );

        // 8. /api/timeseries/solar
        let req = Request::builder()
            .uri(format!("/api/timeseries/solar?lat={}&lon={}", lat, lon))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "Failed on /api/timeseries/solar for {}",
            desc
        );
    }
}

#[tokio::test]
async fn test_stress_axum_malformed_route_parameters_and_types() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    // 1. Unknown ensemble stat / bad param -> 400
    let req = Request::builder()
        .uri("/api/data/unknown_ensemble/0")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 2. Non-numeric wind height -> 400 or 422
    let req = Request::builder()
        .uri("/api/data/wind/not_a_number/0")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(
        res.status() == StatusCode::BAD_REQUEST || res.status() == StatusCode::UNPROCESSABLE_ENTITY
    );

    // 3. Unrecognized wind height level (e.g. 9999m) -> 404 (No matching step)
    let req = Request::builder()
        .uri("/api/data/wind/9999/0")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 4. Missing required query parameters on /api/value -> 400 or 422
    let req = Request::builder()
        .uri("/api/value")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(
        res.status() == StatusCode::BAD_REQUEST || res.status() == StatusCode::UNPROCESSABLE_ENTITY
    );

    // 5. Malformed string query parameter where float expected -> 400 or 422
    let req = Request::builder()
        .uri("/api/value?ens=med&lat=abc&lon=def&time=0")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(
        res.status() == StatusCode::BAD_REQUEST || res.status() == StatusCode::UNPROCESSABLE_ENTITY
    );
}

// ==============================================================================
// 5. EMPIRICAL BUG RESOLUTION VERIFICATION TESTS
// ==============================================================================

/// Verifies Resolution of Bug Finding 1: interpolate_bilinear handles infinite / large float inputs safely without overflow panics
#[test]
fn test_reproduce_bug_1_interpolation_overflow_on_inf_coordinate() {
    let dummy_values = vec![100u16; 390 * 390];
    // fx = +inf, -inf, NaN, 1e30 must return NODATA cleanly and not panic
    assert_eq!(
        interpolate_bilinear(f64::INFINITY, 0.0, 390, 390, &dummy_values),
        NODATA
    );
    assert_eq!(
        interpolate_bilinear(f64::NEG_INFINITY, 0.0, 390, 390, &dummy_values),
        NODATA
    );
    assert_eq!(
        interpolate_bilinear(f64::NAN, 0.0, 390, 390, &dummy_values),
        NODATA
    );
    assert_eq!(
        interpolate_bilinear(1e30, 0.0, 390, 390, &dummy_values),
        NODATA
    );
    assert_eq!(
        interpolate_bilinear(0.0, 1e30, 390, 390, &dummy_values),
        NODATA
    );
}

/// Verifies Resolution of Bug Finding 2: lat=NaN, lon=NaN in /api/value returns out_of_bounds with null value instead of aliasing to cell (0, 0)
#[tokio::test]
async fn test_reproduce_bug_2_radar_value_nan_coordinate_aliasing() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req = Request::builder()
        .uri("/api/value?ens=med&lat=NaN&lon=NaN&time=0")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json_body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(json_body["status"], "out_of_bounds");
    assert!(json_body["value"].is_null());
}
