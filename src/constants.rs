//! Configuration constants for the weather radar service.

/// Sentinel value used in the u16 grid to indicate missing / no-data pixels.
pub const NODATA: u16 = 65535;

/// Conversion factor from raw u16 grid values to mm/h.
pub const SCALE_FACTOR: f64 = 0.01;

/// Raw value threshold: members with `val >= RAIN_THRESHOLD` count as "raining"
/// when computing probability.
pub const RAIN_THRESHOLD: u16 = 10;

/// KNMI Open Data dataset identifier.
pub const KNMI_DATASET: &str = "seamless_precipitation_ensemble_forecast_members";

/// NetCDF variable name for precipitation intensity.
pub const PRECIP_VAR: &str = "precip_intensity";

// Target Web Mercator grid dimensions and bounds
pub const GRID_W: u32 = 700;
pub const GRID_H: u32 = 765;
pub const MERCATOR_LEFT: f64 = 0.0;
pub const MERCATOR_RIGHT: f64 = 1210000.0;
pub const MERCATOR_BOTTOM: f64 = 6250000.0;
pub const MERCATOR_TOP: f64 = 7560000.0;

// KNMI grid parameters
pub const KNMI_DX: f64 = 1000.0026129808;
pub const KNMI_DY: f64 = -1000.0050704712;
pub const KNMI_X0: f64 = 500.00130649042126;
pub const KNMI_Y0: f64 = -3650495.413595936;
pub const KNMI_GRID_W: usize = 700;
pub const KNMI_GRID_H: usize = 765;
