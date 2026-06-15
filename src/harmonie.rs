use std::sync::Arc;
use std::time::Duration;
use chrono::TimeZone;
use serde::Deserialize;
use crate::constants::NODATA;
use crate::models::{TempForecast, TempStep, WindForecast, WindStep, FileUrlResponse};
use crate::state::AppState;
use crate::rendering::{render_temp_webp_bytes, render_wind_webp_bytes};

pub fn parse_run_time_from_name(filename: &str) -> Option<i64> {
    let parts: Vec<&str> = filename.split('_').collect();
    if parts.len() >= 3 {
        let date_str = parts[2];
        if date_str.len() == 12 {
            let year = date_str[0..4].parse::<i32>().ok()?;
            let month = date_str[4..6].parse::<u32>().ok()?;
            let day = date_str[6..8].parse::<u32>().ok()?;
            let hour = date_str[8..10].parse::<u32>().ok()?;
            let minute = date_str[10..12].parse::<u32>().ok()?;
            
            let utc = chrono::Utc.with_ymd_and_hms(year, month, day, hour, minute, 0).single()?;
            return Some(utc.timestamp());
        }
    }
    None
}

pub fn parse_tar_run_time(filename: &str) -> Option<i64> {
    if filename.starts_with("HARM43_V1_P1_") && filename.ends_with(".tar") {
        let date_part = &filename["HARM43_V1_P1_".len()..filename.len() - 4];
        if date_part.len() == 10 {
            let year = date_part[0..4].parse::<i32>().ok()?;
            let month = date_part[4..6].parse::<u32>().ok()?;
            let day = date_part[6..8].parse::<u32>().ok()?;
            let hour = date_part[8..10].parse::<u32>().ok()?;
            
            let utc = chrono::Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).single()?;
            return Some(utc.timestamp());
        }
    }
    None
}

pub fn process_harmonie_tar_combined(tar_path: &str) -> Result<(TempForecast, WindForecast), Box<dyn std::error::Error + Send + Sync>> {
    use std::io::Read;
    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(file);
    let entries = archive.entries()?;

    let mut temp_steps = Vec::new();
    let mut wind_steps = Vec::new();
    let mut reference_time = 0;

    for entry_res in entries {
        let mut entry = entry_res?;
        let path = entry.path()?.to_path_buf();
        let filename = path.file_name().ok_or("Invalid path")?.to_string_lossy().to_string();

        if !filename.contains("_GB") {
            continue;
        }

        if reference_time == 0 {
            if let Some(t) = parse_run_time_from_name(&filename) {
                reference_time = t;
            }
        }

        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;

        let grib_file = grib_reader::GribFile::from_bytes(data)?;
        let mut temp_vals = None;
        let mut wind_by_level: std::collections::HashMap<u32, (Option<Vec<u16>>, Option<Vec<u16>>)> = std::collections::HashMap::new();
        let mut forecast_hour = 0;

        for idx in 0..grib_file.message_count() {
            let msg = grib_file.message(idx)?;
            if let Some(pds) = msg.grib1_product_definition() {
                if pds.parameter_number == 11 && pds.level_type == 105 && pds.level_value == 2 {
                    forecast_hour = pds.forecast_time().unwrap_or(0) as i32;
                    let vals_f64 = msg.read_flat_data_as_f64()?;
                    if vals_f64.len() == 152100 {
                        let mut values = vec![NODATA; 152100];
                        for (i, &v) in vals_f64.iter().enumerate() {
                            if v.is_finite() {
                                values[i] = (v * 10.0).round() as u16;
                            }
                        }
                        temp_vals = Some(values);
                    }
                } else if pds.level_type == 105 && [10, 50, 100, 200, 300].contains(&(pds.level_value as u32)) {
                    let lvl = pds.level_value as u32;
                    if pds.parameter_number == 33 {
                        forecast_hour = pds.forecast_time().unwrap_or(0) as i32;
                        let vals_f64 = msg.read_flat_data_as_f64()?;
                        if vals_f64.len() == 152100 {
                            let mut values = vec![NODATA; 152100];
                            for (i, &v) in vals_f64.iter().enumerate() {
                                if v.is_finite() {
                                    values[i] = ((v + 100.0) * 100.0).round() as u16;
                                }
                            }
                            wind_by_level.entry(lvl).or_insert((None, None)).0 = Some(values);
                        }
                    } else if pds.parameter_number == 34 {
                        let vals_f64 = msg.read_flat_data_as_f64()?;
                        if vals_f64.len() == 152100 {
                            let mut values = vec![NODATA; 152100];
                            for (i, &v) in vals_f64.iter().enumerate() {
                                if v.is_finite() {
                                    values[i] = ((v + 100.0) * 100.0).round() as u16;
                                }
                            }
                            wind_by_level.entry(lvl).or_insert((None, None)).1 = Some(values);
                        }
                    }
                }
            }
        }

        if let Some(t_vals) = temp_vals {
            temp_steps.push(TempStep {
                forecast_hour,
                width: 390,
                height: 390,
                values: Arc::new(t_vals),
            });
        }
        for (lvl, (u_opt, v_opt)) in wind_by_level {
            if let (Some(u), Some(v)) = (u_opt, v_opt) {
                wind_steps.push(WindStep {
                    forecast_hour,
                    height_level: lvl,
                    width: 390,
                    height: 390,
                    u_values: Arc::new(u),
                    v_values: Arc::new(v),
                });
            }
        }
    }

    temp_steps.sort_by_key(|s| s.forecast_hour);
    wind_steps.sort_by_key(|s| (s.forecast_hour, s.height_level));

    Ok((
        TempForecast {
            reference_time,
            steps: temp_steps,
        },
        WindForecast {
            reference_time,
            steps: wind_steps,
        },
    ))
}

