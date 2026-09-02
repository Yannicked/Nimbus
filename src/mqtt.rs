use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use std::sync::Arc;
use std::time::Duration;

use crate::constants::KNMI_DATASET;
use crate::harmonie::download_and_process_combined_tar;
use crate::radar::download_and_update_nc_file;
use crate::state::AppState;

/// Connects to the KNMI MQTT broker and listens for new dataset notifications.
///
/// When a new NetCDF file is published, it is downloaded and written to the
/// current directory so the file watcher can pick it up.
pub async fn start_knmi_mqtt_listener(state: Arc<AppState>) {
    let broker = "wss://mqtt.dataplatform.knmi.nl";
    let port = 443;
    let mqtt_password = std::env::var("KNMI_MQTT_PASSWORD")
        .expect("KNMI_MQTT_PASSWORD environment variable not set!");
    let open_data_api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .expect("KNMI_OPEN_DATA_API_KEY environment variable not set!");
    let topic = format!("dataplatform/file/v1/{}/1.0/#", KNMI_DATASET);

    let latest_target_version = Arc::new(std::sync::atomic::AtomicU64::new(0));

    loop {
        let client_id = format!(
            "weer-service-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        println!(
            "Initializing KNMI MQTT subscriber with Client ID: {}...",
            client_id
        );

        let mut mqttoptions = MqttOptions::new(client_id, broker, port);
        mqttoptions.set_keep_alive(Duration::from_secs(30));
        mqttoptions.set_credentials("token", &mqtt_password);

        let tls_config = TlsConfiguration::default();
        mqttoptions.set_transport(Transport::wss_with_config(tls_config));

        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 50);

        // Subscribe to topic
        if let Err(e) = client.subscribe(&topic, QoS::AtMostOnce).await {
            eprintln!(
                "Failed to subscribe to KNMI MQTT topic: {:?}. Retrying connection in 10 seconds...",
                e
            );
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }
        println!("Subscribed to KNMI topic: {}", topic);

        // Event loop
        loop {
            match eventloop.poll().await {
                Ok(notification) => {
                    if let Event::Incoming(Packet::Publish(publish)) = notification {
                        let payload_str = match String::from_utf8(publish.payload.to_vec()) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        println!("Received KNMI MQTT notification: {}", payload_str);

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload_str) {
                            let data = json.get("data");
                            let file_name = data
                                .and_then(|d| {
                                    d.get("filename")
                                        .or_else(|| d.get("fileName"))
                                        .or_else(|| d.get("file_name"))
                                })
                                .and_then(|v| v.as_str())
                                .or_else(|| {
                                    json.get("fileName")
                                        .or_else(|| json.get("file_name"))
                                        .and_then(|v| v.as_str())
                                });

                            let file_url = data.and_then(|d| d.get("url")).and_then(|v| v.as_str());

                            if let Some(name) = file_name {
                                if name.ends_with(".nc") {
                                    println!("New NetCDF file available: {}", name);
                                    let state_clone = state.clone();
                                    let name_clone = name.to_string();
                                    let url_opt = file_url.map(|s| s.to_string());
                                    let open_data_api_key_clone = open_data_api_key.to_string();
                                    let tracker_clone = latest_target_version.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = download_and_update_nc_file(
                                            &name_clone,
                                             url_opt.as_deref(),
                                             &open_data_api_key_clone,
                                             state_clone,
                                             Some(tracker_clone),
                                        )
                                        .await
                                        {
                                            eprintln!(
                                                "Error processing file update for {}: {:?}",
                                                name_clone, e
                                            );
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "MQTT Connection error: {:?}. Reconnecting in 10 seconds...",
                        e
                    );
                    break;
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Alias for `start_knmi_mqtt_listener` for seamless precipitation radar ensemble ingestion.
#[allow(dead_code)]
pub async fn start_radar_ensemble_mqtt_listener(state: Arc<AppState>) {
    start_knmi_mqtt_listener(state).await;
}

/// Spawn MQTT client to listen for HARMONIE updates from KNMI (combined temp and wind)
pub async fn start_knmi_harmonie_mqtt_listener(state: Arc<AppState>) {
    let broker = "wss://mqtt.dataplatform.knmi.nl";
    let port = 443;
    let mqtt_password = std::env::var("KNMI_MQTT_PASSWORD")
        .expect("KNMI_MQTT_PASSWORD environment variable not set!");
    let open_data_api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .expect("KNMI_OPEN_DATA_API_KEY environment variable not set!");
    let topic = "dataplatform/file/v1/harmonie_arome_cy43_p1/1.0/#";

    loop {
        let client_id = format!(
            "weer-harmonie-service-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        println!(
            "Initializing KNMI MQTT subscriber for HARMONIE with Client ID: {}...",
            client_id
        );

        let mut mqttoptions = MqttOptions::new(client_id, broker, port);
        mqttoptions.set_keep_alive(Duration::from_secs(30));
        mqttoptions.set_credentials("token", &mqtt_password);

        let tls_config = TlsConfiguration::default();
        mqttoptions.set_transport(Transport::wss_with_config(tls_config));

        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 50);

        if let Err(e) = client.subscribe(topic, QoS::AtMostOnce).await {
            eprintln!(
                "Failed to subscribe to KNMI HARMONIE MQTT topic: {:?}. Retrying connection in 10 seconds...",
                e
            );
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }
        println!("Subscribed to KNMI HARMONIE topic: {}", topic);

        loop {
            match eventloop.poll().await {
                Ok(notification) => {
                    if let Event::Incoming(Packet::Publish(publish)) = notification {
                        let payload_str = match String::from_utf8(publish.payload.to_vec()) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        println!("Received KNMI HARMONIE MQTT notification: {}", payload_str);

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload_str) {
                            let data = json.get("data");
                            let file_name = data
                                .and_then(|d| {
                                    d.get("filename")
                                        .or_else(|| d.get("fileName"))
                                        .or_else(|| d.get("file_name"))
                                })
                                .and_then(|v| v.as_str())
                                .or_else(|| {
                                    json.get("fileName")
                                        .or_else(|| json.get("file_name"))
                                        .and_then(|v| v.as_str())
                                });

                            let file_url = data.and_then(|d| d.get("url")).and_then(|v| v.as_str());

                            if let Some(name) = file_name {
                                if name.ends_with(".tar") {
                                    println!(
                                        "New HARMONIE tar file available (combined): {}",
                                        name
                                    );
                                    let state_clone = state.clone();
                                    let name_clone = name.to_string();
                                    let url_opt = file_url.map(|s| s.to_string());
                                    let api_key = open_data_api_key.to_string();
                                    tokio::spawn(async move {
                                        match download_and_process_combined_tar(
                                            &name_clone,
                                            url_opt.as_deref(),
                                            &api_key,
                                        )
                                        .await
                                        {
                                            Ok((temp_fc, wind_fc, solar_fc, rain_fc)) => {
                                                if let Err(e) = temp_fc.write_to_file(&format!(
                                                    "{}/harmonie_temp.bin",
                                                    crate::constants::CACHE_DIR
                                                )) {
                                                    eprintln!("Failed to save new temperature forecast to bin: {:?}", e);
                                                }
                                                if let Err(e) = wind_fc.write_to_file(&format!(
                                                    "{}/harmonie_wind.bin",
                                                    crate::constants::CACHE_DIR
                                                )) {
                                                    eprintln!("Failed to save new wind forecast to bin: {:?}", e);
                                                }
                                                if let Err(e) = solar_fc.write_to_file(&format!(
                                                    "{}/harmonie_solar.bin",
                                                    crate::constants::CACHE_DIR
                                                )) {
                                                    eprintln!("Failed to save new solar forecast to bin: {:?}", e);
                                                }
                                                if let Err(e) = rain_fc.write_to_file(&format!(
                                                    "{}/harmonie_rain.bin",
                                                    crate::constants::CACHE_DIR
                                                )) {
                                                    eprintln!("Failed to save new rain forecast to bin: {:?}", e);
                                                }

                                                // Create staged forecast objects
                                                let new_temp_data =
                                                    Arc::new(crate::state::TempData::new(temp_fc));
                                                let new_wind_data =
                                                    Arc::new(crate::state::WindData::new(wind_fc));
                                                let new_solar_data = Arc::new(
                                                    crate::state::SolarData::new(solar_fc),
                                                );
                                                let new_rain_data =
                                                    Arc::new(crate::state::RainData::new(rain_fc));

                                                // Precalculate all WebPs into the staged objects before swapping
                                                let temp_fut =
                                                    crate::harmonie::precalculate_temp_data_into(
                                                        &new_temp_data.forecast,
                                                        &state_clone.temp_projection_lut,
                                                        &new_temp_data.data_cache,
                                                    );
                                                let wind_fut =
                                                    crate::harmonie::precalculate_wind_data_into(
                                                        &new_wind_data.forecast,
                                                        &state_clone.wind_projection_lut,
                                                        &new_wind_data.data_cache,
                                                    );
                                                let solar_fut =
                                                    crate::harmonie::precalculate_solar_data_into(
                                                        &new_solar_data.forecast,
                                                        &state_clone.solar_projection_lut,
                                                        &new_solar_data.data_cache,
                                                    );

                                                let radar_ref_info = {
                                                    let radar_opt =
                                                        state_clone.radar_data.read().await;
                                                    radar_opt.as_ref().and_then(|r| {
                                                        let ref_t =
                                                            crate::harmonie::parse_reference_time(
                                                                &r.metadata.reference_time_str,
                                                            )?;
                                                        let last_t = r
                                                            .metadata
                                                            .times
                                                            .last()
                                                            .copied()
                                                            .unwrap_or(0);
                                                        Some((ref_t, last_t))
                                                    })
                                                };

                                                if let Some((ref_t, last_t)) = radar_ref_info {
                                                    let rain_fut = crate::harmonie::precalculate_rain_data_into(
                                                        &new_rain_data.forecast,
                                                        &state_clone.temp_projection_lut,
                                                        &new_rain_data.data_cache,
                                                        ref_t,
                                                        last_t,
                                                    );
                                                    tokio::join!(
                                                        temp_fut, wind_fut, solar_fut, rain_fut
                                                    );
                                                } else {
                                                    tokio::join!(temp_fut, wind_fut, solar_fut);
                                                }

                                                // Atomically swap the new forecasts and precalculated caches into active state
                                                {
                                                    let mut temp_write =
                                                        state_clone.temp_data.write().await;
                                                    *temp_write = Some(new_temp_data);
                                                }
                                                {
                                                    let mut wind_write =
                                                        state_clone.wind_data.write().await;
                                                    *wind_write = Some(new_wind_data);
                                                }
                                                {
                                                    let mut solar_write =
                                                        state_clone.solar_data.write().await;
                                                    *solar_write = Some(new_solar_data);
                                                }
                                                {
                                                    let mut rain_write =
                                                        state_clone.rain_data.write().await;
                                                    *rain_write = Some(new_rain_data);
                                                }

                                                println!("Successfully activated updated temperature, wind, solar, and rain forecasts and precalculated caches.");
                                            }
                                            Err(e) => {
                                                eprintln!("Error processing HARMONIE combined tar file update for {}: {:?}", name_clone, e);
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "HARMONIE MQTT Connection error: {:?}. Reconnecting in 10 seconds...",
                        e
                    );
                    break;
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Spawn MQTT client to listen for real-time corrected radar observations from KNMI (`nl_rdr_data_rtcor_5m`)
pub async fn start_knmi_rtcor_mqtt_listener(state: Arc<AppState>) {
    let broker = "wss://mqtt.dataplatform.knmi.nl";
    let port = 443;
    let mqtt_password = std::env::var("KNMI_MQTT_PASSWORD")
        .expect("KNMI_MQTT_PASSWORD environment variable not set!");
    let open_data_api_key = std::env::var("KNMI_OPEN_DATA_API_KEY")
        .expect("KNMI_OPEN_DATA_API_KEY environment variable not set!");
    let topic = "dataplatform/file/v1/nl_rdr_data_rtcor_5m/1.0/#";

    loop {
        let client_id = format!(
            "weer-rtcor-service-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        println!(
            "Initializing KNMI MQTT subscriber for RTCOR actuals with Client ID: {}...",
            client_id
        );

        let mut mqttoptions = MqttOptions::new(client_id, broker, port);
        mqttoptions.set_keep_alive(Duration::from_secs(30));
        mqttoptions.set_credentials("token", &mqtt_password);

        let tls_config = TlsConfiguration::default();
        mqttoptions.set_transport(Transport::wss_with_config(tls_config));

        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 50);

        if let Err(e) = client.subscribe(topic, QoS::AtMostOnce).await {
            eprintln!(
                "Failed to subscribe to KNMI RTCOR MQTT topic: {:?}. Retrying connection in 10 seconds...",
                e
            );
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }
        println!("Subscribed to KNMI RTCOR topic: {}", topic);

        loop {
            match eventloop.poll().await {
                Ok(notification) => {
                    if let Event::Incoming(Packet::Publish(publish)) = notification {
                        let payload_str = match String::from_utf8(publish.payload.to_vec()) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        println!("Received KNMI RTCOR MQTT notification: {}", payload_str);

                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload_str) {
                            let data = json.get("data");
                            let file_name = data
                                .and_then(|d| {
                                    d.get("filename")
                                        .or_else(|| d.get("fileName"))
                                        .or_else(|| d.get("file_name"))
                                })
                                .and_then(|v| v.as_str())
                                .or_else(|| {
                                    json.get("fileName")
                                        .or_else(|| json.get("file_name"))
                                        .and_then(|v| v.as_str())
                                });

                            let file_url = data.and_then(|d| d.get("url")).and_then(|v| v.as_str());

                            if let Some(name) = file_name {
                                if name.ends_with(".h5") {
                                    println!("New RTCOR radar actuals file available: {}", name);
                                    let state_clone = state.clone();
                                    let name_clone = name.to_string();
                                    let url_opt = file_url.map(|s| s.to_string());
                                    let api_key = open_data_api_key.to_string();
                                    tokio::spawn(async move {
                                        match crate::rtcor::download_and_process_rtcor_file(
                                            &name_clone,
                                            url_opt.as_deref(),
                                            &api_key,
                                            &state_clone.actuals_projection_lut,
                                        )
                                        .await
                                        {
                                            Ok(frame) => {
                                                let mut actuals_guard =
                                                    state_clone.actuals_data.write().await;
                                                let mut actuals = match actuals_guard.as_ref() {
                                                    Some(a) => (**a).clone(),
                                                    None => crate::state::ActualsData::new(),
                                                };
                                                actuals.insert_or_update(
                                                    frame,
                                                    crate::constants::RTCOR_MAX_HISTORY_FRAMES,
                                                );
                                                *actuals_guard = Some(Arc::new(actuals));
                                                println!(
                                                    "Successfully ingested new RTCOR observation frame for: {}",
                                                    name_clone
                                                );
                                                crate::rtcor::cleanup_old_rtcor_files().await;
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "Error processing RTCOR file update for {}: {:?}",
                                                    name_clone, e
                                                );
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "RTCOR MQTT Connection error: {:?}. Reconnecting in 10 seconds...",
                        e
                    );
                    break;
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
