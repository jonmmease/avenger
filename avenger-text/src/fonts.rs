use std::{
    io::{Cursor, Read},
    sync::{Arc, OnceLock},
};

use crate::{FontResolutionOptions, MathFontBytesId, RegisteredFont};

const LATO_LIGHT: &[u8] = include_bytes!("../fonts/Lato/Lato-Light.ttf.br");
const LATO_ITALIC: &[u8] = include_bytes!("../fonts/Lato/Lato-Italic.ttf.br");
const LATO_MEDIUM: &[u8] = include_bytes!("../fonts/Lato/Lato-Medium.ttf.br");
const LATO_BOLD: &[u8] = include_bytes!("../fonts/Lato/Lato-Bold.ttf.br");
const DEJAVU_SANS_MONO: &[u8] = include_bytes!("../fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br");
const LETE_SANS_MATH: &[u8] = include_bytes!("../fonts/Lete_Sans_Math/LeteSansMath.otf.br");
const LETE_SANS_MATH_BOLD: &[u8] =
    include_bytes!("../fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br");

struct DefaultFont {
    id: u64,
    name: &'static str,
    compressed_data: &'static [u8],
}

const DEFAULT_FONTS: &[DefaultFont] = &[
    DefaultFont {
        id: 1,
        name: "Lato-Light",
        compressed_data: LATO_LIGHT,
    },
    DefaultFont {
        id: 2,
        name: "Lato-Italic",
        compressed_data: LATO_ITALIC,
    },
    DefaultFont {
        id: 3,
        name: "Lato-Medium",
        compressed_data: LATO_MEDIUM,
    },
    DefaultFont {
        id: 4,
        name: "Lato-Bold",
        compressed_data: LATO_BOLD,
    },
    DefaultFont {
        id: 5,
        name: "DejaVuSansMono",
        compressed_data: DEJAVU_SANS_MONO,
    },
    DefaultFont {
        id: 6,
        name: "LeteSansMath",
        compressed_data: LETE_SANS_MATH,
    },
    DefaultFont {
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

pub fn registered_default_fonts() -> Vec<RegisteredFont> {
    decompressed_default_fonts()
        .iter()
        .zip(DEFAULT_FONTS)
        .map(|(data, font)| RegisteredFont::new(MathFontBytesId(font.id), data.clone()))
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

    if let Some(family) = &options.default_sans_serif_family {
        fontdb.set_sans_serif_family(family);
    }
    if let Some(family) = &options.default_monospace_family {
        fontdb.set_monospace_family(family);
    }

    if options.load_system_fonts {
        fontdb.load_system_fonts();
    }

    for font_dir in &options.extra_font_dirs {
        fontdb.load_fonts_dir(font_dir);
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
    fn build_fontdb_uses_registered_font_defaults() {
        let font_data =
            include_bytes!("../../avenger-vega-test-data/fonts/Caveat/static/Caveat-Regular.ttf");
        let options = crate::FontResolutionOptions {
            registered_fonts: vec![avenger_typst_label::RegisteredFont::new(
                avenger_typst_label::MathFontBytesId(1),
                font_data.as_slice(),
            )],
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
