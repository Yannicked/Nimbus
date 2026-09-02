use crate::constants::{
    FORECAST_GRID_H, FORECAST_GRID_W, GRID_H, GRID_W, KNMI_DATASET, MERCATOR_BOTTOM, MERCATOR_LEFT,
    MERCATOR_RIGHT, MERCATOR_TOP, NEP_RADIUS, NODATA, PRECIP_VAR, RAIN_THRESHOLD, SCALE_FACTOR,
};
use crate::models::{reduce_ensemble, EnsembleStat, FileUrlResponse, Metadata};
use crate::rendering::render_data_webp_bytes;
use crate::state::AppState;
use axum::http::StatusCode;
use rayon::prelude::*;
use serde::Deserialize;
use std::sync::Arc;

/// Scans a directory for the most-recently-modified `.nc` file and returns its path.
pub async fn find_latest_nc_file(dir: &str) -> Option<String> {
    let mut latest: Option<(std::path::PathBuf, std::time::SystemTime)> = None;
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "nc") {
                if let Ok(meta) = entry.metadata().await {
                    if let Ok(modified) = meta.modified() {
                        if latest
                            .as_ref()
                            .is_none_or(|(_, last_mod)| modified > *last_mod)
                        {
                            latest = Some((path, modified));
                        }
                    }
                }
            }
        }
    }
    latest.map(|(path, _)| path.to_string_lossy().to_string())
}

/// Loads dimension sizes and coordinate variables from a NetCDF file and
/// returns a [`Metadata`] struct suitable for JSON serialisation.
pub async fn load_metadata(
    file_path: &str,
) -> Result<Metadata, Box<dyn std::error::Error + Send + Sync>> {
    let path = file_path.to_string();
    let (ensembles, times, time_units) = tokio::task::spawn_blocking(move || {
        let file = netcdf::open(&path)?;
        let ens_var = file
            .variable("ens_number")
            .ok_or("ens_number variable not found")?;
        let time_var = file.variable("time").ok_or("time variable not found")?;

        let ensembles: Vec<i32> = ens_var.get_values(..)?;
        let times: Vec<i64> = time_var.get_values(..)?;

        let time_units = match time_var
            .attribute("units")
            .ok_or("time units attribute not found")?
            .value()?
        {
            netcdf::AttributeValue::Str(s) => s,
            val => return Err(format!("Unexpected time units type: {:?}", val).into()),
        };
        Ok::<(Vec<i32>, Vec<i64>, String), Box<dyn std::error::Error + Send + Sync>>((
            ensembles, times, time_units,
        ))
    })
    .await??;

    // Use file modified time as the version number for client-side cache invalidation
    let metadata_fs = tokio::fs::metadata(file_path).await?;
    let modified = metadata_fs.modified()?;
    let version = modified
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let radar_times_len = times.len();
    Ok(Metadata {
        left: MERCATOR_LEFT,
        right: MERCATOR_RIGHT,
        bottom: MERCATOR_BOTTOM,
        top: MERCATOR_TOP,
        width: GRID_W,
        height: GRID_H,
        ensembles,
        times,
        reference_time_str: time_units,
        version,
        radar_times_len,
    })
}

/// Reads a 2D slice `(y, x)` for a given ensemble and time index from a NetCDF file.
pub fn read_netcdf_slice(
    file_path: &str,
    ens_idx: usize,
    time_idx: usize,
) -> Result<Vec<u16>, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let var = file
        .variable(PRECIP_VAR)
        .ok_or("precip_intensity variable not found")?;

    let slice = var.get_values::<u16, _>((
        &[ens_idx, time_idx, 0, 0][..],
        &[1, 1, FORECAST_GRID_H, FORECAST_GRID_W][..],
    ))?;
    Ok(slice)
}

/// Reads all ensemble slices for a given time index from a NetCDF file in a single I/O call.
pub fn read_netcdf_all_ensembles(
    file_path: &str,
    time_idx: usize,
    num_ensembles: usize,
) -> Result<Vec<u16>, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let var = file
        .variable(PRECIP_VAR)
        .ok_or("precip_intensity variable not found")?;

    let values = var.get_values::<u16, _>((
        &[0, time_idx, 0, 0][..],
        &[num_ensembles, 1, FORECAST_GRID_H, FORECAST_GRID_W][..],
    ))?;
    Ok(values)
}

/// Helper function to compute the dilated binary mask for Neighborhood Ensemble Probability (NEP).
/// The mask represents whether there is any precipitation exceeding RAIN_THRESHOLD within
/// the specified circular radius (in grid cells) of each pixel.
pub fn compute_dilated_mask(member_data: &[u16], radius: usize) -> Vec<bool> {
    compute_dilated_mask_with_dims(member_data, radius, FORECAST_GRID_W, FORECAST_GRID_H)
}

