//! Coordinate projection utilities for converting between Web Mercator
//! (EPSG:3857), WGS84 geographic coordinates, and the KNMI Polar
//! Stereographic grid used by the precipitation dataset.

use crate::constants::{GRIB_DLAT, GRIB_DLON, GRIB_LAT_0, GRIB_LON_0};
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
    let fx = (lon - GRIB_LON_0) / GRIB_DLON;
    let fy = (lat - GRIB_LAT_0) / GRIB_DLAT;
    (fx, fy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mercator_to_lonlat_origin() {
        let (lon, lat) = mercator_to_lonlat(0.0, 0.0);
        assert!((lon - 0.0).abs() < 1e-9);
        assert!((lat - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_mercator_to_lonlat_debilt() {
        // De Bilt, Netherlands: approx (5.1768, 52.1112)
        // Mercator: (576281.8, 6820252.2)
        let (lon, lat) = mercator_to_lonlat(576281.8, 6820252.2);
        assert!((lon - 5.1768).abs() < 1e-4);
        assert!((lat - 52.1112).abs() < 1e-4);
    }

    #[test]
    fn test_mercator_to_lonlat_bounds() {
        // North-East corner of the valid web mercator map
        // (20037508.34, 20037508.34) -> (180.0, 85.0511)
        let (lon, lat) = mercator_to_lonlat(20037508.342789244, 20037508.342789244);
        assert!((lon - 180.0).abs() < 1e-7);
        assert!((lat - 85.05112878).abs() < 1e-7);

        // South-West corner
        let (lon, lat) = mercator_to_lonlat(-20037508.342789244, -20037508.342789244);
        assert!((lon + 180.0).abs() < 1e-7);
        assert!((lat + 85.05112878).abs() < 1e-7);
    }

    #[test]
    fn test_lonlat_to_grib_indices() {
        // Test origin
        let (fx, fy) = lonlat_to_grib_indices(GRIB_LON_0, GRIB_LAT_0);
        assert!((fx - 0.0).abs() < 1e-9);
        assert!((fy - 0.0).abs() < 1e-9);

        // Test one grid cell offset
        let (fx, fy) = lonlat_to_grib_indices(GRIB_LON_0 + GRIB_DLON, GRIB_LAT_0 + GRIB_DLAT);
        assert!((fx - 1.0).abs() < 1e-9);
        assert!((fy - 1.0).abs() < 1e-9);

        // Test larger offset
        let (fx, fy) = lonlat_to_grib_indices(
            GRIB_LON_0 + 100.0 * GRIB_DLON,
            GRIB_LAT_0 + 100.0 * GRIB_DLAT,
        );
        assert!((fx - 100.0).abs() < 1e-9);
        assert!((fy - 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_polar_stereographic_bounds() {
        use crate::constants::{KNMI_DX, KNMI_DY, KNMI_X0, KNMI_Y0};

        // South-West corner of domain (approx lat 49.3663, lon 0.0065)
        let (px_sw, py_sw) = lonlat_to_polar_stereographic(0.0065, 49.3663);
        let ix_sw = ((px_sw - KNMI_X0) / KNMI_DX).round() as i32;
        let iy_sw = ((py_sw - KNMI_Y0) / KNMI_DY).round() as i32;
        assert_eq!(ix_sw, 0);
        assert_eq!(iy_sw, 0);

        // North-West corner of domain (approx lat 55.9692, lon 0.0078)
        let (px_nw, py_nw) = lonlat_to_polar_stereographic(0.0078, 55.9692);
        let ix_nw = ((px_nw - KNMI_X0) / KNMI_DX).round() as i32;
        let iy_nw = ((py_nw - KNMI_Y0) / KNMI_DY).round() as i32;
        assert_eq!(ix_nw, 0);
        assert_eq!(iy_nw, 764);
    }
}
