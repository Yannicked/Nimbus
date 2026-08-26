use image::codecs::webp::WebPEncoder;
use image::ImageEncoder;
use rayon::prelude::*;
use std::io::Cursor;

use crate::constants::{GRID_H, GRID_W, NODATA};
use crate::interpolation::{interpolate_bilinear_lut, interpolate_bilinear_lut_pair};
use crate::models::LutEntry;

pub fn render_data_webp_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    let mut pixels = vec![0u8; (GRID_W * GRID_H * 4) as usize];

    pixels
        .par_chunks_exact_mut(4)
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
        encoder
            .write_image(&pixels, GRID_W, GRID_H, image::ExtendedColorType::Rgba8)
            .unwrap();
    }
    webp_bytes
}

pub fn render_temp_webp_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    render_data_webp_bytes(raw_slice, lut)
}

pub fn render_wind_webp_bytes(u_slice: &[u16], v_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    let mut pixels = vec![0u8; (GRID_W * GRID_H * 4) as usize];

    pixels
        .par_chunks_exact_mut(4)
        .enumerate()
        .for_each(|(idx, pixel)| {
            let entry = &lut[idx];
            let (u_raw, v_raw) = interpolate_bilinear_lut_pair(entry, u_slice, v_slice);
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
        encoder
            .write_image(&pixels, GRID_W, GRID_H, image::ExtendedColorType::Rgba8)
            .unwrap();
    }
    webp_bytes
}

pub fn render_solar_webp_bytes(raw_slice: &[u16], lut: &[LutEntry]) -> Vec<u8> {
    render_data_webp_bytes(raw_slice, lut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{GRID_H, GRID_W, NODATA};
    use crate::models::LutEntry;
    use image::ImageReader;
    use std::io::Cursor;

    fn create_mock_lut() -> Vec<LutEntry> {
        let mut lut = Vec::with_capacity((GRID_W * GRID_H) as usize);
        for i in 0..(GRID_W * GRID_H) {
            lut.push(LutEntry {
                indices: [i, i, i, i],
                weights: [0.25, 0.25, 0.25, 0.25],
            });
        }
        lut
    }

    #[test]
    fn test_render_data_webp_bytes() {
        let lut = create_mock_lut();
        let mut raw_slice = vec![0u16; (GRID_W * GRID_H) as usize];
        // Set a specific value: 0x1234
        raw_slice[0] = 0x1234;

        let bytes = render_data_webp_bytes(&raw_slice, &lut);
        assert!(!bytes.is_empty());

        // Decode WebP
        let img = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8();

        let pixel = img.get_pixel(0, 0);
        assert_eq!(pixel[0], 0x12);
        assert_eq!(pixel[1], 0x34);
        assert_eq!(pixel[2], 0);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn test_render_data_webp_nodata() {
        let lut = create_mock_lut();
        let raw_slice = vec![NODATA; (GRID_W * GRID_H) as usize];

        let bytes = render_data_webp_bytes(&raw_slice, &lut);
        assert!(!bytes.is_empty());

        let img = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8();

        let pixel = img.get_pixel(0, 0);
        assert_eq!(pixel[0], 0);
        assert_eq!(pixel[1], 0);
        assert_eq!(pixel[2], 0);
        assert_eq!(pixel[3], 0);
    }

    #[test]
    fn test_render_temp_webp_bytes() {
        let lut = create_mock_lut();
        let mut raw_slice = vec![0u16; (GRID_W * GRID_H) as usize];
        raw_slice[0] = 0xABCD;

        let bytes = render_temp_webp_bytes(&raw_slice, &lut);
        assert!(!bytes.is_empty());

        let img = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8();

        let pixel = img.get_pixel(0, 0);
        assert_eq!(pixel[0], 0xAB);
        assert_eq!(pixel[1], 0xCD);
        assert_eq!(pixel[2], 0);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn test_render_wind_webp_bytes() {
        let lut = create_mock_lut();
        let mut u_slice = vec![0u16; (GRID_W * GRID_H) as usize];
        let mut v_slice = vec![0u16; (GRID_W * GRID_H) as usize];
        u_slice[0] = 0x1122;
        v_slice[0] = 0x3344;

        let bytes = render_wind_webp_bytes(&u_slice, &v_slice, &lut);
        assert!(!bytes.is_empty());

        let img = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8();

        let pixel = img.get_pixel(0, 0);
        assert_eq!(pixel[0], 0x11);
        assert_eq!(pixel[1], 0x22);
        assert_eq!(pixel[2], 0x33);
        assert_eq!(pixel[3], 0x44);
    }

    #[test]
    fn test_render_solar_webp_bytes() {
        let lut = create_mock_lut();
        let mut raw_slice = vec![0u16; (GRID_W * GRID_H) as usize];
        raw_slice[0] = 0x5566;

        let bytes = render_solar_webp_bytes(&raw_slice, &lut);
        assert!(!bytes.is_empty());

        let img = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8();

        let pixel = img.get_pixel(0, 0);
        assert_eq!(pixel[0], 0x55);
        assert_eq!(pixel[1], 0x66);
    }
}
