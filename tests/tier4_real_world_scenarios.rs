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

fn create_scenario_app_state() -> Arc<AppState> {
    let projection_lut = Arc::new(init_projection_lut());
    let actuals_lut = Arc::new(init_actuals_projection_lut());
    let grib_lut = Arc::new(init_temp_projection_lut());

    let ref_time = 1700000000i64;
    let times = vec![0, 300, 600, 900, 1200, 1500, 1800, 2100, 2400];
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
        version: 1700000000,
        radar_times_len: times.len(),
    };

    let radar_data = Arc::new(RadarData::new("scenario_radar.nc".to_string(), meta));

    let fc_len = FORECAST_GRID_W * FORECAST_GRID_H;
    for &t in &times {
        // Storm peak at t = 900 (15m)
        let intensity_mult = if t == 900 { 20 } else if t == 600 || t == 1200 { 10 } else { 2 };
        for e in 0..5 {
            let mut grid = vec![0u16; fc_len];
            let storm_center = fc_len / 2 + 390;
            for item in grid.iter_mut().skip(storm_center - 50).take(100) {
                *item = (250 * intensity_mult + (e as u16) * 50).min(65534);
            }
            radar_data.grid_cache.insert((e.to_string(), t), Arc::new(grid));
        }

        let mut med_grid = vec![0u16; fc_len];
        let mut max_grid = vec![0u16; fc_len];
        let mut pmm_grid = vec![0u16; fc_len];
        let storm_center = fc_len / 2 + 390;
        for idx in (storm_center - 50)..(storm_center + 50) {
            med_grid[idx] = (250 * intensity_mult).min(65534);
            max_grid[idx] = (300 * intensity_mult).min(65534);
            pmm_grid[idx] = (280 * intensity_mult).min(65534);
        }
        radar_data.grid_cache.insert(("med".to_string(), t), Arc::new(med_grid));
        radar_data.grid_cache.insert(("max".to_string(), t), Arc::new(max_grid));
        radar_data.grid_cache.insert(("pmm".to_string(), t), Arc::new(pmm_grid));
    }

    let temp_steps = (0..9)
        .map(|hour| {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let values = vec![(2931 + hour * 15) as u16; grib_len]; // 20 C to 33.5 C heat diurnal
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

    let mut wind_steps = Vec::new();
    for &h in &[10, 50, 100, 200, 300] {
        for hour in 0..9 {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            // Storm wind: up to 28 m/s at t=900 (hour 0-1)
            let u_val = if hour == 0 { 12500u16 } else { 10800u16 }; // 25 m/s vs 8 m/s
            let v_val = if hour == 0 { 11500u16 } else { 10200u16 }; // 15 m/s vs 2 m/s
            wind_steps.push(WindStep {
                forecast_hour: hour,
                height_level: h,
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                u_values: Arc::new(vec![u_val; grib_len]),
                v_values: Arc::new(vec![v_val; grib_len]),
            });
        }
    }
    let wind_fc = WindForecast {
        reference_time: ref_time,
        steps: wind_steps,
    };
    let wind_data = Arc::new(WindData::new(wind_fc));

    let solar_steps = (0..9)
        .map(|hour| {
            let grib_len = GRIB_WIDTH * GRIB_HEIGHT;
            let solar_val = if (2..=6).contains(&hour) { 850u16 } else { 100u16 };
            SolarStep {
                forecast_hour: hour,
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                values: Arc::new(vec![solar_val; grib_len]),
            }
        })
        .collect();
    let solar_fc = SolarForecast {
        reference_time: ref_time,
        steps: solar_steps,
    };
    let solar_data = Arc::new(SolarData::new(solar_fc));

    let rain_fc = RainForecast {
        reference_time: ref_time,
        steps: (0..9)
            .map(|hour| RainStep {
                forecast_hour: hour,
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                values: Arc::new(vec![100u16; GRIB_WIDTH * GRIB_HEIGHT]),
            })
            .collect(),
    };
    let rain_data = Arc::new(RainData::new(rain_fc));

    let mut actuals = ActualsData::new();
    let act_len = RTCOR_GRID_W * RTCOR_GRID_H;
    for offset in &[-1800i64, -1500, -1200, -900, -600, -300] {
        let frame = ActualsFrame {
            timestamp: ref_time + offset,
            raw_values: Arc::new(vec![350u16; act_len]), // 3.5 mm/h pre-storm actuals
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
// Scenario 1: Extreme Convective Storm Event (Rapid Rain & Wind Shift)
// Features: F01, F02, F03, F08, F11, F14, F15, F16
// =========================================================================

#[tokio::test]
async fn test_scenario_s1_extreme_convective_storm() {
    let state = create_scenario_app_state();
    let app = create_test_router(state);

    // 1. Inspect rain timeseries across storm passage (-30m actuals -> +40m forecast)
    let req_ts = Request::builder()
        .uri("/api/timeseries?ens=pmm&lon=5.64&lat=52.5")
        .body(Body::empty())
        .unwrap();
    let res_ts = app.clone().oneshot(req_ts).await.unwrap();
    assert_eq!(res_ts.status(), StatusCode::OK);

    let bytes_ts = axum::body::to_bytes(res_ts.into_body(), usize::MAX).await.unwrap();
    let ts_json: serde_json::Value = serde_json::from_slice(&bytes_ts).unwrap();
    let values = ts_json["values"].as_array().unwrap();
    assert!(!values.is_empty());

    // 2. Query peak rain point value during storm
    let req_val = Request::builder()
        .uri("/api/value?ens=max&time=900&lon=5.64&lat=52.5")
        .body(Body::empty())
        .unwrap();
    let res_val = app.clone().oneshot(req_val).await.unwrap();
    assert_eq!(res_val.status(), StatusCode::OK);

    // 3. Query wind during storm peak (severe gale gusts)
    let req_wind = Request::builder()
        .uri("/api/value/wind?time=0&lon=5.64&lat=52.5&height=10")
        .body(Body::empty())
        .unwrap();
    let res_wind = app.clone().oneshot(req_wind).await.unwrap();
    assert_eq!(res_wind.status(), StatusCode::OK);
    let bytes_wind = axum::body::to_bytes(res_wind.into_body(), usize::MAX).await.unwrap();
    let wind_json: serde_json::Value = serde_json::from_slice(&bytes_wind).unwrap();
    let wind_speed = wind_json["speed"].as_f64().unwrap();
    assert!(wind_speed > 20.0); // Severe gale > 20 m/s

    // 4. Fetch radar image at peak storm
    let req_img = Request::builder().uri("/api/data/pmm/900").body(Body::empty()).unwrap();
    let res_img = app.oneshot(req_img).await.unwrap();
    assert_eq!(res_img.status(), StatusCode::OK);
    assert_eq!(res_img.headers().get("Content-Type").unwrap(), "image/webp");
}

// =========================================================================
// Scenario 2: Multi-Model Run Transition During Active User Scrubbing
// Features: F05, F07, F10, F11, F12, F15, F17
// =========================================================================

#[tokio::test]
async fn test_scenario_s2_multi_model_run_transition_scrubbing() {
    let state = create_scenario_app_state();
    let app = create_test_router(state.clone());

    let app_clone = app.clone();
    // Timeline scrubbing worker simulation
    let scrub_task = tokio::spawn(async move {
        for _cycle in 0..3 {
            for &t in &[0, 300, 600, 900, 1200] {
                let req = Request::builder()
                    .uri(format!("/api/data/med/{}", t))
                    .body(Body::empty())
                    .unwrap();
                let res = app_clone.clone().oneshot(req).await.unwrap();
                assert_eq!(res.status(), StatusCode::OK);

                let req_val = Request::builder()
                    .uri(format!("/api/value?ens=med&time={}&lon=5.2&lat=52.1", t))
                    .body(Body::empty())
                    .unwrap();
                let res_val = app_clone.clone().oneshot(req_val).await.unwrap();
                assert_eq!(res_val.status(), StatusCode::OK);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    });

    // Model run swap simulation
    let swap_task = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        let radar_curr = state.radar_data.read().await.clone().unwrap();
        let mut new_meta = radar_curr.metadata.clone();
        new_meta.version = 1700003600; // New model run (+1 hour)
        let new_radar = Arc::new(RadarData::new("new_run.nc".to_string(), new_meta));
        let fc_len = FORECAST_GRID_W * FORECAST_GRID_H;
        for &t in &[0, 300, 600, 900, 1200] {
            new_radar.grid_cache.insert(("med".to_string(), t), Arc::new(vec![150u16; fc_len]));
        }
        *state.radar_data.write().await = Some(new_radar);
    });

    let (r1, r2) = tokio::join!(scrub_task, swap_task);
    assert!(r1.is_ok());
    assert!(r2.is_ok());

    // Verify metadata reflects the updated model run version
    let req_meta = Request::builder().uri("/api/metadata").body(Body::empty()).unwrap();
    let res_meta = app.oneshot(req_meta).await.unwrap();
    let bytes = axum::body::to_bytes(res_meta.into_body(), usize::MAX).await.unwrap();
    let meta: Metadata = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(meta.version, 1700003600);
}

// =========================================================================
// Scenario 3: Mobile Low-Memory Device Extended Exploration & Compare Mode
// Features: F09, F10, F12, F13, F14, F18
// =========================================================================

#[tokio::test]
async fn test_scenario_s3_mobile_low_memory_exploration_and_compare() {
    let state = create_scenario_app_state();
    let app = create_test_router(state);

    // Simulate 50 sequential image requests across 4 layer modes in split screen
    for i in 0..50 {
        let t = (i % 8) * 300;
        // Left screen: Rain image
        let req_l = Request::builder().uri(format!("/api/data/med/{}", t)).body(Body::empty()).unwrap();
        let res_l = app.clone().oneshot(req_l).await.unwrap();
        assert_eq!(res_l.status(), StatusCode::OK);

        // Right screen: Temp or Wind image
        let req_r = if i % 2 == 0 {
            Request::builder().uri(format!("/api/data/temp/{}", t)).body(Body::empty()).unwrap()
        } else {
            Request::builder().uri(format!("/api/data/wind/10/{}", t)).body(Body::empty()).unwrap()
        };
        let res_r = app.clone().oneshot(req_r).await.unwrap();
        assert_eq!(res_r.status(), StatusCode::OK);
    }
}

// =========================================================================
// Scenario 4: Offline / Corrupted Data Network Interruption & Recovery
// Features: F04, F05, F06, F07, F16, F18
// =========================================================================

#[tokio::test]
async fn test_scenario_s4_corrupted_data_interruption_and_recovery() {
    let state = create_scenario_app_state();
    let app = create_test_router(state.clone());

    // 1. Ingest corrupted temp forecast (empty steps)
    let corrupted_temp_fc = TempForecast {
        reference_time: 1700000000,
        steps: Vec::new(),
    };
    *state.temp_data.write().await = Some(Arc::new(TempData::new(corrupted_temp_fc)));

    // Requesting corrupted temp value returns graceful "no_data" rather than 500
    let req_bad = Request::builder().uri("/api/value/temp?time=0&lon=5.2&lat=52.1").body(Body::empty()).unwrap();
    let res_bad = app.clone().oneshot(req_bad).await.unwrap();
    assert_eq!(res_bad.status(), StatusCode::OK);
    let bytes_bad = axum::body::to_bytes(res_bad.into_body(), usize::MAX).await.unwrap();
    let json_bad: serde_json::Value = serde_json::from_slice(&bytes_bad).unwrap();
    assert_eq!(json_bad["status"], "no_data");

    // 2. Recovery: restore valid temp forecast
    let valid_temp_fc = TempForecast {
        reference_time: 1700000000,
        steps: vec![TempStep {
            forecast_hour: 0,
            width: GRIB_WIDTH,
            height: GRIB_HEIGHT,
            values: Arc::new(vec![2931u16; GRIB_WIDTH * GRIB_HEIGHT]),
        }],
    };
    *state.temp_data.write().await = Some(Arc::new(TempData::new(valid_temp_fc)));

    // Query recovered temp endpoint
    let req_ok = Request::builder().uri("/api/value/temp?time=0&lon=5.2&lat=52.1").body(Body::empty()).unwrap();
    let res_ok = app.oneshot(req_ok).await.unwrap();
    assert_eq!(res_ok.status(), StatusCode::OK);
    let bytes_ok = axum::body::to_bytes(res_ok.into_body(), usize::MAX).await.unwrap();
    let json_ok: serde_json::Value = serde_json::from_slice(&bytes_ok).unwrap();
    assert_eq!(json_ok["status"], "ok");
    assert!(json_ok["value"].as_f64().is_some());
}

// =========================================================================
// Scenario 5: High-Resolution Solar Radiation & 2m Temperature Spatial Inspection
// Features: F02, F07, F08, F14, F16, F17
// =========================================================================

#[tokio::test]
async fn test_scenario_s5_solar_and_temperature_spatial_inspection() {
    let state = create_scenario_app_state();
    let app = create_test_router(state);

    // 5 Major Dutch reference stations across geographic envelope
    let stations = [
        ("Amsterdam", 4.8951, 52.3702),
        ("Vlissingen", 3.5731, 51.4425),
        ("Leeuwarden", 5.7999, 53.2012),
        ("Maastricht", 5.6909, 50.8514),
        ("Enschede", 6.8958, 52.2215),
    ];

    for (name, lon, lat) in stations {
        // Query Temperature
        let req_temp = Request::builder()
            .uri(format!("/api/value/temp?time=0&lon={}&lat={}", lon, lat))
            .body(Body::empty())
            .unwrap();
        let res_temp = app.clone().oneshot(req_temp).await.unwrap();
        assert_eq!(res_temp.status(), StatusCode::OK, "Failed for {}", name);

        let bytes_t = axum::body::to_bytes(res_temp.into_body(), usize::MAX).await.unwrap();
        let json_t: serde_json::Value = serde_json::from_slice(&bytes_t).unwrap();
        assert_eq!(json_t["status"], "ok");
        assert!(json_t["value"].as_f64().unwrap() > 10.0);

        // Query Solar Irradiance
        let req_solar = Request::builder()
            .uri(format!("/api/value/solar?time=0&lon={}&lat={}", lon, lat))
            .body(Body::empty())
            .unwrap();
        let res_solar = app.clone().oneshot(req_solar).await.unwrap();
        assert_eq!(res_solar.status(), StatusCode::OK, "Failed for {}", name);

        let bytes_s = axum::body::to_bytes(res_solar.into_body(), usize::MAX).await.unwrap();
        let json_s: serde_json::Value = serde_json::from_slice(&bytes_s).unwrap();
        assert_eq!(json_s["status"], "ok");
        assert!(json_s["value"].as_f64().is_some());
    }
}