pub async fn fetch_latest_harmonie_filename(api_key: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let list_url = "https://api.dataplatform.knmi.nl/open-data/v1/datasets/harmonie_arome_cy43_p1/versions/1.0/files?maxKeys=1&sorting=desc";
    let list_res = client
        .get(list_url)
        .header("Authorization", api_key)
        .send()
        .await?;

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

    let list_data: KnmiListResponse = list_res.json().await?;
    let entry = list_data.files.first().ok_or("No files returned by KNMI API")?;
    Ok(entry.filename.clone())
}

pub async fn download_and_process_combined_tar(
    filename: &str,
    file_url: Option<&str>,
    api_key: &str,
) -> Result<(TempForecast, WindForecast), Box<dyn std::error::Error + Send + Sync>> {
    println!("Requesting download URL for HARMONIE tar (combined): {}...", filename);
    let url = match file_url {
        Some(u) => u.to_string(),
        None => format!(
            "https://api.dataplatform.knmi.nl/open-data/v1/datasets/harmonie_arome_cy43_p1/versions/1.0/files/{}/url",
            filename
        ),
    };

    let client = reqwest::Client::builder().build()?;
    let res = client.get(&url).header("Authorization", api_key).send().await?;
    if !res.status().is_success() {
        return Err(format!("Failed to get download URL, HTTP status: {}", res.status()).into());
    }

    let url_resp: FileUrlResponse = res.json().await?;
    let download_url = url_resp.temporary_download_url;

    println!("Downloading HARMONIE tar (combined) from temporary URL to temp file...");
    let mut file_res = client.get(&download_url).send().await?;
    if !file_res.status().is_success() {
        return Err(format!("Failed to download tar content, HTTP status: {}", file_res.status()).into());
    }

    let temp_tar_path = "temp_harmonie_combined.tar";
    {
        let mut f = tokio::fs::File::create(temp_tar_path).await?;
        while let Some(chunk) = file_res.chunk().await? {
            tokio::io::copy(&mut &*chunk, &mut f).await?;
        }
    }
    
    println!("Extracting and processing GRIB1 files from tar (combined)...");
    let forecasts = tokio::task::spawn_blocking(move || {
        let res = process_harmonie_tar_combined(temp_tar_path);
        let _ = std::fs::remove_file(temp_tar_path);
        res
    }).await??;

    println!("HARMONIE forecast (combined) processed successfully: {} temp steps, {} wind steps", forecasts.0.steps.len(), forecasts.1.steps.len());
    Ok(forecasts)
}

