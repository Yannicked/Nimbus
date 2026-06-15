use image::codecs::png::{PngEncoder, CompressionType, FilterType};
use image::ImageEncoder;
use std::io::Cursor;
use rayon::prelude::*;

use crate::constants::{GRID_W, GRID_H, NODATA};
use crate::models::LutEntry;
use crate::interpolation::interpolate_bilinear_lut;

pub fn render_data_png_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    let mut pixels = vec![0u8; (GRID_W * GRID_H * 4) as usize];

    pixels.par_chunks_exact_mut(4)
        .enumerate()
        .for_each(|(idx, pixel)| {
            let entry = &lut[idx];
            let val_raw = interpolate_bilinear_lut(entry, raw_slice);
            if val_raw != NODATA {
                pixel[0] = (val_raw >> 8) as u8;
                pixel[1] = (val_raw & 0xFF) as u8;
                pixel[2] = 0;
                pixel[3] = 255;
            } else {
                pixel[0] = 0;
                pixel[1] = 0;
                pixel[2] = 0;
                pixel[3] = 0;
            }
        });

    let mut png_bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut png_bytes);
        let encoder = PngEncoder::new_with_quality(
            cursor,
            CompressionType::Fast,
            FilterType::NoFilter,
        );
        encoder.write_image(&pixels, GRID_W, GRID_H, image::ExtendedColorType::Rgba8).unwrap();
    }
    png_bytes
}

pub fn render_temp_png_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    let mut pixels = vec![0u8; (GRID_W * GRID_H * 4) as usize];

    pixels.par_chunks_exact_mut(4)
        .enumerate()
        .for_each(|(idx, pixel)| {
            let entry = &lut[idx];
            let val_raw = interpolate_bilinear_lut(entry, raw_slice);
            if val_raw != NODATA {
                pixel[0] = (val_raw >> 8) as u8;
                pixel[1] = (val_raw & 0xFF) as u8;
                pixel[2] = 0;
                pixel[3] = 255;
            } else {
                pixel[0] = 0;
                pixel[1] = 0;
                pixel[2] = 0;
                pixel[3] = 0;
            }
        });

    let mut png_bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut png_bytes);
        let encoder = PngEncoder::new_with_quality(
            cursor,
            CompressionType::Fast,
            FilterType::NoFilter,
        );
        encoder.write_image(&pixels, GRID_W, GRID_H, image::ExtendedColorType::Rgba8).unwrap();
    }
    png_bytes
}

pub fn render_wind_png_bytes(u_slice: &[u16], v_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    let mut pixels = vec![0u8; (GRID_W * GRID_H * 2 * 4) as usize];

    let (top_pixels, bottom_pixels) = pixels.split_at_mut((GRID_W * GRID_H * 4) as usize);

    top_pixels.par_chunks_exact_mut(4)
        .enumerate()
        .for_each(|(idx, pixel)| {
            let entry = &lut[idx];
            let val_raw = interpolate_bilinear_lut(entry, u_slice);
            if val_raw != NODATA {
                pixel[0] = (val_raw >> 8) as u8;
                pixel[1] = (val_raw & 0xFF) as u8;
                pixel[2] = 0;
                pixel[3] = 255;
            } else {
                pixel[0] = 0;
                pixel[1] = 0;
                pixel[2] = 0;
                pixel[3] = 0;
            }
        });

    bottom_pixels.par_chunks_exact_mut(4)
        .enumerate()
        .for_each(|(idx, pixel)| {
            let entry = &lut[idx];
            let val_raw = interpolate_bilinear_lut(entry, v_slice);
            if val_raw != NODATA {
                pixel[0] = (val_raw >> 8) as u8;
                pixel[1] = (val_raw & 0xFF) as u8;
                pixel[2] = 0;
                pixel[3] = 255;
            } else {
                pixel[0] = 0;
                pixel[1] = 0;
                pixel[2] = 0;
                pixel[3] = 0;
            }
        });

    let mut png_bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut png_bytes);
        let encoder = PngEncoder::new_with_quality(
            cursor,
            CompressionType::Fast,
            FilterType::NoFilter,
        );
        encoder.write_image(&pixels, GRID_W, GRID_H * 2, image::ExtendedColorType::Rgba8).unwrap();
    }
    png_bytes
}
