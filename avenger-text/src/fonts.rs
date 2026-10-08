use std::sync::{Arc, OnceLock};

use crate::{FontOptions, RegisteredFont};

use avenger_fonts::{
    DEJAVU_SANS_MONO, LATO_BOLD, LATO_ITALIC, LATO_LIGHT, LATO_REGULAR, LETE_SANS_MATH,
    LETE_SANS_MATH_BOLD,
};

// Font selection breaks weight ties by registration order. Bold and Light come before Regular,
// so that ties resolve as CSS does: 350 picks Light, and 550 picks Bold.
const DEFAULT_FONTS: &[&[u8]] = &[
    LATO_BOLD,
    LATO_LIGHT,
    LATO_REGULAR,
    LATO_ITALIC,
    DEJAVU_SANS_MONO,
    LETE_SANS_MATH,
    LETE_SANS_MATH_BOLD,
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
            .map(|font| Arc::from(avenger_fonts::decompress(font)))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_options_register_text_and_math_families() {
        let engine = crate::TextEngine::new(&FontOptions {
            load_system_fonts: false,
            missing_font: crate::MissingFontPolicy::Error,
            ..default_font_options()
        });
        for family in ["Lato", "DejaVu Sans Mono", "Lete Sans Math"] {
            let metrics = engine.font_metrics(&crate::measurement::FontMetricsConfig {
                font: family,
                font_size: 12.0,
                font_weight: avenger_common::types::FontWeight::default(),
                font_style: avenger_common::types::FontStyle::default(),
            });
            assert!(metrics.is_ok(), "{family} should be registered");
        }
    }

    #[test]
    fn bundled_lato_weights_resolve_as_css_does() {
        use crate::types::TextConfig;
        use avenger_common::types::FontWeight;
        let engine = crate::TextEngine::new(&FontOptions {
            load_system_fonts: false,
            ..default_font_options()
        });
        for (requested, resolved) in [
            (300.0, 300),
            (350.0, 300),
            (400.0, 400),
            (500.0, 400),
            (550.0, 700),
            (600.0, 700),
            (700.0, 700),
        ] {
            let buffer = engine
                .extract_paths(&TextConfig {
                    text: "Weight",
                    font: "Lato",
                    font_weight: FontWeight::from(requested),
                    ..Default::default()
                })
                .unwrap();
            let Some(crate::path::TextPathItem::Run(run)) = buffer.items.first() else {
                panic!("{:?}", buffer.items);
            };
            assert_eq!(
                run.weight,
                avenger_typst_label::FontWeight::from_number(resolved),
                "weight {requested}"
            );
        }
    }
}
