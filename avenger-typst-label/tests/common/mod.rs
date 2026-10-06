pub mod oracle;

use std::{io::Read, sync::Arc};

use avenger_typst_label::{EngineOptions, RegisteredFont};

pub fn engine_options() -> EngineOptions {
    let mut options = EngineOptions::default();
    options.fonts.default_sans_serif_family = Some("Lato".into());
    options.fonts.default_monospace_family = Some("DejaVu Sans Mono".into());
    options.fonts.default_math_family = Some("Lete Sans Math".into());
    options.fonts.registered_fonts = FONT_BYTES
        .iter()
        .map(|compressed| {
            let mut bytes = Vec::new();
            brotli::Decompressor::new(*compressed, 4096)
                .read_to_end(&mut bytes)
                .expect("fixture font should decompress");
            RegisteredFont::new(Arc::<[u8]>::from(bytes))
        })
        .collect();
    options
}

/// The fixture fonts, in the probe's order: sorted by file name. The book breaks ties
/// between equally good faces by order, so `#text(weight: 600)` takes Lato Bold.
const FONT_BYTES: &[&[u8]] = &[
    include_bytes!("../fixtures/fonts/AuditHebrewRegular.ttf.br"),
    include_bytes!("../fixtures/fonts/AuditNoScriptMetrics.ttf.br"),
    include_bytes!("../fixtures/fonts/AuditScriptOffsets.ttf.br"),
    avenger_fonts::DEJAVU_SANS_MONO,
    avenger_fonts::LATO_BOLD,
    avenger_fonts::LATO_ITALIC,
    avenger_fonts::LATO_LIGHT,
    include_bytes!("../fixtures/fonts/Lato-Medium.ttf.br"),
    avenger_fonts::LETE_SANS_MATH_BOLD,
    avenger_fonts::LETE_SANS_MATH,
    include_bytes!("../fixtures/fonts/NotoSansDevanagari.ttf.br"),
    include_bytes!("../fixtures/fonts/NotoSansHebrew.ttf.br"),
];
