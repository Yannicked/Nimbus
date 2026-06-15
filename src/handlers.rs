use std::sync::Arc;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::state::AppState;
use crate::constants::{
    KNMI_GRID_H, KNMI_GRID_W, KNMI_X0, KNMI_Y0, KNMI_DX, KNMI_DY,
    MERCATOR_LEFT, MERCATOR_RIGHT, MERCATOR_BOTTOM, MERCATOR_TOP,
    GRID_W, GRID_H, NODATA, PRECIP_VAR
};
use crate::models::{
    ValueQuery, ValueResponse, TimeseriesQuery, TimeseriesResponse,
    WindMetadata, WindValueQuery, WindValueResponse, WindTimeseriesQuery, WindTimeseriesResponse,
    TempMetadata, TempValueQuery, TempTimeseriesQuery, TempTimeseriesResponse,
    EnsembleStat, reduce_ensemble
};
use crate::projection;
use crate::rendering::{render_data_png_bytes, render_temp_png_bytes, render_wind_png_bytes};
use crate::interpolation::interpolate_bilinear;
use crate::radar::{compute_raw_slice, raw_to_value};

/// Serves an empty favicon response to prevent 404 console errors.
pub async fn favicon() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

/// Returns the current dataset metadata as JSON.
pub async fn get_metadata(
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

/// Serves the lossless R/G packed raw radar data PNG for a timeframe.
pub async fn get_data_image(
    Path((ens_str, time)): Path<(String, i64)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check cache
    if let Some(cached_data) = state.data_cache.get(&(ens_str.clone(), time)) {
        return Ok(Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "no-store, no-cache, must-revalidate")
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
    let state_clone = state.clone();
    let raw_slice_clone = raw_slice.clone();
    let png_bytes = tokio::task::spawn_blocking(move || {
        render_data_png_bytes(&raw_slice_clone, &state_clone.projection_lut)
    }).await.unwrap();

    // Cache results
    state
        .data_cache
        .insert((ens_str, time), png_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/png")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(png_bytes))
        .unwrap())
}

/// Returns the precipitation value (or ensemble statistic) at a single
/// geographic point as JSON.
pub async fn get_value(
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

    // Try reading from cache first
    if let Some(slice) = state.grid_cache.get(&(q.ens.clone(), q.time)) {
        let val_raw = slice[iy as usize * KNMI_GRID_W + ix as usize];
        let (status_out, value_out) = if q.ens == "prob" {
            ("probability".to_string(), val_raw as f64)
        } else {
            let val_mmh = raw_to_value(val_raw);
            let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
            (status.to_string(), val_mmh)
        };
        return Ok(axum::Json(ValueResponse {
            status: status_out,
            value: Some(value_out),
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
pub async fn get_timeseries(
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

    // Try reading all times from cache first
    let mut all_cached = true;
    for &time_val in &meta.times {
        if !state.grid_cache.contains_key(&(q.ens.clone(), time_val)) {
            all_cached = false;
            break;
        }
    }

    if all_cached {
        let mut values = Vec::with_capacity(meta.times.len());
        for &time_val in &meta.times {
            if let Some(slice) = state.grid_cache.get(&(q.ens.clone(), time_val)) {
                let val_raw = slice[iy as usize * KNMI_GRID_W + ix as usize];
                if q.ens == "prob" {
                    values.push(val_raw as f64);
                } else {
                    values.push(raw_to_value(val_raw));
                }
            }
        }
        return Ok(axum::Json(TimeseriesResponse {
            status: "ok".to_string(),
            lat: q.lat,
            lon: q.lon,
            ens: q.ens,
            times: meta.times.clone(),
            values,
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

pub async fn get_wind_metadata(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.wind_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    let mut times: Vec<i64> = forecast
        .steps
        .iter()
        .map(|s| (s.forecast_hour as i64) * 3600)
        .collect();
    times.sort_unstable();
    times.dedup();

    let reference_time_str = {
        use chrono::TimeZone;
        if let Some(utc_dt) = chrono::Utc.timestamp_opt(forecast.reference_time, 0).single() {
            format!("seconds since {}", utc_dt.format("%Y-%m-%d %H:%M:%S"))
        } else {
            "seconds since 1970-01-01 00:00:00".to_string()
        }
    };

    Ok(axum::Json(WindMetadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        times,
        reference_time: forecast.reference_time,
        reference_time_str,
        version: 1,
        heights: vec![10, 50, 100, 200, 300],
    }))
}

pub async fn get_wind_data_image(
    Path((height, time)): Path<(u32, i64)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if let Some(cached) = state.wind_data_cache.get(&(height, time)) {
        return Ok(Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "no-store, no-cache, must-revalidate")
            .body(axum::body::Body::from(cached.value().clone()))
            .unwrap());
    }

    let forecast_opt = state.wind_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    if forecast.steps.is_empty() {
        return Err((StatusCode::NOT_FOUND, "No wind forecast steps".to_string()));
    }

    let step = forecast
        .steps
        .iter()
        .filter(|s| s.height_level == height)
        .min_by_key(|s| {
            let step_offset = (s.forecast_hour as i64) * 3600;
            (step_offset - time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    let u_vals = step.u_values.clone();
    let v_vals = step.v_values.clone();
    let state_clone = state.clone();
    let png_bytes = tokio::task::spawn_blocking(move || {
        render_wind_png_bytes(&u_vals, &v_vals, &state_clone.wind_projection_lut)
    }).await.unwrap();
    state.wind_data_cache.insert((height, time), png_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/png")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(png_bytes))
        .unwrap())
}

pub async fn get_wind_data_image_legacy(
    Path(time): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    get_wind_data_image(Path((10, time)), State(state)).await
}

pub async fn get_wind_value(
    Query(q): Query<WindValueQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.wind_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    if forecast.steps.is_empty() {
        return Ok(axum::Json(WindValueResponse {
            status: "no_data".to_string(),
            u: None,
            v: None,
            speed: None,
            direction: None,
        }));
    }

    let req_height = q.height.unwrap_or(10);

    let step = forecast
        .steps
        .iter()
        .filter(|s| s.height_level == req_height)
        .min_by_key(|s| {
            let step_offset = (s.forecast_hour as i64) * 3600;
            (step_offset - q.time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    let fx = (q.lon - 0.0) / 0.029;
    let fy = (q.lat - 49.0) / 0.018;

    let u_raw = interpolate_bilinear(fx, fy, 390, 390, &step.u_values);
    let v_raw = interpolate_bilinear(fx, fy, 390, 390, &step.v_values);

    if u_raw == NODATA || v_raw == NODATA {
        Ok(axum::Json(WindValueResponse {
            status: "out_of_bounds".to_string(),
            u: None,
            v: None,
            speed: None,
            direction: None,
        }))
    } else {
        let u = u_raw as f64 / 100.0 - 100.0;
        let v = v_raw as f64 / 100.0 - 100.0;
        let speed = (u * u + v * v).sqrt();
        let mut dir_rad = u.atan2(v) + std::f64::consts::PI;
        if dir_rad < 0.0 {
            dir_rad += 2.0 * std::f64::consts::PI;
        }
        let direction = dir_rad.to_degrees();

        Ok(axum::Json(WindValueResponse {
            status: "ok".to_string(),
            u: Some(u),
            v: Some(v),
            speed: Some(speed),
            direction: Some(direction),
        }))
    }
}

pub async fn get_wind_timeseries(
    Query(q): Query<WindTimeseriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.wind_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    let fx = (q.lon - 0.0) / 0.029;
    let fy = (q.lat - 49.0) / 0.018;

    let mut times = Vec::new();
    let mut speeds = Vec::new();
    let mut directions = Vec::new();

    let req_height = q.height.unwrap_or(10);

    for step in &forecast.steps {
        if step.height_level != req_height {
            continue;
        }
        let u_raw = interpolate_bilinear(fx, fy, 390, 390, &step.u_values);
        let v_raw = interpolate_bilinear(fx, fy, 390, 390, &step.v_values);

        if u_raw != NODATA && v_raw != NODATA {
            let u = u_raw as f64 / 100.0 - 100.0;
            let v = v_raw as f64 / 100.0 - 100.0;
            let speed = (u * u + v * v).sqrt();
            let mut dir_rad = u.atan2(v) + std::f64::consts::PI;
            if dir_rad < 0.0 {
                dir_rad += 2.0 * std::f64::consts::PI;
            }
            let direction = dir_rad.to_degrees();

            let step_offset = (step.forecast_hour as i64) * 3600;
            times.push(step_offset);
            speeds.push(speed);
            directions.push(direction);
        }
    }

    Ok(axum::Json(WindTimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        times,
        speeds,
        directions,
    }))
}

pub async fn get_temp_metadata(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.temp_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    let times: Vec<i64> = forecast
        .steps
        .iter()
        .map(|s| (s.forecast_hour as i64) * 3600)
        .collect();

    let reference_time_str = {
        use chrono::TimeZone;
        if let Some(utc_dt) = chrono::Utc.timestamp_opt(forecast.reference_time, 0).single() {
            format!("seconds since {}", utc_dt.format("%Y-%m-%d %H:%M:%S"))
        } else {
            "seconds since 1970-01-01 00:00:00".to_string()
        }
    };

    Ok(axum::Json(TempMetadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        times,
        reference_time: forecast.reference_time,
        reference_time_str,
        version: forecast.reference_time as u64,
    }))
}

pub async fn get_temp_data_image(
    Path(time): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if let Some(cached) = state.temp_data_cache.get(&time) {
        return Ok(Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "no-store, no-cache, must-revalidate")
            .body(axum::body::Body::from(cached.value().clone()))
            .unwrap());
    }

    let forecast_opt = state.temp_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    if forecast.steps.is_empty() {
        return Err((StatusCode::NOT_FOUND, "No temperature forecast steps".to_string()));
    }

    let step = forecast
        .steps
        .iter()
        .min_by_key(|s| {
            let step_offset = (s.forecast_hour as i64) * 3600;
            (step_offset - time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    let vals = step.values.clone();
    let state_clone = state.clone();
    let png_bytes = tokio::task::spawn_blocking(move || {
        render_temp_png_bytes(&vals, &state_clone.temp_projection_lut)
    }).await.unwrap();
    state.temp_data_cache.insert(time, png_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/png")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(png_bytes))
        .unwrap())
}

pub async fn get_temp_value(
    Query(q): Query<TempValueQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.temp_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    if forecast.steps.is_empty() {
        return Ok(axum::Json(ValueResponse {
            status: "no_data".to_string(),
            value: None,
        }));
    }

    let step = forecast
        .steps
        .iter()
        .min_by_key(|s| {
            let step_offset = (s.forecast_hour as i64) * 3600;
            (step_offset - q.time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    let fx = (q.lon - 0.0) / 0.029;
    let fy = (q.lat - 49.0) / 0.018;

    let val_raw = interpolate_bilinear(fx, fy, 390, 390, &step.values);
    if val_raw == NODATA {
        Ok(axum::Json(ValueResponse {
            status: "out_of_bounds".to_string(),
            value: None,
        }))
    } else {
        let temp_c = val_raw as f64 / 10.0 - 273.15;
        Ok(axum::Json(ValueResponse {
            status: "ok".to_string(),
            value: Some(temp_c),
        }))
    }
}

pub async fn get_temp_timeseries(
    Query(q): Query<TempTimeseriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.temp_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    let fx = (q.lon - 0.0) / 0.029;
    let fy = (q.lat - 49.0) / 0.018;

    let mut times = Vec::new();
    let mut values = Vec::new();

    for step in &forecast.steps {
        let val_raw = interpolate_bilinear(fx, fy, 390, 390, &step.values);
        if val_raw != NODATA {
            let temp_c = val_raw as f64 / 10.0 - 273.15;
            let step_offset = (step.forecast_hour as i64) * 3600;
            times.push(step_offset);
            values.push(temp_c);
        }
    }

    Ok(axum::Json(TempTimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        times,
        values,
    }))
}
