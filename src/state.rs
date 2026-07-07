use crate::models::{LutEntry, Metadata, RainForecast, SolarForecast, TempForecast, WindForecast};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state accessible from all request handlers.
pub struct AppState {
    pub file_path: RwLock<Arc<String>>,
    /// Key: (ens, time), value: raw grid slice
    pub grid_cache: DashMap<(String, i64), Arc<Vec<u16>>>,
    /// Key: (ens, time), value: PNG data image bytes
    pub data_cache: DashMap<(String, i64), Vec<u8>>,
    pub metadata: RwLock<Option<Arc<Metadata>>>,
    pub projection_lut: Vec<LutEntry>,

    // 2m Temperature Forecast
    pub temp_forecast: RwLock<Option<TempForecast>>,
    pub temp_projection_lut: Vec<LutEntry>,
    pub temp_data_cache: DashMap<i64, Vec<u8>>,

    // 10m Wind Forecast
    pub wind_forecast: RwLock<Option<WindForecast>>,
    pub wind_projection_lut: Vec<LutEntry>,
    pub wind_data_cache: DashMap<(u32, i64), Vec<u8>>,

    // Solar Radiation Forecast
    pub solar_forecast: RwLock<Option<SolarForecast>>,
    pub solar_projection_lut: Vec<LutEntry>,
    pub solar_data_cache: DashMap<i64, Vec<u8>>,

    // Rain Forecast (Harmonie)
    pub rain_forecast: RwLock<Option<RainForecast>>,

    /// Cache for construction of timeseries.
    /// Key: (ens, ix, iy), value: raw timeseries values for the radar steps
    pub timeseries_cache: DashMap<(String, i32, i32), Arc<Vec<f64>>>,
}
