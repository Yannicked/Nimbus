use crate::constants::{
    GRID_H, GRID_W, KNMI_DATASET, KNMI_GRID_H, KNMI_GRID_W, MERCATOR_BOTTOM, MERCATOR_LEFT,
    MERCATOR_RIGHT, MERCATOR_TOP, NEP_RADIUS, NODATA, PRECIP_VAR, RAIN_THRESHOLD, SCALE_FACTOR,
};
use crate::models::{reduce_ensemble, EnsembleSelector, EnsembleStat, FileUrlResponse, Metadata};
use crate::rendering::render_data_webp_bytes;
use crate::state::AppState;
use axum::http::StatusCode;
use rayon::prelude::*;
use serde::Deserialize;
use std::sync::Arc;

/// Scans a directory for the most-recently-modified `.nc` file and returns its path.
pub fn find_latest_nc_file(dir: &str) -> Option<String> {
    let mut latest: Option<(std::path::PathBuf, std::time::SystemTime)> = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "nc") {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if latest.is_none() || modified > latest.as_ref().unwrap().1 {
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
pub fn load_metadata(
    file_path: &str,
) -> Result<Metadata, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let ens_var = file
        .variable("ens_number")
        .ok_or("ens_number variable not found")?;
    let time_var = file.variable("time").ok_or("time variable not found")?;

    let ensembles = ens_var.get_values::<i32, _>(..)?;
    let times = time_var.get_values::<i64, _>(..)?;

    let time_units = match time_var
        .attribute("units")
        .ok_or("time units attribute not found")?
        .value()?
    {
        netcdf::AttributeValue::Str(s) => s,
        val => return Err(format!("Unexpected time units type: {:?}", val).into()),
    };

    // Use file modified time as the version number for client-side cache invalidation
    let metadata_fs = std::fs::metadata(file_path)?;
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
        &[1, 1, KNMI_GRID_H, KNMI_GRID_W][..],
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
        &[num_ensembles, 1, KNMI_GRID_H, KNMI_GRID_W][..],
    ))?;
    Ok(values)
}

