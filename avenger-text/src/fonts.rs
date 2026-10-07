use std::{
    io::{Cursor, Read},
    sync::{Arc, OnceLock},
};

use crate::{FontOptions, RegisteredFont};

use avenger_fonts::{
    DEJAVU_SANS_MONO, LATO_BOLD, LATO_ITALIC, LATO_LIGHT, LATO_REGULAR, LETE_SANS_MATH,
    LETE_SANS_MATH_BOLD,
};

struct DefaultFont {
    name: &'static str,
    compressed_data: &'static [u8],
}

// Font selection breaks weight ties by registration order. Bold and Light come before Regular,
// so that ties resolve as CSS does: 350 picks Light, and 550 picks Bold.
const DEFAULT_FONTS: &[DefaultFont] = &[
    DefaultFont {
        name: "Lato-Bold",
        compressed_data: LATO_BOLD,
    },
    DefaultFont {
        name: "Lato-Light",
        compressed_data: LATO_LIGHT,
    },
    DefaultFont {
        name: "Lato-Regular",
        compressed_data: LATO_REGULAR,
    },
    DefaultFont {
        name: "Lato-Italic",
        compressed_data: LATO_ITALIC,
    },
    DefaultFont {
        name: "DejaVuSansMono",
        compressed_data: DEJAVU_SANS_MONO,
    },
    DefaultFont {
        name: "LeteSansMath",
        compressed_data: LETE_SANS_MATH,
    },
    DefaultFont {
        name: "LeteSansMath-Bold",
        compressed_data: LETE_SANS_MATH_BOLD,
    },
];

pub fn default_font_options() -> FontOptions {
    FontOptions {
        load_system_fonts: true,
        registered_fonts: registered_default_fonts(),
        default_sans_serif_family: Some("Lato".to_string()),
        default_monospace_family: Some("DejaVu Sans Mono".to_string()),
        default_math_family: Some("Lete Sans Math".to_string()),
        ..Default::default()
    }
}

pub fn registered_default_fonts() -> Vec<RegisteredFont> {
    decompressed_default_fonts()
        .iter()
        .map(|data| RegisteredFont::new(data.clone()))
        .collect()
}

fn decompressed_default_fonts() -> &'static [Arc<[u8]>] {
    static DECOMPRESSED: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();
    DECOMPRESSED.get_or_init(|| {
        DEFAULT_FONTS
            .iter()
            .map(|font| {
                Arc::<[u8]>::from(
                    decompress_brotli_font(font.name, font.compressed_data).unwrap_or_else(|err| {
                        panic!("failed to decompress text font {}: {err}", font.name)
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
            format!("text font {name} decompressed to empty data"),
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_options_register_text_and_math_families() {
        let engine = crate::TextEngine::with_fonts(&FontOptions {
            load_system_fonts: false,
            missing_font: crate::MissingFontPolicy::Error,
            ..default_font_options()
        });
        for family in ["Lato", "DejaVu Sans Mono", "Lete Sans Math"] {
            let metrics = engine.font_metrics(&crate::measurement::FontMetricsConfig {
                font: family,
                font_size: 12.0,
                font_weight: crate::types::FontWeight::default(),
                font_style: crate::types::FontStyle::default(),
            });
            assert!(metrics.is_ok(), "{family} should be registered");
        }
    }

    #[test]
    fn bundled_lato_weights_resolve_as_css_does() {
        use crate::{
            path::TextPathExtractionConfig,
            types::{FontStyle, FontWeight, TextSyntaxMode},
        };
        let engine = crate::TextEngine::with_fonts(&FontOptions {
            load_system_fonts: false,
            ..default_font_options()
        });
        for (requested, resolved) in [
            (300.0, 300.0),
            (350.0, 300.0),
            (400.0, 400.0),
            (500.0, 400.0),
            (550.0, 700.0),
            (600.0, 700.0),
            (700.0, 700.0),
        ] {
            let buffer = engine
                .extract_paths(&TextPathExtractionConfig {
                    text: "Weight",
                    color: [0.0, 0.0, 0.0, 1.0],
                    font: "Lato",
                    font_size: 12.0,
                    font_weight: FontWeight::Number(requested),
                    font_style: FontStyle::Normal,
                    limit: f32::INFINITY,
                    syntax_mode: TextSyntaxMode::Plain,
                    params: crate::empty_label_params(),
                    number_format: None,
                    datetime_format: None,
                })
                .unwrap();
            assert_eq!(
                buffer.plain_runs[0].font_weight,
                FontWeight::Number(resolved),
                "weight {requested}"
            );
        }
    }
}