/// Core logic for mask dilation with custom dimensions.
pub fn compute_dilated_mask_with_dims(
    member_data: &[u16],
    radius: usize,
    width: usize,
    height: usize,
) -> Vec<bool> {
    let mut has_rain = vec![false; height * width];
    let mut has_any_rain = false;
    for i in 0..has_rain.len() {
        let v = member_data[i];
        if v != NODATA && v >= RAIN_THRESHOLD {
            has_rain[i] = true;
            has_any_rain = true;
        }
    }

    if !has_any_rain {
        return vec![false; height * width];
    }

    let mut dilated = vec![false; height * width];
    let r_sq = (radius * radius) as i32;

    // Precompute offsets for circular neighborhood of radius
    let mut offsets = Vec::new();
    for dy in -(radius as i32)..=radius as i32 {
        for dx in -(radius as i32)..=radius as i32 {
            if dx * dx + dy * dy <= r_sq {
                offsets.push((dx, dy));
            }
        }
    }

    // Dilate
    for y in 0..height {
        for x in 0..width {
            if has_rain[y * width + x] {
                for &(dx, dy) in &offsets {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if (0..width as i32).contains(&nx) && (0..height as i32).contains(&ny) {
                        dilated[ny as usize * width + nx as usize] = true;
                    }
                }
            }
        }
    }

    dilated
}

/// Computes (or reads from cache) the raw u16 grid for a given ensemble selector
/// and time step, applying ensemble statistics when `ens_str` is `"med"`, `"max"`,
/// or `"prob"`.
pub fn compute_raw_slice(
    file_path: &str,
    meta: &Metadata,
    ens_str: &str,
    time: i64,
) -> Result<Vec<u16>, (StatusCode, String)> {
    if let Some(stat) = EnsembleStat::from_str(ens_str) {
        let time_idx = meta
            .times
            .iter()
            .position(|&t| t == time)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", time)))?;

        let all_members_data = read_netcdf_all_ensembles(file_path, time_idx, meta.ensembles.len())
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Error reading all ensembles: {}", e),
                )
            })?;

        let grid_size = FORECAST_GRID_H * FORECAST_GRID_W;
        let member_slices: Vec<&[u16]> = all_members_data.chunks_exact(grid_size).collect();

        if matches!(stat, EnsembleStat::Pmm) {
            let grid_size = FORECAST_GRID_H * FORECAST_GRID_W;
            let num_ensembles = member_slices.len();
            let mut transposed = vec![0u16; grid_size * num_ensembles];
            for ens_idx in 0..num_ensembles {
                for i in 0..grid_size {
                    transposed[i * num_ensembles + ens_idx] = member_slices[ens_idx][i];
                }
            }

            let mut valid_indices = Vec::with_capacity(grid_size);
            for i in 0..grid_size {
                if transposed[i * num_ensembles] != NODATA {
                    valid_indices.push(i);
                }
            }

            let mut mean_pairs: Vec<(usize, f32)> = valid_indices
                .par_iter()
                .map(|&i| {
                    let start = i * num_ensembles;
                    let end = start + num_ensembles;
                    let slice = &transposed[start..end];
                    let mut sum = 0.0f32;
                    let mut count = 0;
                    for &v in slice {
                        if v != NODATA {
                            sum += v as f32;
                            count += 1;
                        }
                    }
                    let mean = if count > 0 { sum / count as f32 } else { 0.0 };
                    (i, mean)
                })
                .collect();

            let mut pooled_values = Vec::with_capacity(valid_indices.len() * num_ensembles);
            for &i in &valid_indices {
                let start = i * num_ensembles;
                let end = start + num_ensembles;
                for &v in &transposed[start..end] {
                    if v != NODATA {
                        pooled_values.push(v);
                    }
                }
            }

            pooled_values.par_sort_unstable();
            mean_pairs.par_sort_unstable_by(|a, b| {
                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut pmm_slice = vec![NODATA; grid_size];
            let n_valid = mean_pairs.len();
            let n_pooled = pooled_values.len();
            if n_valid > 0 && n_pooled > 0 {
                let updates: Vec<(usize, u16)> = (0..n_valid)
                    .into_par_iter()
                    .map(|r| {
                        let idx = mean_pairs[r].0;
                        let start_idx = (r * n_pooled) / n_valid;
                        let end_idx = ((r + 1) * n_pooled) / n_valid;
                        let block_len = end_idx - start_idx;
                        let val = if block_len > 0 {
                            let mut sum = 0u64;
                            for &pval in &pooled_values[start_idx..end_idx] {
                                sum += pval as u64;
                            }
                            ((sum + (block_len as u64 / 2)) / block_len as u64) as u16
                        } else {
                            pooled_values[start_idx.min(n_pooled - 1)]
                        };
                        (idx, val)
                    })
                    .collect();

                for (idx, val) in updates {
                    pmm_slice[idx] = val;
                }
            }
            return Ok(pmm_slice);
        }

        if matches!(stat, EnsembleStat::Probability) {
            let num_ensembles = member_slices.len();
            let dilated_masks: Vec<Vec<bool>> = (0..num_ensembles)
                .into_par_iter()
                .map(|ens_idx| compute_dilated_mask(member_slices[ens_idx], NEP_RADIUS))
                .collect();

            let mut raw_slice = vec![NODATA; grid_size];
            for i in 0..grid_size {
                let first_val = member_slices[0][i];
                if first_val == NODATA {
                    raw_slice[i] = NODATA;
                    continue;
                }

                let mut over = 0;
                let mut count = 0;
                for ens_idx in 0..num_ensembles {
                    let v = member_slices[ens_idx][i];
                    if v != NODATA {
                        count += 1;
                        if dilated_masks[ens_idx][i] {
                            over += 1;
                        }
                    }
                }
                raw_slice[i] = if count > 0 {
                    ((over * 100) / count) as u16
                } else {
                    NODATA
                };
            }
            return Ok(raw_slice);
        }

        // Compute statistics for each cell
        let grid_size = FORECAST_GRID_H * FORECAST_GRID_W;
        let mut raw_slice = vec![NODATA; grid_size];
        for i in 0..grid_size {
            let mut vals: Vec<u16> = member_slices.iter().map(|s| s[i]).collect();
            raw_slice[i] = reduce_ensemble(&stat, &mut vals);
        }
        Ok(raw_slice)
    } else {
        // Individual member
        let ens_num: i32 = ens_str.parse().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid ensemble parameter: {}", ens_str),
            )
        })?;

        let ens_idx = meta.ensembles.iter().position(|&e| e == ens_num).ok_or((
            StatusCode::BAD_REQUEST,
            format!("Invalid ensemble number: {}", ens_num),
        ))?;

        let time_idx = meta
            .times
            .iter()
            .position(|&t| t == time)
            .ok_or((StatusCode::BAD_REQUEST, format!("Invalid time: {}", time)))?;

        read_netcdf_slice(file_path, ens_idx, time_idx).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error reading slice: {}", e),
            )
        })
    }
}

