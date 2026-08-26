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

/// Single historical 5-minute radar observation frame from KNMI RTCOR (`nl_rdr_data_rtcor_5m`).
#[derive(Clone)]
pub struct ActualsFrame {
    /// UTC timestamp of the observation (seconds since Unix epoch)
    pub timestamp: i64,
    /// Raw radar pixel slice (u16 mm/h * 100, NODATA=65535)
    pub raw_values: Arc<Vec<u16>>,
    /// Precalculated WebP image bytes projected onto the Web Mercator tile
    pub webp_bytes: Vec<u8>,
}

/// In-memory store holding the most recent historical radar observation frames (sorted ascending by timestamp).
#[derive(Clone, Default)]
pub struct ActualsData {
    pub frames: Vec<ActualsFrame>,
}

impl ActualsData {
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    pub fn insert_or_update(&mut self, frame: ActualsFrame, max_frames: usize) {
        if let Some(pos) = self
            .frames
            .iter()
            .position(|f| f.timestamp == frame.timestamp)
        {
            self.frames[pos] = frame;
        } else {
            self.frames.push(frame);
            self.frames.sort_by_key(|f| f.timestamp);
        }

        if self.frames.len() > max_frames {
            let overflow = self.frames.len() - max_frames;
            self.frames.drain(0..overflow);
        }
    }
}

/// Shared application state accessible from all request handlers.
pub struct AppState {
    pub radar_data: RwLock<Option<Arc<RadarData>>>,
    pub projection_lut: Arc<Vec<LutEntry>>,

    // 5-minute Real-Time Corrected Radar Observations (Actuals)
    pub actuals_data: RwLock<Option<Arc<ActualsData>>>,

    // 2m Temperature Forecast
    pub temp_data: RwLock<Option<Arc<TempData>>>,
    pub temp_projection_lut: Arc<Vec<LutEntry>>,

    // 10m Wind Forecast
    pub wind_data: RwLock<Option<Arc<WindData>>>,
    pub wind_projection_lut: Arc<Vec<LutEntry>>,

    // Solar Radiation Forecast
    pub solar_data: RwLock<Option<Arc<SolarData>>>,
    pub solar_projection_lut: Arc<Vec<LutEntry>>,

    // Rain Forecast (Harmonie)
    pub rain_data: RwLock<Option<Arc<RainData>>>,
}
