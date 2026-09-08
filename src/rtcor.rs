//! Ingestion and processing of KNMI real-time gauge-corrected radar composites (`nl_rdr_data_rtcor_5m`).
//!
//! Dataset provides 5-minute radar precipitation accumulations (mm) on the same 700x765
//! Polar Stereographic grid as the forecast product.

use crate::constants::{
    CACHE_DIR, KNMI_RTCOR_DATASET, NODATA, RTCOR_GRID_H, RTCOR_GRID_W, RTCOR_MAX_HISTORY_FRAMES,
};
use crate::models::{FileUrlResponse, LutEntry};
use crate::rendering::render_data_webp_bytes;
use crate::state::{ActualsData, ActualsFrame, AppState};
use chrono::{NaiveDateTime, TimeZone, Utc};
use std::path::Path;
use std::sync::Arc;

/// Parses the UTC unix timestamp from an RTCOR filename (`RAD_NL25_RAC_RT_YYYYMMDDHHMM.h5` or `RAD_NL25_RAC_MFBS_EM_YYYYMMDDHHMM.h5`).
pub fn parse_rtcor_filename_timestamp(filename: &str) -> Option<i64> {
    let clean_name = Path::new(filename).file_name()?.to_str()?;

    if !clean_name.ends_with(".h5") {
        return None;
    }

    let stem = &clean_name[..clean_name.len() - 3];
    let timestamp_str = if let Some(pos) = stem.rfind('_') {
        &stem[pos + 1..]
    } else {
        stem
    };

    if timestamp_str.len() != 12 || !timestamp_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let naive =
        NaiveDateTime::parse_from_str(&format!("{}00", timestamp_str), "%Y%m%d%H%M%S").ok()?;

    Some(Utc.from_utc_datetime(&naive).timestamp())
}

/// Alias for `parse_rtcor_filename_timestamp` for convenience.
#[allow(dead_code)]
pub fn parse_rtcor_timestamp(filename: &str) -> Option<i64> {
    parse_rtcor_filename_timestamp(filename)
}