pub async fn precalculate_all_data(
    radar_data: Arc<crate::state::RadarData>,
    projection_lut: Arc<Vec<crate::models::LutEntry>>,
    cancel_tracker: Option<(Arc<std::sync::atomic::AtomicU64>, u64)>,
) -> bool {
    let target_version = radar_data.metadata.version;
    let file_path = radar_data.file_path.clone();
    let num_times = radar_data.metadata.times.len();
    let num_ensembles = radar_data.metadata.ensembles.len();

    println!(
        "Starting background precalculation for NetCDF version {} ({} times, {} ensembles)...",
        target_version, num_times, num_ensembles
    );

    // Limit concurrency of rendering tasks to the number of CPU cores (min 2)
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cpus));
    let mut render_handles = Vec::new();

    let grid_size = FORECAST_GRID_H * FORECAST_GRID_W;

    // Loop over time steps
    for (time_idx, &time_val) in radar_data.metadata.times.iter().enumerate() {
        // Check for cancellation if a newer forecast arrived
        if let Some((ref tracker, expected_ver)) = cancel_tracker {
            if tracker.load(std::sync::atomic::Ordering::Relaxed) != expected_ver {
                println!(
                    "Precalculation for version {} cancelled (newer version active).",
                    target_version
                );
                return false;
            }
        }

        if time_idx % 10 == 0 || time_idx == num_times - 1 {
            println!(
                "Precalculating version {}... {}% done ({}/{})",
                target_version,
                (time_idx * 100) / num_times,
                time_idx + 1,
                num_times
            );
        }

        // 1. Offload file I/O to a blocking thread so it doesn't freeze the Tokio executor
        let file_path_clone = file_path.clone();
        let all_members_data = match tokio::task::spawn_blocking(move || {
            read_netcdf_all_ensembles(&file_path_clone, time_idx, num_ensembles)
        })
        .await
        {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => {
                eprintln!(
                    "Error reading all ensemble slices for time index {}: {}",
                    time_idx, e
                );
                continue;
            }
            Err(e) => {
                eprintln!(
                    "Join error reading all ensemble slices for time index {}: {:?}",
                    time_idx, e
                );
                continue;
            }
        };

        // 2. Offload heavy CPU-bound (Rayon) work to prevent starving the Tokio runtime
        let (arc_med, arc_max, arc_prob, arc_spread, arc_pmm, all_members_data) = match
            tokio::task::spawn_blocking(move || {
                // Fast, Cache-Friendly Parallel Transpose
                let mut transposed = vec![0u16; grid_size * num_ensembles];
                transposed
                    .par_chunks_exact_mut(num_ensembles)
                    .enumerate()
                    .for_each(|(i, dest_chunk)| {
                        for ens_idx in 0..num_ensembles {
                            // Writes are contiguous, reads jump (hardware prefetchers handle this better)
                            dest_chunk[ens_idx] = all_members_data[ens_idx * grid_size + i];
                        }
                    });

                // Precompute dilated masks for each member
                let dilated_masks: Vec<Vec<bool>> = (0..num_ensembles)
                    .into_par_iter()
                    .map(|ens_idx| {
                        let start = ens_idx * grid_size;
                        let end = start + grid_size;
                        compute_dilated_mask(&all_members_data[start..end], NEP_RADIUS)
                    })
                    .collect();

                // Allocate stats slices
                let mut med_slice = vec![NODATA; grid_size];
                let mut max_slice = vec![NODATA; grid_size];
                let mut prob_slice = vec![NODATA; grid_size];
                let mut spread_slice = vec![NODATA; grid_size];

                // Compute stats in parallel
                transposed
                    .par_chunks_exact(num_ensembles)
                    .zip(0..grid_size)
                    .zip(med_slice.par_iter_mut())
                    .zip(max_slice.par_iter_mut())
                    .zip(prob_slice.par_iter_mut())
                    .zip(spread_slice.par_iter_mut())
                    .for_each(
                        |(((((ens_vals, i), med_val), max_val), prob_val), spread_val)| {
                            if ens_vals[0] == NODATA {
                                *med_val = NODATA;
                                *max_val = NODATA;
                                *prob_val = NODATA;
                                *spread_val = NODATA;
                                return;
                            }

                            // Copy non-NODATA values into a dynamically-sized buffer
                            let mut valid_vals = Vec::with_capacity(num_ensembles);
                            for &v in ens_vals {
                                if v != NODATA {
                                    valid_vals.push(v);
                                }
                            }

                            if valid_vals.is_empty() {
                                *med_val = NODATA;
                                *max_val = NODATA;
                                *prob_val = NODATA;
                                *spread_val = NODATA;
                                return;
                            }

                            let count = valid_vals.len();
                            let active_vals = &mut valid_vals[..];
                            active_vals.sort_unstable();

                            // Median
                            *med_val = active_vals[count / 2];

                            // Max
                            *max_val = active_vals[count - 1];

                            // Probability (Neighborhood Ensemble Probability)
                            let mut over = 0;
                            for ens_idx in 0..num_ensembles {
                                if ens_vals[ens_idx] != NODATA && dilated_masks[ens_idx][i] {
                                    over += 1;
                                }
                            }
                            *prob_val = ((over * 100) / count) as u16;

                            // Spread (standard deviation)
                            let mut sum = 0.0f64;
                            for &v in active_vals.iter() {
                                sum += v as f64;
                            }
                            let mean = sum / count as f64;

                            let mut variance_sum = 0.0f64;
                            for &v in active_vals.iter() {
                                let diff = v as f64 - mean;
                                variance_sum += diff * diff;
                            }
                            let variance = variance_sum / count as f64;
                            *spread_val = variance.sqrt().round() as u16;
                        },
                    );

                // Compute PMM
                let mut pmm_slice = vec![NODATA; grid_size];
                let mut valid_indices = Vec::with_capacity(grid_size);
                for i in 0..grid_size {
                    if transposed[i * num_ensembles] != NODATA {
                        valid_indices.push(i);
                    }
                }

                let mut mean_pairs: Vec<(usize, f32)> = valid_indices
                    .par_iter()
                    .map(|&i| {
                        let start = i * num_ensembles;
                        let end = start + num_ensembles;
                        let slice = &transposed[start..end];
                        let mut sum = 0.0f32;
                        let mut count = 0;
                        for &v in slice {
                            if v != NODATA {
                                sum += v as f32;
                                count += 1;
                            }
                        }
                        let mean = if count > 0 { sum / count as f32 } else { 0.0 };
                        (i, mean)
                    })
                    .collect();

                let mut pooled_values = Vec::with_capacity(valid_indices.len() * num_ensembles);
                for &i in &valid_indices {
                    let start = i * num_ensembles;
                    let end = start + num_ensembles;
                    for &v in &transposed[start..end] {
                        if v != NODATA {
                            pooled_values.push(v);
                        }
                    }
                }

                pooled_values.par_sort_unstable();
                mean_pairs.par_sort_unstable_by(|a, b| {
                    a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
                });

                let n_valid = mean_pairs.len();
                let n_pooled = pooled_values.len();
                if n_valid > 0 && n_pooled > 0 {
                    let updates: Vec<(usize, u16)> = (0..n_valid)
                        .into_par_iter()
                        .map(|r| {
                            let idx = mean_pairs[r].0;
                            let start_idx = (r * n_pooled) / n_valid;
                            let end_idx = ((r + 1) * n_pooled) / n_valid;
                            let block_len = end_idx - start_idx;
                            let val = if block_len > 0 {
                                let mut sum = 0u64;
                                for &pval in &pooled_values[start_idx..end_idx] {
                                    sum += pval as u64;
                                }
                                ((sum + (block_len as u64 / 2)) / block_len as u64) as u16
                            } else {
                                pooled_values[start_idx.min(n_pooled - 1)]
                            };
                            (idx, val)
                        })
                        .collect();

                    for (idx, val) in updates {
                        pmm_slice[idx] = val;
                    }
                }

                // Return data back to Tokio domain
                (
                    Arc::new(med_slice),
                    Arc::new(max_slice),
                    Arc::new(prob_slice),
                    Arc::new(spread_slice),
                    Arc::new(pmm_slice),
                    all_members_data,
                )
            })
            .await
            {
                Ok(res) => res,
                Err(e) => {
                    eprintln!("Failed to join Rayon statistical reduction task: {:?}", e);
                    continue;
                }
            };

        // Insert stats into grid_cache
        radar_data
            .grid_cache
            .insert(("med".to_string(), time_val), arc_med.clone());
        radar_data
            .grid_cache
            .insert(("max".to_string(), time_val), arc_max.clone());
        radar_data
            .grid_cache
            .insert(("prob".to_string(), time_val), arc_prob.clone());
        radar_data
            .grid_cache
            .insert(("spread".to_string(), time_val), arc_spread.clone());
        radar_data
            .grid_cache
            .insert(("pmm".to_string(), time_val), arc_pmm.clone());

        // Insert individual member slices utilizing zero-math chunking
        for (ens_num, chunk) in radar_data
            .metadata
            .ensembles
            .iter()
            .zip(all_members_data.chunks_exact(grid_size))
        {
            radar_data
                .grid_cache
                .insert((ens_num.to_string(), time_val), Arc::new(chunk.to_vec()));
        }

        // Render WebPs for stats (med, max, prob, spread, pmm)
        let render_items = vec![
            ("med".to_string(), arc_med),
            ("max".to_string(), arc_max),
            ("prob".to_string(), arc_prob),
            ("spread".to_string(), arc_spread),
            ("pmm".to_string(), arc_pmm),
        ];

        for (ens_str, slice) in render_items {
            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                eprintln!("Failed to acquire semaphore permit for rendering WebP");
                continue;
            };
            let radar_data_clone = radar_data.clone();
            let lut_clone = projection_lut.clone();

            let handle = tokio::spawn(async move {
                let slice_clone = slice.clone();
                match tokio::task::spawn_blocking(move || {
                    render_data_webp_bytes(&slice_clone, &lut_clone)
                })
                .await
                {
                    Ok(webp_bytes) => {
                        radar_data_clone
                            .data_cache
                            .insert((ens_str, time_val), webp_bytes);
                    }
                    Err(e) => {
                        eprintln!("Failed to join WebP rendering task for {}: {:?}", ens_str, e);
                    }
                }

                drop(permit);
            });
            render_handles.push(handle);
        }

        // Yield control back to executor
        tokio::task::yield_now().await;
    }

    // Wait for all spawned WebP render tasks to complete before concluding precalculation
    for handle in render_handles {
        let _ = handle.await;
    }

    println!(
        "Background precalculation completed for NetCDF version {}.",
        target_version
    );
    true
}

