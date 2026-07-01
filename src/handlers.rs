use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::constants::{
    GRID_H, GRID_W, KNMI_DX, KNMI_DY, KNMI_GRID_H, KNMI_GRID_W, KNMI_X0, KNMI_Y0, MERCATOR_BOTTOM,
    MERCATOR_LEFT, MERCATOR_RIGHT, MERCATOR_TOP, NEP_RADIUS, NODATA, PRECIP_VAR, RAIN_THRESHOLD,
};
use crate::harmonie::parse_reference_time;
use crate::interpolation::interpolate_bilinear;
use crate::models::{
    reduce_ensemble, EnsembleStat, SolarMetadata, SolarTimeseriesQuery, SolarTimeseriesResponse,
    SolarValueQuery, TempMetadata, TempTimeseriesQuery, TempTimeseriesResponse, TempValueQuery,
    TimeseriesQuery, TimeseriesResponse, ValueQuery, ValueResponse, WindMetadata,
    WindTimeseriesQuery, WindTimeseriesResponse, WindValueQuery, WindValueResponse,
};
use crate::projection;
use crate::radar::{compute_raw_slice, raw_to_value};
use crate::rendering::{
    render_data_webp_bytes, render_solar_webp_bytes, render_temp_webp_bytes, render_wind_webp_bytes,
};
use crate::state::AppState;

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
        Some(mut m) => {
            if let Some(ref rain_fc) = *state.rain_forecast.read().await {
                if let Some(radar_ref_time) = parse_reference_time(&m.reference_time_str) {
                    let last_radar_time = m.times.last().copied().unwrap_or(0);
                    let mut extended_times = m.times.clone();
                    for step in &rain_fc.steps {
                        let absolute_time =
                            rain_fc.reference_time + (step.forecast_hour as i64) * 3600;
                        let relative_offset = absolute_time - radar_ref_time;
                        if relative_offset > last_radar_time {
                            extended_times.push(relative_offset);
                        }
                    }
                    extended_times.sort_unstable();
                    extended_times.dedup();
                    m.times = extended_times;
                }
            }
            Ok(axum::Json(m))
        }
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
            .header("Content-Type", "image/webp")
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

    // Check if this time step is for Harmonie (i.e. past the radar forecast)
    if !meta.times.contains(&time) {
        if ens_str != "pmm" {
            // Return transparent empty image
            let empty_pixels = vec![0u8; (GRID_W * GRID_H * 4) as usize];
            let mut webp_bytes = Vec::new();
            {
                use image::codecs::webp::WebPEncoder;
                use image::ImageEncoder;
                let cursor = std::io::Cursor::new(&mut webp_bytes);
                let encoder = WebPEncoder::new_lossless(cursor);
                encoder
                    .write_image(
                        &empty_pixels,
                        GRID_W,
                        GRID_H,
                        image::ExtendedColorType::Rgba8,
                    )
                    .unwrap();
            }
            // Cache it
            state
                .data_cache
                .insert((ens_str.clone(), time), webp_bytes.clone());

            return Ok(Response::builder()
                .header("Content-Type", "image/webp")
                .header("Cache-Control", "no-store, no-cache, must-revalidate")
                .body(axum::body::Body::from(webp_bytes))
                .unwrap());
        }

        let rain_fc_opt = state.rain_forecast.read().await;
        let rain_fc = rain_fc_opt.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Harmonie rain forecast not loaded".to_string(),
        ))?;

        let radar_ref_time = parse_reference_time(&meta.reference_time_str).ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to parse radar reference time".to_string(),
        ))?;

        let absolute_time = radar_ref_time + time;

        let step = rain_fc
            .steps
            .iter()
            .min_by_key(|s| {
                let step_abs = rain_fc.reference_time + (s.forecast_hour as i64) * 3600;
                (step_abs - absolute_time).abs()
            })
            .ok_or((
                StatusCode::NOT_FOUND,
                "No matching Harmonie step".to_string(),
            ))?;

        let raw_values = step.values.clone();
        let state_clone = state.clone();
        let webp_bytes = tokio::task::spawn_blocking(move || {
            render_data_webp_bytes(&raw_values, &state_clone.temp_projection_lut)
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Blocking task join error: {}", e),
            )
        })?;

        // Cache results
        state.data_cache.insert((ens_str, time), webp_bytes.clone());

        return Ok(Response::builder()
            .header("Content-Type", "image/webp")
            .header("Cache-Control", "no-store, no-cache, must-revalidate")
            .body(axum::body::Body::from(webp_bytes))
            .unwrap());
    }

    // Retrieve or compute raw slice
    let raw_slice = if let Some(cached) = state.grid_cache.get(&(ens_str.clone(), time)) {
        cached.value().clone()
    } else {
        let file_path_clone = file_path.clone();
        let meta_clone = meta.clone();
        let ens_str_clone = ens_str.clone();
        let computed = tokio::task::spawn_blocking(move || {
            compute_raw_slice(&file_path_clone, &meta_clone, &ens_str_clone, time)
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Blocking task join error: {}", e),
            )
        })??;
        let arc = Arc::new(computed);
        state
            .grid_cache
            .insert((ens_str.clone(), time), arc.clone());
        arc
    };

    // Render data webp bytes using LUT
    let state_clone = state.clone();
    let raw_slice_clone = raw_slice.clone();
    let webp_bytes = tokio::task::spawn_blocking(move || {
        render_data_webp_bytes(&raw_slice_clone, &state_clone.projection_lut)
    })
    .await
    .unwrap();

    // Cache results
    state.data_cache.insert((ens_str, time), webp_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(webp_bytes))
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

    // Check if this time step is for Harmonie (i.e. past the radar forecast)
    if !meta.times.contains(&q.time) {
        if q.ens != "pmm" {
            return Ok(axum::Json(ValueResponse {
                status: "no_data".to_string(),
                value: None,
            }));
        }

        let rain_fc_opt = state.rain_forecast.read().await;
        let rain_fc = rain_fc_opt.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Harmonie rain forecast not loaded".to_string(),
        ))?;

        let radar_ref_time = parse_reference_time(&meta.reference_time_str).ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to parse radar reference time".to_string(),
        ))?;

        let absolute_time = radar_ref_time + q.time;

        let step = rain_fc
            .steps
            .iter()
            .min_by_key(|s| {
                let step_abs = rain_fc.reference_time + (s.forecast_hour as i64) * 3600;
                (step_abs - absolute_time).abs()
            })
            .ok_or((
                StatusCode::NOT_FOUND,
                "No matching Harmonie step".to_string(),
            ))?;

        let fx = (q.lon - 0.0) / 0.029;
        let fy = (q.lat - 49.0) / 0.018;

        let val_raw = interpolate_bilinear(fx, fy, 390, 390, &step.values);
        if val_raw == NODATA {
            return Ok(axum::Json(ValueResponse {
                status: "out_of_bounds".to_string(),
                value: None,
            }));
        }

        let val_mmh = raw_to_value(val_raw);
        let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };

        return Ok(axum::Json(ValueResponse {
            status: status.to_string(),
            value: Some(val_mmh),
        }));
    }

    // Get grid cell index
    let ix = ((px - KNMI_X0) / KNMI_DX).round() as i32;
    let iy = ((py - KNMI_Y0) / KNMI_DY).round() as i32;

    if ix < 0 || ix >= KNMI_GRID_W as i32 || iy < 0 || iy >= KNMI_GRID_H as i32 {
        return Ok(axum::Json(ValueResponse {
            status: "out_of_bounds".to_string(),
            value: None,
        }));
    }

    if q.ens == "pmm" {
        let raw_slice = if let Some(slice) = state.grid_cache.get(&(q.ens.clone(), q.time)) {
            slice.value().clone()
        } else {
            let file_path_clone = file_path.clone();
            let meta_clone = meta.clone();
            let ens_clone = q.ens.clone();
            let computed = tokio::task::spawn_blocking(move || {
                compute_raw_slice(&file_path_clone, &meta_clone, &ens_clone, q.time)
            })
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Blocking task join error: {}", e),
                )
            })??;
            let arc = Arc::new(computed);
            state
                .grid_cache
                .insert((q.ens.clone(), q.time), arc.clone());
            arc
        };
        let val_raw = raw_slice[iy as usize * KNMI_GRID_W + ix as usize];
        let val_mmh = raw_to_value(val_raw);
        let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
        return Ok(axum::Json(ValueResponse {
            status: status.to_string(),
            value: Some(val_mmh),
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
    let file_path_clone = file_path.clone();
    let q_ens_clone = q.ens.clone();
    let meta_clone = meta.clone();
    let (status_out, value_out) = tokio::task::spawn_blocking(move || {
        let file = netcdf::open(&file_path_clone)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let var = file.variable(PRECIP_VAR).ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "precip_intensity variable missing".to_string(),
        ))?;

        if let Some(stat) = EnsembleStat::from_str(&q_ens_clone) {
            // Read value at target cell across all members
            let time_idx = meta_clone
                .times
                .iter()
                .position(|&t| t == q.time)
                .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", q.time)))?;

            if matches!(stat, EnsembleStat::Probability) {
                let mut over = 0;
                let mut count = 0;
                let r_sq = (NEP_RADIUS * NEP_RADIUS) as i32;

                let num_ensembles = meta_clone.ensembles.len();
                let y_min = std::cmp::max(0, iy - NEP_RADIUS as i32) as usize;
                let y_max = std::cmp::min(KNMI_GRID_H as i32 - 1, iy + NEP_RADIUS as i32) as usize;
                let x_min = std::cmp::max(0, ix - NEP_RADIUS as i32) as usize;
                let x_max = std::cmp::min(KNMI_GRID_W as i32 - 1, ix + NEP_RADIUS as i32) as usize;
                let height_box = y_max - y_min + 1;
                let width_box = x_max - x_min + 1;

                let raw_grid = var
                    .get_values::<u16, _>((
                        &[0, time_idx, y_min, x_min][..],
                        &[num_ensembles, 1, height_box, width_box][..],
                    ))
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                for ens_idx in 0..num_ensembles {
                    let center_dy = (iy as usize) - y_min;
                    let center_dx = (ix as usize) - x_min;
                    let center_val = raw_grid
                        [ens_idx * height_box * width_box + center_dy * width_box + center_dx];

                    if center_val == NODATA {
                        continue;
                    }
                    count += 1;

                    let mut member_has_rain = false;
                    for dy_idx in 0..height_box {
                        let ny = y_min + dy_idx;
                        let dy = ny as i32 - iy;
                        for dx_idx in 0..width_box {
                            let nx = x_min + dx_idx;
                            let dx = nx as i32 - ix;
                            if dx * dx + dy * dy <= r_sq {
                                let val = raw_grid[ens_idx * height_box * width_box
                                    + dy_idx * width_box
                                    + dx_idx];
                                if val != NODATA && val >= RAIN_THRESHOLD {
                                    member_has_rain = true;
                                    break;
                                }
                            }
                        }
                        if member_has_rain {
                            break;
                        }
                    }

                    if member_has_rain {
                        over += 1;
                    }
                }

                let nep = if count > 0 {
                    ((over * 100) / count) as f64
                } else {
                    NODATA as f64
                };

                return Ok(("probability".to_string(), nep));
            }

            let mut vals = var
                .get_values::<u16, _>((
                    &[0, time_idx, iy as usize, ix as usize][..],
                    &[meta_clone.ensembles.len(), 1, 1, 1][..],
                ))
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            let reduced = reduce_ensemble(&stat, &mut vals);

            Ok(match stat {
                EnsembleStat::Probability => unreachable!(),
                _ => {
                    let val_mmh = raw_to_value(reduced);
                    let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
                    (status.to_string(), val_mmh)
                }
            })
        } else {
            // Individual member
            let ens_num: i32 = q_ens_clone.parse().map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid ensemble parameter: {}", q_ens_clone),
                )
            })?;

            let ens_idx = meta_clone
                .ensembles
                .iter()
                .position(|&e| e == ens_num)
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    format!("Invalid ensemble: {}", ens_num),
                ))?;

            let time_idx = meta_clone
                .times
                .iter()
                .position(|&t| t == q.time)
                .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", q.time)))?;

            let val_raw: u16 = var
                .get_value((ens_idx, time_idx, iy as usize, ix as usize))
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            let val_mmh = raw_to_value(val_raw);
            let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
            Ok((status.to_string(), val_mmh))
        }
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Blocking task join error: {}", e),
        )
    })??;

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

    // Determine extended times array
    let mut extended_times = meta.times.clone();
    if let Some(ref rain_fc) = *state.rain_forecast.read().await {
        if let Some(radar_ref_time) = parse_reference_time(&meta.reference_time_str) {
            let last_radar_time = meta.times.last().copied().unwrap_or(0);
            for step in &rain_fc.steps {
                let absolute_time = rain_fc.reference_time + (step.forecast_hour as i64) * 3600;
                let relative_offset = absolute_time - radar_ref_time;
                if relative_offset > last_radar_time {
                    extended_times.push(relative_offset);
                }
            }
            extended_times.sort_unstable();
            extended_times.dedup();
        }
    }

    // Try reading all radar times from cache first
    let mut all_cached = true;
    for &time_val in &meta.times {
        if !state.grid_cache.contains_key(&(q.ens.clone(), time_val)) {
            all_cached = false;
            break;
        }
    }

    let mut values = Vec::with_capacity(extended_times.len());

    if all_cached {
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
    } else if q.ens == "pmm" {
        let mut handles = Vec::with_capacity(meta.times.len());
        for &time_val in &meta.times {
            if let Some(slice) = state.grid_cache.get(&(q.ens.clone(), time_val)) {
                let val = slice.value().clone();
                handles.push(tokio::spawn(async move { Ok(val) }));
            } else {
                let file_path_clone = file_path.clone();
                let meta_clone = meta.clone();
                let ens_clone = q.ens.clone();
                let state_clone = state.clone();
                handles.push(tokio::spawn(async move {
                    let ens_for_block = ens_clone.clone();
                    let computed = tokio::task::spawn_blocking(move || {
                        compute_raw_slice(&file_path_clone, &meta_clone, &ens_for_block, time_val)
                    })
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Blocking task join error: {}", e),
                        )
                    })??;
                    let arc = Arc::new(computed);
                    state_clone
                        .grid_cache
                        .insert((ens_clone, time_val), arc.clone());
                    Ok(arc)
                }));
            }
        }
        for handle in handles {
            let raw_slice = handle.await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Task join error: {}", e),
                )
            })??;
            let val_raw = raw_slice[iy as usize * KNMI_GRID_W + ix as usize];
            values.push(raw_to_value(val_raw));
        }
    } else {
        let file_path_clone = file_path.clone();
        let q_ens_clone = q.ens.clone();
        let meta_clone = meta.clone();
        let radar_values = tokio::task::spawn_blocking(move || {
            let file = netcdf::open(&file_path_clone)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let var = file.variable(PRECIP_VAR).ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                "precip_intensity variable missing".to_string(),
            ))?;

            let num_times = meta_clone.times.len();
            let num_ensembles = meta_clone.ensembles.len();
            let mut vals = Vec::with_capacity(num_times);

            if let Some(stat) = EnsembleStat::from_str(&q_ens_clone) {
                if matches!(stat, EnsembleStat::Probability) {
                    let r_sq = (NEP_RADIUS * NEP_RADIUS) as i32;
                    let y_min = std::cmp::max(0, iy - NEP_RADIUS as i32) as usize;
                    let y_max =
                        std::cmp::min(KNMI_GRID_H as i32 - 1, iy + NEP_RADIUS as i32) as usize;
                    let x_min = std::cmp::max(0, ix - NEP_RADIUS as i32) as usize;
                    let x_max =
                        std::cmp::min(KNMI_GRID_W as i32 - 1, ix + NEP_RADIUS as i32) as usize;

                    let height_box = y_max - y_min + 1;
                    let width_box = x_max - x_min + 1;

                    // Read neighborhood for all ensembles and all times
                    let raw_grid = var
                        .get_values::<u16, _>((
                            &[0, 0, y_min, x_min][..],
                            &[num_ensembles, num_times, height_box, width_box][..],
                        ))
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                    // For each time step, compute NEP
                    for t in 0..num_times {
                        let mut over = 0;
                        let mut count = 0;

                        for ens_idx in 0..num_ensembles {
                            // Check if center cell is NODATA
                            let center_dy = iy as usize - y_min;
                            let center_dx = ix as usize - x_min;
                            let center_idx = ens_idx * (num_times * height_box * width_box)
                                + t * (height_box * width_box)
                                + center_dy * width_box
                                + center_dx;
                            let center_val = raw_grid[center_idx];
                            if center_val == NODATA {
                                continue;
                            }
                            count += 1;

                            let mut member_has_rain = false;
                            for dy_idx in 0..height_box {
                                let ny = y_min + dy_idx;
                                let dy = ny as i32 - iy;
                                for dx_idx in 0..width_box {
                                    let nx = x_min + dx_idx;
                                    let dx = nx as i32 - ix;
                                    if dx * dx + dy * dy <= r_sq {
                                        let idx = ens_idx * (num_times * height_box * width_box)
                                            + t * (height_box * width_box)
                                            + dy_idx * width_box
                                            + dx_idx;
                                        let val = raw_grid[idx];
                                        if val != NODATA && val >= RAIN_THRESHOLD {
                                            member_has_rain = true;
                                            break;
                                        }
                                    }
                                }
                                if member_has_rain {
                                    break;
                                }
                            }

                            if member_has_rain {
                                over += 1;
                            }
                        }

                        let nep = if count > 0 {
                            ((over * 100) / count) as f64
                        } else {
                            NODATA as f64
                        };
                        vals.push(nep);
                    }
                } else {
                    // Read values for all ensembles and all times at the target pixel (non-probability)
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
                        vals.push(raw_to_value(reduced));
                    }
                }
            } else {
                // Individual member
                let ens_num: i32 = q_ens_clone.parse().map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("Invalid ensemble parameter: {}", q_ens_clone),
                    )
                })?;

                let ens_idx = meta_clone
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
                    vals.push(raw_to_value(val_raw));
                }
            }
            Ok(vals)
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Blocking task join error: {}", e),
            )
        })??;

        values = radar_values;
    }

    // Append Harmonie forecast values for the extended time steps
    let last_radar_time = meta.times.last().copied().unwrap_or(0);
    if let Some(ref rain_fc) = *state.rain_forecast.read().await {
        if let Some(radar_ref_time) = parse_reference_time(&meta.reference_time_str) {
            let fx = (q.lon - 0.0) / 0.029;
            let fy = (q.lat - 49.0) / 0.018;

            for &time_val in &extended_times {
                if time_val > last_radar_time && q.ens == "pmm" {
                    let absolute_time = radar_ref_time + time_val;
                    let step = rain_fc.steps.iter().min_by_key(|s| {
                        let step_abs = rain_fc.reference_time + (s.forecast_hour as i64) * 3600;
                        (step_abs - absolute_time).abs()
                    });
                    if let Some(s) = step {
                        let val_raw = interpolate_bilinear(fx, fy, 390, 390, &s.values);
                        if val_raw != NODATA {
                            values.push(raw_to_value(val_raw));
                        } else {
                            values.push(0.0);
                        }
                    } else {
                        values.push(0.0);
                    }
                }
            }
        }
    }

    let is_pmm = q.ens == "pmm";
    Ok(axum::Json(TimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        ens: q.ens,
        times: if is_pmm { extended_times } else { meta.times },
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
        if let Some(utc_dt) = chrono::Utc
            .timestamp_opt(forecast.reference_time, 0)
            .single()
        {
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
            .header("Content-Type", "image/webp")
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
    let webp_bytes = tokio::task::spawn_blocking(move || {
        render_wind_webp_bytes(&u_vals, &v_vals, &state_clone.wind_projection_lut)
    })
    .await
    .unwrap();
    state
        .wind_data_cache
        .insert((height, time), webp_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(webp_bytes))
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
        if let Some(utc_dt) = chrono::Utc
            .timestamp_opt(forecast.reference_time, 0)
            .single()
        {
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
            .header("Content-Type", "image/webp")
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
        return Err((
            StatusCode::NOT_FOUND,
            "No temperature forecast steps".to_string(),
        ));
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
    let webp_bytes = tokio::task::spawn_blocking(move || {
        render_temp_webp_bytes(&vals, &state_clone.temp_projection_lut)
    })
    .await
    .unwrap();
    state.temp_data_cache.insert(time, webp_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(webp_bytes))
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

pub async fn get_solar_metadata(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.solar_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
    ))?;

    let times: Vec<i64> = forecast
        .steps
        .iter()
        .map(|s| (s.forecast_hour as i64) * 3600)
        .collect();

    let reference_time_str = {
        use chrono::TimeZone;
        if let Some(utc_dt) = chrono::Utc
            .timestamp_opt(forecast.reference_time, 0)
            .single()
        {
            format!("seconds since {}", utc_dt.format("%Y-%m-%d %H:%M:%S"))
        } else {
            "seconds since 1970-01-01 00:00:00".to_string()
        }
    };

    Ok(axum::Json(SolarMetadata {
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

pub async fn get_solar_data_image(
    Path(time): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if let Some(cached) = state.solar_data_cache.get(&time) {
        return Ok(Response::builder()
            .header("Content-Type", "image/webp")
            .header("Cache-Control", "no-store, no-cache, must-revalidate")
            .body(axum::body::Body::from(cached.value().clone()))
            .unwrap());
    }

    let forecast_opt = state.solar_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
    ))?;

    if forecast.steps.is_empty() {
        return Err((StatusCode::NOT_FOUND, "No solar forecast steps".to_string()));
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
    let webp_bytes = tokio::task::spawn_blocking(move || {
        render_solar_webp_bytes(&vals, &state_clone.solar_projection_lut)
    })
    .await
    .unwrap();
    state.solar_data_cache.insert(time, webp_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(webp_bytes))
        .unwrap())
}

pub async fn get_solar_value(
    Query(q): Query<SolarValueQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.solar_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
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
        let solar_w = val_raw as f64;
        Ok(axum::Json(ValueResponse {
            status: "ok".to_string(),
            value: Some(solar_w),
        }))
    }
}

pub async fn get_solar_timeseries(
    Query(q): Query<SolarTimeseriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let forecast_opt = state.solar_forecast.read().await;
    let forecast = forecast_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
    ))?;

    let fx = (q.lon - 0.0) / 0.029;
    let fy = (q.lat - 49.0) / 0.018;

    let mut times = Vec::new();
    let mut values = Vec::new();

    for step in &forecast.steps {
        let val_raw = interpolate_bilinear(fx, fy, 390, 390, &step.values);
        if val_raw != NODATA {
            let solar_w = val_raw as f64;
            let step_offset = (step.forecast_hour as i64) * 3600;
            times.push(step_offset);
            values.push(solar_w);
        }
    }

    Ok(axum::Json(SolarTimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        times,
        values,
    }))
}
