// Embedded font data for Avenger's bundled chart text faces.
// Include the font files at compile time.
const ATKINSON_HYPERLEGIBLE_REGULAR: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Regular.ttf"
);
const ATKINSON_HYPERLEGIBLE_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Italic.ttf"
);
const ATKINSON_HYPERLEGIBLE_BOLD: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Bold.ttf"
);
const ATKINSON_HYPERLEGIBLE_BOLD_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-BoldItalic.ttf"
);
const ATKINSON_HYPERLEGIBLE_EXTRA_BOLD: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraBold.ttf"
);
const ATKINSON_HYPERLEGIBLE_EXTRA_BOLD_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraBoldItalic.ttf"
);
const ATKINSON_HYPERLEGIBLE_EXTRA_LIGHT: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraLight.ttf"
);
const ATKINSON_HYPERLEGIBLE_EXTRA_LIGHT_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraLightItalic.ttf"
);
const ATKINSON_HYPERLEGIBLE_LIGHT: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Light.ttf"
);
const ATKINSON_HYPERLEGIBLE_LIGHT_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-LightItalic.ttf"
);
const ATKINSON_HYPERLEGIBLE_MEDIUM: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Medium.ttf"
);
const ATKINSON_HYPERLEGIBLE_MEDIUM_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-MediumItalic.ttf"
);
const ATKINSON_HYPERLEGIBLE_SEMIBOLD: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-SemiBold.ttf"
);
const ATKINSON_HYPERLEGIBLE_SEMIBOLD_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-SemiBoldItalic.ttf"
);

#[derive(Debug, Clone, Copy)]
pub struct EmbeddedFont {
    pub name: &'static str,
    pub data: &'static [u8],
}

const EMBEDDED_FONTS: &[EmbeddedFont] = &[
    EmbeddedFont {
        name: "Lato-Light",
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Light.ttf"),
    },
    EmbeddedFont {
        name: "Lato-LightItalic",
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-LightItalic.ttf"),
    },
    EmbeddedFont {
        name: "Lato-Medium",
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Medium.ttf"),
    },
    EmbeddedFont {
        name: "Lato-MediumItalic",
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-MediumItalic.ttf"),
    },
    EmbeddedFont {
        name: "Lato-Bold",
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Bold.ttf"),
    },
    EmbeddedFont {
        name: "Lato-BoldItalic",
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-BoldItalic.ttf"),
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-Regular",
        data: ATKINSON_HYPERLEGIBLE_REGULAR,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-Italic",
        data: ATKINSON_HYPERLEGIBLE_ITALIC,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-Bold",
        data: ATKINSON_HYPERLEGIBLE_BOLD,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-BoldItalic",
        data: ATKINSON_HYPERLEGIBLE_BOLD_ITALIC,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-ExtraBold",
        data: ATKINSON_HYPERLEGIBLE_EXTRA_BOLD,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-ExtraBoldItalic",
        data: ATKINSON_HYPERLEGIBLE_EXTRA_BOLD_ITALIC,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-ExtraLight",
        data: ATKINSON_HYPERLEGIBLE_EXTRA_LIGHT,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-ExtraLightItalic",
        data: ATKINSON_HYPERLEGIBLE_EXTRA_LIGHT_ITALIC,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-Light",
        data: ATKINSON_HYPERLEGIBLE_LIGHT,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-LightItalic",
        data: ATKINSON_HYPERLEGIBLE_LIGHT_ITALIC,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-Medium",
        data: ATKINSON_HYPERLEGIBLE_MEDIUM,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-MediumItalic",
        data: ATKINSON_HYPERLEGIBLE_MEDIUM_ITALIC,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-SemiBold",
        data: ATKINSON_HYPERLEGIBLE_SEMIBOLD,
    },
    EmbeddedFont {
        name: "AtkinsonHyperlegibleNext-SemiBoldItalic",
        data: ATKINSON_HYPERLEGIBLE_SEMIBOLD_ITALIC,
    },
];

pub fn embedded_fonts() -> &'static [EmbeddedFont] {
    EMBEDDED_FONTS
}

pub fn load_embedded_fonts_into_fontdb(fontdb: &mut fontdb::Database) {
    for font in embedded_fonts() {
        fontdb.load_font_data(Vec::from(font.data));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_registry_contains_bundled_default_and_legacy_faces() {
        let names = embedded_fonts()
            .iter()
            .map(|font| font.name)
            .collect::<Vec<_>>();

        assert_eq!(names.len(), 20);
        assert!(names.contains(&"Lato-Light"));
        assert!(names.contains(&"Lato-LightItalic"));
        assert!(names.contains(&"Lato-Medium"));
        assert!(names.contains(&"Lato-MediumItalic"));
        assert!(names.contains(&"Lato-Bold"));
        assert!(names.contains(&"Lato-BoldItalic"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-Regular"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-Italic"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-Bold"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-BoldItalic"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-ExtraBold"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-ExtraBoldItalic"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-ExtraLight"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-ExtraLightItalic"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-Light"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-LightItalic"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-Medium"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-MediumItalic"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-SemiBold"));
        assert!(names.contains(&"AtkinsonHyperlegibleNext-SemiBoldItalic"));
    }

    #[test]
    fn build_fontdb_loads_default_lato_family_and_weight_style_faces() {
        let options = crate::FontResolutionOptions::default();
        let fontdb = build_fontdb(&options);

        for weight in [300, 500, 700] {
            for style in [fontdb::Style::Normal, fontdb::Style::Italic] {
                let families = [fontdb::Family::Name("Lato")];
                let query = fontdb::Query {
                    families: &families,
                    weight: fontdb::Weight(weight),
                    stretch: fontdb::Stretch::Normal,
                    style,
                };
                let id = fontdb
                    .query(&query)
                    .unwrap_or_else(|| panic!("Lato {weight} {style:?} should resolve"));
                let face = fontdb.face(id).expect("resolved Lato face should exist");
                assert!(face.families.iter().any(|(family, _)| family == "Lato"));
            }
        }

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
