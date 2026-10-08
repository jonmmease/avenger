#![doc = include_str!("../README.md")]

use std::io::Read;

/// Brotli-compressed Lato Light TrueType font.
pub const LATO_LIGHT: &[u8] = include_bytes!("../fonts/Lato/Lato-Light.ttf.br");

/// Brotli-compressed Lato Regular TrueType font.
pub const LATO_REGULAR: &[u8] = include_bytes!("../fonts/Lato/Lato-Regular.ttf.br");

/// Brotli-compressed Lato Italic TrueType font.
pub const LATO_ITALIC: &[u8] = include_bytes!("../fonts/Lato/Lato-Italic.ttf.br");

/// Brotli-compressed Lato Bold TrueType font.
pub const LATO_BOLD: &[u8] = include_bytes!("../fonts/Lato/Lato-Bold.ttf.br");

/// Brotli-compressed DejaVu Sans Mono TrueType font.
pub const DEJAVU_SANS_MONO: &[u8] =
    include_bytes!("../fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br");

/// Brotli-compressed Lete Sans Math OpenType font.
pub const LETE_SANS_MATH: &[u8] = include_bytes!("../fonts/Lete_Sans_Math/LeteSansMath.otf.br");

/// Brotli-compressed Lete Sans Math Bold OpenType font.
pub const LETE_SANS_MATH_BOLD: &[u8] =
    include_bytes!("../fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br");

/// Decompresses one of the fonts, or other Brotli data. Panics if the data is corrupt, which
/// bundled data can't be.
pub fn decompress(compressed: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    brotli::Decompressor::new(compressed, 4096)
        .read_to_end(&mut data)
        .expect("bundled Brotli data decompresses");
    data
}
