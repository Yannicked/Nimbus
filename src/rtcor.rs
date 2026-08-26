//! Ingestion and processing of KNMI real-time gauge-corrected radar composites (`nl_rdr_data_rtcor_5m`).
//!
//! Dataset provides 5-minute radar precipitation accumulations (mm) on the same 700x765
//! Polar Stereographic grid as the forecast product.

use crate::constants::{
    CACHE_DIR, KNMI_GRID_H, KNMI_GRID_W, KNMI_RTCOR_DATASET, NODATA, RTCOR_MAX_HISTORY_FRAMES,
};
use crate::models::{FileUrlResponse, LutEntry};
use crate::rendering::render_data_webp_bytes;
use crate::state::{ActualsData, ActualsFrame, AppState};
use chrono::{NaiveDateTime, TimeZone, Utc};
use std::path::Path;
use std::sync::Arc;

/// Parses the UTC unix timestamp from an RTCOR filename (`RAD_NL25_RAC_RT_YYYYMMDDHHMM.h5`).
pub fn parse_rtcor_filename_timestamp(filename: &str) -> Option<i64> {
    let clean_name = Path::new(filename).file_name()?.to_str()?;

    let prefix = "RAD_NL25_RAC_RT_";
    if !clean_name.starts_with(prefix) || !clean_name.ends_with(".h5") {
        return None;
    }

    let timestamp_str = &clean_name[prefix.len()..clean_name.len() - 3];
    if timestamp_str.len() != 12 {
        return None;
    }

    let naive =
        NaiveDateTime::parse_from_str(&format!("{}00", timestamp_str), "%Y%m%d%H%M%S").ok()?;

    Some(Utc.from_utc_datetime(&naive).timestamp())
}

/// Reads a 5-minute precipitation accumulation HDF5 file, converts pixel values
/// from 5-minute accumulation (0.01 mm) to standard rate units (0.01 mm/h),
/// and returns the 700x765 grid slice.
pub fn read_rtcor_slice(
    file_path: &str,
) -> Result<Vec<u16>, Box<dyn std::error::Error + Send + Sync>> {
    let file = netcdf::open(file_path)?;
    let img_group = file
        .group("image1")?
        .ok_or("Missing image1 group in RTCOR HDF5 file")?;
    let var = img_group
        .variable("image_data")
        .ok_or("Missing image_data variable in RTCOR HDF5 file")?;

    let expected_len = KNMI_GRID_H * KNMI_GRID_W;
    let raw_data: Vec<u16> = var.get_values((.., ..))?;

    if raw_data.len() != expected_len {
        return Err(format!(
            "Unexpected image_data dimensions: expected {}, got {}",
            expected_len,
            raw_data.len()
        )
        .into());
    }

    // Convert:
    // Missing (65534) or Out of Image (65535) -> NODATA (65535)
    // 5-min accumulation * 12 = instantaneous rate in mm/h
    let converted: Vec<u16> = raw_data
        .into_iter()
        .map(|pv| {
            if pv == 65534 || pv == 65535 {
                NODATA
            } else {
                // pv is in 0.01 mm accumulation -> pv * 12 is in 0.01 mm/h
                (pv as u32 * 12).min(65534) as u16
            }
        })
        .collect();

    Ok(converted)
}

/// Downloads and processes a single RTCOR `.h5` file into an [`ActualsFrame`].
pub async fn download_and_process_rtcor_file(
    filename: &str,
    file_url: Option<&str>,
    api_key: &str,
    lut: &[LutEntry],
) -> Result<ActualsFrame, Box<dyn std::error::Error + Send + Sync>> {
    let safe_filename = Path::new(filename)
        .file_name()
        .ok_or("Invalid filename in MQTT payload")?
        .to_str()
        .ok_or("Invalid filename characters")?;

    let timestamp = parse_rtcor_filename_timestamp(safe_filename)
        .ok_or_else(|| format!("Cannot parse timestamp from filename: {}", safe_filename))?;

    let final_path = format!("{}/{}", CACHE_DIR, safe_filename);

    if !Path::new(&final_path).exists() {
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
                KNMI_RTCOR_DATASET, safe_filename
            ),
        };

        let client = reqwest::Client::builder().build()?;
        let res = client
            .get(&url)
            .header("Authorization", api_key)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(format!(
                "Failed to get download URL for {}, HTTP status: {}",
                safe_filename,
                res.status()
            )
            .into());
        }

        let url_resp: FileUrlResponse = res.json().await?;
        let download_url = url_resp.temporary_download_url;

        let file_res = client.get(&download_url).send().await?;
        if !file_res.status().is_success() {
            return Err(format!(
                "Failed to download {}, HTTP status: {}",
                safe_filename,
                file_res.status()
            )
            .into());
        }

        let bytes = file_res.bytes().await?;
        let temp_path = format!("{}/{}.tmp", CACHE_DIR, safe_filename);
        tokio::fs::write(&temp_path, &bytes).await?;
        tokio::fs::rename(&temp_path, &final_path).await?;
        println!("Successfully downloaded RTCOR observation: {}", final_path);
    }

    let final_path_clone = final_path.clone();
    let raw_slice = tokio::task::spawn_blocking(move || read_rtcor_slice(&final_path_clone))
        .await
        .map_err(|e| format!("Task join error: {}", e))??;

    let lut_vec = lut.to_vec();
    let raw_slice_clone = raw_slice.clone();
    let webp_bytes =
        tokio::task::spawn_blocking(move || render_data_webp_bytes(&raw_slice_clone, &lut_vec))
            .await
            .map_err(|e| format!("Task join error: {}", e))?;

    Ok(ActualsFrame {
        timestamp,
        raw_values: Arc::new(raw_slice),
        webp_bytes,
    })
}

