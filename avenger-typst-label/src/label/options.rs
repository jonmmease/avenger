//! The options of an engine and of each label.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use avenger_color::AbsoluteColor;
use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::params::LabelParams;
use crate::typst_library::text::{FontStyle, FontWeight, Lang, Region};

/// The options of a [`LabelEngine`](super::LabelEngine).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EngineOptions {
    /// Where fonts come from and how missing ones are reported.
    pub fonts: FontOptions,
}

/// Where an engine's fonts come from and how missing ones are reported.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FontOptions {
    /// Whether to use the system's fonts, after the registered fonts and the extra
    /// directories' fonts.
    pub load_system_fonts: bool,
    /// What happens when families of a label's font lists are not available.
    pub missing_font: MissingFontPolicy,
    /// Directories whose fonts come after the registered fonts.
    pub extra_font_dirs: Vec<PathBuf>,
    /// Font data, which comes first in the given order.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub registered_fonts: Vec<RegisteredFont>,
    /// The family `sans-serif` names, and the first fallback family.
    pub default_sans_serif_family: Option<String>,
    /// The family `monospace` names, which raw text uses.
    pub default_monospace_family: Option<String>,
    /// The math family of labels whose math style names none, and the first fallback
    /// family in math.
    pub default_math_family: Option<String>,
}

impl Default for FontOptions {
    fn default() -> Self {
        Self {
            load_system_fonts: true,
            missing_font: MissingFontPolicy::Fallback,
            extra_font_dirs: Vec::new(),
            registered_fonts: Vec::new(),
            default_sans_serif_family: None,
            default_monospace_family: None,
            default_math_family: None,
        }
    }
}

/// Font data that an application provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredFont {
    /// The font file's data, which the engine shares rather than copies.
    pub data: Arc<[u8]>,
    /// The face's index in a font collection, or zero for a single font.
    pub face_index: u32,
}

impl RegisteredFont {
    /// The first face of the font data.
    pub fn new(data: impl Into<Arc<[u8]>>) -> Self {
        Self { data: data.into(), face_index: 0 }
    }

    /// Selects a face of a font collection.
    pub fn with_face_index(mut self, face_index: u32) -> Self {
        self.face_index = face_index;
        self
    }
}

/// What happens when families of a label's font lists are not available.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MissingFontPolicy {
    /// Fail when none of a list's families is available.
    Error,
    /// Warn about each family that is not available.
    Warn,
    /// Fall back silently.
    #[default]
    Fallback,
}

/// The options of one label.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelOptions {
    /// The label's text style.
    pub text: TextStyle,
    /// The label's style for math, over the text style.
    pub math: MathStyle,
    /// How wide the label is.
    pub width: LabelWidth,
    /// Whether lines wrap at the width, as in Typst, rather than end only at explicit breaks.
    /// Without wrapping, a line wider than the width overflows it or, with an ellipsis, is
    /// shortened to fit.
    pub wrap: bool,
    /// How the label's lines align within its width.
    pub align: LabelAlign,
    /// The most lines the label keeps, or all of them. Lines past the limit are dropped.
    pub max_lines: Option<NonZeroUsize>,
    /// Whether a sign that starts a line, `+`, `−`, `-`, `±` or `∓`, hangs out of it by its
    /// full width, so that numbers with and without signs align by their digits. The sign
    /// lies outside the label's width and its line's metrics. A line starts at its left in
    /// left-to-right text, where the sign of an equation, as in `#numfmt`'s scientific
    /// notation, counts too, and at its right in right-to-left text.
    pub hanging_signs: bool,
    /// Whether "…" marks cut text. With an ellipsis, each line wider than the width, and the
    /// last line when dropped lines show anything, ends in "…" and is shortened to fit.
    /// [`LabelFlags::truncated`](super::LabelFlags::truncated) says whether text was cut.
    pub ellipsis: bool,
    /// The values that the label's source can refer to by name.
    pub params: LabelParams,
    /// Bounds on the label's work.
    pub limits: LabelLimits,
}

impl Default for LabelOptions {
    fn default() -> Self {
        Self {
            text: TextStyle::default(),
            math: MathStyle::default(),
            width: LabelWidth::default(),
            wrap: true,
            align: LabelAlign::default(),
            max_lines: None,
            hanging_signs: false,
            ellipsis: false,
            params: LabelParams::default(),
            limits: LabelLimits::default(),
        }
    }
}

/// A label's text style.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextStyle {
    /// A CSS-style list of font families, such as `"Lato, sans-serif"`. The generic families
    /// `sans-serif`, `serif` and `monospace` name the engine's families for them.
    pub font_family: String,
    /// The font size, in points.
    pub font_size: f32,
    /// The text's fill.
    #[cfg_attr(feature = "serde", serde(with = "serde_impls::color"))]
    pub fill: AbsoluteColor,
    /// The font weight.
    pub font_weight: FontWeight,
    /// The font style.
    pub font_style: FontStyle,
    /// The text's language, which selects smart quotes and the text direction.
    pub lang: Lang,
    /// The text's region, which refines the language.
    pub region: Option<Region>,
    /// The base direction of the text. `Auto` takes it from the language.
    pub dir: TextDir,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: "sans-serif".into(),
            font_size: 12.0,
            fill: AbsoluteColor::from_srgb(0.0, 0.0, 0.0, 1.0),
            font_weight: FontWeight::REGULAR,
            font_style: FontStyle::Normal,
            lang: Lang::ENGLISH,
            region: None,
            dir: TextDir::Auto,
        }
    }
}

