use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::label::EngineOptions;

use super::FontStyle;

pub(crate) struct EmbeddedFontFace {
    family: EmbeddedFontFamily,
    pub(crate) name: &'static str,
    index: usize,
    pub(crate) weight: u16,
    pub(crate) style: FontStyle,
    pub(crate) compressed_data: &'static [u8],
}

impl EmbeddedFontFace {
    pub(crate) fn decompressed_data(&self) -> Arc<[u8]> {
        match self.family {
            EmbeddedFontFamily::Lato => decompressed_lato_faces()[self.index].clone(),
            EmbeddedFontFamily::DejaVuSansMono => {
                decompressed_dejavu_sans_mono_faces()[self.index].clone()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddedFontFamily {
    Lato,
    DejaVuSansMono,
}

pub(crate) struct EmbeddedMathFontFace {
    pub(crate) name: &'static str,
    index: usize,
    pub(crate) weight: u16,
    pub(crate) compressed_data: &'static [u8],
}

impl EmbeddedMathFontFace {
    pub(crate) fn decompressed_data(&self) -> Arc<[u8]> {
        decompressed_math_faces()[self.index].clone()
    }
}

pub(crate) const LATO_FACES: &[EmbeddedFontFace] = &[
    EmbeddedFontFace {
        family: EmbeddedFontFamily::Lato,
        name: "Lato-Light",
        index: 0,
        weight: 300,
        style: FontStyle::Normal,
        compressed_data: include_bytes!("../../../../avenger-chart/fonts/Lato/Lato-Light.ttf.br"),
    },
    EmbeddedFontFace {
        family: EmbeddedFontFamily::Lato,
        name: "Lato-Italic",
        index: 1,
        weight: 400,
        style: FontStyle::Italic,
        compressed_data: include_bytes!("../../../../avenger-chart/fonts/Lato/Lato-Italic.ttf.br"),
    },
    EmbeddedFontFace {
        family: EmbeddedFontFamily::Lato,
        name: "Lato-Medium",
        index: 2,
        weight: 500,
        style: FontStyle::Normal,
        compressed_data: include_bytes!("../../../../avenger-chart/fonts/Lato/Lato-Medium.ttf.br"),
    },
    EmbeddedFontFace {
        family: EmbeddedFontFamily::Lato,
        name: "Lato-Bold",
        index: 3,
        weight: 700,
        style: FontStyle::Normal,
        compressed_data: include_bytes!("../../../../avenger-chart/fonts/Lato/Lato-Bold.ttf.br"),
    },
];

pub(crate) const DEJAVU_SANS_MONO_FACES: &[EmbeddedFontFace] = &[EmbeddedFontFace {
    family: EmbeddedFontFamily::DejaVuSansMono,
    name: "DejaVuSansMono",
    index: 0,
    weight: 400,
    style: FontStyle::Normal,
    compressed_data: include_bytes!(
        "../../../../avenger-chart/fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br"
    ),
}];

pub(crate) fn bundled_math_fonts() -> &'static [EmbeddedMathFontFace] {
    &[
        EmbeddedMathFontFace {
            name: "LeteSansMath",
            index: 0,
            weight: 400,
            compressed_data: include_bytes!(
                "../../../../avenger-chart/fonts/Lete_Sans_Math/LeteSansMath.otf.br"
            ),
        },
        EmbeddedMathFontFace {
            name: "LeteSansMath-Bold",
            index: 1,
            weight: 700,
            compressed_data: include_bytes!(
                "../../../../avenger-chart/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br"
            ),
        },
    ]
}

pub(crate) fn candidate_math_font_paths(config: &EngineOptions) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut push = |path: PathBuf| {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    };

    for path in hardcoded_math_font_paths() {
        push(path.into());
    }

    for dir in system_font_dirs() {
        collect_font_paths(dir, &mut push, !config.fonts.extra_font_families.is_empty());
    }

    paths
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

fn decompressed_lato_faces() -> &'static [Arc<[u8]>] {
    static DECOMPRESSED_FACES: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();
    DECOMPRESSED_FACES.get_or_init(|| {
        LATO_FACES
            .iter()
            .map(|face| {
                Arc::<[u8]>::from(
                    decompress_brotli_font(face.name, face.compressed_data).unwrap_or_else(|err| {
                        panic!("failed to decompress embedded font {}: {err}", face.name)
                    }),
                )
            })
            .collect()
    })
}

fn decompressed_dejavu_sans_mono_faces() -> &'static [Arc<[u8]>] {
    static DECOMPRESSED_FACES: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();
    DECOMPRESSED_FACES.get_or_init(|| {
        DEJAVU_SANS_MONO_FACES
            .iter()
            .map(|face| {
                Arc::<[u8]>::from(
                    decompress_brotli_font(face.name, face.compressed_data).unwrap_or_else(|err| {
                        panic!("failed to decompress embedded font {}: {err}", face.name)
                    }),
                )
            })
            .collect()
    })
}

fn decompressed_math_faces() -> &'static [Arc<[u8]>] {
    static DECOMPRESSED_FACES: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();
    DECOMPRESSED_FACES.get_or_init(|| {
        bundled_math_fonts()
            .iter()
            .map(|face| {
                Arc::<[u8]>::from(
                    decompress_brotli_font(face.name, face.compressed_data).unwrap_or_else(|err| {
                        panic!("failed to decompress embedded font {}: {err}", face.name)
                    }),
                )
            })
            .collect()
    })
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
