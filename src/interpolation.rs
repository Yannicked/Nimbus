use crate::constants::{
    FORECAST_DLAT, FORECAST_DLON, FORECAST_GRID_H, FORECAST_GRID_W, FORECAST_LAT_0, FORECAST_LON_0,
    GRIB_HEIGHT, GRIB_WIDTH, GRID_H, GRID_W, MERCATOR_BOTTOM, MERCATOR_LEFT, MERCATOR_RIGHT,
    MERCATOR_TOP, NODATA, RTCOR_DX, RTCOR_DY, RTCOR_GRID_H, RTCOR_GRID_W, RTCOR_X0, RTCOR_Y0,
};
use crate::models::LutEntry;
use crate::projection::{self, lonlat_to_grib_indices};

/// Initializes the coordinate projection lookup table for the seamless precipitation forecast
/// (780 x 780 regular lat/lon grid).
pub fn init_projection_lut() -> Vec<LutEntry> {
    let mut lut = Vec::with_capacity((GRID_W * GRID_H) as usize);
    for row in 0..GRID_H {
        for col in 0..GRID_W {
            let col_frac = (col as f64 + 0.5) / GRID_W as f64;
            let row_frac = (row as f64 + 0.5) / GRID_H as f64;

            let x_merc = MERCATOR_LEFT + col_frac * (MERCATOR_RIGHT - MERCATOR_LEFT);
            let y_merc = MERCATOR_TOP - row_frac * (MERCATOR_TOP - MERCATOR_BOTTOM);

            let (lon, lat) = projection::mercator_to_lonlat(x_merc, y_merc);

            let fx = ((lon - FORECAST_LON_0) / FORECAST_DLON) as f32;
            let fy = ((lat - FORECAST_LAT_0) / FORECAST_DLAT) as f32;

            let ix1 = fx.floor() as i32;
            let iy1 = fy.floor() as i32;
            let ix2 = ix1 + 1;
            let iy2 = iy1 + 1;

            let wx = fx - ix1 as f32;
            let wy = fy - iy1 as f32;

            let w00 = (1.0 - wx) * (1.0 - wy);
            let w10 = wx * (1.0 - wy);
            let w01 = (1.0 - wx) * wy;
            let w11 = wx * wy;

            let mut indices = [u32::MAX; 4];
            let weights = [w00, w10, w01, w11];

            let coords = [(ix1, iy1), (ix2, iy1), (ix1, iy2), (ix2, iy2)];

            let grid_w = FORECAST_GRID_W as i32;
            let grid_h = FORECAST_GRID_H as i32;

            for (idx, &(x, y)) in coords.iter().enumerate() {
                if x >= 0 && x < grid_w && y >= 0 && y < grid_h {
                    indices[idx] = (y * grid_w + x) as u32;
                }
            }

            lut.push(LutEntry { indices, weights });
        }
    }
    lut
}

/// Initializes the coordinate projection lookup table for KNMI radar observations (RTCOR)
/// (765 x 700 Polar Stereographic grid).
pub fn init_actuals_projection_lut() -> Vec<LutEntry> {
    let mut lut = Vec::with_capacity((GRID_W * GRID_H) as usize);
    for row in 0..GRID_H {
        for col in 0..GRID_W {
            let col_frac = (col as f64 + 0.5) / GRID_W as f64;
            let row_frac = (row as f64 + 0.5) / GRID_H as f64;

            let x_merc = MERCATOR_LEFT + col_frac * (MERCATOR_RIGHT - MERCATOR_LEFT);
            let y_merc = MERCATOR_TOP - row_frac * (MERCATOR_TOP - MERCATOR_BOTTOM);

            let (lon, lat) = projection::mercator_to_lonlat(x_merc, y_merc);
            let (px, py) = projection::lonlat_to_polar_stereographic(lon, lat);

            let fx = ((px - RTCOR_X0) / RTCOR_DX) as f32;
            let fy = ((py - RTCOR_Y0) / RTCOR_DY) as f32;

            let ix1 = fx.floor() as i32;
            let iy1 = fy.floor() as i32;
            let ix2 = ix1 + 1;
            let iy2 = iy1 + 1;

            let wx = fx - ix1 as f32;
            let wy = fy - iy1 as f32;

            let w00 = (1.0 - wx) * (1.0 - wy);
            let w10 = wx * (1.0 - wy);
            let w01 = (1.0 - wx) * wy;
            let w11 = wx * wy;

            let mut indices = [u32::MAX; 4];
            let weights = [w00, w10, w01, w11];

            let coords = [(ix1, iy1), (ix2, iy1), (ix1, iy2), (ix2, iy2)];

            let grid_w = RTCOR_GRID_W as i32;
            let grid_h = RTCOR_GRID_H as i32;

            for (idx, &(x, y)) in coords.iter().enumerate() {
                if x >= 0 && x < grid_w && y >= 0 && y < grid_h {
                    indices[idx] = (y * grid_w + x) as u32;
                }
            }

            lut.push(LutEntry { indices, weights });
        }
    }
    lut
}

