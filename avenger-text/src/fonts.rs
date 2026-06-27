use std::io::{Cursor, Read};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, Copy)]
pub struct EmbeddedFont {
    pub name: &'static str,
    index: usize,
    pub compressed_data: &'static [u8],
}

impl EmbeddedFont {
    pub fn decompressed_data(&self) -> Arc<[u8]> {
        decompressed_fonts()[self.index].clone()
    }
}

const EMBEDDED_FONTS: &[EmbeddedFont] = &[
    EmbeddedFont {
        name: "Lato-Light",
        index: 0,
        compressed_data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Light.ttf.br"),
    },
    EmbeddedFont {
        name: "Lato-Italic",
        index: 1,
        compressed_data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Italic.ttf.br"),
    },
    EmbeddedFont {
        name: "Lato-Medium",
        index: 2,
        compressed_data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Medium.ttf.br"),
    },
    EmbeddedFont {
        name: "Lato-Bold",
        index: 3,
        compressed_data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Bold.ttf.br"),
    },
];

pub fn embedded_fonts() -> &'static [EmbeddedFont] {
    EMBEDDED_FONTS
}

pub fn load_embedded_fonts_into_fontdb(fontdb: &mut fontdb::Database) {
    for font in embedded_fonts() {
        fontdb.load_font_data(font.decompressed_data().to_vec());
    }
}

pub fn build_fontdb(options: &crate::FontResolutionOptions) -> fontdb::Database {
    let mut fontdb = fontdb::Database::new();
    load_embedded_fonts_into_fontdb(&mut fontdb);
    fontdb.set_sans_serif_family("Lato");

    if options.load_system_fonts {
        fontdb.load_system_fonts();
    }

    for font_dir in &options.extra_font_dirs {
        fontdb.load_fonts_dir(font_dir);
    }

    fontdb
}

fn decompressed_fonts() -> &'static [Arc<[u8]>] {
    static DECOMPRESSED_FONTS: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();
    DECOMPRESSED_FONTS.get_or_init(|| {
        embedded_fonts()
            .iter()
            .map(|font| {
                Arc::<[u8]>::from(
                    decompress_brotli_font(font.name, font.compressed_data).unwrap_or_else(|err| {
                        panic!("failed to decompress embedded font {}: {err}", font.name)
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
            format!("embedded font {name} decompressed to empty data"),
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_registry_contains_bundled_lato_faces() {
        let names = embedded_fonts()
            .iter()
            .map(|font| font.name)
            .collect::<Vec<_>>();

        assert_eq!(names.len(), 4);
        assert!(names.contains(&"Lato-Light"));
        assert!(names.contains(&"Lato-Italic"));
        assert!(names.contains(&"Lato-Medium"));
        assert!(names.contains(&"Lato-Bold"));
    }

    #[test]
    fn embedded_font_registry_contains_only_compressed_fonts() {
        for font in embedded_fonts() {
            assert!(
                font.compressed_data.len() < font.decompressed_data().len(),
                "{} should be stored compressed",
                font.name
            );
        }
    }

    #[test]
    fn build_fontdb_loads_default_lato_family_and_weight_style_faces() {
        let options = crate::FontResolutionOptions::default();
        let fontdb = build_fontdb(&options);

        for weight in [300, 500, 700] {
            let families = [fontdb::Family::Name("Lato")];
            let query = fontdb::Query {
                families: &families,
                weight: fontdb::Weight(weight),
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            };
            let id = fontdb
                .query(&query)
                .unwrap_or_else(|| panic!("Lato {weight} normal should resolve"));
            let face = fontdb.face(id).expect("resolved Lato face should exist");
            assert!(face.families.iter().any(|(family, _)| family == "Lato"));
        }

        let families = [fontdb::Family::Name("Lato")];
        let italic_query = fontdb::Query {
            families: &families,
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Italic,
        };
        let italic_id = fontdb
            .query(&italic_query)
            .expect("Lato regular italic should resolve");
        let italic_face = fontdb
            .face(italic_id)
            .expect("resolved Lato italic face should exist");
        assert_eq!(italic_face.style, fontdb::Style::Italic);

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
            .any(|(family, _)| family == "Lato"));
    }
}
