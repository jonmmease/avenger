use std::{
    io::{Cursor, Read},
    sync::{Arc, OnceLock},
};

use avenger_text::{FontResolutionOptions, MathFontBytesId, RegisteredFont, TextEngine};

const LATO_LIGHT: &[u8] = include_bytes!("../fonts/Lato/Lato-Light.ttf.br");
const LATO_ITALIC: &[u8] = include_bytes!("../fonts/Lato/Lato-Italic.ttf.br");
const LATO_MEDIUM: &[u8] = include_bytes!("../fonts/Lato/Lato-Medium.ttf.br");
const LATO_BOLD: &[u8] = include_bytes!("../fonts/Lato/Lato-Bold.ttf.br");
const DEJAVU_SANS_MONO: &[u8] = include_bytes!("../fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br");
const LETE_SANS_MATH: &[u8] = include_bytes!("../fonts/Lete_Sans_Math/LeteSansMath.otf.br");
const LETE_SANS_MATH_BOLD: &[u8] =
    include_bytes!("../fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br");

struct ChartFont {
    id: u64,
    name: &'static str,
    compressed_data: &'static [u8],
}

const CHART_FONTS: &[ChartFont] = &[
    ChartFont {
        id: 1,
        name: "Lato-Light",
        compressed_data: LATO_LIGHT,
    },
    ChartFont {
        id: 2,
        name: "Lato-Italic",
        compressed_data: LATO_ITALIC,
    },
    ChartFont {
        id: 3,
        name: "Lato-Medium",
        compressed_data: LATO_MEDIUM,
    },
    ChartFont {
        id: 4,
        name: "Lato-Bold",
        compressed_data: LATO_BOLD,
    },
    ChartFont {
        id: 5,
        name: "DejaVuSansMono",
        compressed_data: DEJAVU_SANS_MONO,
    },
    ChartFont {
        id: 6,
        name: "LeteSansMath",
        compressed_data: LETE_SANS_MATH,
    },
    ChartFont {
        id: 7,
        name: "LeteSansMath-Bold",
        compressed_data: LETE_SANS_MATH_BOLD,
    },
];

pub fn default_font_resolution() -> FontResolutionOptions {
    FontResolutionOptions {
        load_system_fonts: true,
        registered_fonts: registered_default_fonts(),
        default_sans_serif_family: Some("Lato".to_string()),
        default_monospace_family: Some("DejaVu Sans Mono".to_string()),
        default_math_family: Some("Lete Sans Math".to_string()),
        ..Default::default()
    }
}

pub(crate) fn default_chart_text_engine() -> TextEngine {
    static DEFAULT_TEXT_ENGINE: OnceLock<TextEngine> = OnceLock::new();
    DEFAULT_TEXT_ENGINE
        .get_or_init(|| {
            TextEngine::with_font_resolution(&default_font_resolution())
                .expect("failed to initialize chart Typst text engine")
        })
        .clone()
}

fn registered_default_fonts() -> Vec<RegisteredFont> {
    decompressed_default_fonts()
        .iter()
        .zip(CHART_FONTS)
        .map(|(data, font)| RegisteredFont::new(MathFontBytesId(font.id), data.clone()))
        .collect()
}

fn decompressed_default_fonts() -> &'static [Arc<[u8]>] {
    static DECOMPRESSED: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();
    DECOMPRESSED.get_or_init(|| {
        CHART_FONTS
            .iter()
            .map(|font| {
                Arc::<[u8]>::from(
                    decompress_brotli_font(font.name, font.compressed_data).unwrap_or_else(|err| {
                        panic!("failed to decompress chart font {}: {err}", font.name)
                    }),
                )
            })
            .collect()
    })
}

fn decompress_brotli_font(name: &str, compressed_data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut reader = brotli::Decompressor::new(Cursor::new(compressed_data), 4096);
    let mut data = Vec::new();
    reader.read_to_end(&mut data)?;
    if data.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("chart font {name} decompressed to empty data"),
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_default_fonts_register_text_and_math_families() {
        let options = default_font_resolution();
        let fontdb = avenger_text::fonts::build_fontdb(&options);

        for family in ["Lato", "DejaVu Sans Mono", "Lete Sans Math"] {
            assert!(
                fontdb.faces().any(|face| face
                    .families
                    .iter()
                    .any(|(candidate, _)| candidate == family)),
                "{family} should be registered"
            );
        }
    }
}