pub fn init_temp_projection_lut() -> Vec<LutEntry> {
    let mut lut = Vec::with_capacity((GRID_W * GRID_H) as usize);
    for row in 0..GRID_H {
        for col in 0..GRID_W {
            let col_frac = (col as f64 + 0.5) / GRID_W as f64;
            let row_frac = (row as f64 + 0.5) / GRID_H as f64;

            let x_merc = MERCATOR_LEFT + col_frac * (MERCATOR_RIGHT - MERCATOR_LEFT);
            let y_merc = MERCATOR_TOP - row_frac * (MERCATOR_TOP - MERCATOR_BOTTOM);

            let (lon, lat) = projection::mercator_to_lonlat(x_merc, y_merc);

            // Map (lon, lat) to GRIB1 grid indices (fx, fy)
            let (fx_d, fy_d) = lonlat_to_grib_indices(lon, lat);
            let fx = fx_d as f32;
            let fy = fy_d as f32;

            let ix1 = fx.floor() as i32;
            let iy1 = fy.floor() as i32;
            let ix2 = ix1 + 1;
            let iy2 = iy1 + 1;

            let wx = fx - ix1 as f32;
            let wy = fy - iy1 as f32;

            let w00 = (1.0 - wx) * (1.0 - wy);
            let w10 = wx * (1.0 - wy);
            let w01 = (1.0 - wx) * wy;
            let w11 = wx * wy;

            let mut indices = [u32::MAX; 4];
            let weights = [w00, w10, w01, w11];

            let coords = [(ix1, iy1), (ix2, iy1), (ix1, iy2), (ix2, iy2)];

            let grid_w = GRIB_WIDTH as i32;
            let grid_h = GRIB_HEIGHT as i32;

            for (idx, &(x, y)) in coords.iter().enumerate() {
                if x >= 0 && x < grid_w && y >= 0 && y < grid_h {
                    indices[idx] = (y * grid_w + x) as u32;
                }
            }

            lut.push(LutEntry { indices, weights });
        }
    }
    lut
}

/// Bilinear interpolation of a raw u16 grid value using a precalculated LutEntry.
///
/// Returns [`NODATA`] when no valid neighbors are found.
pub fn interpolate_bilinear_lut(entry: &LutEntry, raw_slice: &[u16]) -> u16 {
    let mut sum_val = 0.0f64;
    let mut sum_weight = 0.0f64;

    for i in 0..4 {
        let idx = entry.indices[i];
        if idx != u32::MAX {
            let val = raw_slice[idx as usize];
            if val != NODATA {
                let w = entry.weights[i] as f64;
                sum_val += (val as f64) * w;
                sum_weight += w;
            }
        }
    }

    if sum_weight > 0.001 {
        (sum_val / sum_weight).round() as u16
    } else {
        NODATA
    }
}

