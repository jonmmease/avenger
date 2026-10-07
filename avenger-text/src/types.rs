use ordered_float::OrderedFloat;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use strum::VariantNames;

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

/// How a label lays out its lines, with the label crate's options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLayout {
    /// How wide the label is.
    pub width: avenger_typst_label::LabelWidth,
    /// Whether lines wrap at the width, rather than end only at explicit breaks.
    pub wrap: bool,
    /// The most lines the label keeps, or all of them.
    pub max_lines: Option<std::num::NonZeroUsize>,
    /// Whether "…" marks text that the width or the line limit cuts.
    pub ellipsis: bool,
    /// The distance between the label's baselines.
    pub line_height: avenger_typst_label::LabelLineHeight,
    /// How the label's lines align within its width.
    pub align: avenger_typst_label::LabelAlign,
}

impl Default for TextLayout {
    /// One line per explicit break, at Typst's spacing.
    fn default() -> Self {
        Self {
            width: avenger_typst_label::LabelWidth::Auto,
            wrap: true,
            max_lines: None,
            ellipsis: false,
            line_height: avenger_typst_label::LabelLineHeight::Auto,
            align: avenger_typst_label::LabelAlign::Start,
        }
    }
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

impl std::hash::Hash for FontWeight {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
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
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[strum(serialize_all = "snake_case")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}
