use crate::style::FontStyle;

pub(crate) struct EmbeddedFontFace {
    pub(crate) name: &'static str,
    pub(crate) weight: u16,
    pub(crate) style: FontStyle,
    pub(crate) data: &'static [u8],
}

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
