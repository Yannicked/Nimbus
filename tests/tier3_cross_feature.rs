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

    let mut actuals = ActualsData::new();
    let act_len = RTCOR_GRID_W * RTCOR_GRID_H;
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
// Tier 3: Pairwise Combinatorial Tests (18 tests)
// =========================================================================

#[tokio::test]
async fn test_t3_01_concurrent_api_requests_during_mqtt_dataset_swap() {
    let state = create_mock_app_state();
    let app = create_test_router(state.clone());

    let app_clone = app.clone();
    let query_task = tokio::spawn(async move {
        for _ in 0..25 {
            let req = Request::builder()
                .uri("/api/value?ens=med&time=0&lon=5.2&lat=52.1")
                .body(Body::empty())
                .unwrap();
            let res = app_clone.clone().oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
    });

    let swap_task = tokio::spawn(async move {
        for i in 0..10 {
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
            let current_radar = state.radar_data.read().await.clone().unwrap();
            let mut new_meta = current_radar.metadata.clone();
            new_meta.version = 2000 + i;
            let staged = Arc::new(RadarData::new("new_swap.nc".to_string(), new_meta));
            *state.radar_data.write().await = Some(staged);
        }
    });

    let (r1, r2) = tokio::join!(query_task, swap_task);
    assert!(r1.is_ok());
    assert!(r2.is_ok());
}

#[tokio::test]
async fn test_t3_02_rapid_layer_mode_transition_and_point_inspection() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let endpoints = [
        "/api/value?ens=med&time=0&lon=5.2&lat=52.1",
        "/api/value/temp?time=0&lon=5.2&lat=52.1",
        "/api/value/wind?time=0&lon=5.2&lat=52.1&height=10",
        "/api/value/solar?time=0&lon=5.2&lat=52.1",
    ];

    for _ in 0..5 {
        for &uri in &endpoints {
            let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
            let res = app.clone().oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
    }
}

#[tokio::test]
async fn test_t3_03_compare_mode_dual_layer_state_synchronization() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    // Left map query: Rain
    let req_left = Request::builder()
        .uri("/api/value?ens=pmm&time=0&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_left = app.clone().oneshot(req_left).await.unwrap();
    assert_eq!(res_left.status(), StatusCode::OK);

    // Right map query: Wind
    let req_right = Request::builder()
        .uri("/api/value/wind?time=0&lon=5.2&lat=52.1&height=50")
        .body(Body::empty())
        .unwrap();
    let res_right = app.oneshot(req_right).await.unwrap();
    assert_eq!(res_right.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_t3_04_radar_actuals_to_forecast_transition_crossing_zero() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    // Request timeseries covering negative actuals (-300) and forecast (0, 300, 600)
    let req = Request::builder()
        .uri("/api/timeseries?ens=med&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let ts_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let times = ts_res["times"].as_array().unwrap();
    assert!(times.iter().any(|t| t.as_i64().unwrap() < 0));
    assert!(times.iter().any(|t| t.as_i64().unwrap() >= 0));
}

#[tokio::test]
async fn test_t3_05_extended_harmonie_forecast_stitching_with_radar_ensemble() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    // Request PMM timeseries (which extends into Harmonie forecast past radar)
    let req = Request::builder()
        .uri("/api/timeseries?ens=pmm&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let ts_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(ts_res["status"], "ok");
    assert!(!ts_res["values"].as_array().unwrap().is_empty());
}

#[test]
fn test_t3_06_pmm_reduction_under_extreme_spatial_gradients() {
    let member1 = [0u16, 5000, 0]; // 50 mm/h cell next to dry cells
    let member2 = [0u16, 4000, 0];
    let member3 = [0u16, 6000, 0];

    // Verify statistical sanity across gradient
    let med_center = reduce_ensemble(
        &EnsembleStat::Median,
        &mut [member1[1], member2[1], member3[1]],
    );
    assert_eq!(med_center, 5000);

    let max_center = reduce_ensemble(
        &EnsembleStat::Maximum,
        &mut [member1[1], member2[1], member3[1]],
    );
    assert_eq!(max_center, 6000);
}

#[tokio::test]
async fn test_t3_07_rtcor_backfill_interleaving_with_active_image_render() {
    let state = create_mock_app_state();
    let app = create_test_router(state.clone());

    // Insert new actuals frame
    let frame = ActualsFrame {
        timestamp: 1700000000 - 600,
        raw_values: Arc::new(vec![250u16; RTCOR_GRID_W * RTCOR_GRID_H]),
        webp_bytes: vec![0u8; 64],
    };
    if let Some(ref mut act) = *state.actuals_data.write().await {
        Arc::make_mut(act).insert_or_update(frame, 24);
    }

    // Simultaneously fetch image
    let req = Request::builder()
        .uri("/api/data/med/-600")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_t3_08_wind_multi_height_and_direction_consistency() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    for &h in &[10, 50, 100, 200, 300] {
        let req = Request::builder()
            .uri(format!(
                "/api/value/wind?time=0&lon=5.2&lat=52.1&height={}",
                h
            ))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let wind_res: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(wind_res["status"], "ok");
        assert!(wind_res["speed"].as_f64().unwrap() >= 0.0);
        assert!(wind_res["direction"].as_f64().unwrap() >= 0.0);
    }
}

#[tokio::test]
async fn test_t3_09_timeseries_caching_and_invalidation_across_dataset_swaps() {
    let state = create_mock_app_state();
    let app = create_test_router(state.clone());

    // Initial query caches timeseries
    let req1 = Request::builder()
        .uri("/api/timeseries?ens=med&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);

    // Swap dataset
    let current_radar = state.radar_data.read().await.clone().unwrap();
    let mut new_meta = current_radar.metadata.clone();
    new_meta.version = 99999;
    let new_radar = Arc::new(RadarData::new("swapped.nc".to_string(), new_meta));
    *state.radar_data.write().await = Some(new_radar);

    // Subsequent query should read from new dataset state
    let req2 = Request::builder()
        .uri("/api/metadata")
        .body(Body::empty())
        .unwrap();
    let res2 = app.oneshot(req2).await.unwrap();
    let body_bytes = axum::body::to_bytes(res2.into_body(), usize::MAX)
        .await
        .unwrap();
    let meta: Metadata = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(meta.version, 99999);
}

#[tokio::test]
async fn test_t3_10_grid_cache_hit_vs_miss_concurrency() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let mut handles = Vec::new();
    for t in &[0, 300, 600] {
        let app_clone = app.clone();
        let uri = format!("/api/data/med/{}", t);
        handles.push(tokio::spawn(async move {
            let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
            let res = app_clone.oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
}

#[tokio::test]
async fn test_t3_11_out_of_grid_inspection_across_all_layers() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let uris = [
        "/api/value?ens=med&time=0&lon=0.0&lat=0.0",
        "/api/value/temp?time=0&lon=0.0&lat=0.0",
        "/api/value/wind?time=0&lon=0.0&lat=0.0&height=10",
        "/api/value/solar?time=0&lon=0.0&lat=0.0",
    ];

    for &uri in &uris {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["status"], "out_of_bounds");
    }
}

#[test]
fn test_t3_12_bilinear_interpolation_near_polar_projection_edges() {
    let grid_w = 700;
    let grid_h = 765;
    let values = vec![500u16; grid_w * grid_h];

    // Query corner coordinates right on the bounds: (0.1, 0.1), (698.9, 763.9)
    let v_sw = interpolate_bilinear(0.1, 0.1, grid_w, grid_h, &values);
    assert_eq!(v_sw, 500);

    let v_ne = interpolate_bilinear(698.9, 763.9, grid_w, grid_h, &values);
    assert_eq!(v_ne, 500);
}

#[test]
fn test_t3_13_corrupted_binary_cache_fallback_resilience() {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir
        .join("test_t3_corrupted.bin")
        .to_string_lossy()
        .to_string();

    std::fs::write(&file_path, b"CORRUPTED_GARBAGE_PAYLOAD").expect("Write failed");
    assert!(TempForecast::read_from_file(&file_path).is_err());
    assert!(WindForecast::read_from_file(&file_path).is_err());

    let _ = std::fs::remove_file(file_path);
}

#[tokio::test]
async fn test_t3_14_rapid_ensemble_selector_cycling() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let stats = ["med", "max", "prob", "spread", "pmm", "0", "1"];
    for stat in &stats {
        let req = Request::builder()
            .uri(format!("/api/value?ens={}&time=0&lon=5.2&lat=52.1", stat))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn test_t3_15_extreme_storm_point_query_with_gale_wind() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req_rain = Request::builder()
        .uri("/api/value?ens=max&time=0&lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_rain = app.clone().oneshot(req_rain).await.unwrap();
    assert_eq!(res_rain.status(), StatusCode::OK);

    let req_wind = Request::builder()
        .uri("/api/value/wind?time=0&lon=5.2&lat=52.1&height=10")
        .body(Body::empty())
        .unwrap();
    let res_wind = app.oneshot(req_wind).await.unwrap();
    assert_eq!(res_wind.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_t3_16_solar_and_temperature_correlation_inspection() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    let req_temp = Request::builder()
        .uri("/api/timeseries/temp?lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_temp = app.clone().oneshot(req_temp).await.unwrap();
    assert_eq!(res_temp.status(), StatusCode::OK);

    let req_solar = Request::builder()
        .uri("/api/timeseries/solar?lon=5.2&lat=52.1")
        .body(Body::empty())
        .unwrap();
    let res_solar = app.oneshot(req_solar).await.unwrap();
    assert_eq!(res_solar.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_t3_17_multi_variable_metadata_geometry_alignment() {
    let state = create_mock_app_state();
    let app = create_test_router(state);

    // Radar metadata
    let req_radar = Request::builder()
        .uri("/api/metadata")
        .body(Body::empty())
        .unwrap();
    let res_radar = app.clone().oneshot(req_radar).await.unwrap();
    let bytes_radar = axum::body::to_bytes(res_radar.into_body(), usize::MAX)
        .await
        .unwrap();
    let meta_radar: Metadata = serde_json::from_slice(&bytes_radar).unwrap();

    // Temp metadata
    let req_temp = Request::builder()
        .uri("/api/metadata/temp")
        .body(Body::empty())
        .unwrap();
    let res_temp = app.oneshot(req_temp).await.unwrap();
    let bytes_temp = axum::body::to_bytes(res_temp.into_body(), usize::MAX)
        .await
        .unwrap();
    let meta_temp: serde_json::Value = serde_json::from_slice(&bytes_temp).unwrap();

    // Geometrical alignment checks
    assert_eq!(
        meta_radar.width,
        meta_temp["width"].as_u64().unwrap() as u32
    );
    assert_eq!(
        meta_radar.height,
        meta_temp["height"].as_u64().unwrap() as u32
    );
    assert_eq!(meta_radar.left, meta_temp["left"].as_f64().unwrap());
    assert_eq!(meta_radar.right, meta_temp["right"].as_f64().unwrap());
    assert_eq!(meta_radar.bottom, meta_temp["bottom"].as_f64().unwrap());
    assert_eq!(meta_radar.top, meta_temp["top"].as_f64().unwrap());
}

#[tokio::test]
async fn test_t3_18_cold_boot_state_initialization_with_partial_layers() {
    let projection_lut = Arc::new(init_projection_lut());
    let actuals_lut = Arc::new(init_actuals_projection_lut());
    let grib_lut = Arc::new(init_temp_projection_lut());

    // Empty state without radar or actuals
    let partial_state = Arc::new(AppState {
        radar_data: RwLock::new(None),
        projection_lut,
        actuals_data: RwLock::new(None),
        actuals_projection_lut: actuals_lut,
        temp_data: RwLock::new(None),
        temp_projection_lut: grib_lut.clone(),
        wind_data: RwLock::new(None),
        wind_projection_lut: grib_lut.clone(),
        solar_data: RwLock::new(None),
        solar_projection_lut: grib_lut,
        rain_data: RwLock::new(None),
    });

    let app = create_test_router(partial_state);

    // Favicon should still work
    let req = Request::builder()
        .uri("/favicon.ico")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // Unloaded metadata should return 500 without crashing
    let req_meta = Request::builder()
        .uri("/api/metadata")
        .body(Body::empty())
        .unwrap();
    let res_meta = app.oneshot(req_meta).await.unwrap();
    assert_eq!(res_meta.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
