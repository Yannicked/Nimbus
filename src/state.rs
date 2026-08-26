use crate::models::{LutEntry, Metadata, RainForecast, SolarForecast, TempForecast, WindForecast};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Holds the active NetCDF radar dataset, its metadata, and all precalculated caches.
pub struct RadarData {
    pub file_path: String,
    pub metadata: Metadata,
    /// Key: (ens, time), value: raw grid slice
    pub grid_cache: DashMap<(String, i64), Arc<Vec<u16>>>,
    /// Key: (ens, time), value: WebP data image bytes
    pub data_cache: DashMap<(String, i64), Vec<u8>>,
    /// Cache for construction of timeseries.
    /// Key: (ens, ix, iy), value: raw timeseries values for the radar steps
    pub timeseries_cache: DashMap<(String, i32, i32), Arc<Vec<f64>>>,
}

impl RadarData {
    pub fn new(file_path: String, metadata: Metadata) -> Self {
        Self {
            file_path,
            metadata,
            grid_cache: DashMap::new(),
            data_cache: DashMap::new(),
            timeseries_cache: DashMap::new(),
        }
    }
}

/// Holds the active 2m temperature forecast and its precalculated WebP image cache.
pub struct TempData {
    pub forecast: TempForecast,
    pub data_cache: DashMap<i64, Vec<u8>>,
}

impl TempData {
    pub fn new(forecast: TempForecast) -> Self {
        Self {
            forecast,
            data_cache: DashMap::new(),
        }
    }
}

/// Holds the active 10m wind forecast and its precalculated WebP image cache.
pub struct WindData {
    pub forecast: WindForecast,
    pub data_cache: DashMap<(u32, i64), Vec<u8>>,
}

impl WindData {
    pub fn new(forecast: WindForecast) -> Self {
        Self {
            forecast,
            data_cache: DashMap::new(),
        }
    }
}

/// Holds the active solar radiation forecast and its precalculated WebP image cache.
pub struct SolarData {
    pub forecast: SolarForecast,
    pub data_cache: DashMap<i64, Vec<u8>>,
}

impl SolarData {
    pub fn new(forecast: SolarForecast) -> Self {
        Self {
            forecast,
            data_cache: DashMap::new(),
        }
    }
}

/// Holds the active Harmonie rain forecast and its precalculated WebP image cache.
pub struct RainData {
    pub forecast: RainForecast,
    pub data_cache: DashMap<i64, Vec<u8>>,
}

impl RainData {
    pub fn new(forecast: RainForecast) -> Self {
        Self {
            forecast,
            data_cache: DashMap::new(),
        }
    }
}

/// Shared application state accessible from all request handlers.
pub struct AppState {
    pub radar_data: RwLock<Option<Arc<RadarData>>>,
    pub projection_lut: Vec<LutEntry>,

    // 2m Temperature Forecast
    pub temp_data: RwLock<Option<Arc<TempData>>>,
    pub temp_projection_lut: Vec<LutEntry>,

    // 10m Wind Forecast
    pub wind_data: RwLock<Option<Arc<WindData>>>,
    pub wind_projection_lut: Vec<LutEntry>,

    // Solar Radiation Forecast
    pub solar_data: RwLock<Option<Arc<SolarData>>>,
    pub solar_projection_lut: Vec<LutEntry>,

    // Rain Forecast (Harmonie)
    pub rain_data: RwLock<Option<Arc<RainData>>>,
}
