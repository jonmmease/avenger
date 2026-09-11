#[cfg(test)]
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;

use crate::label::EngineOptions;
use crate::typst_library::MathFontBytesId;

pub(crate) fn load_registered_fonts_into_fontdb(
    fontdb: &mut fontdb::Database,
    config: &EngineOptions,
) {
    for font in &config.fonts.registered_fonts {
        fontdb.load_font_data(font.data.to_vec());
    }
}

#[cfg(test)]
pub(crate) fn load_test_fonts_into_fontdb(fontdb: &mut fontdb::Database) {
    for (_name, compressed_data) in TEST_FONTS {
        let data = decompress_test_font(compressed_data);
        fontdb.load_font_data(data);
    }
    fontdb.set_sans_serif_family("Lato");
    fontdb.set_monospace_family("DejaVu Sans Mono");
}

pub(crate) fn registered_font_data(
    config: &EngineOptions,
    id: MathFontBytesId,
) -> Option<(Arc<[u8]>, u32)> {
    config
        .fonts
        .registered_fonts
        .iter()
        .find(|font| font.id == id)
        .map(|font| (font.data.clone(), font.face_index))
}

pub(crate) fn candidate_math_font_paths(config: &EngineOptions) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut push = |path: PathBuf| {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    };

    for dir in &config.fonts.extra_font_dirs {
        collect_font_paths(dir.clone(), &mut push, true);
    }

    if config.fonts.load_system_fonts {
        for path in hardcoded_math_font_paths() {
            push(path.into());
        }

        for dir in system_font_dirs() {
            collect_font_paths(dir, &mut push, !config.fonts.extra_font_families.is_empty());
        }
    }

    paths
}

fn hardcoded_math_font_paths() -> &'static [&'static str] {
    &[
        "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
        "/Library/Fonts/STIXTwoMath.otf",
        "/usr/share/fonts/opentype/stix/STIXTwoMath-Regular.otf",
        "/usr/share/fonts/opentype/stix/STIXTwoMath.otf",
        "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
        "C:\\Windows\\Fonts\\cambria.ttc",
    ]
}

fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
        PathBuf::from("C:\\Windows\\Fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Fonts"));
    }
    dirs
}

fn collect_font_paths(dir: PathBuf, push: &mut impl FnMut(PathBuf), include_all_fonts: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_paths(path, push, include_all_fonts);
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let name = file_name.to_ascii_lowercase();
        let is_font = name.ends_with(".otf") || name.ends_with(".ttf") || name.ends_with(".ttc");
        let is_math_font = name.contains("math") || name.contains("cambria");
        if is_font && (is_math_font || include_all_fonts) {
            push(path);
        }
    }
}

#[cfg(test)]
const TEST_FONTS: &[(&str, &[u8])] = &[
    (
        "Lato-Light",
        include_bytes!("../../../avenger-text/fonts/Lato/Lato-Light.ttf.br"),
    ),
    (
        "Lato-Italic",
        include_bytes!("../../../avenger-text/fonts/Lato/Lato-Italic.ttf.br"),
    ),
    (
        "Lato-Medium",
        include_bytes!("../../../avenger-text/fonts/Lato/Lato-Medium.ttf.br"),
    ),
    (
        "Lato-Bold",
        include_bytes!("../../../avenger-text/fonts/Lato/Lato-Bold.ttf.br"),
    ),
    (
        "DejaVuSansMono",
        include_bytes!("../../../avenger-text/fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br"),
    ),
    (
        "LeteSansMath",
        include_bytes!("../../../avenger-text/fonts/Lete_Sans_Math/LeteSansMath.otf.br"),
    ),
    (
        "LeteSansMath-Bold",
        include_bytes!("../../../avenger-text/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br"),
    ),
];

#[cfg(test)]
fn decompress_test_font(compressed_data: &[u8]) -> Vec<u8> {
    let mut reader = brotli::Decompressor::new(Cursor::new(compressed_data), 4096);
    let mut data = Vec::new();
    reader
        .read_to_end(&mut data)
        .expect("test font should decompress");
    assert!(!data.is_empty(), "test font should not decompress empty");
    data
}
