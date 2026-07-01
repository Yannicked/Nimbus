//! Coordinate projection utilities for converting between Web Mercator
//! (EPSG:3857), WGS84 geographic coordinates, and the KNMI Polar
//! Stereographic grid used by the precipitation dataset.

use crate::constants::{GRIB_DX, GRIB_DY, GRIB_X0, GRIB_Y0};
use std::f64::consts::PI;

const WGS84_A: f64 = 6378137.0; // semi-major axis
const WGS84_F_INV: f64 = 298.257223563; // inverse flattening

/// Convert Web Mercator (EPSG:3857) coordinates to WGS84 Lon/Lat (degrees)
pub fn mercator_to_lonlat(x: f64, y: f64) -> (f64, f64) {
    let lon = (x / WGS84_A).to_degrees();
    let lat = (2.0 * (y / WGS84_A).exp().atan() - PI / 2.0).to_degrees();
    (lon, lat)
}

/// Convert WGS84 Lon/Lat (degrees) to KNMI Polar Stereographic (EPSG:3857-like but stereographic)
/// matches projection: +proj=stere +lat_0=90 +lon_0=0 +lat_ts=60 +x_0=0 +y_0=0 +ellps=WGS84 +units=m
pub fn lonlat_to_polar_stereographic(lon: f64, lat: f64) -> (f64, f64) {
    let f = 1.0 / WGS84_F_INV;
    let e = (2.0 * f - f * f).sqrt();

    let lat_rad = lat.to_radians();
    let lon_rad = lon.to_radians();

    // Standard parallel: 60 degrees North
    let phi_f = 60.0_f64.to_radians();

    // Calculate standard parallel scale constant (m_f)
    let sin_phi_f = phi_f.sin();
    let cos_phi_f = phi_f.cos();
    let m_f = cos_phi_f / (1.0 - e * e * sin_phi_f * sin_phi_f).sqrt();

    // Calculate isometric latitude for standard parallel (t_f)
    let t_f = (PI / 4.0 - phi_f / 2.0).tan()
        * ((1.0 + e * sin_phi_f) / (1.0 - e * sin_phi_f)).powf(e / 2.0);

    // Calculate isometric latitude for target point (t)
    // Avoid tan(pi/4 - pi/4) at the pole
    let t = if lat >= 89.99999 {
        0.0
    } else {
        let sin_lat = lat_rad.sin();
        (PI / 4.0 - lat_rad / 2.0).tan() * ((1.0 + e * sin_lat) / (1.0 - e * sin_lat)).powf(e / 2.0)
    };

    // Calculate radius from pole
    let rho = WGS84_A * m_f * t / t_f;

    // Project coordinates
    let x = rho * lon_rad.sin();
    let y = -rho * lon_rad.cos();

    (x, y)
}

/// Convert WGS84 Lon/Lat (degrees) to GRIB1 grid fractional indices
pub fn lonlat_to_grib_indices(lon: f64, lat: f64) -> (f64, f64) {
    let fx = (lon - GRIB_X0) / GRIB_DX;
    let fy = (lat - GRIB_Y0) / GRIB_DY;
    (fx, fy)
}
