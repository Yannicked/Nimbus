use crate::constants::{
    GRID_H, GRID_W, KNMI_DX, KNMI_DY, KNMI_GRID_H, KNMI_GRID_W, KNMI_X0, KNMI_Y0, MERCATOR_BOTTOM,
    MERCATOR_LEFT, MERCATOR_RIGHT, MERCATOR_TOP, NODATA,
};
use crate::models::LutEntry;
use crate::projection;

/// Initializes the coordinate projection lookup table.
pub fn init_projection_lut() -> Vec<LutEntry> {
    let mut lut = Vec::with_capacity((GRID_W * GRID_H) as usize);
    for row in 0..GRID_H {
        for col in 0..GRID_W {
            let col_frac = (col as f64 + 0.5) / GRID_W as f64;
            let row_frac = (row as f64 + 0.5) / GRID_H as f64;

            let x_merc = MERCATOR_LEFT + col_frac * (MERCATOR_RIGHT - MERCATOR_LEFT);
            let y_merc = MERCATOR_TOP - row_frac * (MERCATOR_TOP - MERCATOR_BOTTOM);

            let (lon, lat) = projection::mercator_to_lonlat(x_merc, y_merc);
            let (px, py) = projection::lonlat_to_polar_stereographic(lon, lat);

            let fx = ((px - KNMI_X0) / KNMI_DX) as f32;
            let fy = ((py - KNMI_Y0) / KNMI_DY) as f32;

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

            let grid_w = KNMI_GRID_W as i32;
            let grid_h = KNMI_GRID_H as i32;

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

            // Map (lon, lat) to GRIB1 390x390 grid indices (fx, fy)
            let fx = ((lon - 0.0) / 0.029) as f32;
            let fy = ((lat - 49.0) / 0.018) as f32;

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

            let grid_w = 390;
            let grid_h = 390;

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
    let ix1 = fx.floor() as i32;
    let iy1 = fy.floor() as i32;
    let ix2 = ix1 + 1;
    let iy2 = iy1 + 1;

    if ix1 < -1 || ix1 >= grid_w as i32 || iy1 < -1 || iy1 >= grid_h as i32 {
        return NODATA;
    }

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
    use crate::models::LutEntry;
    use crate::constants::NODATA;

    #[test]
    fn test_interpolate_bilinear_exact() {
        let grid = vec![10, 20, 30, 40];
        let val = interpolate_bilinear(0.0, 0.0, 2, 2, &grid);
        assert_eq!(val, 10);
    }

    #[test]
    fn test_interpolate_bilinear_center() {
        let grid = vec![
            10, 20,
            30, 40
        ];
        // Center of the 4 pixels
        let val = interpolate_bilinear(0.5, 0.5, 2, 2, &grid);
        assert_eq!(val, 25); // (10+20+30+40)/4 = 100/4 = 25
    }

    #[test]
    fn test_interpolate_bilinear_nodata() {
        let grid = vec![
            10, NODATA,
            30, 40
        ];
        // Point where one neighbor is NODATA
        let val = interpolate_bilinear(0.5, 0.5, 2, 2, &grid);
        // Weights: 0.25 each.
        // sum_val = 10*0.25 + 30*0.25 + 40*0.25 = 2.5 + 7.5 + 10 = 20
        // sum_weight = 0.25 + 0.25 + 0.25 = 0.75
        // result = 20 / 0.75 = 26.666 -> 27
        assert_eq!(val, 27);
    }

    #[test]
    fn test_interpolate_bilinear_out_of_bounds() {
        let grid = vec![10, 20, 30, 40];
        let val = interpolate_bilinear(5.0, 5.0, 2, 2, &grid);
        assert_eq!(val, NODATA);
    }

    #[test]
    fn test_interpolate_bilinear_lut_basic() {
        let grid = vec![10, 20, 30, 40];
        let entry = LutEntry {
            indices: [0, 1, 2, 3],
            weights: [0.25, 0.25, 0.25, 0.25],
        };
        let val = interpolate_bilinear_lut(&entry, &grid);
        assert_eq!(val, 25);
    }
}
