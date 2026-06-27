use std::path::PathBuf;

use crate::style::FontStyle;
use crate::TypstEngineConfig;

pub(crate) struct EmbeddedFontFace {
    pub(crate) name: &'static str,
    pub(crate) weight: u16,
    pub(crate) style: FontStyle,
    pub(crate) data: &'static [u8],
}

pub(crate) struct EmbeddedMathFontFace {
    pub(crate) weight: u16,
    pub(crate) data: &'static [u8],
}

pub(crate) const LATO_FACES: &[EmbeddedFontFace] = &[
    EmbeddedFontFace {
        name: "Lato-Light",
        weight: 300,
        style: FontStyle::Normal,
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Light.ttf"),
    },
    EmbeddedFontFace {
        name: "Lato-LightItalic",
        weight: 300,
        style: FontStyle::Italic,
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-LightItalic.ttf"),
    },
    EmbeddedFontFace {
        name: "Lato-Medium",
        weight: 500,
        style: FontStyle::Normal,
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Medium.ttf"),
    },
    EmbeddedFontFace {
        name: "Lato-MediumItalic",
        weight: 500,
        style: FontStyle::Italic,
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-MediumItalic.ttf"),
    },
    EmbeddedFontFace {
        name: "Lato-Bold",
        weight: 700,
        style: FontStyle::Normal,
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-Bold.ttf"),
    },
    EmbeddedFontFace {
        name: "Lato-BoldItalic",
        weight: 700,
        style: FontStyle::Italic,
        data: include_bytes!("../../avenger-chart/fonts/Lato/Lato-BoldItalic.ttf"),
    },
];

pub(crate) const ATKINSON_FACES: &[EmbeddedFontFace] = &[
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-Regular",
        weight: 400,
        style: FontStyle::Normal,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Regular.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-Italic",
        weight: 400,
        style: FontStyle::Italic,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Italic.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-Bold",
        weight: 700,
        style: FontStyle::Normal,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Bold.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-BoldItalic",
        weight: 700,
        style: FontStyle::Italic,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-BoldItalic.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-ExtraBold",
        weight: 800,
        style: FontStyle::Normal,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraBold.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-ExtraBoldItalic",
        weight: 800,
        style: FontStyle::Italic,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraBoldItalic.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-ExtraLight",
        weight: 250,
        style: FontStyle::Normal,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraLight.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-ExtraLightItalic",
        weight: 250,
        style: FontStyle::Italic,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-ExtraLightItalic.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-Light",
        weight: 300,
        style: FontStyle::Normal,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Light.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-LightItalic",
        weight: 300,
        style: FontStyle::Italic,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-LightItalic.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-Medium",
        weight: 500,
        style: FontStyle::Normal,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Medium.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-MediumItalic",
        weight: 500,
        style: FontStyle::Italic,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-MediumItalic.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-SemiBold",
        weight: 600,
        style: FontStyle::Normal,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-SemiBold.ttf"
        ),
    },
    EmbeddedFontFace {
        name: "AtkinsonHyperlegibleNext-SemiBoldItalic",
        weight: 600,
        style: FontStyle::Italic,
        data: include_bytes!(
            "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-SemiBoldItalic.ttf"
        ),
    },
];

pub(crate) fn bundled_math_fonts() -> &'static [EmbeddedMathFontFace] {
    &[
        EmbeddedMathFontFace {
            weight: 400,
            data: include_bytes!("../../avenger-chart/fonts/Lete_Sans_Math/LeteSansMath.otf"),
        },
        EmbeddedMathFontFace {
            weight: 700,
            data: include_bytes!("../../avenger-chart/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf"),
        },
    ]
}

pub(crate) fn candidate_math_font_paths(config: &TypstEngineConfig) -> Vec<PathBuf> {
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
        collect_font_paths(
            dir,
            &mut push,
            !config.font_config.extra_font_families.is_empty(),
        );
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

        if !is_font_file(&path) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if include_all_fonts || file_name.contains("math") || file_name.contains("stix") {
            push(path);
        }
    }
}

fn is_font_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("otf" | "ttf" | "ttc")
    )
}
