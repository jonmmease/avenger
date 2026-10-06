use std::{
    io::{Cursor, Read},
    sync::{Arc, OnceLock},
};

use crate::{FontResolutionOptions, RegisteredFont};

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

pub fn build_fontdb(options: &crate::FontResolutionOptions) -> fontdb::Database {
    let mut fontdb = fontdb::Database::new();
    for font in &options.registered_fonts {
        fontdb.load_font_data(font.data.to_vec());
    }

    if options.load_system_fonts {
        fontdb.load_system_fonts();
    }

    for font_dir in &options.extra_font_dirs {
        fontdb.load_fonts_dir(font_dir);
    }

    // Font discovery can replace generic mappings on Linux. Apply explicit
    // application choices after loading every font source.
    if let Some(family) = &options.default_sans_serif_family {
        fontdb.set_sans_serif_family(family);
    }
    if let Some(family) = &options.default_monospace_family {
        fontdb.set_monospace_family(family);
    }

    fontdb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_resolution_registers_text_and_math_families() {
        let options = default_font_resolution();
        let fontdb = build_fontdb(&options);

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

    #[test]
    fn bundled_lato_weights_resolve_as_css_does() {
        use crate::{
            path::TextPathExtractionConfig,
            types::{FontStyle, FontWeight, TextSyntaxMode},
        };
        let engine = crate::TextEngine::with_font_resolution(&FontResolutionOptions {
            load_system_fonts: false,
            ..default_font_resolution()
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

    #[test]
    fn build_fontdb_uses_registered_font_defaults() {
        let font_data =
            include_bytes!("../../avenger-vega-test-data/fonts/Caveat/static/Caveat-Regular.ttf");
        let options = crate::FontResolutionOptions {
            registered_fonts: vec![RegisteredFont::new(font_data.as_slice())],
            default_sans_serif_family: Some("Caveat".to_string()),
            ..Default::default()
        };
        let fontdb = build_fontdb(&options);

        let families = [fontdb::Family::SansSerif];
        let query = fontdb::Query {
            families: &families,
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let sans_id = fontdb.query(&query).expect("sans-serif should resolve");
        let sans_face = fontdb.face(sans_id).expect("sans-serif face should exist");
        assert!(sans_face
            .families
            .iter()
            .any(|(family, _)| family == "Caveat"));
    }
}