/// Bilinear interpolation of a pair of raw u16 grid values (such as u and v wind components)
/// using a precalculated LutEntry in a single traversal pass.
pub fn interpolate_bilinear_lut_pair(
    entry: &LutEntry,
    u_slice: &[u16],
    v_slice: &[u16],
) -> (u16, u16) {
    let mut sum_u = 0.0f64;
    let mut sum_v = 0.0f64;
    let mut sum_weight_u = 0.0f64;
    let mut sum_weight_v = 0.0f64;

    for i in 0..4 {
        let idx = entry.indices[i];
        if idx != u32::MAX {
            let u_val = u_slice[idx as usize];
            let v_val = v_slice[idx as usize];
            let w = entry.weights[i] as f64;

            if u_val != NODATA {
                sum_u += (u_val as f64) * w;
                sum_weight_u += w;
            }
            if v_val != NODATA {
                sum_v += (v_val as f64) * w;
                sum_weight_v += w;
            }
        }
    }

    let u_out = if sum_weight_u > 0.001 {
        (sum_u / sum_weight_u).round() as u16
    } else {
        NODATA
    };

    let v_out = if sum_weight_v > 0.001 {
        (sum_v / sum_weight_v).round() as u16
    } else {
        NODATA
    };

    (u_out, v_out)
}

