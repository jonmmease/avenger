//! What the examples share: the engine's fonts, and a white canvas that labels draw onto.

#![allow(dead_code, reason = "each example uses part of this module")]

use std::error::Error;
use std::path::Path;

use avenger_typst_label::{EngineOptions, RegisteredFont, RgbaImageData};

/// Lato, DejaVu Sans Mono and Lete Sans Math, the test fixtures' Hebrew and Devanagari fonts,
/// and no system fonts.
pub fn engine_options() -> EngineOptions {
    let mut options = EngineOptions::default();
    options.fonts.load_system_fonts = false;
    options.fonts.default_sans_serif_family = Some("Lato".into());
    options.fonts.default_monospace_family = Some("DejaVu Sans Mono".into());
    options.fonts.default_math_family = Some("Lete Sans Math".into());
    options.fonts.registered_fonts = FONT_BYTES
        .iter()
        .map(|compressed| RegisteredFont::new(avenger_fonts::decompress(compressed)))
        .collect();
    options
}

const FONT_BYTES: &[&[u8]] = &[
    avenger_fonts::LATO_LIGHT,
    avenger_fonts::LATO_REGULAR,
    avenger_fonts::LATO_ITALIC,
    avenger_fonts::LATO_BOLD,
    avenger_fonts::DEJAVU_SANS_MONO,
    avenger_fonts::LETE_SANS_MATH,
    avenger_fonts::LETE_SANS_MATH_BOLD,
    include_bytes!("../../tests/fixtures/fonts/NotoSansHebrew.ttf.br"),
    include_bytes!("../../tests/fixtures/fonts/NotoSansDevanagari.ttf.br"),
];

/// An opaque RGBA image, white until drawn on.
pub struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![255; width * height * 4],
        }
    }

    /// Draws an image over the canvas with its top left at a pixel. What falls outside the
    /// canvas is dropped.
    pub fn draw(&mut self, image: &RgbaImageData, left: i64, top: i64) {
        for y in 0..image.height as i64 {
            for x in 0..image.width as i64 {
                let src = ((y * image.width as i64 + x) * 4) as usize;
                let rgba = image.data[src..src + 4].try_into().unwrap();
                self.blend(left + x, top + y, rgba);
            }
        }
    }

    /// Outlines a rectangle, one pixel wide, around the pixels from `left`/`top` to
    /// `right`/`bottom`.
    pub fn outline(
        &mut self,
        left: i64,
        top: i64,
        right: i64,
        bottom: i64,
        rgba: [u8; 4],
    ) {
        for x in left - 1..=right {
            self.blend(x, top - 1, rgba);
            self.blend(x, bottom, rgba);
        }
        for y in top..bottom {
            self.blend(left - 1, y, rgba);
            self.blend(right, y, rgba);
        }
    }

    /// Writes the canvas as a PNG file, creating its directory.
    pub fn save(&self, path: &str) -> Result<(), Box<dyn Error>> {
        if let Some(parent) =
            Path::new(path).parent().filter(|p| !p.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(path)?;
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(file),
            self.width as u32,
            self.height as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&self.pixels)?;
        Ok(())
    }

    /// Blends a straight-alpha pixel over the canvas.
    fn blend(&mut self, x: i64, y: i64, rgba: [u8; 4]) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let dst = (y as usize * self.width + x as usize) * 4;
        let alpha = rgba[3] as u16;
        for (under, over) in self.pixels[dst..dst + 3].iter_mut().zip(rgba) {
            *under =
                ((over as u16 * alpha + *under as u16 * (255 - alpha) + 127) / 255) as u8;
        }
    }
}
