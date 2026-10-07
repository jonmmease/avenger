use std::{
    hash::{Hash, Hasher},
    sync::{Arc, OnceLock},
};

use avenger_typst_label::{LabelAlign, LabelLineHeight, LabelParams, LabelWidth};
use ordered_float::OrderedFloat;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use strum::VariantNames;

use crate::{DateTimeFormatProvider, NumberFormatProvider};

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[strum(serialize_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// How a label's text reads. The syntaxes differ only in how they read a newline: all of them
/// wrap alike under the label's layout.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[strum(serialize_all = "snake_case")]
pub enum TextSyntaxMode {
    /// Literal text, whose newlines are spaces.
    #[default]
    Plain,
    /// Literal text, whose newlines end lines.
    PlainLines,
    /// Typst markup, where a newline is a space and `\` ends a line.
    TypstMarkup,
}

/// How a label lays out its lines, with the label crate's options. Layouts compare and hash their
/// distances by their bits.
#[derive(Debug, Clone, Copy)]
pub struct TextLayout {
    /// How wide the label is.
    pub width: LabelWidth,
    /// Whether lines wrap at the width, rather than end only at explicit breaks.
    pub wrap: bool,
    /// The most lines the label keeps, or all of them.
    pub max_lines: Option<std::num::NonZeroUsize>,
    /// Whether "…" marks text that the width or the line limit cuts.
    pub ellipsis: bool,
    /// The distance between the label's baselines.
    pub line_height: LabelLineHeight,
    /// How the label's lines align within its width.
    pub align: LabelAlign,
}

impl TextLayout {
    /// The layout's fields, with distances as their bits.
    fn bits(&self) -> impl Eq + Hash {
        let width = match self.width {
            LabelWidth::Auto => (0u8, 0u32),
            LabelWidth::Max(width) => (1, width.to_bits()),
            LabelWidth::Fixed(width) => (2, width.to_bits()),
        };
        let line_height = match self.line_height {
            LabelLineHeight::Auto => (0u8, 0u32),
            LabelLineHeight::Fixed(distance) => (1, distance.to_bits()),
            LabelLineHeight::Relative(multiple) => (2, multiple.to_bits()),
        };
        (
            width,
            self.wrap,
            self.max_lines,
            self.ellipsis,
            line_height,
            self.align as u8,
        )
    }
}

impl PartialEq for TextLayout {
    fn eq(&self, other: &Self) -> bool {
        self.bits() == other.bits()
    }
}

impl Eq for TextLayout {}

impl Hash for TextLayout {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bits().hash(state);
    }
}

impl Default for TextLayout {
    /// One line per explicit break, at Typst's spacing.
    fn default() -> Self {
        Self {
            width: LabelWidth::Auto,
            wrap: true,
            max_lines: None,
            ellipsis: false,
            line_height: LabelLineHeight::Auto,
            align: LabelAlign::Start,
        }
    }
}

/// A label: its source, how it reads, its style and how it lays out.
#[derive(Debug, Clone)]
pub struct TextConfig<'a> {
    /// The label's source.
    pub text: &'a str,
    /// How the source reads.
    pub syntax_mode: TextSyntaxMode,
    /// A CSS-style list of font families. An empty list names the default sans-serif family.
    pub font: &'a str,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    /// The fill, as straight RGBA. Measurement ignores it.
    pub color: [f32; 4],
    /// How the label lays out its lines.
    pub layout: TextLayout,
    /// The values that markup refers to by name.
    pub params: &'a LabelParams,
    /// The provider of `#numfmt`, in place of the engine's.
    pub number_format: Option<&'a Arc<dyn NumberFormatProvider>>,
    /// The provider of `#datetimefmt`, in place of the engine's.
    pub datetime_format: Option<&'a Arc<dyn DateTimeFormatProvider>>,
}

impl Default for TextConfig<'_> {
    /// Empty plain text in the default family, at size 12, in black.
    fn default() -> Self {
        Self {
            text: "",
            syntax_mode: TextSyntaxMode::Plain,
            font: "",
            font_size: 12.0,
            font_weight: FontWeight::default(),
            font_style: FontStyle::default(),
            color: [0.0, 0.0, 0.0, 1.0],
            layout: TextLayout::default(),
            params: empty_label_params(),
            number_format: None,
            datetime_format: None,
        }
    }
}

/// Label parameters without any values.
pub fn empty_label_params() -> &'static LabelParams {
    static EMPTY: OnceLock<LabelParams> = OnceLock::new();
    EMPTY.get_or_init(LabelParams::default)
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[strum(serialize_all = "snake_case")]
/// Where a label's position lies down it.
pub enum TextBaseline {
    /// On the first line's baseline.
    Alphabetic,
    /// On the top of the label's box, which pads its lines to at least the font size.
    Top,
    /// On the middle of the box.
    Middle,
    /// On the bottom of the box.
    #[default]
    Bottom,
    /// On the top of the line box: the box with half the gap between lines above it.
    LineTop,
    /// On the bottom of the line box: the box with half the gap between lines below it.
    LineBottom,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, VariantNames)]
#[cfg_attr(feature = "serde", serde(untagged))]
#[strum(serialize_all = "snake_case")]
pub enum FontWeight {
    Name(FontWeightNameSpec),
    Number(f32),
}

impl Hash for FontWeight {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Name(spec) => spec.hash(state),
            Self::Number(num) => OrderedFloat::from(*num).hash(state),
        }
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::Name(FontWeightNameSpec::Normal)
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[strum(serialize_all = "snake_case")]
pub enum FontWeightNameSpec {
    #[default]
    Normal,
    Bold,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[strum(serialize_all = "snake_case")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}
