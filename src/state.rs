use std::sync::Arc;
use tokio::sync::RwLock;
use dashmap::DashMap;
use crate::models::{Metadata, TempForecast, WindForecast, LutEntry};

/// Shared application state accessible from all request handlers.
pub struct AppState {
    pub file_path: RwLock<String>,
    /// Key: (ens, time), value: raw grid slice
    pub grid_cache: DashMap<(String, i64), Arc<Vec<u16>>>,
    /// Key: (ens, time), value: PNG data image bytes
    pub data_cache: DashMap<(String, i64), Vec<u8>>,
    pub metadata: RwLock<Option<Metadata>>,
    pub projection_lut: Vec<LutEntry>,
    
    // 2m Temperature Forecast
    pub temp_forecast: RwLock<Option<TempForecast>>,
    pub temp_projection_lut: Vec<LutEntry>,
    pub temp_data_cache: DashMap<i64, Vec<u8>>,

    // 10m Wind Forecast
    pub wind_forecast: RwLock<Option<WindForecast>>,
    pub wind_projection_lut: Vec<LutEntry>,
    pub wind_data_cache: DashMap<(u32, i64), Vec<u8>>,
}
