//! Configuration constants for the weather radar service.

/// Sentinel value used in the u16 grid to indicate missing / no-data pixels.
pub const NODATA: u16 = 65535;

/// Conversion factor from raw u16 grid values to mm/h.
pub const SCALE_FACTOR: f64 = 0.01;

/// Raw value threshold: members with `val >= RAIN_THRESHOLD` count as "raining"
/// when computing probability.
pub const RAIN_THRESHOLD: u16 = 10;

/// The radius (in grid cells) for Neighborhood Ensemble Probability.
/// At 1 km per pixel, this represents 10 km.
pub const NEP_RADIUS: usize = 10;

/// KNMI Open Data dataset identifier for seamless forecast.
pub const KNMI_DATASET: &str = "seamless_precipitation_ensemble_forecast_members";

/// KNMI Open Data dataset identifier for 5-minute real-time gauge-corrected radar observations (RTCOR).
pub const KNMI_RTCOR_DATASET: &str = "nl_rdr_data_rtcor_5m";

/// Maximum number of historical 5-minute actuals frames to retain in memory (24 frames = 2 hours).
pub const RTCOR_MAX_HISTORY_FRAMES: usize = 24;

/// NetCDF variable name for precipitation intensity.
pub const PRECIP_VAR: &str = "precip_intensity";

// Target Web Mercator grid dimensions and bounds
pub const GRID_W: u32 = 700;
pub const GRID_H: u32 = 765;
pub const MERCATOR_LEFT: f64 = 0.0;
pub const MERCATOR_RIGHT: f64 = 1210000.0;
pub const MERCATOR_BOTTOM: f64 = 6250000.0;
pub const MERCATOR_TOP: f64 = 7560000.0;

// KNMI seamless precipitation forecast grid parameters (780 x 780 regular lat/lon)
pub const FORECAST_LON_0: f64 = -0.00725;
pub const FORECAST_LAT_0: f64 = 48.9955;
pub const FORECAST_DLON: f64 = 0.0145;
pub const FORECAST_DLAT: f64 = 0.009;
pub const FORECAST_GRID_W: usize = 780;
pub const FORECAST_GRID_H: usize = 780;

// KNMI radar observation RTCOR grid parameters (765 x 700 Polar Stereographic)
pub const RTCOR_DX: f64 = 1000.0026129808;
pub const RTCOR_DY: f64 = 1000.0050704712;
pub const RTCOR_X0: f64 = 500.00130649042126;
pub const RTCOR_Y0: f64 = -4414499.287435932;
pub const RTCOR_GRID_W: usize = 700;
pub const RTCOR_GRID_H: usize = 765;

// Harmonie GRIB1 grid parameters (for Temp, Solar, Wind)
pub const GRIB_LON_0: f64 = 0.0;
pub const GRIB_LAT_0: f64 = 49.0;
pub const GRIB_DLON: f64 = 0.029;
pub const GRIB_DLAT: f64 = 0.018;
pub const GRIB_WIDTH: usize = 390;
pub const GRIB_HEIGHT: usize = 390;
pub const GRIB_CELL_COUNT: usize = GRIB_WIDTH * GRIB_HEIGHT;

/// Directory where all cached/downloaded files are stored.
pub const CACHE_DIR: &str = "./cache";