/// Reads a 5-minute precipitation accumulation HDF5 file, converts pixel values
/// from 5-minute accumulation (0.01 mm) to standard rate units (0.01 mm/h),
/// and returns the 700x765 grid slice.
pub fn read_rtcor_slice(
    file_path: &str,
) -> Result<Vec<u16>, Box<dyn std::error::Error + Send + Sync>> {
    let cpath = std::ffi::CString::new(file_path)?;
    let cdata = std::ffi::CString::new("/image1/image_data")?;
    let expected_len = RTCOR_GRID_H * RTCOR_GRID_W;
    let mut raw_data = vec![0u16; expected_len];

    unsafe {
        let _g = hdf5_metno_sys::LOCK.lock();
        hdf5_metno_sys::h5::H5open();

        let fid = hdf5_metno_sys::h5f::H5Fopen(
            cpath.as_ptr(),
            hdf5_metno_sys::h5f::H5F_ACC_RDONLY,
            hdf5_metno_sys::h5p::H5P_DEFAULT,
        );
        if fid < 0 {
            return Err(format!("Failed to open HDF5 file: {}", file_path).into());
        }

        let did =
            hdf5_metno_sys::h5d::H5Dopen2(fid, cdata.as_ptr(), hdf5_metno_sys::h5p::H5P_DEFAULT);
        if did < 0 {
            hdf5_metno_sys::h5f::H5Fclose(fid);
            return Err(format!("Missing /image1/image_data dataset in {}", file_path).into());
        }

        let read_res = hdf5_metno_sys::h5d::H5Dread(
            did,
            *hdf5_metno_sys::h5t::H5T_NATIVE_UINT16,
            hdf5_metno_sys::h5s::H5S_ALL,
            hdf5_metno_sys::h5s::H5S_ALL,
            hdf5_metno_sys::h5p::H5P_DEFAULT,
            raw_data.as_mut_ptr().cast(),
        );

        hdf5_metno_sys::h5d::H5Dclose(did);
        hdf5_metno_sys::h5f::H5Fclose(fid);

        if read_res < 0 {
            return Err(format!("Failed to read image_data from {}", file_path).into());
        }
    }

    // Convert:
    // Missing (65534) or Out of Image (65535) -> NODATA (65535)
    // 5-min accumulation * 12 = instantaneous rate in mm/h
    //
    // Note on vertical orientation:
    // In KNMI HDF5 RTCOR files, image_data has DISPLAY_ORIGIN = "UL" (Upper-Left),
    // meaning row 0 is North (top) and row 764 is South (bottom).
    // In our Polar Stereographic coordinate grid, index 0 is South (iy = 0) and
    // index 764 is North (iy = 764), matching the Polar Stereographic LUT orientation.
    // We flip the rows vertically so spatial positions align seamlessly.
    let mut converted = vec![NODATA; expected_len];
    for row in 0..RTCOR_GRID_H {
        let target_row = RTCOR_GRID_H - 1 - row;
        for col in 0..RTCOR_GRID_W {
            let pv = raw_data[row * RTCOR_GRID_W + col];
            let val = if pv == 65534 || pv == 65535 {
                NODATA
            } else {
                (pv as u32 * 12).min(65534) as u16
            };
            converted[target_row * RTCOR_GRID_W + col] = val;
        }
    }

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
            &state.actuals_projection_lut,
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

    fn create_test_h5(path: &str) {
        let cpath = std::ffi::CString::new(path).unwrap();
        let cgrp = std::ffi::CString::new("image1").unwrap();
        let cdata = std::ffi::CString::new("image_data").unwrap();
        unsafe {
            let _g = hdf5_metno_sys::LOCK.lock();
            hdf5_metno_sys::h5::H5open();
            let fid = hdf5_metno_sys::h5f::H5Fcreate(
                cpath.as_ptr(),
                hdf5_metno_sys::h5f::H5F_ACC_TRUNC,
                hdf5_metno_sys::h5p::H5P_DEFAULT,
                hdf5_metno_sys::h5p::H5P_DEFAULT,
            );
            assert!(fid >= 0, "Failed to create test HDF5 file");
            let gid = hdf5_metno_sys::h5g::H5Gcreate2(
                fid,
                cgrp.as_ptr(),
                hdf5_metno_sys::h5p::H5P_DEFAULT,
                hdf5_metno_sys::h5p::H5P_DEFAULT,
                hdf5_metno_sys::h5p::H5P_DEFAULT,
            );
            assert!(gid >= 0, "Failed to create image1 group");
            let dims: [hdf5_metno_sys::h5::hsize_t; 2] = [RTCOR_GRID_H as _, RTCOR_GRID_W as _];
            let sid = hdf5_metno_sys::h5s::H5Screate_simple(2, dims.as_ptr(), std::ptr::null());
            assert!(sid >= 0, "Failed to create dataspace");
            let did = hdf5_metno_sys::h5d::H5Dcreate2(
                gid,
                cdata.as_ptr(),
                *hdf5_metno_sys::h5t::H5T_STD_U16LE,
                sid,
                hdf5_metno_sys::h5p::H5P_DEFAULT,
                hdf5_metno_sys::h5p::H5P_DEFAULT,
                hdf5_metno_sys::h5p::H5P_DEFAULT,
            );
            assert!(did >= 0, "Failed to create image_data dataset");
            hdf5_metno_sys::h5d::H5Dclose(did);
            hdf5_metno_sys::h5s::H5Sclose(sid);
            hdf5_metno_sys::h5g::H5Gclose(gid);
            hdf5_metno_sys::h5f::H5Fclose(fid);
        }
    }

    #[test]
    fn test_read_rtcor_slice_fd_leak() {
        let temp_dir = std::env::temp_dir();
        let mut paths = Vec::new();
        for i in 0..10 {
            let p = temp_dir
                .join(format!("rtcor_leak_test_{}.h5", i))
                .to_string_lossy()
                .into_owned();
            create_test_h5(&p);
            paths.push(p);
        }

        for p in &paths {
            let slice = read_rtcor_slice(p).expect("read_rtcor_slice should succeed");
            assert_eq!(slice.len(), RTCOR_GRID_H * RTCOR_GRID_W);
        }

        // Check if any open file descriptors in /proc/self/fd still point to any of our temp files
        let leaked_fds: Vec<String> = std::fs::read_dir("/proc/self/fd")
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| std::fs::read_link(e.path()).ok())
                    .map(|t| t.to_string_lossy().into_owned())
                    .filter(|target| target.contains("rtcor_leak_test_"))
                    .collect()
            })
            .unwrap_or_default();

        for p in &paths {
            let _ = std::fs::remove_file(p);
        }

        assert!(
            leaked_fds.is_empty(),
            "Leaked file descriptors found: {:?}",
            leaked_fds
        );
    }
}
