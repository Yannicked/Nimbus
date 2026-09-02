use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::constants::{
    FORECAST_DLAT, FORECAST_DLON, FORECAST_GRID_H, FORECAST_GRID_W, FORECAST_LAT_0, FORECAST_LON_0,
    GRIB_HEIGHT, GRIB_WIDTH, GRID_H, GRID_W, MERCATOR_BOTTOM, MERCATOR_LEFT, MERCATOR_RIGHT,
    MERCATOR_TOP, NEP_RADIUS, NODATA, PRECIP_VAR, RAIN_THRESHOLD, RTCOR_DX, RTCOR_DY, RTCOR_GRID_H,
    RTCOR_GRID_W, RTCOR_X0, RTCOR_Y0,
};
use crate::harmonie::parse_reference_time;
use crate::interpolation::interpolate_bilinear;
use crate::models::{
    reduce_ensemble, EnsembleStat, ForecastStep, RainForecast, SolarMetadata, SolarTimeseriesQuery,
    SolarTimeseriesResponse, SolarValueQuery, TempMetadata, TempTimeseriesQuery,
    TempTimeseriesResponse, TempValueQuery, TimeseriesQuery, TimeseriesResponse, ValueQuery,
    ValueResponse, WindMetadata, WindTimeseriesQuery, WindTimeseriesResponse, WindValueQuery,
    WindValueResponse,
};
use crate::projection::{self, lonlat_to_grib_indices};
use crate::radar::{compute_raw_slice, raw_to_value};
use crate::rendering::{
    render_data_webp_bytes, render_solar_webp_bytes, render_temp_webp_bytes, render_wind_webp_bytes,
};
use crate::state::AppState;

use std::sync::LazyLock;

static EMPTY_WEBP_IMAGE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    use image::codecs::webp::WebPEncoder;
    use image::ImageEncoder;
    let empty_pixels = vec![0u8; (GRID_W * GRID_H * 4) as usize];
    let mut webp_bytes = Vec::new();
    let cursor = std::io::Cursor::new(&mut webp_bytes);
    let encoder = WebPEncoder::new_lossless(cursor);
    encoder
        .write_image(
            &empty_pixels,
            GRID_W,
            GRID_H,
            image::ExtendedColorType::Rgba8,
        )
        .expect("Failed to encode static empty WebP image");
    webp_bytes
});

/// Helper to construct a standard WebP image response with cache headers.
fn webp_response(bytes: Vec<u8>) -> Response {
    Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::empty())
                .unwrap_or_default()
        })
}