pub async fn load_or_fetch_combined_forecast(api_key: &str) -> (TempForecast, WindForecast) {
    let temp_bin_path = "./harmonie_temp.bin";
    let wind_bin_path = "./harmonie_wind.bin";

    let temp_fc_opt = if std::path::Path::new(temp_bin_path).exists() {
        println!("Found local temperature cache: {}", temp_bin_path);
        TempForecast::read_from_file(temp_bin_path).ok()
    } else {
        None
    };

    let wind_fc_opt = if std::path::Path::new(wind_bin_path).exists() {
        println!("Found local wind cache: {}", wind_bin_path);
        WindForecast::read_from_file(wind_bin_path).ok()
    } else {
        None
    };

    // If both exist, check if there's a newer run
    if let (Some(temp_fc), Some(wind_fc)) = (temp_fc_opt, wind_fc_opt) {
        let cached_ref_time = temp_fc.reference_time.min(wind_fc.reference_time);
        println!("Successfully loaded cached temperature and wind forecast runs. Cached ref time: {}", cached_ref_time);
        
        match fetch_latest_harmonie_filename(api_key).await {
            Ok(latest_filename) => {
                if let Some(api_time) = parse_tar_run_time(&latest_filename) {
                    if api_time > cached_ref_time {
                        println!("Newer run available on KNMI API: {} (cached is {}). Downloading...", api_time, cached_ref_time);
                        if let Ok((new_temp, new_wind)) = download_and_process_combined_tar(&latest_filename, None, api_key).await {
                            if let Err(e) = new_temp.write_to_file(temp_bin_path) {
                                eprintln!("Failed to save new temperature forecast to bin: {:?}", e);
                            }
                            if let Err(e) = new_wind.write_to_file(wind_bin_path) {
                                eprintln!("Failed to save new wind forecast to bin: {:?}", e);
                            }
                            return (new_temp, new_wind);
                        }
                    } else {
                        println!("Local forecast caches are up to date with API: {}", cached_ref_time);
                        return (temp_fc, wind_fc);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to query latest run from KNMI API: {:?}", e);
            }
        }
        return (temp_fc, wind_fc);
    }

    // If either is missing, we must download the latest run
    println!("One or both HARMONIE caches are missing or invalid. Downloading latest run...");
    loop {
        match fetch_latest_harmonie_filename(api_key).await {
            Ok(latest_filename) => {
                match download_and_process_combined_tar(&latest_filename, None, api_key).await {
                    Ok((temp_fc, wind_fc)) => {
                        if let Err(e) = temp_fc.write_to_file(temp_bin_path) {
                            eprintln!("Failed to save temperature forecast to bin: {:?}", e);
                        }
                        if let Err(e) = wind_fc.write_to_file(wind_bin_path) {
                            eprintln!("Failed to save wind forecast to bin: {:?}", e);
                        }
                        return (temp_fc, wind_fc);
                    }
                    Err(e) => {
                        eprintln!("Failed to download/process latest combined run: {:?}. Retrying in 10 seconds...", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to get latest filename: {:?}. Retrying in 10 seconds...", e);
            }
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

pub fn cleanup_tar_files() {
    if let Ok(entries) = std::fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "tar" {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            if stem.starts_with("HARM43_") || stem == "temp_harmonie" || stem == "temp_harmonie_wind" || stem == "temp_harmonie_combined" {
                                println!("Cleaning up leftover tar file: {:?}", path);
                                let _ = std::fs::remove_file(path);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Precalculates all temperature forecast step WebPs in the background.
pub async fn precalculate_temp_data(state: Arc<AppState>) {
    let forecast_opt = state.temp_forecast.read().await;
    let forecast = match forecast_opt.as_ref() {
        Some(fc) => fc,
        None => {
            println!("No temperature forecast loaded, skipping precalculation.");
            return;
        }
    };

    let num_steps = forecast.steps.len();
    if num_steps == 0 {
        return;
    }

    println!(
        "Starting temperature WebP precalculation for {} steps...",
        num_steps
    );

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cpus));

    // Collect step info while we hold the read lock
    let steps_info: Vec<(i64, Arc<Vec<u16>>)> = forecast
        .steps
        .iter()
        .map(|s| {
            let time_key = (s.forecast_hour as i64) * 3600;
            (time_key, s.values.clone())
        })
        .collect();

    // Drop the read lock before spawning tasks
    drop(forecast_opt);

    for (i, (time_key, values)) in steps_info.into_iter().enumerate() {
        let state_clone = state.clone();
        let state_clone_for_blocking = state.clone();
        let sem = semaphore.clone();

        tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let webp_bytes = tokio::task::spawn_blocking(move || {
                render_temp_webp_bytes(&values, &state_clone_for_blocking.temp_projection_lut)
            }).await.unwrap();
            state_clone.temp_data_cache.insert(time_key, webp_bytes);
        });

        if (i + 1) % 10 == 0 || i == num_steps - 1 {
            println!(
                "Temperature precalculation... {}% done ({}/{})",
                ((i + 1) * 100) / num_steps,
                i + 1,
                num_steps
            );
        }
    }

    println!("Temperature WebP precalculation tasks spawned for all {} steps.", num_steps);
}

pub async fn precalculate_wind_data(state: Arc<AppState>) {
    let forecast_opt = state.wind_forecast.read().await;
    let forecast = match forecast_opt.as_ref() {
        Some(fc) => fc,
        None => {
            println!("No wind forecast loaded, skipping precalculation.");
            return;
        }
    };

    let num_steps = forecast.steps.len();
    if num_steps == 0 {
        return;
    }

    println!(
        "Starting wind WebP precalculation for {} steps...",
        num_steps
    );

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cpus));

    let steps_info: Vec<_> = forecast
        .steps
        .iter()
        .map(|s| {
            let time_key = (s.forecast_hour as i64) * 3600;
            (s.height_level, time_key, s.u_values.clone(), s.v_values.clone())
        })
        .collect();

    drop(forecast_opt);

    for (i, (height_level, time_key, u_vals, v_vals)) in steps_info.into_iter().enumerate() {
        let state_clone = state.clone();
        let state_clone_for_blocking = state.clone();
        let sem = semaphore.clone();

        tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let webp_bytes = tokio::task::spawn_blocking(move || {
                render_wind_webp_bytes(&u_vals, &v_vals, &state_clone_for_blocking.wind_projection_lut)
            }).await.unwrap();
            state_clone.wind_data_cache.insert((height_level, time_key), webp_bytes);
        });

        if (i + 1) % 10 == 0 || i == num_steps - 1 {
            println!(
                "Wind precalculation... {}% done ({}/{})",
                ((i + 1) * 100) / num_steps,
                i + 1,
                num_steps
            );
        }
    }

    println!("Wind WebP precalculation tasks spawned for all {} steps.", num_steps);
}
