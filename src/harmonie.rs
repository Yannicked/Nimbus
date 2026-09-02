use crate::constants::{GRIB_CELL_COUNT, GRIB_HEIGHT, GRIB_WIDTH, NODATA};
use crate::models::{
    FileUrlResponse, RainForecast, RainStep, SolarForecast, SolarStep, TempForecast, TempStep,
    WindForecast, WindStep,
};
use crate::rendering::{
    render_data_webp_bytes, render_solar_webp_bytes, render_temp_webp_bytes, render_wind_webp_bytes,
};
use crate::state::AppState;
use chrono::TimeZone;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

pub fn parse_forecast_hour_from_name(filename: &str) -> Option<i32> {
    let parts: Vec<&str> = filename.split('_').collect();
    if parts.len() >= 4 {
        let fc_part = parts[3];
        if fc_part.len() >= 3 {
            if let Ok(hours) = fc_part[0..3].parse::<i32>() {
                return Some(hours);
            }
        }
    }
    None
}

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

            let utc = chrono::Utc
                .with_ymd_and_hms(year, month, day, hour, minute, 0)
                .single()?;
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

            let utc = chrono::Utc
                .with_ymd_and_hms(year, month, day, hour, 0, 0)
                .single()?;
            return Some(utc.timestamp());
        }
    }
    None
}

pub fn process_harmonie_tar_combined(
    tar_path: &str,
) -> Result<
    (TempForecast, WindForecast, SolarForecast, RainForecast),
    Box<dyn std::error::Error + Send + Sync>,