/// Generic helper to extract a value from a GRIB-based forecast (Temp, Solar, etc.)
/// by finding the closest step and interpolating.
#[allow(clippy::too_many_arguments)]
fn with_grib_step<S, F, R>(
    forecast: &F,
    time: i64,
    lon: f64,
    lat: f64,
    get_steps: impl Fn(&F) -> &[S],
    get_forecast_hour: impl Fn(&S) -> i32,
    extract_value: impl Fn(&S, f64, f64) -> Option<R>,
    to_response: impl Fn(R) -> ValueResponse,
) -> Result<axum::Json<ValueResponse>, (StatusCode, String)> {
    if !lon.is_finite() || !lat.is_finite() {
        return Ok(axum::Json(ValueResponse {
            status: "out_of_bounds".to_string(),
            value: None,
        }));
    }

    let steps = get_steps(forecast);
    if steps.is_empty() {
        return Ok(axum::Json(ValueResponse {
            status: "no_data".to_string(),
            value: None,
        }));
    }

    let step = steps
        .iter()
        .min_by_key(|s| {
            let step_offset = (get_forecast_hour(s) as i64) * 3600;
            (step_offset - time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    let (fx, fy) = lonlat_to_grib_indices(lon, lat);

    match extract_value(step, fx, fy) {
        Some(val) => Ok(axum::Json(to_response(val))),
        None => Ok(axum::Json(ValueResponse {
            status: "out_of_bounds".to_string(),
            value: None,
        })),
    }
}

/// Serves an empty favicon response to prevent 404 console errors.
pub async fn favicon() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

fn interpolate_temp(fx: f64, fy: f64, values: &[u16]) -> Option<f64> {
    let val_raw = interpolate_bilinear(fx, fy, GRIB_WIDTH, GRIB_HEIGHT, values);
    if val_raw != NODATA {
        Some(val_raw as f64 / 10.0 - 273.15)
    } else {
        None
    }
}

fn extract_timeseries<S, T, F>(steps: &[S], mut map_fn: F) -> (Vec<i64>, Vec<T>)
where
    S: ForecastStep,
    F: FnMut(&S) -> Option<T>,
{
    let mut times = Vec::with_capacity(steps.len());
    let mut values = Vec::with_capacity(steps.len());

    for step in steps {
        if let Some(val) = map_fn(step) {
            let step_offset = (step.forecast_hour() as i64) * 3600;
            times.push(step_offset);
            values.push(val);
        }
    }

    (times, values)
}

fn format_forecast_metadata<S>(steps: &[S], reference_time: i64) -> (Vec<i64>, String)
where
    S: ForecastStep,
{
    let mut times: Vec<i64> = steps
        .iter()
        .map(|s| (s.forecast_hour() as i64) * 3600)
        .collect();
    times.sort_unstable();
    times.dedup();

    let reference_time_str = {
        use chrono::TimeZone;
        if let Some(utc_dt) = chrono::Utc.timestamp_opt(reference_time, 0).single() {
            format!("seconds since {}", utc_dt.format("%Y-%m-%d %H:%M:%S"))
        } else {
            "seconds since 1970-01-01 00:00:00".to_string()
        }
    };

    (times, reference_time_str)
}

fn interpolate_solar(fx: f64, fy: f64, values: &[u16]) -> Option<f64> {
    let val_raw = interpolate_bilinear(fx, fy, GRIB_WIDTH, GRIB_HEIGHT, values);
    if val_raw != NODATA {
        Some(val_raw as f64)
    } else {
        None
    }
}

fn interpolate_wind(fx: f64, fy: f64, u_values: &[u16], v_values: &[u16]) -> Option<(f64, f64)> {
    let u_raw = interpolate_bilinear(fx, fy, GRIB_WIDTH, GRIB_HEIGHT, u_values);
    let v_raw = interpolate_bilinear(fx, fy, GRIB_WIDTH, GRIB_HEIGHT, v_values);

    if u_raw != NODATA && v_raw != NODATA {
        let u = u_raw as f64 / 100.0 - 100.0;
        let v = v_raw as f64 / 100.0 - 100.0;
        Some((u, v))
    } else {
        None
    }
}

fn compute_extended_times(
    base_times: &[i64],
    rain_fc: &RainForecast,
    reference_time_str: &str,
) -> Vec<i64> {
    let radar_ref_time = match parse_reference_time(reference_time_str) {
        Some(t) => t,
        None => return base_times.to_vec(),
    };

    let last_radar_time = base_times.last().copied().unwrap_or(0);
    let mut extended_times = base_times.to_vec();
    for step in &rain_fc.steps {
        let absolute_time = rain_fc.reference_time + (step.forecast_hour as i64) * 3600;
        let relative_offset = absolute_time - radar_ref_time;
        if relative_offset > last_radar_time {
            extended_times.push(relative_offset);
        }
    }
    extended_times.sort_unstable();
    extended_times.dedup();
    extended_times
}

/// Prepends negative relative offsets for historical radar observations (actuals) to base forecast times.
fn merge_actuals_and_forecast_times(
    base_times: &[i64],
    radar_ref_time_str: &str,
    actuals: Option<&crate::state::ActualsData>,
) -> Vec<i64> {
    let radar_ref_time = match parse_reference_time(radar_ref_time_str) {
        Some(t) => t,
        None => return base_times.to_vec(),
    };

    let mut merged = Vec::new();
    if let Some(actuals) = actuals {
        for frame in &actuals.frames {
            let rel = frame.timestamp - radar_ref_time;
            if rel < 0 {
                merged.push(rel);
            }
        }
    }
    merged.sort_unstable();
    merged.dedup();
    merged.extend_from_slice(base_times);
    merged
}

/// Returns the current dataset metadata as JSON.
pub async fn get_metadata(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let radar_opt = state.radar_data.read().await;
    let radar = radar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not loaded".to_string(),
    ))?;
    let mut m = radar.metadata.clone();
    let actuals_guard = state.actuals_data.read().await;
    m.times = merge_actuals_and_forecast_times(
        &m.times,
        &m.reference_time_str,
        actuals_guard.as_ref().map(|a| &**a),
    );
    if let Some(ref rain_data) = *state.rain_data.read().await {
        m.times = compute_extended_times(&m.times, &rain_data.forecast, &m.reference_time_str);
    }
    Ok(axum::Json(m))
}

/// Serves the lossless R/G packed raw radar data PNG for a timeframe.
pub async fn get_data_image(
    Path((ens_str, time)): Path<(String, i64)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let radar_opt = state.radar_data.read().await;
    let radar = radar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    // Check if requested time is for historical actuals (negative relative offset)
    if time < 0 {
        if let Some(ref actuals) = *state.actuals_data.read().await {
            if let Some(radar_ref_time) = parse_reference_time(&radar.metadata.reference_time_str) {
                if let Some(frame) = actuals
                    .frames
                    .iter()
                    .find(|f| f.timestamp - radar_ref_time == time || f.timestamp == time)
                {
                    return Ok(webp_response(frame.webp_bytes.clone()));
                }
            }
        }
    }

    // Check cache
    if let Some(cached_data) = radar.data_cache.get(&(ens_str.clone(), time)) {
        return Ok(webp_response(cached_data.value().clone()));
    }

    let meta = &radar.metadata;
    let file_path = &radar.file_path;

    // Check if this time step is for Harmonie (i.e. past the radar forecast)
    if !meta.times.contains(&time) {
        if ens_str != "pmm" {
            // Return transparent empty image
            let webp_bytes = EMPTY_WEBP_IMAGE.clone();
            // Cache it in active radar dataset
            radar
                .data_cache
                .insert((ens_str.clone(), time), webp_bytes.clone());

            return Ok(webp_response(webp_bytes));
        }

        let rain_data_opt = state.rain_data.read().await;
        let rain_data = rain_data_opt.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Harmonie rain forecast not loaded".to_string(),
        ))?;

        if let Some(cached) = rain_data.data_cache.get(&time) {
            radar
                .data_cache
                .insert((ens_str.clone(), time), cached.value().clone());
            return Ok(webp_response(cached.value().clone()));
        }

        let radar_ref_time = parse_reference_time(&meta.reference_time_str).ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to parse radar reference time".to_string(),
        ))?;

        let absolute_time = radar_ref_time + time;

        let step = rain_data
            .forecast
            .steps
            .iter()
            .min_by_key(|s| {
                let step_abs = rain_data.forecast.reference_time + (s.forecast_hour as i64) * 3600;
                (step_abs - absolute_time).abs()
            })
            .ok_or((
                StatusCode::NOT_FOUND,
                "No matching Harmonie step".to_string(),
            ))?;

        let raw_values = step.values.clone();
        let lut = state.temp_projection_lut.clone();
        let webp_bytes =
            tokio::task::spawn_blocking(move || render_data_webp_bytes(&raw_values, &lut))
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Blocking task join error: {}", e),
                    )
                })?;

        // Cache results
        radar.data_cache.insert((ens_str, time), webp_bytes.clone());
        rain_data.data_cache.insert(time, webp_bytes.clone());

        return Ok(webp_response(webp_bytes));
    }

    // Retrieve or compute raw slice
    let raw_slice = if let Some(cached) = radar.grid_cache.get(&(ens_str.clone(), time)) {
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
        radar
            .grid_cache
            .insert((ens_str.clone(), time), arc.clone());
        arc
    };

    // Render data webp bytes using LUT
    let lut = state.projection_lut.clone();
    let raw_slice_clone = raw_slice.clone();
    let webp_bytes =
        tokio::task::spawn_blocking(move || render_data_webp_bytes(&raw_slice_clone, &lut))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Blocking task join error: {}", e),
                )
            })?;

    // Cache results
    radar.data_cache.insert((ens_str, time), webp_bytes.clone());

    Ok(webp_response(webp_bytes))
}

