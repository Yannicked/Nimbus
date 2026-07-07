import re

def fix_file():
    with open('src/handlers.rs', 'r') as f:
        content = f.read()

    # Ensure mutability in signatures
    content = content.replace('pub async fn get_data_image(\n    Path((ens_str, time)): Path<(String, i64)>,', 'pub async fn get_data_image(\n    Path((mut ens_str, time)): Path<(String, i64)>,')
    content = content.replace('pub async fn get_value(\n    Query(q): Query<ValueQuery>,', 'pub async fn get_value(\n    Query(mut q): Query<ValueQuery>,')
    content = content.replace('pub async fn get_timeseries(\n    Query(q): Query<TimeseriesQuery>,', 'pub async fn get_timeseries(\n    Query(mut q): Query<TimeseriesQuery>,')

    # Correct implementations using a local ens_str to avoid repeated q.ens access issues

    get_data_image_replacement = r'''pub async fn get_data_image(
    Path((mut ens_str, time)): Path<(String, i64)>,
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check cache using "move and move back" trick to avoid cloning
    let cached_bytes = {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(ens_str), time);
        let res = state.data_cache.get(&key).map(|r| r.value().clone());
        ens_str = key.0.into_owned();
        res
    };

    if let Some(webp_bytes) = cached_bytes {
        return Ok(Response::builder()
            .header("Content-Type", "image/webp")
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(axum::body::Body::from(webp_bytes))
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
                .insert((Cow::Owned(ens_str), time), webp_bytes.clone());

            return Ok(Response::builder()
                .header("Content-Type", "image/webp")
                .header("Cache-Control", "public, max-age=31536000, immutable")
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
        state
            .data_cache
            .insert((Cow::Owned(ens_str), time), webp_bytes.clone());

        return Ok(Response::builder()
            .header("Content-Type", "image/webp")
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(axum::body::Body::from(webp_bytes))
            .unwrap());
    }

    // Retrieve or compute raw slice using "move and move back" trick to avoid cloning
    let raw_slice = {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(ens_str), time);
        let cached_res = state.grid_cache.get(&key).map(|r| r.value().clone());
        ens_str = key.0.into_owned();

        if let Some(cached) = cached_res {
            cached
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
                .insert((Cow::Owned(ens_str.clone()), time), arc.clone());
            arc
        }
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
    state
        .data_cache
        .insert((Cow::Owned(ens_str), time), webp_bytes.clone());

    Ok(Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "public, max-age=31536000, immutable")
        .body(axum::body::Body::from(webp_bytes))
        .unwrap())
}'''

    get_value_replacement = r'''pub async fn get_value(
    Query(mut q): Query<ValueQuery>,
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

        let harmonie_time = absolute_time - rain_fc.reference_time;
        return with_grib_step(
            rain_fc,
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
        let raw_slice = {
            let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
            let cached_res = state.grid_cache.get(&key).map(|r| r.value().clone());
            q.ens = key.0.into_owned();

            if let Some(slice) = cached_res {
                slice
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
                    .insert((Cow::Owned(q.ens.clone()), q.time), arc.clone());
                arc
            }
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
    let cached_res = {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), q.time);
        let res = state.grid_cache.get(&key).map(|r| r.value().clone());
        q.ens = key.0.into_owned();
        res
    };

    if let Some(slice) = cached_res {
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
}'''

    get_timeseries_replacement = r'''pub async fn get_timeseries(
    Query(mut q): Query<TimeseriesQuery>,
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
    let mut ens_str = std::mem::take(&mut q.ens);
    for &time_val in &meta.times {
        let key: (Cow<'static, str>, i64) = (Cow::Owned(ens_str), time_val);
        let found = state.grid_cache.contains_key(&key);
        ens_str = key.0.into_owned();
        if !found {
            all_cached = false;
            break;
        }
    }
    q.ens = ens_str;

    let mut values = Vec::with_capacity(extended_times.len());

    if all_cached {
        let cached_ts = {
            let key: (Cow<'static, str>, i32, i32) = (Cow::Owned(q.ens), ix, iy);
            let res = state.timeseries_cache.get(&key).map(|r| r.value().clone());
            q.ens = key.0.into_owned();
            res
        };

        if let Some(ts) = cached_ts {
            values.extend_from_slice(&ts);
        } else {
            let mut ts_values = Vec::with_capacity(meta.times.len());
            for &time_val in &meta.times {
                let cached_slice = {
                    let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), time_val);
                    let res = state.grid_cache.get(&key).map(|r| r.value().clone());
                    q.ens = key.0.into_owned();
                    res
                };

                if let Some(slice) = cached_slice {
                    let val_raw = slice[iy as usize * KNMI_GRID_W + ix as usize];
                    if q.ens == "prob" {
                        ts_values.push(val_raw as f64);
                    } else {
                        ts_values.push(raw_to_value(val_raw));
                    }
                }
            }
            state.timeseries_cache.insert(
                (Cow::Owned(q.ens.clone()), ix, iy),
                Arc::new(ts_values.clone()),
            );
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
            let cached_res = {
                let key: (Cow<'static, str>, i64) = (Cow::Owned(q.ens), time_val);
                let res = state.grid_cache.get(&key).map(|r| r.value().clone());
                q.ens = key.0.into_owned();
                res
            };

            if let Some(slice) = cached_res {
                tasks.push(TaskResult::Cached(slice));
            } else {
                let file_path_clone = file_path.clone();
                let meta_clone = meta.clone();
                let ens_clone = q.ens.clone();
                let state_clone = state.clone();
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
                    state_clone
                        .grid_cache
                        .insert((Cow::Owned(ens_clone), time_val), arc.clone());
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
            let (fx, fy) = lonlat_to_grib_indices(q.lon, q.lat);

            for &time_val in &extended_times {
                if time_val > last_radar_time && q.ens == "pmm" {
                    let absolute_time = radar_ref_time + time_val;
                    let harmonie_time = absolute_time - rain_fc.reference_time;
                    let step = rain_fc.steps.iter().min_by_key(|s| {
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
    Ok(axum::Json(TimeseriesResponse {
        status: "ok".to_string(),
        lat: q.lat,
        lon: q.lon,
        ens: q.ens.clone(),
        times: if is_pmm { extended_times } else { meta.times },
        values,
    }))
}'''

    def replace_func_full(name, new_signature_and_body):
        nonlocal content
        # Signature is everything from 'pub async fn {name}(' to the first '{'
        start_marker = f'pub async fn {name}('
        start_idx = content.find(start_marker)
        if start_idx == -1: return

        # Find the opening brace of the function body
        open_brace_idx = content.find('{', start_idx)
        # Find the matching closing brace
        count = 1
        i = open_brace_idx + 1
        while count > 0 and i < len(content):
            if content[i] == '{': count += 1
            elif content[i] == '}': count -= 1
            i += 1

        content = content[:start_idx] + f'pub async fn {name}(' + new_signature_and_body + content[i:]

    replace_func_full('get_data_image', get_data_image_replacement)
    replace_func_full('get_value', get_value_replacement)
    replace_func_full('get_timeseries', get_timeseries_replacement)

    with open('src/handlers.rs', 'w') as f:
        f.write(content)

fix_file()