/// Helper function to compute the dilated binary mask for Neighborhood Ensemble Probability (NEP).
/// The mask represents whether there is any precipitation exceeding RAIN_THRESHOLD within
/// the specified circular radius (in grid cells) of each pixel.
pub fn compute_dilated_mask(member_data: &[u16], radius: usize) -> Vec<bool> {
    let mut has_rain = vec![false; KNMI_GRID_H * KNMI_GRID_W];
    let mut has_any_rain = false;
    for i in 0..has_rain.len() {
        let v = member_data[i];
        if v != NODATA && v >= RAIN_THRESHOLD {
            has_rain[i] = true;
            has_any_rain = true;
        }
    }

    if !has_any_rain {
        return vec![false; KNMI_GRID_H * KNMI_GRID_W];
    }

    let mut dilated = vec![false; KNMI_GRID_H * KNMI_GRID_W];
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
    for y in 0..KNMI_GRID_H {
        for x in 0..KNMI_GRID_W {
            if has_rain[y * KNMI_GRID_W + x] {
                for &(dx, dy) in &offsets {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && nx < KNMI_GRID_W as i32 && ny >= 0 && ny < KNMI_GRID_H as i32 {
                        dilated[ny as usize * KNMI_GRID_W + nx as usize] = true;
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

        let grid_size = KNMI_GRID_H * KNMI_GRID_W;
        let member_slices: Vec<&[u16]> = all_members_data.chunks_exact(grid_size).collect();

        if matches!(stat, EnsembleStat::Pmm) {
            let grid_size = KNMI_GRID_H * KNMI_GRID_W;
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
        let grid_size = KNMI_GRID_H * KNMI_GRID_W;
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

pub async fn precalculate_all_data(state: Arc<AppState>, meta: Metadata) {
    let target_version = meta.version;
    let file_path = state.file_path.read().await.clone();
    let num_times = meta.times.len();
    let num_ensembles = meta.ensembles.len();

    println!(
        "Starting background precalculation for NetCDF version {} ({} times, {} ensembles)...",
        target_version, num_times, num_ensembles
    );

    // Limit concurrency of rendering tasks to the number of CPU cores (min 2)
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cpus));

    let grid_size = KNMI_GRID_H * KNMI_GRID_W;

    // Loop over time steps
    for (time_idx, &time_val) in meta.times.iter().enumerate() {
        // Check for cancellation
        if state.metadata.read().await.as_ref().map(|m| m.version) != Some(target_version) {
            println!("Precalculation for version {} cancelled.", target_version);
            return;
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
        .unwrap()
        {
            Ok(data) => data,
            Err(e) => {
                eprintln!(
                    "Error reading all ensemble slices for time index {}: {}",
                    time_idx, e
                );
                continue;
            }
        };

        // 2. Offload heavy CPU-bound (Rayon) work to prevent starving the Tokio runtime
        let (arc_med, arc_max, arc_prob, arc_spread, arc_pmm, all_members_data) =
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

                            // Copy non-NODATA values into a local array
                            let mut valid_vals = [0u16; 32];
                            let mut count = 0;
                            for &v in ens_vals {
                                if v != NODATA {
                                    valid_vals[count] = v;
                                    count += 1;
                                }
                            }

                            if count == 0 {
                                *med_val = NODATA;
                                *max_val = NODATA;
                                *prob_val = NODATA;
                                *spread_val = NODATA;
                                return;
                            }

                            let active_vals = &mut valid_vals[..count];
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
            .unwrap();

        // Insert stats into grid_cache
        state.grid_cache.insert(
            (EnsembleSelector::Stat(EnsembleStat::Median), time_val),
            arc_med.clone(),
        );
        state.grid_cache.insert(
            (EnsembleSelector::Stat(EnsembleStat::Maximum), time_val),
            arc_max.clone(),
        );
        state.grid_cache.insert(
            (EnsembleSelector::Stat(EnsembleStat::Probability), time_val),
            arc_prob.clone(),
        );
        state.grid_cache.insert(
            (EnsembleSelector::Stat(EnsembleStat::Spread), time_val),
            arc_spread.clone(),
        );
        state.grid_cache.insert(
            (EnsembleSelector::Stat(EnsembleStat::Pmm), time_val),
            arc_pmm.clone(),
        );

        // Insert individual member slices utilizing zero-math chunking
        for (ens_num, chunk) in meta
            .ensembles
            .iter()
            .zip(all_members_data.chunks_exact(grid_size))
        {
            state.grid_cache.insert(
                (EnsembleSelector::Member(*ens_num), time_val),
                Arc::new(chunk.to_vec()),
            );
        }

        // Render WebPs for stats (med, max, prob, spread, pmm)
        let render_items = vec![
            (EnsembleSelector::Stat(EnsembleStat::Median), arc_med),
            (EnsembleSelector::Stat(EnsembleStat::Maximum), arc_max),
            (EnsembleSelector::Stat(EnsembleStat::Probability), arc_prob),
            (EnsembleSelector::Stat(EnsembleStat::Spread), arc_spread),
            (EnsembleSelector::Stat(EnsembleStat::Pmm), arc_pmm),
        ];

        for (ens, slice) in render_items {
            // 3. Acquire BEFORE spawning. This exerts backpressure so the loop doesn't read gigabytes
            // of NetCDF files into memory while waiting for the GPU/CPU to finish rendering WebPs.
            let permit = semaphore.clone().acquire_owned().await.unwrap();

            let state_clone = state.clone();

            tokio::spawn(async move {
                let state_for_blocking = state_clone.clone();
                let slice_clone = slice.clone();

                let webp_bytes = tokio::task::spawn_blocking(move || {
                    render_data_webp_bytes(&slice_clone, &state_for_blocking.projection_lut)
                })
                .await
                .unwrap();

                state_clone.data_cache.insert((ens, time_val), webp_bytes);

                // Drop the permit to signal the semaphore that a core has opened up
                drop(permit);
            });
        }

        // Yield control back to executor
        tokio::task::yield_now().await;
    }

    println!(
        "Background precalculation completed for NetCDF version {}.",
        target_version
    );
}

/// Downloads a new NetCDF file from the KNMI Open Data API, saves it to the
/// current directory, and removes stale files.
pub async fn download_and_update_nc_file(
    filename: &str,
    file_url: Option<&str>,
    api_key: &str,
    state: Arc<AppState>,
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

    // Perform atomic in-memory state reloading
    match load_metadata(&final_path) {
        Ok(meta) => {
            let mut file_write = state.file_path.write().await;
            *file_write = final_path.clone();

            let mut meta_write = state.metadata.write().await;
            *meta_write = Some(meta.clone());

            state.grid_cache.clear();
            state.data_cache.clear();
            state.timeseries_cache.clear();
            println!(
                "Successfully reloaded metadata and cleared caches for new file: {}",
                final_path
            );

            let state_clone = state.clone();
            tokio::spawn(async move {
                precalculate_all_data(state_clone, meta).await;
            });
        }
        Err(e) => {
            eprintln!(
                "Failed to load new NetCDF metadata from {}: {}",
                final_path, e
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
}