/// Returns the precipitation value (or ensemble statistic) at a single
/// geographic point as JSON.
pub async fn get_value(
    Query(q): Query<ValueQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !q.lon.is_finite() || !q.lat.is_finite() {
        return Ok(axum::Json(ValueResponse {
            status: "out_of_bounds".to_string(),
            value: None,
        }));
    }

    let radar_opt = state.radar_data.read().await;
    let radar = radar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    let file_path = &radar.file_path;
    let meta = &radar.metadata;

    // Check if time is for historical actuals (negative relative offset)
    if q.time < 0 {
        let (px, py) = projection::lonlat_to_polar_stereographic(q.lon, q.lat);
        let ix = ((px - RTCOR_X0) / RTCOR_DX).round() as i32;
        let iy = ((py - RTCOR_Y0) / RTCOR_DY).round() as i32;

        if let Some(ref actuals) = *state.actuals_data.read().await {
            if let Some(radar_ref_time) = parse_reference_time(&meta.reference_time_str) {
                if let Some(frame) = actuals
                    .frames
                    .iter()
                    .find(|f| f.timestamp - radar_ref_time == q.time || f.timestamp == q.time)
                {
                    if ix < 0 || ix >= RTCOR_GRID_W as i32 || iy < 0 || iy >= RTCOR_GRID_H as i32 {
                        return Ok(axum::Json(ValueResponse {
                            status: "out_of_bounds".to_string(),
                            value: None,
                        }));
                    }
                    let val_raw = frame.raw_values[iy as usize * RTCOR_GRID_W + ix as usize];
                    let val_mmh = raw_to_value(val_raw);
                    let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
                    return Ok(axum::Json(ValueResponse {
                        status: status.to_string(),
                        value: Some(val_mmh),
                    }));
                }
            }
        }
        return Ok(axum::Json(ValueResponse {
            status: "no_data".to_string(),
            value: None,
        }));
    }

    // Check if this time step is for Harmonie (i.e. past the radar forecast)
    if !meta.times.contains(&q.time) {
        if q.ens != "pmm" {
            return Ok(axum::Json(ValueResponse {
                status: "no_data".to_string(),
                value: None,
            }));
        }

        let rain_data_opt = state.rain_data.read().await;
        let rain_data = rain_data_opt.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Harmonie rain forecast not loaded".to_string(),
        ))?;

        let radar_ref_time = parse_reference_time(&meta.reference_time_str).ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to parse radar reference time".to_string(),
        ))?;

        let absolute_time = radar_ref_time + q.time;

        let harmonie_time = absolute_time - rain_data.forecast.reference_time;
        return with_grib_step(
            &rain_data.forecast,
            harmonie_time,
            q.lon,
            q.lat,
            |f| &f.steps,
            |s| s.forecast_hour,
            |s, fx, fy| {
                let val_raw = interpolate_bilinear(fx, fy, GRIB_WIDTH, GRIB_HEIGHT, &s.values);
                if val_raw == NODATA {
                    None
                } else {
                    Some(val_raw)
                }
            },
            |val_raw| {
                let val_mmh = raw_to_value(val_raw);
                let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
                ValueResponse {
                    status: status.to_string(),
                    value: Some(val_mmh),
                }
            },
        );
    }

    // Get grid cell index in 780x780 regular lat/lon forecast grid
    let ix = ((q.lon - FORECAST_LON_0) / FORECAST_DLON).round() as i32;
    let iy = ((q.lat - FORECAST_LAT_0) / FORECAST_DLAT).round() as i32;

    if ix < 0 || ix >= FORECAST_GRID_W as i32 || iy < 0 || iy >= FORECAST_GRID_H as i32 {
        return Ok(axum::Json(ValueResponse {
            status: "out_of_bounds".to_string(),
            value: None,
        }));
    }

    if q.ens == "pmm" {
        let raw_slice = if let Some(slice) = radar.grid_cache.get(&(q.ens.clone(), q.time)) {
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
            radar
                .grid_cache
                .insert((q.ens.clone(), q.time), arc.clone());
            arc
        };
        let val_raw = raw_slice[iy as usize * FORECAST_GRID_W + ix as usize];
        let val_mmh = raw_to_value(val_raw);
        let status = if val_mmh > 0.0 { "ok" } else { "no_rain" };
        return Ok(axum::Json(ValueResponse {
            status: status.to_string(),
            value: Some(val_mmh),
        }));
    }

    // Try reading from cache first
    if let Some(slice) = radar.grid_cache.get(&(q.ens.clone(), q.time)) {
        let val_raw = slice[iy as usize * FORECAST_GRID_W + ix as usize];
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
                let y_max =
                    std::cmp::min(FORECAST_GRID_H as i32 - 1, iy + NEP_RADIUS as i32) as usize;
                let x_min = std::cmp::max(0, ix - NEP_RADIUS as i32) as usize;
                let x_max =
                    std::cmp::min(FORECAST_GRID_W as i32 - 1, ix + NEP_RADIUS as i32) as usize;
                let height_box = y_max - y_min + 1;
                let width_box = x_max - x_min + 1;

                let raw_grid = var
                    .get_values::<u16, _>((
                        &[0, time_idx, y_min, x_min][..],
                        &[num_ensembles, 1, height_box, width_box][..],
                    ))
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                // Precompute neighbor offsets within radius
                let mut offsets = Vec::with_capacity(height_box * width_box);
                for dy_idx in 0..height_box {
                    let dy = (y_min + dy_idx) as i32 - iy;
                    for dx_idx in 0..width_box {
                        let dx = (x_min + dx_idx) as i32 - ix;
                        if dx * dx + dy * dy <= r_sq {
                            offsets.push(dy_idx * width_box + dx_idx);
                        }
                    }
                }

                for ens_idx in 0..num_ensembles {
                    let ens_offset = ens_idx * height_box * width_box;
                    let center_dy = (iy as usize) - y_min;
                    let center_dx = (ix as usize) - x_min;
                    let center_val = raw_grid[ens_offset + center_dy * width_box + center_dx];

                    if center_val == NODATA {
                        continue;
                    }
                    count += 1;

                    let mut member_has_rain = false;
                    for &offset in &offsets {
                        let val = raw_grid[ens_offset + offset];
                        if val != NODATA && val >= RAIN_THRESHOLD {
                            member_has_rain = true;
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
    if !q.lon.is_finite() || !q.lat.is_finite() {
        return Ok(axum::Json(TimeseriesResponse {
            status: "out_of_bounds".to_string(),
            lat: q.lat,
            lon: q.lon,
            ens: q.ens,
            times: Vec::new(),
            values: Vec::new(),
        }));
    }

    let radar_opt = state.radar_data.read().await;
    let radar = radar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Metadata not available".to_string(),
    ))?;

    let file_path = &radar.file_path;
    let meta = &radar.metadata;

    // Forecast grid cell indices (780 x 780 regular lat/lon)
    let ix_fc = ((q.lon - FORECAST_LON_0) / FORECAST_DLON).round() as i32;
    let iy_fc = ((q.lat - FORECAST_LAT_0) / FORECAST_DLAT).round() as i32;

    // Actuals radar grid cell indices (765 x 700 Polar Stereographic)
    let (px, py) = projection::lonlat_to_polar_stereographic(q.lon, q.lat);
    let ix_act = ((px - RTCOR_X0) / RTCOR_DX).round() as i32;
    let iy_act = ((py - RTCOR_Y0) / RTCOR_DY).round() as i32;

    if ix_fc < 0 || ix_fc >= FORECAST_GRID_W as i32 || iy_fc < 0 || iy_fc >= FORECAST_GRID_H as i32
    {
        return Ok(axum::Json(TimeseriesResponse {
            status: "out_of_bounds".to_string(),
            lat: q.lat,
            lon: q.lon,
            ens: q.ens,
            times: Vec::new(),
            values: Vec::new(),
        }));
    }

    // Determine extended times array including actuals and Harmonie
    let actuals_guard = state.actuals_data.read().await;
    let merged_times = merge_actuals_and_forecast_times(
        &meta.times,
        &meta.reference_time_str,
        actuals_guard.as_ref().map(|a| &**a),
    );
    let extended_times = if let Some(ref rain_data) = *state.rain_data.read().await {
        compute_extended_times(&merged_times, &rain_data.forecast, &meta.reference_time_str)
    } else {
        merged_times.clone()
    };

    // Calculate past values for historical actuals
    let radar_ref_time = parse_reference_time(&meta.reference_time_str).unwrap_or(0);
    let mut past_values = Vec::new();
    for &time_val in &merged_times {
        if time_val < 0 {
            let val =
                if let Some(actuals) = actuals_guard.as_deref() {
                    if let Some(frame) = actuals.frames.iter().find(|f| {
                        f.timestamp - radar_ref_time == time_val || f.timestamp == time_val
                    }) {
                        if ix_act >= 0
                            && ix_act < RTCOR_GRID_W as i32
                            && iy_act >= 0
                            && iy_act < RTCOR_GRID_H as i32
                        {
                            let val_raw =
                                frame.raw_values[iy_act as usize * RTCOR_GRID_W + ix_act as usize];
                            if q.ens == "prob" {
                                if val_raw != NODATA && val_raw >= RAIN_THRESHOLD {
                                    100.0
                                } else {
                                    0.0
                                }
                            } else {
                                raw_to_value(val_raw)
                            }
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
            past_values.push(val);
        }
    }

    // Try reading all radar times from cache first
    let mut all_cached = true;
    for &time_val in &meta.times {
        if !radar.grid_cache.contains_key(&(q.ens.clone(), time_val)) {
            all_cached = false;
            break;
        }
    }

    let mut values = Vec::with_capacity(extended_times.len());

    if all_cached {
        if let Some(cached_ts) = radar.timeseries_cache.get(&(q.ens.clone(), ix_fc, iy_fc)) {
            values.extend_from_slice(&cached_ts);
        } else {
            let mut ts_values = Vec::with_capacity(meta.times.len());
            for &time_val in &meta.times {
                if let Some(slice) = radar.grid_cache.get(&(q.ens.clone(), time_val)) {
                    let val_raw = slice[iy_fc as usize * FORECAST_GRID_W + ix_fc as usize];
                    if q.ens == "prob" {
                        ts_values.push(val_raw as f64);
                    } else {
                        ts_values.push(raw_to_value(val_raw));
                    }
                }
            }
            radar
                .timeseries_cache
                .insert((q.ens.clone(), ix_fc, iy_fc), Arc::new(ts_values.clone()));
            values.extend(ts_values);
        }
    } else if q.ens == "pmm" {
        #[allow(clippy::type_complexity)]
        enum TaskResult {
            Cached(Arc<Vec<u16>>),
            Spawned(tokio::task::JoinHandle<Result<Arc<Vec<u16>>, (StatusCode, String)>>),
        }

        let mut tasks = Vec::with_capacity(meta.times.len());
        for &time_val in &meta.times {
            if let Some(slice) = radar.grid_cache.get(&(q.ens.clone(), time_val)) {
                tasks.push(TaskResult::Cached(slice.value().clone()));
            } else {
                let file_path_clone = file_path.clone();
                let meta_clone = meta.clone();
                let ens_clone = q.ens.clone();
                let radar_clone = radar.clone();
                tasks.push(TaskResult::Spawned(tokio::spawn(async move {
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
                    radar_clone
                        .grid_cache
                        .insert((ens_clone, time_val), arc.clone());
                    Ok(arc)
                })));
            }
        }
        for task in tasks {
            let raw_slice = match task {
                TaskResult::Cached(val) => val,
                TaskResult::Spawned(handle) => handle.await.map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Task join error: {}", e),
                    )
                })??,
            };
            let val_raw = raw_slice[iy_fc as usize * FORECAST_GRID_W + ix_fc as usize];
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
                    let y_min = std::cmp::max(0, iy_fc - NEP_RADIUS as i32) as usize;
                    let y_max = std::cmp::min(FORECAST_GRID_H as i32 - 1, iy_fc + NEP_RADIUS as i32)
                        as usize;
                    let x_min = std::cmp::max(0, ix_fc - NEP_RADIUS as i32) as usize;
                    let x_max = std::cmp::min(FORECAST_GRID_W as i32 - 1, ix_fc + NEP_RADIUS as i32)
                        as usize;

                    let height_box = y_max - y_min + 1;
                    let width_box = x_max - x_min + 1;

                    // Read neighborhood for all ensembles and all times
                    let raw_grid = var
                        .get_values::<u16, _>((
                            &[0, 0, y_min, x_min][..],
                            &[num_ensembles, num_times, height_box, width_box][..],
                        ))
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                    // Precompute neighbor offsets within radius
                    let mut offsets = Vec::with_capacity(height_box * width_box);
                    for dy_idx in 0..height_box {
                        let dy = (y_min + dy_idx) as i32 - iy_fc;
                        for dx_idx in 0..width_box {
                            let dx = (x_min + dx_idx) as i32 - ix_fc;
                            if dx * dx + dy * dy <= r_sq {
                                offsets.push(dy_idx * width_box + dx_idx);
                            }
                        }
                    }

                    let time_stride = height_box * width_box;
                    let ens_stride = num_times * time_stride;
                    let center_dy = iy_fc as usize - y_min;
                    let center_dx = ix_fc as usize - x_min;
                    let center_offset_in_time = center_dy * width_box + center_dx;

                    // For each time step, compute NEP
                    for t in 0..num_times {
                        let mut over = 0;
                        let mut count = 0;

                        for ens_idx in 0..num_ensembles {
                            let time_offset = ens_idx * ens_stride + t * time_stride;
                            // Check if center cell is NODATA
                            let center_idx = time_offset + center_offset_in_time;
                            let center_val = raw_grid[center_idx];
                            if center_val == NODATA {
                                continue;
                            }
                            count += 1;

                            let mut member_has_rain = false;
                            for &offset in &offsets {
                                let val = raw_grid[time_offset + offset];
                                if val != NODATA && val >= RAIN_THRESHOLD {
                                    member_has_rain = true;
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
                            &[0, 0, iy_fc as usize, ix_fc as usize][..],
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
                        &[ens_idx, 0, iy_fc as usize, ix_fc as usize][..],
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
    if let Some(ref rain_data) = *state.rain_data.read().await {
        if let Some(radar_ref_time) = parse_reference_time(&meta.reference_time_str) {
            let (fx, fy) = lonlat_to_grib_indices(q.lon, q.lat);

            for &time_val in &extended_times {
                if time_val > last_radar_time && q.ens == "pmm" {
                    let absolute_time = radar_ref_time + time_val;
                    let harmonie_time = absolute_time - rain_data.forecast.reference_time;
                    let step = rain_data.forecast.steps.iter().min_by_key(|s| {
                        let step_offset = (s.forecast_hour as i64) * 3600;
                        (step_offset - harmonie_time).abs()
                    });
                    if let Some(s) = step {
                        let val_raw =
                            interpolate_bilinear(fx, fy, GRIB_WIDTH, GRIB_HEIGHT, &s.values);
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
    values.splice(0..0, past_values);

    Ok(axum::Json(TimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        ens: q.ens,
        times: if is_pmm { extended_times } else { merged_times },
        values,
    }))
}

pub async fn get_wind_metadata(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let wind_opt = state.wind_data.read().await;
    let wind_data = wind_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    let (times, reference_time_str) =
        format_forecast_metadata(&wind_data.forecast.steps, wind_data.forecast.reference_time);

    Ok(axum::Json(WindMetadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        times,
        reference_time: wind_data.forecast.reference_time,
        reference_time_str,
        version: wind_data.forecast.reference_time as u64,
        heights: vec![10, 50, 100, 200, 300],
    }))
}

pub async fn get_wind_data_image(
    Path((height, time)): Path<(u32, i64)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let wind_opt = state.wind_data.read().await;
    let wind_data = wind_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    if let Some(cached) = wind_data.data_cache.get(&(height, time)) {
        return Ok(webp_response(cached.value().clone()));
    }

    if wind_data.forecast.steps.is_empty() {
        return Err((StatusCode::NOT_FOUND, "No wind forecast steps".to_string()));
    }

    let step = wind_data
        .forecast
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
    let lut = state.wind_projection_lut.clone();
    let webp_bytes =
        tokio::task::spawn_blocking(move || render_wind_webp_bytes(&u_vals, &v_vals, &lut))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Blocking task join error: {}", e),
                )
            })?;
    wind_data
        .data_cache
        .insert((height, time), webp_bytes.clone());

    Ok(webp_response(webp_bytes))
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
    if !q.lon.is_finite() || !q.lat.is_finite() {
        return Ok(axum::Json(WindValueResponse {
            status: "out_of_bounds".to_string(),
            u: None,
            v: None,
            speed: None,
            direction: None,
        }));
    }

    let wind_opt = state.wind_data.read().await;
    let wind_data = wind_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    if wind_data.forecast.steps.is_empty() {
        return Ok(axum::Json(WindValueResponse {
            status: "no_data".to_string(),
            u: None,
            v: None,
            speed: None,
            direction: None,
        }));
    }

    let req_height = q.height.unwrap_or(10);
    let (fx, fy) = lonlat_to_grib_indices(q.lon, q.lat);

    let step = wind_data
        .forecast
        .steps
        .iter()
        .filter(|s| s.height_level == req_height)
        .min_by_key(|s| {
            let step_offset = (s.forecast_hour as i64) * 3600;
            (step_offset - q.time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    if let Some((u, v)) = interpolate_wind(fx, fy, &step.u_values, &step.v_values) {
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
    } else {
        Ok(axum::Json(WindValueResponse {
            status: "out_of_bounds".to_string(),
            u: None,
            v: None,
            speed: None,
            direction: None,
        }))
    }
}

pub async fn get_wind_timeseries(
    Query(q): Query<WindTimeseriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !q.lon.is_finite() || !q.lat.is_finite() {
        return Ok(axum::Json(WindTimeseriesResponse {
            status: "out_of_bounds".to_string(),
            lat: q.lat,
            lon: q.lon,
            times: Vec::new(),
            speeds: Vec::new(),
            directions: Vec::new(),
        }));
    }

    let wind_opt = state.wind_data.read().await;
    let wind_data = wind_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Wind forecast not loaded".to_string(),
    ))?;

    let (fx, fy) = lonlat_to_grib_indices(q.lon, q.lat);
    let req_height = q.height.unwrap_or(10);

    let steps: Vec<_> = wind_data
        .forecast
        .steps
        .iter()
        .filter(|s| s.height_level == req_height)
        .collect();

    let (times, values) = extract_timeseries(steps.as_slice(), |step| {
        interpolate_wind(fx, fy, &step.u_values, &step.v_values)
    });

    let mut speeds = Vec::with_capacity(values.len());
    let mut directions = Vec::with_capacity(values.len());

    for (u, v) in values {
        let speed = (u * u + v * v).sqrt();
        let mut dir_rad = u.atan2(v) + std::f64::consts::PI;
        if dir_rad < 0.0 {
            dir_rad += 2.0 * std::f64::consts::PI;
        }
        let direction = dir_rad.to_degrees();
        speeds.push(speed);
        directions.push(direction);
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
    let temp_opt = state.temp_data.read().await;
    let temp_data = temp_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    let (times, reference_time_str) =
        format_forecast_metadata(&temp_data.forecast.steps, temp_data.forecast.reference_time);

    Ok(axum::Json(TempMetadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        times,
        reference_time: temp_data.forecast.reference_time,
        reference_time_str,
        version: temp_data.forecast.reference_time as u64,
    }))
}

pub async fn get_temp_data_image(
    Path(time): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let temp_opt = state.temp_data.read().await;
    let temp_data = temp_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    if let Some(cached) = temp_data.data_cache.get(&time) {
        return Ok(webp_response(cached.value().clone()));
    }

    if temp_data.forecast.steps.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            "No temperature forecast steps".to_string(),
        ));
    }

    let step = temp_data
        .forecast
        .steps
        .iter()
        .min_by_key(|s| {
            let step_offset = (s.forecast_hour as i64) * 3600;
            (step_offset - time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    let vals = step.values.clone();
    let lut = state.temp_projection_lut.clone();
    let webp_bytes = tokio::task::spawn_blocking(move || render_temp_webp_bytes(&vals, &lut))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Blocking task join error: {}", e),
            )
        })?;
    temp_data.data_cache.insert(time, webp_bytes.clone());

    Ok(webp_response(webp_bytes))
}

pub async fn get_temp_value(
    Query(q): Query<TempValueQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let temp_opt = state.temp_data.read().await;
    let temp_data = temp_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    with_grib_step(
        &temp_data.forecast,
        q.time,
        q.lon,
        q.lat,
        |f| &f.steps,
        |s| s.forecast_hour,
        |s, fx, fy| interpolate_temp(fx, fy, &s.values),
        |temp_c| ValueResponse {
            status: "ok".to_string(),
            value: Some(temp_c),
        },
    )
}

pub async fn get_temp_timeseries(
    Query(q): Query<TempTimeseriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !q.lon.is_finite() || !q.lat.is_finite() {
        return Ok(axum::Json(TempTimeseriesResponse {
            status: "out_of_bounds".to_string(),
            lat: q.lat,
            lon: q.lon,
            times: Vec::new(),
            values: Vec::new(),
        }));
    }

    let temp_opt = state.temp_data.read().await;
    let temp_data = temp_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Temperature forecast not loaded".to_string(),
    ))?;

    let (fx, fy) = lonlat_to_grib_indices(q.lon, q.lat);

    let (times, values) = extract_timeseries(&temp_data.forecast.steps, |step| {
        interpolate_temp(fx, fy, &step.values)
    });

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
    let solar_opt = state.solar_data.read().await;
    let solar_data = solar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
    ))?;

    let (times, reference_time_str) = format_forecast_metadata(
        &solar_data.forecast.steps,
        solar_data.forecast.reference_time,
    );

    Ok(axum::Json(SolarMetadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        times,
        reference_time: solar_data.forecast.reference_time,
        reference_time_str,
        version: solar_data.forecast.reference_time as u64,
    }))
}