#[derive(serde::Deserialize)]
struct KnmiFilesListResponse {
    files: Vec<KnmiFileInfo>,
}

#[derive(serde::Deserialize)]
struct KnmiFileInfo {
    filename: String,
}

/// Backfills recent RTCOR frames on startup and loads them into AppState.
pub async fn backfill_recent_rtcor_frames(state: Arc<AppState>, api_key: &str) {
    println!("Starting RTCOR real-time radar observation backfill...");

    let url = format!(
        "https://api.dataplatform.knmi.nl/open-data/v1/datasets/{}/versions/1.0/files?maxKeys={}&sorting=desc&orderBy=created",
        KNMI_RTCOR_DATASET, RTCOR_MAX_HISTORY_FRAMES
    );

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create HTTP client for RTCOR backfill: {:?}", e);
            return;
        }
    };

    let res = match client
        .get(&url)
        .header("Authorization", api_key)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to query RTCOR files list: {:?}", e);
            return;
        }
    };

    if !res.status().is_success() {
        eprintln!(
            "Failed to fetch RTCOR files list, HTTP status: {}",
            res.status()
        );
        return;
    }

    let files_list: KnmiFilesListResponse = match res.json().await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to deserialize RTCOR files list: {:?}", e);
            return;
        }
    };

    let mut added_count = 0;
    for file_info in files_list.files.into_iter().rev() {
        match download_and_process_rtcor_file(
            &file_info.filename,
            None,
            api_key,
            &state.projection_lut,
        )
        .await
        {
            Ok(frame) => {
                let mut actuals_guard = state.actuals_data.write().await;
                let mut actuals = match actuals_guard.as_ref() {
                    Some(a) => (**a).clone(),
                    None => ActualsData::new(),
                };
                actuals.insert_or_update(frame, RTCOR_MAX_HISTORY_FRAMES);
                *actuals_guard = Some(Arc::new(actuals));
                added_count += 1;
            }
            Err(e) => {
                eprintln!(
                    "Failed to process backfill RTCOR file {}: {:?}",
                    file_info.filename, e
                );
            }
        }
    }

    println!(
        "RTCOR backfill completed: loaded {} recent 5-min radar frames.",
        added_count
    );

    cleanup_old_rtcor_files().await;
}

/// Deletes `.h5` files from cache older than 4 hours.
pub async fn cleanup_old_rtcor_files() {
    let now = Utc::now().timestamp();
    let cutoff = now - (4 * 3600);

    if let Ok(mut entries) = tokio::fs::read_dir(CACHE_DIR).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "h5" {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if let Some(ts) = parse_rtcor_filename_timestamp(name) {
                                if ts < cutoff {
                                    println!("Removing expired RTCOR observation: {:?}", path);
                                    let _ = tokio::fs::remove_file(&path).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rtcor_timestamp() {
        let filename = "RAD_NL25_RAC_RT_202608261300.h5";
        let ts = parse_rtcor_filename_timestamp(filename).expect("Should parse timestamp");
        let dt = Utc.timestamp_opt(ts, 0).unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-08-26 13:00:00"
        );
    }

    #[test]
    fn test_parse_rtcor_invalid_filename() {
        assert!(parse_rtcor_filename_timestamp("invalid_name.h5").is_none());
        assert!(parse_rtcor_filename_timestamp("RAD_NL25_RAC_RT_2026.h5").is_none());
        assert!(parse_rtcor_filename_timestamp("RAD_NL25_RAC_RT_202608261300.nc").is_none());
    }

    #[test]
    fn test_actuals_store_retention_and_sorting() {
        let mut store = ActualsData::new();
        for i in (0..10).rev() {
            store.insert_or_update(
                ActualsFrame {
                    timestamp: 1000 + (i * 300),
                    raw_values: Arc::new(vec![i as u16]),
                    webp_bytes: vec![1, 2, 3],
                },
                5,
            );
        }

        // Must retain exactly the latest 5 frames sorted ascending
        assert_eq!(store.frames.len(), 5);
        assert_eq!(store.frames[0].timestamp, 1000 + (5 * 300));
        assert_eq!(store.frames[4].timestamp, 1000 + (9 * 300));
        assert_eq!(*store.frames[4].raw_values, vec![9]);
    }
}