/// Downloads a new NetCDF file from the KNMI Open Data API, saves it to the
/// current directory, precalculates all slices into a staged RadarData,
/// and atomically activates it without interrupting active requests.
pub async fn download_and_update_nc_file(
    filename: &str,
    file_url: Option<&str>,
    api_key: &str,
    state: Arc<AppState>,
    latest_target_version: Option<Arc<std::sync::atomic::AtomicU64>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Sanitize filename to prevent path traversal
    let safe_filename = std::path::Path::new(filename)
        .file_name()
        .ok_or("Invalid filename in MQTT notification")?
        .to_str()
        .ok_or("Invalid filename characters")?;

    println!(
        "Requesting download URL for {} from KNMI Open Data API...",
        safe_filename
    );

    // Validate that the file_url uses the trusted KNMI domain to prevent SSRF / credential leakage
    let trusted_base = "https://api.dataplatform.knmi.nl/";
    if let Some(ref u) = file_url {
        if !u.starts_with(trusted_base) {
            return Err(format!("Untrusted download URL in MQTT payload: {}", u).into());
        }
    }

    let url = match file_url {
        Some(u) => u.to_string(),
        None => format!(
            "https://api.dataplatform.knmi.nl/open-data/v1/datasets/{}/versions/1.0/files/{}/url",
            KNMI_DATASET, safe_filename
        ),
    };

    let client = reqwest::Client::builder().build()?;

    let res = client
        .get(&url)
        .header("Authorization", api_key)
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(format!("Failed to get download URL, HTTP status: {}", res.status()).into());
    }

    let url_resp: FileUrlResponse = res.json().await?;
    let download_url = url_resp.temporary_download_url;

    // Validate download url is also trusted
    if !download_url.starts_with("https://open-data.dataplatform.knmi.nl/")
        && !download_url.starts_with(trusted_base)
    {
        println!(
            "Warning: Download URL domain differs from KNMI API: {}",
            download_url
        );
    }

    println!("Downloading file from temporary URL: {}...", safe_filename);

    let file_res = client.get(&download_url).send().await?;
    if !file_res.status().is_success() {
        return Err(format!(
            "Failed to download file content, HTTP status: {}",
            file_res.status()
        )
        .into());
    }

    let bytes = file_res.bytes().await?;

    // Use .tmp extension to prevent directory watcher/scanners from opening a half-written file
    let temp_path = format!("{}/{}.tmp", crate::constants::CACHE_DIR, safe_filename);
    tokio::fs::write(&temp_path, &bytes).await?;

    let final_path = format!("{}/{}", crate::constants::CACHE_DIR, safe_filename);
    tokio::fs::rename(&temp_path, &final_path).await?;
    println!("Successfully downloaded and saved: {}", final_path);

    // Load metadata from new file
    let meta = load_metadata(&final_path).await?;
    let target_version = meta.version;

    if let Some(ref tracker) = latest_target_version {
        tracker.store(target_version, std::sync::atomic::Ordering::Relaxed);
    }

    // Create staged RadarData instance and precalculate all data into it
    // The active radar_data continues serving requests seamlessly without interruption!
    let new_radar_data = Arc::new(crate::state::RadarData::new(final_path.clone(), meta));
    let lut_arc = state.projection_lut.clone();

    let tracker_param = latest_target_version
        .as_ref()
        .map(|t| (t.clone(), target_version));
    let success = precalculate_all_data(new_radar_data.clone(), lut_arc, tracker_param).await;

    if success {
        let is_latest = match latest_target_version {
            Some(ref tracker) => {
                tracker.load(std::sync::atomic::Ordering::Relaxed) == target_version
            }
            None => true,
        };

        if is_latest {
            let mut radar_write = state.radar_data.write().await;
            *radar_write = Some(new_radar_data);
            println!(
                "Successfully activated new NetCDF metadata and precalculated caches for: {}",
                final_path
            );
        }
    }

    // Delete old NetCDF files to save space
    if let Ok(mut entries) = tokio::fs::read_dir(crate::constants::CACHE_DIR).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "nc" {
                        if let Some(file_name_str) = path.file_name().and_then(|n| n.to_str()) {
                            if file_name_str != safe_filename
                                && file_name_str.starts_with("KNMI_PYSTEPS_BLEND_ENS_")
                            {
                                println!("Deleting old NetCDF file: {:?}", path);
                                let _ = tokio::fs::remove_file(path).await;
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Fetches the latest NetCDF filename from the KNMI listing endpoint and downloads it.
pub async fn fetch_latest_nc_file(
    dest_dir: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .map_err(|_| "KNMI_OPEN_DATA_API_KEY environment variable missing")?;

    let client = reqwest::Client::new();

    // 1. Query the list endpoint to find the latest file
    let list_url = "https://api.dataplatform.knmi.nl/open-data/v1/datasets/seamless_precipitation_ensemble_forecast_members/versions/1.0/files?maxKeys=1&sorting=desc";
    let list_res = client
        .get(list_url)
        .header("Authorization", &api_key)
        .send()
        .await
        .map_err(|e| format!("Failed to send file listing request: {}", e))?;

    if !list_res.status().is_success() {
        return Err(format!("Failed to list files, HTTP status: {}", list_res.status()).into());
    }

    #[derive(Deserialize)]
    struct KnmiFileEntry {
        filename: String,
    }

    #[derive(Deserialize)]
    struct KnmiListResponse {
        files: Vec<KnmiFileEntry>,
    }

    let list_data: KnmiListResponse = list_res
        .json()
        .await
        .map_err(|e| format!("Failed to parse file listing JSON: {}", e))?;
    let entry = list_data
        .files
        .first()
        .ok_or("No files returned by KNMI API")?;
    let filename = &entry.filename;

    // Sanitize filename
    let safe_filename = std::path::Path::new(filename)
        .file_name()
        .ok_or("Invalid filename from listing")?
        .to_str()
        .ok_or("Invalid characters in filename")?;

    println!("Latest file on KNMI API: {}", safe_filename);

    // 2. Request download URL for this file
    let url_endpoint = format!(
        "https://api.dataplatform.knmi.nl/open-data/v1/datasets/{}/versions/1.0/files/{}/url",
        KNMI_DATASET, safe_filename
    );
    let url_res = client
        .get(&url_endpoint)
        .header("Authorization", &api_key)
        .send()
        .await
        .map_err(|e| format!("Failed to request download URL: {}", e))?;

    if !url_res.status().is_success() {
        return Err(format!(
            "Failed to get download URL, HTTP status: {}",
            url_res.status()
        )
        .into());
    }

    let url_resp: FileUrlResponse = url_res
        .json()
        .await
        .map_err(|e| format!("Failed to parse download URL JSON: {}", e))?;
    let download_url = url_resp.temporary_download_url;

    // 3. Download and save the file
    println!("Downloading file: {}...", safe_filename);
    let file_res = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to send download request: {}", e))?;
    if !file_res.status().is_success() {
        return Err(format!(
            "Failed to download file content, HTTP status: {}",
            file_res.status()
        )
        .into());
    }

    let bytes = file_res
        .bytes()
        .await
        .map_err(|e| format!("Failed to read file bytes: {}", e))?;
    let temp_path = format!("{}/{}.tmp", dest_dir, safe_filename);
    tokio::fs::write(&temp_path, &bytes)
        .await
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    let final_path = format!("{}/{}", dest_dir, safe_filename);
    tokio::fs::rename(&temp_path, &final_path)
        .await
        .map_err(|e| format!("Failed to rename final file: {}", e))?;
    println!(
        "Successfully downloaded and saved initial file: {}",
        final_path
    );

    Ok(final_path)
}

/// Converts a raw u16 grid value to a floating-point value in mm/h.
///
/// [`NODATA`] is mapped to `0.0`.
pub fn raw_to_value(raw: u16) -> f64 {
    if raw == NODATA {
        0.0
    } else {
        raw as f64 * SCALE_FACTOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_to_value() {
        // NODATA should be mapped to 0.0
        assert_eq!(raw_to_value(NODATA), 0.0);

        // 0 should be 0.0
        assert_eq!(raw_to_value(0), 0.0);

        // Typical values
        assert!((raw_to_value(1) - 0.01).abs() < f64::EPSILON);
        assert!((raw_to_value(100) - 1.0).abs() < f64::EPSILON);
        assert!((raw_to_value(1234) - 12.34).abs() < f64::EPSILON);
    }

    #[test]
    fn test_dilated_mask_empty() {
        let width = 5;
        let height = 5;
        let data = vec![0u16; width * height];
        let mask = compute_dilated_mask_with_dims(&data, 1, width, height);
        assert_eq!(mask, vec![false; width * height]);
    }

    #[test]
    fn test_dilated_mask_below_threshold() {
        let width = 5;
        let height = 5;
        let mut data = vec![0u16; width * height];
        data[12] = RAIN_THRESHOLD - 1; // Just below threshold
        let mask = compute_dilated_mask_with_dims(&data, 1, width, height);
        assert_eq!(mask, vec![false; width * height]);
    }

    #[test]
    fn test_dilated_mask_nodata() {
        let width = 5;
        let height = 5;
        let mut data = vec![0u16; width * height];
        data[12] = NODATA; // NODATA should be ignored even if it's "numerically" > RAIN_THRESHOLD
        let mask = compute_dilated_mask_with_dims(&data, 1, width, height);
        assert_eq!(mask, vec![false; width * height]);
    }

    #[test]
    fn test_dilated_mask_single_point() {
        let width = 5;
        let height = 5;
        let mut data = vec![0u16; width * height];
        data[12] = RAIN_THRESHOLD; // Center point (2, 2)
        let mask = compute_dilated_mask_with_dims(&data, 1, width, height);

        // With radius 1, center (2,2) and its 4 direct neighbors should be true
        // and also diagonals if radius^2 allows (dx^2 + dy^2 <= 1^2)
        // dx=1, dy=1 => 1^2 + 1^2 = 2 > 1^2, so diagonals NOT included for radius 1.
        // Expected true at (2,2), (1,2), (3,2), (2,1), (2,3)
        let mut expected = vec![false; width * height];
        expected[12] = true; // (2,2)
        expected[7] = true; // (2,1)
        expected[17] = true; // (2,3)
        expected[11] = true; // (1,2)
        expected[13] = true; // (3,2)

        assert_eq!(mask, expected);
    }

    #[test]
    fn test_dilated_mask_single_point_radius_2() {
        let width = 5;
        let height = 5;
        let mut data = vec![0u16; width * height];
        data[12] = RAIN_THRESHOLD; // Center point (2, 2)
        let mask = compute_dilated_mask_with_dims(&data, 2, width, height);

        // With radius 2, points within dist sqrt(4)=2.
        // dx^2 + dy^2 <= 4
        // (0,0), (1,0), (2,0), (1,1), (0,1), (0,2) etc relative to center.
        // Diagonal (1,1) => 1^2 + 1^2 = 2 <= 4. (Included)
        // Diagonal (2,1) => 2^2 + 1^2 = 5 > 4. (Excluded)
        // Diagonal (2,2) => 2^2 + 2^2 = 8 > 4. (Excluded)

        let mut expected = vec![false; width * height];
        for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                if dx * dx + dy * dy <= 4 {
                    let nx = 2 + dx;
                    let ny = 2 + dy;
                    if (0..5).contains(&nx) && (0..5).contains(&ny) {
                        expected[ny as usize * 5 + nx as usize] = true;
                    }
                }
            }
        }

        assert_eq!(mask, expected);
    }

    #[test]
    fn test_dilated_mask_boundaries() {
        let width = 5;
        let height = 5;
        let mut data = vec![0u16; width * height];
        data[0] = RAIN_THRESHOLD; // Top-left corner (0, 0)
        let mask = compute_dilated_mask_with_dims(&data, 1, width, height);

        let mut expected = vec![false; width * height];
        expected[0] = true; // (0,0)
        expected[1] = true; // (1,0)
        expected[5] = true; // (0,1)

        assert_eq!(mask, expected);
    }

    #[test]
    fn test_dilated_mask_overlap() {
        let width = 5;
        let height = 5;
        let mut data = vec![0u16; width * height];
        data[6] = RAIN_THRESHOLD; // (1, 1)
        data[8] = RAIN_THRESHOLD; // (3, 1)
        let mask = compute_dilated_mask_with_dims(&data, 1, width, height);

        let mut expected = vec![false; width * height];
        // Dilation for (1, 1) radius 1: (1,1), (0,1), (2,1), (1,0), (1,2)
        for &(dx, dy) in &[(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = 1 + dx;
            let ny = 1 + dy;
            expected[ny as usize * width + nx as usize] = true;
        }
        // Dilation for (3, 1) radius 1: (3,1), (2,1), (4,1), (3,0), (3,2)
        for &(dx, dy) in &[(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = 3 + dx;
            let ny = 1 + dy;
            expected[ny as usize * width + nx as usize] = true;
        }

        assert_eq!(mask, expected);
        assert!(expected[7]); // Overlap point (2, 1) is true
    }

    #[tokio::test]
    async fn test_atomic_staged_dataset_swapping() {
        use crate::models::Metadata;
        use crate::state::{AppState, RadarData};
        use std::sync::Arc;

        let meta1 = Metadata {
            left: 0.0,
            right: 1.0,
            bottom: 0.0,
            top: 1.0,
            width: 10,
            height: 10,
            ensembles: vec![0],
            times: vec![0, 300],
            reference_time_str: "2026-01-01T00:00:00Z".to_string(),
            version: 1,
            radar_times_len: 2,
        };

        let meta2 = Metadata {
            left: 0.0,
            right: 1.0,
            bottom: 0.0,
            top: 1.0,
            width: 10,
            height: 10,
            ensembles: vec![0],
            times: vec![300, 600],
            reference_time_str: "2026-01-01T00:05:00Z".to_string(),
            version: 2,
            radar_times_len: 2,
        };

        let radar_data_v1 = Arc::new(RadarData::new("file_v1.nc".to_string(), meta1));
        radar_data_v1
            .data_cache
            .insert(("med".to_string(), 0), vec![1, 2, 3]);

        let state = Arc::new(AppState {
            radar_data: tokio::sync::RwLock::new(Some(radar_data_v1.clone())),
            projection_lut: Arc::new(Vec::new()),
            actuals_data: tokio::sync::RwLock::new(None),
            actuals_projection_lut: Arc::new(Vec::new()),
            temp_data: tokio::sync::RwLock::new(None),
            temp_projection_lut: Arc::new(Vec::new()),
            wind_data: tokio::sync::RwLock::new(None),
            wind_projection_lut: Arc::new(Vec::new()),
            solar_data: tokio::sync::RwLock::new(None),
            solar_projection_lut: Arc::new(Vec::new()),
            rain_data: tokio::sync::RwLock::new(None),
        });

        // Verify active dataset is v1
        {
            let active = state.radar_data.read().await;
            let rd = active.as_ref().unwrap();
            assert_eq!(rd.metadata.version, 1);
            assert_eq!(
                rd.data_cache.get(&("med".to_string(), 0)).unwrap().value(),
                &vec![1, 2, 3]
            );
        }

        // Staging v2 while v1 is serving
        let radar_data_v2 = Arc::new(RadarData::new("file_v2.nc".to_string(), meta2));
        radar_data_v2
            .data_cache
            .insert(("med".to_string(), 300), vec![4, 5, 6]);

        // Concurrent reads still see v1 without any cache clearing or lock starvation
        {
            let active = state.radar_data.read().await;
            let rd = active.as_ref().unwrap();
            assert_eq!(rd.metadata.version, 1);
            assert_eq!(
                rd.data_cache.get(&("med".to_string(), 0)).unwrap().value(),
                &vec![1, 2, 3]
            );
        }

        // Atomically swap
        {
            let mut write = state.radar_data.write().await;
            *write = Some(radar_data_v2);
        }

        // Active dataset is now v2 with warm precalculated caches
        {
            let active = state.radar_data.read().await;
            let rd = active.as_ref().unwrap();
            assert_eq!(rd.metadata.version, 2);
            assert_eq!(
                rd.data_cache
                    .get(&("med".to_string(), 300))
                    .unwrap()
                    .value(),
                &vec![4, 5, 6]
            );
        }
    }

    #[test]
    fn test_read_rtcor_h5() {
        if std::path::Path::new("scratch/test_rtcor.h5").exists() {
            let file = netcdf::open("scratch/test_rtcor.h5").unwrap();
            let image1 = file.group("image1").unwrap().unwrap();
            let var = image1.variable("image_data").unwrap();
            let values: Vec<u16> = var.get_values((.., ..)).unwrap();
            assert_eq!(values.len(), 765 * 700);
        }
    }

    #[test]
    fn test_reduce_ensemble_large_member_count() {
        use crate::models::{reduce_ensemble, EnsembleStat};
        let mut vals: Vec<u16> = (1..=50).collect();
        let med = reduce_ensemble(&EnsembleStat::Median, &mut vals);
        assert_eq!(med, 26);
        let max = reduce_ensemble(&EnsembleStat::Maximum, &mut vals);
        assert_eq!(max, 50);
    }
}