/// Bilinear interpolation of a raw u16 grid value at fractional grid coordinates.
///
/// Returns [`NODATA`] when the query point falls entirely outside the grid or
/// when no valid neighbours are found.
pub fn interpolate_bilinear(
    fx: f64,
    fy: f64,
    grid_w: usize,
    grid_h: usize,
    raw_slice: &[u16],
) -> u16 {
    if !fx.is_finite() || !fy.is_finite() {
        return NODATA;
    }

    let ix1 = fx.floor() as i32;
    let iy1 = fy.floor() as i32;

    if ix1 < -1 || ix1 >= grid_w as i32 || iy1 < -1 || iy1 >= grid_h as i32 {
        return NODATA;
    }

    let ix2 = ix1 + 1;
    let iy2 = iy1 + 1;

    let wx = (fx - ix1 as f64) as f32;
    let wy = (fy - iy1 as f64) as f32;

    let w00 = (1.0 - wx) * (1.0 - wy);
    let w10 = wx * (1.0 - wy);
    let w01 = (1.0 - wx) * wy;
    let w11 = wx * wy;

    let get_val = |x: i32, y: i32| -> Option<(u16, f32)> {
        if x >= 0 && x < grid_w as i32 && y >= 0 && y < grid_h as i32 {
            let val = raw_slice[(y * grid_w as i32 + x) as usize];
            if val != NODATA {
                Some((val, 1.0))
            } else {
                None
            }
        } else {
            None
        }
    };

    let mut sum_val = 0.0;
    let mut sum_weight = 0.0;

    let neighbors = [
        (get_val(ix1, iy1), w00),
        (get_val(ix2, iy1), w10),
        (get_val(ix1, iy2), w01),
        (get_val(ix2, iy2), w11),
    ];

    for (opt, w) in neighbors {
        if let Some((val, _)) = opt {
            sum_val += (val as f64) * (w as f64);
            sum_weight += w as f64;
        }
    }

    if sum_weight > 0.001 {
        (sum_val / sum_weight).round() as u16
    } else {
        NODATA
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::NODATA;
    use crate::models::LutEntry;

    #[test]
    fn test_interpolate_bilinear_exact() {
        let grid = [10, 20, 30, 40];
        let w = 2;
        let h = 2;

        assert_eq!(interpolate_bilinear(0.0, 0.0, w, h, &grid), 10);
        assert_eq!(interpolate_bilinear(1.0, 0.0, w, h, &grid), 20);
        assert_eq!(interpolate_bilinear(0.0, 1.0, w, h, &grid), 30);
        assert_eq!(interpolate_bilinear(1.0, 1.0, w, h, &grid), 40);
    }

    #[test]
    fn test_interpolate_bilinear_midpoints() {
        let grid = [10, 20, 30, 40];
        let w = 2;
        let h = 2;

        // Horizontal midpoint
        assert_eq!(interpolate_bilinear(0.5, 0.0, w, h, &grid), 15);
        // Vertical midpoint
        assert_eq!(interpolate_bilinear(0.0, 0.5, w, h, &grid), 20);
        // Center midpoint
        assert_eq!(interpolate_bilinear(0.5, 0.5, w, h, &grid), 25);
    }

    #[test]
    fn test_interpolate_bilinear_out_of_bounds() {
        let grid = [10, 20, 30, 40];
        let w = 2;
        let h = 2;

        assert_eq!(interpolate_bilinear(-1.1, 0.0, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(2.0, 0.0, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(0.0, -1.1, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(0.0, 2.0, w, h, &grid), NODATA);

        // Non-finite and extreme coordinates
        assert_eq!(interpolate_bilinear(f64::NAN, 0.0, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(0.0, f64::NAN, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(f64::INFINITY, 0.0, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(0.0, f64::INFINITY, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(f64::NEG_INFINITY, 0.0, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(0.0, f64::NEG_INFINITY, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(1e30, 0.0, w, h, &grid), NODATA);
        assert_eq!(interpolate_bilinear(-1e30, 0.0, w, h, &grid), NODATA);
    }

    #[test]
    fn test_interpolate_bilinear_partial_out_of_bounds() {
        let grid = [10, 20, 30, 40];
        let w = 2;
        let h = 2;

        // At fx = -0.5, ix1 = -1, ix2 = 0. wx = 0.5.
        // Neighbors are (ix1, 0) which is None, and (ix2, 0) which is 10.
        // Weights: w00 = 0.5, w10 = 0.5.
        // Only w10 is used. sum_val = 10 * 0.5, sum_weight = 0.5. Result = 10.
        assert_eq!(interpolate_bilinear(-0.5, 0.0, w, h, &grid), 10);
        assert_eq!(interpolate_bilinear(1.5, 0.0, w, h, &grid), 20);
    }

    #[test]
    fn test_interpolate_bilinear_lut_basic() {
        let grid = [10, 20, 30, 40];
        // Center midpoint (0.5, 0.5)
        let entry = LutEntry {
            indices: [0, 1, 2, 3],
            weights: [0.25, 0.25, 0.25, 0.25],
        };
        assert_eq!(interpolate_bilinear_lut(&entry, &grid), 25);
    }

    #[test]
    fn test_interpolate_bilinear_lut_nodata() {
        let grid = [10, NODATA, 30, 40];
        // Center midpoint (0.5, 0.5)
        // Neighbors: 10 (0.25), NODATA (0.25), 30 (0.25), 40 (0.25)
        // sum_val = 10*0.25 + 30*0.25 + 40*0.25 = 2.5 + 7.5 + 10 = 20
        // sum_weight = 0.25 + 0.25 + 0.25 = 0.75
        // Result = 20 / 0.75 = 26.666 -> 27
        let entry = LutEntry {
            indices: [0, 1, 2, 3],
            weights: [0.25, 0.25, 0.25, 0.25],
        };
        assert_eq!(interpolate_bilinear_lut(&entry, &grid), 27);

        let grid_all_nodata = [NODATA, NODATA, NODATA, NODATA];
        assert_eq!(interpolate_bilinear_lut(&entry, &grid_all_nodata), NODATA);
    }

    #[test]
    fn test_interpolate_bilinear_lut_partial_indices() {
        let grid = [10, 20, 30, 40];
        let entry = LutEntry {
            indices: [0, u32::MAX, 2, u32::MAX],
            weights: [0.5, 0.5, 0.0, 0.0],
        };
        // sum_val = 10 * 0.5 = 5.0
        // sum_weight = 0.5
        // Result = 5.0 / 0.5 = 10
        assert_eq!(interpolate_bilinear_lut(&entry, &grid), 10);
    }

    #[test]
    fn test_interpolate_bilinear_lut_pair() {
        let u_grid = [10, 20, 30, 40];
        let v_grid = [100, 200, 300, 400];
        let entry = LutEntry {
            indices: [0, 1, 2, 3],
            weights: [0.25, 0.25, 0.25, 0.25],
        };
        let (u, v) = interpolate_bilinear_lut_pair(&entry, &u_grid, &v_grid);
        assert_eq!(u, interpolate_bilinear_lut(&entry, &u_grid));
        assert_eq!(v, interpolate_bilinear_lut(&entry, &v_grid));
        assert_eq!(u, 25);
        assert_eq!(v, 250);
    }
}