pub async fn get_solar_data_image(
    Path(time): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let solar_opt = state.solar_data.read().await;
    let solar_data = solar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
    ))?;

    if let Some(cached) = solar_data.data_cache.get(&time) {
        return Ok(webp_response(cached.value().clone()));
    }

    if solar_data.forecast.steps.is_empty() {
        return Err((StatusCode::NOT_FOUND, "No solar forecast steps".to_string()));
    }

    let step = solar_data
        .forecast
        .steps
        .iter()
        .min_by_key(|s| {
            let step_offset = (s.forecast_hour as i64) * 3600;
            (step_offset - time).abs()
        })
        .ok_or((StatusCode::NOT_FOUND, "No matching step".to_string()))?;

    let vals = step.values.clone();
    let lut = state.solar_projection_lut.clone();
    let webp_bytes = tokio::task::spawn_blocking(move || render_solar_webp_bytes(&vals, &lut))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Blocking task join error: {}", e),
            )
        })?;
    solar_data.data_cache.insert(time, webp_bytes.clone());

    Ok(webp_response(webp_bytes))
}

pub async fn get_solar_value(
    Query(q): Query<SolarValueQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let solar_opt = state.solar_data.read().await;
    let solar_data = solar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
    ))?;

    with_grib_step(
        &solar_data.forecast,
        q.time,
        q.lon,
        q.lat,
        |f| &f.steps,
        |s| s.forecast_hour,
        |s, fx, fy| interpolate_solar(fx, fy, &s.values),
        |solar_w| ValueResponse {
            status: "ok".to_string(),
            value: Some(solar_w),
        },
    )
}

pub async fn get_solar_timeseries(
    Query(q): Query<SolarTimeseriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !q.lon.is_finite() || !q.lat.is_finite() {
        return Ok(axum::Json(SolarTimeseriesResponse {
            status: "out_of_bounds".to_string(),
            lat: q.lat,
            lon: q.lon,
            times: Vec::new(),
            values: Vec::new(),
        }));
    }

    let solar_opt = state.solar_data.read().await;
    let solar_data = solar_opt.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Solar forecast not loaded".to_string(),
    ))?;

    let (fx, fy) = lonlat_to_grib_indices(q.lon, q.lat);

    let (times, values) = extract_timeseries(&solar_data.forecast.steps, |step| {
        interpolate_solar(fx, fy, &step.values)
    });

    Ok(axum::Json(SolarTimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        times,
        values,
    }))
}
