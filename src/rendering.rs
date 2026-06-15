use image::codecs::webp::WebPEncoder;
use image::ImageEncoder;
use std::io::Cursor;
use rayon::prelude::*;

use crate::constants::{GRID_W, GRID_H, NODATA};
use crate::models::LutEntry;
use crate::interpolation::interpolate_bilinear_lut;

pub fn render_data_webp_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
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

    let mut webp_bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut webp_bytes);
        let encoder = WebPEncoder::new_lossless(cursor);
        encoder.write_image(&pixels, GRID_W, GRID_H, image::ExtendedColorType::Rgba8).unwrap();
    }
    webp_bytes
}

pub fn render_temp_webp_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
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

    let mut webp_bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut webp_bytes);
        let encoder = WebPEncoder::new_lossless(cursor);
        encoder.write_image(&pixels, GRID_W, GRID_H, image::ExtendedColorType::Rgba8).unwrap();
    }
    webp_bytes
}

pub fn render_wind_webp_bytes(u_slice: &[u16], v_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    let mut pixels = vec![0u8; (GRID_W * GRID_H * 4) as usize];

    pixels.par_chunks_exact_mut(4)
        .enumerate()
        .for_each(|(idx, pixel)| {
            let entry = &lut[idx];
            let u_raw = interpolate_bilinear_lut(entry, u_slice);
            let v_raw = interpolate_bilinear_lut(entry, v_slice);
            if u_raw != NODATA && v_raw != NODATA {
                pixel[0] = (u_raw >> 8) as u8;
                pixel[1] = (u_raw & 0xFF) as u8;
                pixel[2] = (v_raw >> 8) as u8;
                pixel[3] = (v_raw & 0xFF) as u8;
            } else {
                pixel[0] = 0;
                pixel[1] = 0;
                pixel[2] = 0;
                pixel[3] = 0;
            }
        });

    let mut webp_bytes = Vec::new();
    {
        let cursor = Cursor::new(&mut webp_bytes);
        let encoder = WebPEncoder::new_lossless(cursor);
        encoder.write_image(&pixels, GRID_W, GRID_H, image::ExtendedColorType::Rgba8).unwrap();
    }
    webp_bytes
}

pub fn render_solar_webp_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    render_temp_webp_bytes(raw_slice, lut)
}
