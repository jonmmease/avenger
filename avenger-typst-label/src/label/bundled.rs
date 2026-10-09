//! The fonts Avenger bundles, and an engine with them that callers share.

use std::sync::{Arc, OnceLock};

use avenger_fonts::{
    DEJAVU_SANS_MONO, LATO_BOLD, LATO_ITALIC, LATO_LIGHT, LATO_REGULAR, LETE_SANS_MATH,
    LETE_SANS_MATH_BOLD,
};

use super::engine::LabelEngine;
use super::options::{EngineOptions, FontOptions, RegisteredFont};

// Font selection breaks weight ties by registration order. Bold and Light come before Regular,
// so that ties resolve as CSS does: 350 picks Light, and 550 picks Bold.
const BUNDLED_FONTS: &[&[u8]] = &[
    LATO_BOLD,
    LATO_LIGHT,
    LATO_REGULAR,
    LATO_ITALIC,
    DEJAVU_SANS_MONO,
    LETE_SANS_MATH,
    LETE_SANS_MATH_BOLD,
];

/// The bundled Lato, DejaVu Sans Mono and Lete Sans Math faces, as the default sans-serif,
/// monospace and math families, with the system's fonts.
pub fn bundled_font_options() -> FontOptions {
    FontOptions {
        load_system_fonts: true,
        registered_fonts: decompressed()
            .iter()
            .map(|data| RegisteredFont::new(data.clone()))
            .collect(),
        default_sans_serif_family: Some("Lato".to_string()),
        default_monospace_family: Some("DejaVu Sans Mono".to_string()),
        default_math_family: Some("Lete Sans Math".to_string()),
        ..Default::default()
    }
}

/// An engine with the bundled fonts, which every caller shares.
pub fn bundled_label_engine() -> LabelEngine {
    static ENGINE: OnceLock<LabelEngine> = OnceLock::new();
    ENGINE
        .get_or_init(|| LabelEngine::new(EngineOptions { fonts: bundled_font_options() }))
        .clone()
}

fn decompressed() -> &'static [Arc<[u8]>] {
    static DECOMPRESSED: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();
    DECOMPRESSED.get_or_init(|| {
        BUNDLED_FONTS
            .iter()
            .map(|font| Arc::from(avenger_fonts::decompress(font)))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::{LabelOptions, MissingFontPolicy, TextStyle};
    use crate::typst_library::text::FontWeight;

    fn engine(missing_font: MissingFontPolicy) -> LabelEngine {
        LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                missing_font,
                ..bundled_font_options()
            },
        })
    }

    #[test]
    fn bundled_font_options_register_text_and_math_families() {
        let engine = engine(MissingFontPolicy::Error);
        for family in ["Lato", "DejaVu Sans Mono", "Lete Sans Math"] {
            let style = TextStyle { font_family: family.into(), ..Default::default() };
            assert!(engine.font_metrics(&style).is_ok(), "{family} should be registered");
        }
    }

    #[test]
    fn bundled_lato_weights_resolve_as_css_does() {
        let engine = engine(MissingFontPolicy::Fallback);
        for (requested, face) in [
            (300, "Lato-Light"),
            (350, "Lato-Light"),
            (400, "Lato-Regular"),
            (500, "Lato-Regular"),
            (550, "Lato-Bold"),
            (600, "Lato-Bold"),
            (700, "Lato-Bold"),
        ] {
            let mut options = LabelOptions::default();
            options.text.font_family = "Lato".into();
            options.text.font_weight = FontWeight::from_number(requested);
            let label = engine.compile_text("Weight", &options).unwrap();
            let (_, text) = label.frame.text_items()[0];
            assert_eq!(
                text.font.postscript_name().as_deref(),
                Some(face),
                "weight {requested}"
            );
        }
    }
}
