use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use std::sync::Arc;
use std::time::Duration;

use crate::constants::KNMI_DATASET;
use crate::harmonie::{
    download_and_process_combined_tar, precalculate_rain_data, precalculate_solar_data,
    precalculate_temp_data, precalculate_wind_data,
};
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
                                    tokio::spawn(async move {
                                        if let Err(e) = download_and_update_nc_file(
                                            &name_clone,
                                            url_opt.as_deref(),
                                            &open_data_api_key_clone,
                                            state_clone,
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

                                                // Update temperature forecast in state
                                                {
                                                    let mut temp_write =
                                                        state_clone.temp_forecast.write().await;
                                                    *temp_write = Some(temp_fc);
                                                    state_clone.temp_data_cache.clear();
                                                }

                                                // Update wind forecast in state
                                                {
                                                    let mut wind_write =
                                                        state_clone.wind_forecast.write().await;
                                                    *wind_write = Some(wind_fc);
                                                    state_clone.wind_data_cache.clear();
                                                }

                                                // Update solar forecast in state
                                                {
                                                    let mut solar_write =
                                                        state_clone.solar_forecast.write().await;
                                                    *solar_write = Some(solar_fc);
                                                    state_clone.solar_data_cache.clear();
                                                }

                                                // Update rain forecast in state
                                                {
                                                    let mut rain_write =
                                                        state_clone.rain_forecast.write().await;
                                                    *rain_write = Some(rain_fc);
                                                    state_clone.data_cache.clear();
                                                    state_clone.timeseries_cache.clear();
                                                }

                                                println!("Successfully updated temperature, wind, solar, and rain forecasts and cleared caches.");

                                                // Trigger precalculations in background
                                                let state_precalc_temp = state_clone.clone();
                                                tokio::spawn(async move {
                                                    precalculate_temp_data(state_precalc_temp)
                                                        .await;
                                                });

                                                let state_precalc_wind = state_clone.clone();
                                                tokio::spawn(async move {
                                                    precalculate_wind_data(state_precalc_wind)
                                                        .await;
                                                });

                                                let state_precalc_solar = state_clone.clone();
                                                tokio::spawn(async move {
                                                    precalculate_solar_data(state_precalc_solar)
                                                        .await;
                                                });

                                                let state_precalc_rain = state_clone.clone();
                                                tokio::spawn(async move {
                                                    precalculate_rain_data(state_precalc_rain)
                                                        .await;
                                                });
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
