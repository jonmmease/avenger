use std::{io::Read, sync::Arc};

use avenger_typst_label::{EngineOptions, MathFontBytesId, RegisteredFont};

pub fn engine_options() -> EngineOptions {
    let mut options = EngineOptions::default();
    options.fonts.default_sans_serif_family = Some("Lato".into());
    options.fonts.default_monospace_family = Some("DejaVu Sans Mono".into());
    options.fonts.default_math_family = Some("Lete Sans Math".into());
    options.fonts.registered_fonts = FONT_BYTES
        .iter()
        .enumerate()
        .map(|(index, compressed)| {
            let mut bytes = Vec::new();
            brotli::Decompressor::new(*compressed, 4096)
                .read_to_end(&mut bytes)
                .expect("fixture font should decompress");
            RegisteredFont::new(MathFontBytesId(index as u64 + 1), Arc::<[u8]>::from(bytes))
        })
        .collect();
    options
}

const FONT_BYTES: &[&[u8]] = &[
    include_bytes!("../../../avenger-text/fonts/Lato/Lato-Light.ttf.br"),
    include_bytes!("../../../avenger-text/fonts/Lato/Lato-Italic.ttf.br"),
    include_bytes!("../../../avenger-text/fonts/Lato/Lato-Medium.ttf.br"),
    include_bytes!("../../../avenger-text/fonts/Lato/Lato-Bold.ttf.br"),
    include_bytes!("../../../avenger-text/fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br"),
    include_bytes!("../../../avenger-text/fonts/Lete_Sans_Math/LeteSansMath.otf.br"),
    include_bytes!("../../../avenger-text/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br"),
];