/// The base direction of text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TextDir {
    /// The direction of the text's language.
    #[default]
    Auto,
    /// Left to right.
    Ltr,
    /// Right to left.
    Rtl,
}

/// A label's style for math, over its text style. Math takes the text's size, fill and
/// weight unless this sets them.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathStyle {
    /// A list of math font families. Empty names the engine's default math family.
    pub font_family: String,
    /// The size, relative to the surrounding text.
    pub font_size: Option<Em>,
    /// The fill.
    #[cfg_attr(feature = "serde", serde(with = "serde_impls::optional_color"))]
    pub fill: Option<AbsoluteColor>,
    /// The font weight.
    pub font_weight: Option<FontWeight>,
}

/// A length relative to the font size.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Em(pub f32);

/// How wide a label is. Unless [`LabelOptions::wrap`] is off, lines wrap at the width,
/// greedily at break opportunities as in Typst. A word wider than the width overflows it,
/// unless [`LabelOptions::ellipsis`] shortens its line to fit.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LabelWidth {
    /// No width: lines end only at explicit breaks, and the label is as wide as its widest
    /// line.
    #[default]
    Auto,
    /// A width, in points: the label is as wide as its widest line, up to the width.
    Max(f32),
    /// A width, in points: the label is exactly that wide.
    Fixed(f32),
}

/// How a label's lines align within its width.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LabelAlign {
    /// At the start of the text direction: left in left-to-right text, right in right-to-left
    /// text.
    #[default]
    Start,
    /// At the left.
    Left,
    /// In the middle.
    Center,
    /// At the right.
    Right,
    /// At the end of the text direction.
    End,
}

/// Bounds on the work one label may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LabelLimits {
    /// Largest accepted source, in bytes.
    pub max_source_bytes: usize,
    /// Most equations in one label.
    pub max_math_spans: usize,
    /// Deepest nesting of math constructs: delimiters, attachments, fractions, roots, and
    /// calls each add a level. Layout recurses per level, and the default keeps a label within
    /// a 1 MiB stack, the WebAssembly default.
    pub max_math_depth: usize,
}

impl Default for LabelLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 16 * 1024,
            max_math_spans: 64,
            max_math_depth: 32,
        }
    }
}

/// Formatting providers for one label. Missing providers fall back to the engine's.
#[derive(Debug, Clone, Copy, Default)]
pub struct LabelFormatting<'a> {
    /// The provider of `#numfmt`.
    pub number: Option<&'a Arc<dyn NumberFormatProvider>>,
    /// The provider of `#datetimefmt`.
    pub datetime: Option<&'a Arc<dyn DateTimeFormatProvider>>,
}

/// String and number forms of the re-exported font and language types, and colors as their
/// components.
#[cfg(feature = "serde")]
mod serde_impls {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::typst_library::text::{FontStyle, FontWeight, Lang, Region};

    /// A color's fields.
    #[derive(Serialize, Deserialize)]
    struct Color {
        components: [f32; 3],
        alpha: f32,
        color_space: avenger_color::ColorSpace,
    }

    impl From<&avenger_color::AbsoluteColor> for Color {
        fn from(color: &avenger_color::AbsoluteColor) -> Self {
            Self {
                components: color.components,
                alpha: color.alpha,
                color_space: color.color_space,
            }
        }
    }

    impl From<Color> for avenger_color::AbsoluteColor {
        fn from(color: Color) -> Self {
            let [c0, c1, c2] = color.components;
            Self::new(color.color_space, c0, c1, c2, color.alpha)
        }
    }

    pub mod color {
        use super::*;

        pub fn serialize<S: Serializer>(
            color: &avenger_color::AbsoluteColor,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            Color::from(color).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<avenger_color::AbsoluteColor, D::Error> {
            Ok(Color::deserialize(deserializer)?.into())
        }
    }

    pub mod optional_color {
        use super::*;

        pub fn serialize<S: Serializer>(
            color: &Option<avenger_color::AbsoluteColor>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            color.as_ref().map(Color::from).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<avenger_color::AbsoluteColor>, D::Error> {
            Ok(Option::<Color>::deserialize(deserializer)?.map(Into::into))
        }
    }

    impl Serialize for FontWeight {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.to_number().serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for FontWeight {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            Ok(Self::from_number(u16::deserialize(deserializer)?))
        }
    }

    impl Serialize for FontStyle {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match self {
                Self::Normal => "normal",
                Self::Italic => "italic",
                Self::Oblique => "oblique",
            }
            .serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for FontStyle {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            match String::deserialize(deserializer)?.as_str() {
                "normal" => Ok(Self::Normal),
                "italic" => Ok(Self::Italic),
                "oblique" => Ok(Self::Oblique),
                other => Err(D::Error::custom(format!("unknown font style {other:?}"))),
            }
        }
    }

    impl Serialize for Lang {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.as_str().serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for Lang {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            String::deserialize(deserializer)?.parse().map_err(D::Error::custom)
        }
    }

    impl Serialize for Region {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.as_str().serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for Region {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            String::deserialize(deserializer)?.parse().map_err(D::Error::custom)
        }
    }
}