> {
    use std::io::Read;
    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(file);
    let entries = archive.entries()?;

    let mut temp_steps = Vec::new();
    let mut wind_steps = Vec::new();
    struct RawSolarStep {
        forecast_hour: i32,
        values: Vec<f64>,
    }
    let mut raw_solar_steps = Vec::new();
    struct RawRainStep {
        forecast_hour: i32,
        values: Vec<f64>,
    }
    let mut raw_rain_steps = Vec::new();
    let mut reference_time = 0;

    for entry_res in entries {
        let mut entry = entry_res?;
        let path = entry.path()?.to_path_buf();
        let filename = path
            .file_name()
            .ok_or("Invalid path")?
            .to_string_lossy()
            .to_string();

        if !filename.contains("_GB") {
            continue;
        }

        if reference_time == 0 {
            if let Some(t) = parse_run_time_from_name(&filename) {
                reference_time = t;
            }
        }

        // Limit maximum size read from tar entry to prevent OOM / Tar Bomb attacks
        let entry_size = entry.size();
        if entry_size > 50_000_000 {
            return Err("Tar entry exceeds maximum size limit (50MB)".into());
        }

        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;

        let grib_file = grib_reader::GribFile::from_bytes(data)?;
        let mut temp_vals = None;
        let mut solar_vals = None;
        let mut rain_vals = None;
        #[allow(clippy::type_complexity)]
        let mut wind_by_level: std::collections::HashMap<
            u32,
            (Option<Vec<u16>>, Option<Vec<u16>>),
        > = std::collections::HashMap::new();

        let entry_filename = filename.clone();
        let mut forecast_hour = parse_forecast_hour_from_name(&entry_filename).unwrap_or(0);

        for idx in 0..grib_file.message_count() {
            let msg = grib_file.message(idx)?;
            if let Some(pds) = msg.grib1_product_definition() {
                if forecast_hour == 0 {
                    forecast_hour = pds.forecast_time().unwrap_or(0) as i32;
                }
                if pds.parameter_number == 11 && pds.level_type == 105 && pds.level_value == 2 {
                    let vals_f64 = msg.read_flat_data_as_f64()?;
                    if vals_f64.len() == GRIB_CELL_COUNT {
                        let mut values = vec![NODATA; GRIB_CELL_COUNT];
                        for (i, &v) in vals_f64.iter().enumerate() {
                            if v.is_finite() {
                                values[i] = (v * 10.0).round() as u16;
                            }
                        }
                        temp_vals = Some(values);
                    }
                } else if pds.parameter_number == 117
                    && pds.level_type == 105
                    && pds.level_value == 0
                {
                    let vals_f64 = msg.read_flat_data_as_f64()?;
                    if vals_f64.len() == GRIB_CELL_COUNT {
                        solar_vals = Some(vals_f64);
                    }
                } else if pds.parameter_number == 61
                    && pds.level_type == 105
                    && pds.level_value == 0
                {
                    let vals_f64 = msg.read_flat_data_as_f64()?;
                    if vals_f64.len() == GRIB_CELL_COUNT {
                        rain_vals = Some(vals_f64);
                    }
                } else if pds.level_type == 105
                    && [10, 50, 100, 200, 300].contains(&(pds.level_value as u32))
                {
                    let lvl = pds.level_value as u32;
                    if pds.parameter_number == 33 {
                        let vals_f64 = msg.read_flat_data_as_f64()?;
                        if vals_f64.len() == GRIB_CELL_COUNT {
                            let mut values = vec![NODATA; GRIB_CELL_COUNT];
                            for (i, &v) in vals_f64.iter().enumerate() {
                                if v.is_finite() {
                                    values[i] = ((v + 100.0) * 100.0).round() as u16;
                                }
                            }
                            wind_by_level.entry(lvl).or_insert((None, None)).0 = Some(values);
                        }
                    } else if pds.parameter_number == 34 {
                        let vals_f64 = msg.read_flat_data_as_f64()?;
                        if vals_f64.len() == GRIB_CELL_COUNT {
                            let mut values = vec![NODATA; GRIB_CELL_COUNT];
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
                width: GRIB_WIDTH,
                height: GRIB_HEIGHT,
                values: Arc::new(t_vals),
            });
        }
        if let Some(s_vals) = solar_vals {
            raw_solar_steps.push(RawSolarStep {
                forecast_hour,
                values: s_vals,
            });
        }
        if let Some(r_vals) = rain_vals {
            raw_rain_steps.push(RawRainStep {
                forecast_hour,
                values: r_vals,
            });
        }
        for (lvl, (u_opt, v_opt)) in wind_by_level {
            if let (Some(u), Some(v)) = (u_opt, v_opt) {
                wind_steps.push(WindStep {
                    forecast_hour,
                    height_level: lvl,
                    width: GRIB_WIDTH,
                    height: GRIB_HEIGHT,
                    u_values: Arc::new(u),
                    v_values: Arc::new(v),
                });
            }
        }
    }

    temp_steps.sort_by_key(|s| s.forecast_hour);
    wind_steps.sort_by_key(|s| (s.forecast_hour, s.height_level));
    raw_solar_steps.sort_by_key(|s| s.forecast_hour);
    raw_rain_steps.sort_by_key(|s| s.forecast_hour);

    let mut solar_steps = Vec::new();
    for k in 0..raw_solar_steps.len() {
        let current_step = &raw_solar_steps[k];
        let mut values = vec![NODATA; GRIB_CELL_COUNT];

        let prev_step = if k > 0 {
            Some(&raw_solar_steps[k - 1])
        } else {
            None
        };

        let prev_hour = prev_step.map(|s| s.forecast_hour).unwrap_or(0);
        let dt_hours = current_step.forecast_hour - prev_hour;
        let dt_seconds = (dt_hours as f64) * 3600.0;

        if dt_seconds > 0.0 {
            #[allow(clippy::needless_range_loop)]
            for i in 0..GRIB_CELL_COUNT {
                let curr_val = current_step.values[i];
                if curr_val.is_finite() {
                    let prev_val = match prev_step {
                        Some(p) if p.values[i].is_finite() => p.values[i],
                        _ => 0.0,
                    };

                    let diff = (curr_val - prev_val).max(0.0);
                    let watts = diff / dt_seconds;
                    values[i] = watts.round().min(65535.0) as u16;
                }
            }
        }

        solar_steps.push(SolarStep {
            forecast_hour: current_step.forecast_hour,
            width: GRIB_WIDTH,
            height: GRIB_HEIGHT,
            values: Arc::new(values),
        });
    }

    let mut rain_steps = Vec::new();
    for k in 0..raw_rain_steps.len() {
        let current_step = &raw_rain_steps[k];
        let mut values = vec![NODATA; GRIB_CELL_COUNT];

        let prev_step = if k > 0 {
            Some(&raw_rain_steps[k - 1])
        } else {
            None
        };

        let prev_hour = prev_step.map(|s| s.forecast_hour).unwrap_or(0);
        let dt_hours = current_step.forecast_hour - prev_hour;

        if dt_hours > 0 {
            #[allow(clippy::needless_range_loop)]
            for i in 0..GRIB_CELL_COUNT {
                let curr_val = current_step.values[i];
                if curr_val.is_finite() {
                    let prev_val = match prev_step {
                        Some(p) if p.values[i].is_finite() => p.values[i],
                        _ => 0.0,
                    };

                    let diff = (curr_val - prev_val).max(0.0);
                    let intensity = diff / (dt_hours as f64);
                    values[i] = (intensity * 100.0).round().min(65534.0) as u16;
                }
            }
        }

        rain_steps.push(RainStep {
            forecast_hour: current_step.forecast_hour,
            width: GRIB_WIDTH,
            height: GRIB_HEIGHT,
            values: Arc::new(values),
        });
    }

    Ok((
        TempForecast {
            reference_time,
            steps: temp_steps,
        },
        WindForecast {
            reference_time,
            steps: wind_steps,
        },
        SolarForecast {
            reference_time,
            steps: solar_steps,
        },
        RainForecast {
            reference_time,
            steps: rain_steps,
        },
    ))
}

pub async fn fetch_latest_harmonie_filename(
    api_key: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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
    let entry = list_data
        .files
        .first()
        .ok_or("No files returned by KNMI API")?;
    Ok(entry.filename.clone())
}

pub async fn download_and_process_combined_tar(
    filename: &str,
    file_url: Option<&str>,
    api_key: &str,
) -> Result<
    (TempForecast, WindForecast, SolarForecast, RainForecast),
    Box<dyn std::error::Error + Send + Sync>,
> {
    // Sanitize filename to prevent path traversal
    let safe_filename = std::path::Path::new(filename)
        .file_name()
        .ok_or("Invalid filename in MQTT notification")?
        .to_str()
        .ok_or("Invalid filename characters")?;

    println!(
        "Requesting download URL for HARMONIE tar (combined): {}...",
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
            "https://api.dataplatform.knmi.nl/open-data/v1/datasets/harmonie_arome_cy43_p1/versions/1.0/files/{}/url",
            safe_filename
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
    if !download_url.starts_with("https://knmi-kdp-datasets-eu-west-1.s3.eu-west-1.amazonaws.com/")
        && !download_url.starts_with(trusted_base)
    {
        println!(
            "Warning: Download URL domain differs from KNMI API: {}",
            download_url
        );
    }

    println!("Downloading HARMONIE tar (combined) from temporary URL to temp file...");
    let mut file_res = client.get(&download_url).send().await?;
    if !file_res.status().is_success() {
        return Err(format!(
            "Failed to download tar content, HTTP status: {}",
            file_res.status()
        )
        .into());
    }

    let temp_tar_path = format!("{}/temp_harmonie_combined.tar", crate::constants::CACHE_DIR);
    {
        let mut f = tokio::fs::File::create(&temp_tar_path).await?;
        while let Some(chunk) = file_res.chunk().await? {
            tokio::io::copy(&mut &*chunk, &mut f).await?;
        }
    }

    println!("Extracting and processing GRIB1 files from tar (combined)...");
    let forecasts = tokio::task::spawn_blocking(move || {
        let res = process_harmonie_tar_combined(&temp_tar_path);
        let _ = std::fs::remove_file(&temp_tar_path);
        res
    })
    .await??;

    println!("HARMONIE forecast (combined) processed successfully: {} temp steps, {} wind steps, {} solar steps, {} rain steps", forecasts.0.steps.len(), forecasts.1.steps.len(), forecasts.2.steps.len(), forecasts.3.steps.len());
    Ok(forecasts)
}

pub async fn load_or_fetch_combined_forecast(
    api_key: &str,
) -> (TempForecast, WindForecast, SolarForecast, RainForecast) {
    let temp_bin_path = format!("{}/harmonie_temp.bin", crate::constants::CACHE_DIR);
    let wind_bin_path = format!("{}/harmonie_wind.bin", crate::constants::CACHE_DIR);
    let solar_bin_path = format!("{}/harmonie_solar.bin", crate::constants::CACHE_DIR);
    let rain_bin_path = format!("{}/harmonie_rain.bin", crate::constants::CACHE_DIR);

    let temp_fc_opt = if std::path::Path::new(&temp_bin_path).exists() {
        println!("Found local temperature cache: {}", temp_bin_path);
        TempForecast::read_from_file(&temp_bin_path).ok()
    } else {
        None
    };

    let wind_fc_opt = if std::path::Path::new(&wind_bin_path).exists() {
        println!("Found local wind cache: {}", wind_bin_path);
        WindForecast::read_from_file(&wind_bin_path).ok()
    } else {
        None
    };

    let solar_fc_opt = if std::path::Path::new(&solar_bin_path).exists() {
        println!("Found local solar cache: {}", solar_bin_path);
        SolarForecast::read_from_file(&solar_bin_path).ok()
    } else {
        None
    };

    let rain_fc_opt = if std::path::Path::new(&rain_bin_path).exists() {
        println!("Found local rain cache: {}", rain_bin_path);
        RainForecast::read_from_file(&rain_bin_path).ok()
    } else {
        None
    };

    // If all four exist, check if there's a newer run
    if let (Some(temp_fc), Some(wind_fc), Some(solar_fc), Some(rain_fc)) =
        (temp_fc_opt, wind_fc_opt, solar_fc_opt, rain_fc_opt)
    {
        let cached_ref_time = temp_fc
            .reference_time
            .min(wind_fc.reference_time)
            .min(solar_fc.reference_time)
            .min(rain_fc.reference_time);
        println!("Successfully loaded cached temperature, wind, solar and rain forecast runs. Cached ref time: {}", cached_ref_time);

        match fetch_latest_harmonie_filename(api_key).await {
            Ok(latest_filename) => {
                if let Some(api_time) = parse_tar_run_time(&latest_filename) {
                    if api_time > cached_ref_time {
                        println!(
                            "Newer run available on KNMI API: {} (cached is {}). Downloading...",
                            api_time, cached_ref_time
                        );
                        if let Ok((new_temp, new_wind, new_solar, new_rain)) =
                            download_and_process_combined_tar(&latest_filename, None, api_key).await
                        {
                            if let Err(e) = new_temp.write_to_file(&temp_bin_path) {
                                eprintln!(
                                    "Failed to save new temperature forecast to bin: {:?}",
                                    e
                                );
                            }
                            if let Err(e) = new_wind.write_to_file(&wind_bin_path) {
                                eprintln!("Failed to save new wind forecast to bin: {:?}", e);
                            }
                            if let Err(e) = new_solar.write_to_file(&solar_bin_path) {
                                eprintln!("Failed to save new solar forecast to bin: {:?}", e);
                            }
                            if let Err(e) = new_rain.write_to_file(&rain_bin_path) {
                                eprintln!("Failed to save new rain forecast to bin: {:?}", e);
                            }
                            return (new_temp, new_wind, new_solar, new_rain);
                        }
                    } else {
                        println!(
                            "Local forecast caches are up to date with API: {}",
                            cached_ref_time
                        );
                        return (temp_fc, wind_fc, solar_fc, rain_fc);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to query latest run from KNMI API: {:?}", e);
            }
        }
        return (temp_fc, wind_fc, solar_fc, rain_fc);
    }

    // If any is missing, attempt to download the latest run with bounded retries
    println!("One or more HARMONIE caches are missing or invalid. Downloading latest run...");
    const MAX_STARTUP_RETRIES: usize = 3;
    for attempt in 1..=MAX_STARTUP_RETRIES {
        match fetch_latest_harmonie_filename(api_key).await {
            Ok(latest_filename) => {
                match download_and_process_combined_tar(&latest_filename, None, api_key).await {
                    Ok((temp_fc, wind_fc, solar_fc, rain_fc)) => {
                        if let Err(e) = temp_fc.write_to_file(&temp_bin_path) {
                            eprintln!("Failed to save temperature forecast to bin: {:?}", e);
                        }
                        if let Err(e) = wind_fc.write_to_file(&wind_bin_path) {
                            eprintln!("Failed to save wind forecast to bin: {:?}", e);
                        }
                        if let Err(e) = solar_fc.write_to_file(&solar_bin_path) {
                            eprintln!("Failed to save solar forecast to bin: {:?}", e);
                        }
                        if let Err(e) = rain_fc.write_to_file(&rain_bin_path) {
                            eprintln!("Failed to save rain forecast to bin: {:?}", e);
                        }
                        return (temp_fc, wind_fc, solar_fc, rain_fc);
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to download/process latest combined run (attempt {}/{}): {:?}",
                            attempt, MAX_STARTUP_RETRIES, e
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "Failed to get latest filename (attempt {}/{}): {:?}",
                    attempt, MAX_STARTUP_RETRIES, e
                );
            }
        }
        if attempt < MAX_STARTUP_RETRIES {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    eprintln!(
        "Warning: Could not fetch initial HARMONIE model run after {} attempts. Continuing server startup with available/empty forecasts.",
        MAX_STARTUP_RETRIES
    );

    // Fall back to existing cached forecasts if available, or empty forecasts
    let temp_fc = TempForecast::read_from_file(&temp_bin_path).unwrap_or_else(|_| TempForecast {
        reference_time: 0,
        steps: Vec::new(),
    });
    let wind_fc = WindForecast::read_from_file(&wind_bin_path).unwrap_or_else(|_| WindForecast {
        reference_time: 0,
        steps: Vec::new(),
    });
    let solar_fc =
        SolarForecast::read_from_file(&solar_bin_path).unwrap_or_else(|_| SolarForecast {
            reference_time: 0,
            steps: Vec::new(),
        });
    let rain_fc = RainForecast::read_from_file(&rain_bin_path).unwrap_or_else(|_| RainForecast {
        reference_time: 0,
        steps: Vec::new(),
    });

    (temp_fc, wind_fc, solar_fc, rain_fc)
}

pub async fn cleanup_tar_files() {
    if let Ok(mut entries) = tokio::fs::read_dir(crate::constants::CACHE_DIR).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if let Ok(file_type) = entry.file_type().await {
                if file_type.is_file() {
                    if let Some(ext) = path.extension() {
                        if ext == "tar" {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                if stem.starts_with("HARM43_")
                                    || stem == "temp_harmonie"
                                    || stem == "temp_harmonie_wind"
                                    || stem == "temp_harmonie_combined"
                                {
                                    println!("Cleaning up leftover tar file: {:?}", path);
                                    let _ = tokio::fs::remove_file(path).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Precalculates all temperature forecast step WebPs into a target cache.
pub async fn precalculate_temp_data_into(
    forecast: &TempForecast,
    lut: &[crate::models::LutEntry],
    cache: &dashmap::DashMap<i64, Vec<u8>>,
) {
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
    let lut_arc = Arc::new(lut.to_vec());

    let steps_info: Vec<(i64, Arc<Vec<u16>>)> = forecast
        .steps
        .iter()
        .map(|s| {
            let time_key = (s.forecast_hour as i64) * 3600;
            (time_key, s.values.clone())
        })
        .collect();

    let mut handles = Vec::new();

    for (i, (time_key, values)) in steps_info.into_iter().enumerate() {
        let sem = semaphore.clone();
        let lut_clone = lut_arc.clone();

        let handle = tokio::spawn(async move {
            let Ok(_permit) = sem.acquire().await else {
                eprintln!("Failed to acquire semaphore for temperature precalculation");
                return None;
            };
            match tokio::task::spawn_blocking(move || render_temp_webp_bytes(&values, &lut_clone))
                .await
            {
                Ok(webp_bytes) => Some((time_key, webp_bytes)),
                Err(e) => {
                    eprintln!("Failed to join temperature rendering task: {:?}", e);
                    None
                }
            }
        });
        handles.push(handle);

        if (i + 1) % 10 == 0 || i == num_steps - 1 {
            println!(
                "Temperature precalculation... {}% done ({}/{})",
                ((i + 1) * 100) / num_steps,
                i + 1,
                num_steps
            );
        }
    }

    for handle in handles {
        if let Ok(Some((time_key, webp_bytes))) = handle.await {
            cache.insert(time_key, webp_bytes);
        }
    }

    println!(
        "Temperature WebP precalculation completed for all {} steps.",
        num_steps
    );
}

/// Legacy wrapper for precalculating temperature on startup.
pub async fn precalculate_temp_data(state: Arc<AppState>) {
    let temp_opt = state.temp_data.read().await;
    if let Some(ref temp_data) = *temp_opt {
        precalculate_temp_data_into(
            &temp_data.forecast,
            &state.temp_projection_lut,
            &temp_data.data_cache,
        )
        .await;
    }
}

/// Precalculates all wind forecast step WebPs into a target cache.
pub async fn precalculate_wind_data_into(
    forecast: &WindForecast,
    lut: &[crate::models::LutEntry],
    cache: &dashmap::DashMap<(u32, i64), Vec<u8>>,
) {
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
    let lut_arc = Arc::new(lut.to_vec());

    let steps_info: Vec<_> = forecast
        .steps
        .iter()
        .map(|s| {
            let time_key = (s.forecast_hour as i64) * 3600;
            (
                s.height_level,
                time_key,
                s.u_values.clone(),
                s.v_values.clone(),
            )
        })
        .collect();

    let mut handles = Vec::new();

    for (i, (height_level, time_key, u_vals, v_vals)) in steps_info.into_iter().enumerate() {
        let sem = semaphore.clone();
        let lut_clone = lut_arc.clone();

        let handle = tokio::spawn(async move {
            let Ok(_permit) = sem.acquire().await else {
                eprintln!("Failed to acquire semaphore for wind precalculation");
                return None;
            };
            match tokio::task::spawn_blocking(move || {
                render_wind_webp_bytes(&u_vals, &v_vals, &lut_clone)
            })
            .await
            {
                Ok(webp_bytes) => Some((height_level, time_key, webp_bytes)),
                Err(e) => {
                    eprintln!("Failed to join wind rendering task: {:?}", e);
                    None
                }
            }
        });
        handles.push(handle);

        if (i + 1) % 10 == 0 || i == num_steps - 1 {
            println!(
                "Wind precalculation... {}% done ({}/{})",
                ((i + 1) * 100) / num_steps,
                i + 1,
                num_steps
            );
        }
    }

    for handle in handles {
        if let Ok(Some((height_level, time_key, webp_bytes))) = handle.await {
            cache.insert((height_level, time_key), webp_bytes);
        }
    }

    println!(
        "Wind WebP precalculation completed for all {} steps.",
        num_steps
    );
}

/// Legacy wrapper for precalculating wind on startup.
pub async fn precalculate_wind_data(state: Arc<AppState>) {
    let wind_opt = state.wind_data.read().await;
    if let Some(ref wind_data) = *wind_opt {
        precalculate_wind_data_into(
            &wind_data.forecast,
            &state.wind_projection_lut,
            &wind_data.data_cache,
        )
        .await;
    }
}

/// Precalculates all solar forecast step WebPs into a target cache.
pub async fn precalculate_solar_data_into(
    forecast: &SolarForecast,
    lut: &[crate::models::LutEntry],
    cache: &dashmap::DashMap<i64, Vec<u8>>,
) {
    let num_steps = forecast.steps.len();
    if num_steps == 0 {
        return;
    }

    println!(
        "Starting solar WebP precalculation for {} steps...",
        num_steps
    );

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cpus));
    let lut_arc = Arc::new(lut.to_vec());

    let steps_info: Vec<(i64, Arc<Vec<u16>>)> = forecast
        .steps
        .iter()
        .map(|s| {
            let time_key = (s.forecast_hour as i64) * 3600;
            (time_key, s.values.clone())
        })
        .collect();

    let mut handles = Vec::new();

    for (i, (time_key, values)) in steps_info.into_iter().enumerate() {
        let sem = semaphore.clone();
        let lut_clone = lut_arc.clone();

        let handle = tokio::spawn(async move {
            let Ok(_permit) = sem.acquire().await else {
                eprintln!("Failed to acquire semaphore for solar precalculation");
                return None;
            };
            match tokio::task::spawn_blocking(move || render_solar_webp_bytes(&values, &lut_clone))
                .await
            {
                Ok(webp_bytes) => Some((time_key, webp_bytes)),
                Err(e) => {
                    eprintln!("Failed to join solar rendering task: {:?}", e);
                    None
                }
            }
        });
        handles.push(handle);

        if (i + 1) % 10 == 0 || i == num_steps - 1 {
            println!(
                "Solar precalculation... {}% done ({}/{})",
                ((i + 1) * 100) / num_steps,
                i + 1,
                num_steps
            );
        }
    }

    for handle in handles {
        if let Ok(Some((time_key, webp_bytes))) = handle.await {
            cache.insert(time_key, webp_bytes);
        }
    }

    println!(
        "Solar WebP precalculation completed for all {} steps.",
        num_steps
    );
}

/// Legacy wrapper for precalculating solar on startup.
pub async fn precalculate_solar_data(state: Arc<AppState>) {
    let solar_opt = state.solar_data.read().await;
    if let Some(ref solar_data) = *solar_opt {
        precalculate_solar_data_into(
            &solar_data.forecast,
            &state.solar_projection_lut,
            &solar_data.data_cache,
        )
        .await;
    }
}

pub fn parse_reference_time(ref_str: &str) -> Option<i64> {
    let parts: Vec<&str> = ref_str.split("since").collect();
    if parts.len() == 2 {
        let date_time_str = parts[1].trim();
        let parts: Vec<&str> = date_time_str.split_whitespace().collect();
        if parts.len() >= 2 {
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            let time_parts: Vec<&str> = parts[1].split(':').collect();
            if date_parts.len() == 3 && time_parts.len() >= 3 {
                let year = date_parts[0].parse::<i32>().ok()?;
                let month = date_parts[1].parse::<u32>().ok()?;
                let day = date_parts[2].parse::<u32>().ok()?;
                let hour = time_parts[0].parse::<u32>().ok()?;
                let minute = time_parts[1].parse::<u32>().ok()?;
                let second = time_parts[2][0..2].parse::<u32>().ok()?;

                let utc = chrono::Utc
                    .with_ymd_and_hms(year, month, day, hour, minute, second)
                    .single()?;
                return Some(utc.timestamp());
            }
        }
    }
    None
}

/// Precalculates extended Harmonie rain forecast step WebPs into a target cache.
pub async fn precalculate_rain_data_into(
    forecast: &RainForecast,
    lut: &[crate::models::LutEntry],
    cache: &dashmap::DashMap<i64, Vec<u8>>,
    radar_ref_time: i64,
    last_radar_time: i64,
) {
    let mut steps_info = Vec::new();
    for step in &forecast.steps {
        let abs_time = forecast.reference_time + (step.forecast_hour as i64) * 3600;
        let relative_offset = abs_time - radar_ref_time;
        if relative_offset > last_radar_time {
            steps_info.push((relative_offset, step.values.clone()));
        }
    }

    let num_steps = steps_info.len();
    if num_steps == 0 {
        return;
    }

    println!(
        "Starting HARMONIE rain WebP precalculation for {} steps...",
        num_steps
    );

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cpus));
    let lut_arc = Arc::new(lut.to_vec());
    let mut handles = Vec::new();

    for (i, (time_key, values)) in steps_info.into_iter().enumerate() {
        let sem = semaphore.clone();
        let lut_clone = lut_arc.clone();

        let handle = tokio::spawn(async move {
            let Ok(_permit) = sem.acquire().await else {
                eprintln!("Failed to acquire semaphore for rain precalculation");
                return None;
            };
            match tokio::task::spawn_blocking(move || render_data_webp_bytes(&values, &lut_clone))
                .await
            {
                Ok(webp_bytes) => Some((time_key, webp_bytes)),
                Err(e) => {
                    eprintln!("Failed to join rain rendering task: {:?}", e);
                    None
                }
            }
        });
        handles.push(handle);

        if (i + 1) % 10 == 0 || i == num_steps - 1 {
            println!(
                "Rain precalculation... {}% done ({}/{})",
                ((i + 1) * 100) / num_steps,
                i + 1,
                num_steps
            );
        }
    }

    for handle in handles {
        if let Ok(Some((time_key, webp_bytes))) = handle.await {
            cache.insert(time_key, webp_bytes);
        }
    }

    println!(
        "Rain WebP precalculation completed for all {} steps.",
        num_steps
    );
}

/// Legacy wrapper for precalculating rain on startup.
pub async fn precalculate_rain_data(state: Arc<AppState>) {
    let rain_opt = state.rain_data.read().await;
    let rain_data = match rain_opt.as_ref() {
        Some(r) => r.clone(),
        None => return,
    };
    drop(rain_opt);

    let radar_opt = state.radar_data.read().await;
    let radar_data = match radar_opt.as_ref() {
        Some(m) => m.clone(),
        None => return,
    };
    drop(radar_opt);

    if let Some(radar_ref_time) = parse_reference_time(&radar_data.metadata.reference_time_str) {
        let last_radar_time = radar_data.metadata.times.last().copied().unwrap_or(0);
        precalculate_rain_data_into(
            &rain_data.forecast,
            &state.temp_projection_lut,
            &rain_data.data_cache,
            radar_ref_time,
            last_radar_time,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_forecast_hour_from_name() {
        // Happy paths (2 prefixes + date + hour)
        assert_eq!(
            parse_forecast_hour_from_name("HA43_N25_202411200600_00100_GB"),
            Some(1)
        );
        assert_eq!(
            parse_forecast_hour_from_name("HA43_N25_202411200600_04800_GB"),
            Some(48)
        );

        // Edge cases
        assert_eq!(
            parse_forecast_hour_from_name("HA43_N25_202411200600_ABC00_GB"),
            None
        );
        assert_eq!(parse_forecast_hour_from_name("HA43_N25_202411200600"), None);
        assert_eq!(
            parse_forecast_hour_from_name("HA43_N25_202411200600_01_GB"),
            None
        );
        assert_eq!(parse_forecast_hour_from_name(""), None);
        assert_eq!(parse_forecast_hour_from_name("short_name"), None);

        // Additional prefix/suffix formats
        assert_eq!(
            parse_forecast_hour_from_name("prefix_v1_202310271200_003_suffix.grib"),
            Some(3)
        );
        assert_eq!(
            parse_forecast_hour_from_name("prefix_v1_202310271200_048.grib"),
            Some(48)
        );
        assert_eq!(parse_forecast_hour_from_name("a_b_c"), None);
        assert_eq!(parse_forecast_hour_from_name("a_b_c_0"), None);
        assert_eq!(parse_forecast_hour_from_name("a_b_c_01"), None);
        assert_eq!(parse_forecast_hour_from_name("a_b_c_abc"), None);
    }

    #[test]
    fn test_parse_run_time_from_name() {
        // 2023-10-27 12:00:00 UTC = 1698408000
        assert_eq!(
            parse_run_time_from_name("prefix_v1_202310271200_003_suffix.grib"),
            Some(1698408000)
        );
        assert_eq!(parse_run_time_from_name("a_b"), None);
        assert_eq!(parse_run_time_from_name("a_b_20231027120"), None); // 11 chars
        assert_eq!(parse_run_time_from_name("a_b_2023102712000"), None); // 13 chars
        assert_eq!(parse_run_time_from_name("a_b_202313271200"), None); // Month 13
        assert_eq!(parse_run_time_from_name("a_b_202310272500"), None); // Hour 25
    }

    #[test]
    fn test_parse_tar_run_time() {
        // 2023-10-27 12:00:00 UTC = 1698408000
        assert_eq!(
            parse_tar_run_time("HARM43_V1_P1_2023102712.tar"),
            Some(1698408000)
        );
        assert_eq!(parse_tar_run_time("WRONG_V1_P1_2023102712.tar"), None);
        assert_eq!(parse_tar_run_time("HARM43_V1_P1_2023102712.zip"), None);
        assert_eq!(parse_tar_run_time("HARM43_V1_P1_202310271.tar"), None); // 9 chars
        assert_eq!(parse_tar_run_time("HARM43_V1_P1_20231027123.tar"), None); // 11 chars
        assert_eq!(parse_tar_run_time("HARM43_V1_P1_2023132712.tar"), None); // Month 13
    }

    #[test]
    fn test_parse_reference_time() {
        // 2023-10-27 12:00:00 UTC = 1698408000
        assert_eq!(
            parse_reference_time("seconds since 2023-10-27 12:00:00"),
            Some(1698408000)
        );
        assert_eq!(parse_reference_time("2023-10-27 12:00:00"), None);
        assert_eq!(parse_reference_time("since 2023-10-27 12:00"), None); // Missing seconds
        assert_eq!(parse_reference_time("since 2023-13-27 12:00:00"), None); // Month 13
    }

    #[tokio::test]
    async fn test_load_or_fetch_combined_forecast_graceful_fallback() {
        let (temp_fc, wind_fc, solar_fc, rain_fc) =
            load_or_fetch_combined_forecast("dummy_key").await;
        assert!(temp_fc.reference_time >= 0);
        assert!(wind_fc.reference_time >= 0);
        assert!(solar_fc.reference_time >= 0);
        assert!(rain_fc.reference_time >= 0);
    }
}
