use std::sync::Arc;
use axum::http::StatusCode;
use serde::Deserialize;
use crate::constants::{
    KNMI_DATASET, KNMI_GRID_H, KNMI_GRID_W, MERCATOR_BOTTOM, MERCATOR_LEFT, MERCATOR_RIGHT,
    MERCATOR_TOP, NODATA, PRECIP_VAR, RAIN_THRESHOLD, SCALE_FACTOR, GRID_W, GRID_H
};
use crate::models::{EnsembleStat, FileUrlResponse, Metadata, reduce_ensemble};
use crate::state::AppState;
use crate::rendering::render_data_png_bytes;

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
pub fn load_metadata(file_path: &str) -> Result<Metadata, Box<dyn std::error::Error + Send + Sync>> {
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
        // Read all ensemble members
        let mut member_slices = Vec::with_capacity(meta.ensembles.len());
        for &ens_val in &meta.ensembles {
            let ens_idx = meta.ensembles.iter().position(|&e| e == ens_val).unwrap();
            let time_idx = meta.times.iter().position(|&t| t == time).ok_or((
                StatusCode::BAD_REQUEST,
                format!("Invalid time: {}", time),
            ))?;
            let slice = read_netcdf_slice(file_path, ens_idx, time_idx).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Error reading slice for E{}: {}", ens_val, e),
                )
            })?;
            member_slices.push(slice);
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

        let ens_idx = meta
            .ensembles
            .iter()
            .position(|&e| e == ens_num)
            .ok_or((
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

/// Precalculates all packed PNG data in the background.
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

    // Loop over time steps
    for (time_idx, &time_val) in meta.times.iter().enumerate() {
        // Check for cancellation
        {
            let current_meta = state.metadata.read().await;
            if current_meta.as_ref().map(|m| m.version) != Some(target_version) {
                println!(
                    "Precalculation for version {} cancelled.",
                    target_version
                );
                return;
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

        // Read all ensemble member slices for this time step in a single sequential I/O read call
        let all_members_data = match read_netcdf_all_ensembles(&file_path, time_idx, num_ensembles) {
            Ok(data) => data,
            Err(e) => {
                eprintln!(
                    "Error reading all ensemble slices for time index {}: {}",
                    time_idx, e
                );
                continue;
            }
        };

        // Transpose the data once to make ensemble members for a given pixel contiguous in memory.
        let grid_size = KNMI_GRID_H * KNMI_GRID_W;
        let mut transposed = vec![0u16; grid_size * num_ensembles];
        for ens_idx in 0..num_ensembles {
            let offset = ens_idx * grid_size;
            let src_slice = &all_members_data[offset..offset + grid_size];
            for i in 0..grid_size {
                transposed[i * num_ensembles + ens_idx] = src_slice[i];
            }
        }

        // Compute stats in parallel using rayon
        let mut med_slice = vec![NODATA; grid_size];
        let mut max_slice = vec![NODATA; grid_size];
        let mut prob_slice = vec![NODATA; grid_size];

        use rayon::prelude::*;
        transposed.par_chunks_exact(num_ensembles)
            .zip(med_slice.par_iter_mut())
            .zip(max_slice.par_iter_mut())
            .zip(prob_slice.par_iter_mut())
            .for_each(|(((ens_vals, med_val), max_val), prob_val)| {
                if ens_vals[0] == NODATA {
                    *med_val = NODATA;
                    *max_val = NODATA;
                    *prob_val = NODATA;
                    return;
                }

                let mut local_vals = [0u16; 128];
                let n = ens_vals.len();
                if n <= 128 {
                    local_vals[..n].copy_from_slice(ens_vals);
                    let active_vals = &mut local_vals[..n];
                    active_vals.sort_unstable();

                    *med_val = active_vals[n / 2];

                    let mut max = 0;
                    for &v in active_vals.iter().rev() {
                        if v != NODATA {
                            max = v;
                            break;
                        }
                    }
                    *max_val = max;

                    let mut count = 0;
                    for &v in active_vals.iter() {
                        if v >= RAIN_THRESHOLD && v != NODATA {
                            count += 1;
                        }
                    }
                    *prob_val = ((count * 100) / n) as u16;
                } else {
                    let mut active_vals = ens_vals.to_vec();
                    active_vals.sort_unstable();

                    *med_val = active_vals[n / 2];

                    let mut max = 0;
                    for &v in active_vals.iter().rev() {
                        if v != NODATA {
                            max = v;
                            break;
                        }
                    }
                    *max_val = max;

                    let mut count = 0;
                    for &v in active_vals.iter() {
                        if v >= RAIN_THRESHOLD && v != NODATA {
                            count += 1;
                        }
                    }
                    *prob_val = ((count * 100) / n) as u16;
                }
            });

        // Insert stats into grid_cache
        let arc_med = Arc::new(med_slice);
        let arc_max = Arc::new(max_slice);
        let arc_prob = Arc::new(prob_slice);

        state.grid_cache.insert(("med".to_string(), time_val), arc_med.clone());
        state.grid_cache.insert(("max".to_string(), time_val), arc_max.clone());
        state.grid_cache.insert(("prob".to_string(), time_val), arc_prob.clone());

        // Insert individual member slices into grid_cache
        for (ens_idx, &ens_num) in meta.ensembles.iter().enumerate() {
            let start = ens_idx * grid_size;
            let end = start + grid_size;
            let slice = all_members_data[start..end].to_vec();
            state.grid_cache.insert((ens_num.to_string(), time_val), Arc::new(slice));
        }

        // Render PNGs for stats (med, max, prob)
        let render_items = vec![
            ("med".to_string(), arc_med),
            ("max".to_string(), arc_max),
            ("prob".to_string(), arc_prob),
        ];

        for (ens_str, slice) in render_items {
            let state_clone = state.clone();
            let state_clone_for_blocking = state.clone();
            let sem = semaphore.clone();
            let ens_str_clone = ens_str.clone();
            let time_val_clone = time_val;
            
            tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let png_bytes = tokio::task::spawn_blocking(move || {
                    render_data_png_bytes(&slice, &state_clone_for_blocking.projection_lut)
                }).await.unwrap();
                state_clone.data_cache.insert((ens_str_clone, time_val_clone), png_bytes);
            });
        }

        // Yield control back to executor to let other tasks run
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
    _state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Requesting download URL for {} from KNMI Open Data API...",
        filename
    );

    let url = match file_url {
        Some(u) => u.to_string(),
        None => format!(
            "https://api.dataplatform.knmi.nl/open-data/v1/datasets/{}/versions/1.0/files/{}/url",
            KNMI_DATASET, filename
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

    println!("Downloading file from temporary URL: {}...", filename);

    let file_res = client.get(&download_url).send().await?;
    if !file_res.status().is_success() {
        return Err(
            format!("Failed to download file content, HTTP status: {}", file_res.status()).into(),
        );
    }

    let bytes = file_res.bytes().await?;

    let temp_path = "temp_knmi_download.nc";
    tokio::fs::write(temp_path, &bytes).await?;

    let final_path = format!("./{}", filename);
    tokio::fs::rename(temp_path, &final_path).await?;
    println!("Successfully downloaded and saved: {}", final_path);

    // Delete old NetCDF files to save space
    if let Ok(mut entries) = tokio::fs::read_dir(".").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "nc" {
                        if let Some(file_stem) = path.file_stem().and_then(|n| n.to_str()) {
                            if file_stem != filename
                                && file_stem.starts_with("KNMI_PYSTEPS_BLEND_ENS_")
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
pub async fn fetch_latest_nc_file(dest_dir: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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
    let entry = list_data.files.first().ok_or("No files returned by KNMI API")?;
    let filename = &entry.filename;

    println!("Latest file on KNMI API: {}", filename);

    // 2. Request download URL for this file
    let url_endpoint = format!(
        "https://api.dataplatform.knmi.nl/open-data/v1/datasets/{}/versions/1.0/files/{}/url",
        KNMI_DATASET, filename
    );
    let url_res = client
        .get(&url_endpoint)
        .header("Authorization", &api_key)
        .send()
        .await
        .map_err(|e| format!("Failed to request download URL: {}", e))?;

    if !url_res.status().is_success() {
        return Err(format!("Failed to get download URL, HTTP status: {}", url_res.status()).into());
    }

    let url_resp: FileUrlResponse = url_res
        .json()
        .await
        .map_err(|e| format!("Failed to parse download URL JSON: {}", e))?;
    let download_url = url_resp.temporary_download_url;

    // 3. Download and save the file
    println!("Downloading file: {}...", filename);
    let file_res = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to send download request: {}", e))?;
    if !file_res.status().is_success() {
        return Err(format!("Failed to download file content, HTTP status: {}", file_res.status()).into());
    }

    let bytes = file_res
        .bytes()
        .await
        .map_err(|e| format!("Failed to read file bytes: {}", e))?;
    let temp_path = format!("{}/temp_knmi_download.nc", dest_dir);
    tokio::fs::write(&temp_path, &bytes)
        .await
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    let final_path = format!("{}/{}", dest_dir, filename);
    tokio::fs::rename(&temp_path, &final_path)
        .await
        .map_err(|e| format!("Failed to rename final file: {}", e))?;
    println!("Successfully downloaded and saved initial file: {}", final_path);

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
