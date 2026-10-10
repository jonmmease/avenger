use std::{
    hash::{Hash, Hasher},
    num::NonZeroUsize,
    sync::Arc,
};

use super::mark::{default_interactive, SceneMark};
use avenger_color::{AbsoluteColor, ColorOrGradient};

use avenger_common::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_typst_label::{
    Label, LabelAlign, LabelLineHeight, LabelOptions, LabelSource, LabelWidth, TextBounds,
    TextStyle,
};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    /// The mark's name, which events report.
    pub name: String,
    /// Whether hit tests can find the mark's labels.
    #[serde(default = "default_interactive")]
    pub interactive: bool,
    /// Whether the enclosing group's clip applies.
    pub clip: bool,
    /// The number of labels.
    pub len: u32,
    /// Each label's text.
    pub text: ScalarOrArray<String>,
    /// Whether the text is plain, plain with newlines that end lines, or Typst markup.
    #[serde(default)]
    pub text_syntax: TextSyntaxMode,
    /// Each label's horizontal position.
    pub x: ScalarOrArray<f32>,
    /// Each label's vertical position.
    pub y: ScalarOrArray<f32>,
    /// Where each label's position lies across it.
    pub align: ScalarOrArray<TextAlign>,
    /// Where each label's position lies down it.
    pub baseline: ScalarOrArray<TextBaseline>,
    /// Each label's rotation about its position, in degrees.
    pub angle: ScalarOrArray<f32>,
    /// Each label's color.
    pub color: ScalarOrArray<ColorOrGradient>,
    /// Each label's opacity for chart adjustments, which renderers take from `color`.
    #[serde(default = "default_one_f32_channel")]
    pub opacity: ScalarOrArray<f32>,
    /// Each label's CSS-style list of font families.
    pub font: ScalarOrArray<String>,
    /// Each label's font size.
    pub font_size: ScalarOrArray<f32>,
    /// Each label's font weight.
    pub font_weight: ScalarOrArray<FontWeight>,
    /// Each label's font style.
    pub font_style: ScalarOrArray<FontStyle>,
    /// How wide each label is.
    pub width: ScalarOrArray<LabelWidth>,
    /// Whether lines wrap at the width.
    pub wrap: bool,
    /// The most lines each label keeps, or all of them.
    pub max_lines: Option<NonZeroUsize>,
    /// Whether "…" marks text that the width or `max_lines` cuts.
    pub ellipsis: bool,
    /// Whether a sign that starts a line hangs out of it, so numbers align by their digits.
    pub hanging_signs: bool,
    /// The distance between each label's baselines.
    pub line_height: ScalarOrArray<LabelLineHeight>,
    /// How each label's lines align within its box.
    pub line_align: ScalarOrArray<LabelAlign>,
    /// The labels to draw, in order, or all of them.
    pub indices: Option<Arc<Vec<usize>>>,
    /// The mark's drawing order: a higher zindex draws over a lower one.
    pub zindex: Option<i32>,
}

impl Hash for SceneTextMark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.interactive.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.text.hash(state);
        self.text_syntax.hash(state);
        self.x.hash(state);
        self.y.hash(state);
        self.align.hash(state);
        self.baseline.hash(state);
        self.angle.hash(state);
        self.color.hash(state);
        self.opacity.hash(state);
        self.font.hash(state);
        self.font_size.hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        hash_channel(&self.width, state, |width| match *width {
            LabelWidth::Auto => (0u8, 0u32),
            LabelWidth::Max(width) => (1, width.to_bits()),
            LabelWidth::Fixed(width) => (2, width.to_bits()),
        });
        self.wrap.hash(state);
        self.max_lines.hash(state);
        self.ellipsis.hash(state);
        self.hanging_signs.hash(state);
        hash_channel(&self.line_height, state, |line_height| match *line_height {
            LabelLineHeight::Auto => (0u8, 0u32),
            LabelLineHeight::Fixed(distance) => (1, distance.to_bits()),
            LabelLineHeight::Relative(multiple) => (2, multiple.to_bits()),
        });
        hash_channel(&self.line_align, state, |align| *align as u8);
        self.indices.hash(state);
        self.zindex.hash(state);
    }
}

/// Hashes a channel of the label crate's types, which don't implement `Hash`, by a key of its
/// value or of each of its values.
fn hash_channel<T: Sync + Clone, K: Hash, H: Hasher>(
    channel: &ScalarOrArray<T>,
    state: &mut H,
    key: impl Fn(&T) -> K,
) {
    match channel.value() {
        ScalarOrArrayValue::Scalar(value) => key(value).hash(state),
        ScalarOrArrayValue::Array(values) => values.iter().for_each(|value| key(value).hash(state)),
    }
}

fn default_one_f32_channel() -> ScalarOrArray<f32> {
    ScalarOrArray::new_scalar(1.0)
}

impl SceneTextMark {
    pub fn text_iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.text.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn align_iter(&self) -> Box<dyn Iterator<Item = &TextAlign> + '_> {
        self.align.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn baseline_iter(&self) -> Box<dyn Iterator<Item = &TextBaseline> + '_> {
        self.baseline
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn color_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.color.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn opacity_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.opacity
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.font.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.font_size
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_weight_iter(&self) -> Box<dyn Iterator<Item = &FontWeight> + '_> {
        self.font_weight
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_style_iter(&self) -> Box<dyn Iterator<Item = &FontStyle> + '_> {
        self.font_style
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn width_iter(&self) -> Box<dyn Iterator<Item = &LabelWidth> + '_> {
        self.width.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn line_height_iter(&self) -> Box<dyn Iterator<Item = &LabelLineHeight> + '_> {
        self.line_height
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn line_align_iter(&self) -> Box<dyn Iterator<Item = &LabelAlign> + '_> {
        self.line_align
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    /// The mark's labels. A width or line height that the label crate would reject means none,
    /// since scene data shouldn't keep a label from drawing.
    pub fn labels(&self) -> impl Iterator<Item = TextLabel<'_>> + '_ {
        izip!(
            self.text_iter(),
            self.x_iter(),
            self.y_iter(),
            self.color_iter(),
            self.align_iter(),
            self.baseline_iter(),
            self.angle_iter(),
            self.font_iter(),
            self.font_size_iter(),
            self.font_weight_iter(),
            self.font_style_iter(),
            self.width_iter(),
            self.line_height_iter(),
            self.line_align_iter(),
        )
        .map(
            move |(
                text,
                x,
                y,
                color,
                align,
                baseline,
                angle,
                font,
                font_size,
                font_weight,
                font_style,
                width,
                line_height,
                line_align,
            )| {
                let options = LabelOptions {
                    text: text_style(
                        font,
                        *font_size,
                        *font_weight,
                        *font_style,
                        color.color_or_transparent(),
                    ),
                    width: valid_width(*width),
                    wrap: self.wrap,
                    align: *line_align,
                    line_height: valid_line_height(*line_height),
                    max_lines: self.max_lines,
                    ellipsis: self.ellipsis,
                    hanging_signs: self.hanging_signs,
                    ..Default::default()
                };
                TextLabel {
                    label: text_label(text, self.text_syntax, options),
                    position: [*x, *y],
                    align: *align,
                    baseline: *baseline,
                    angle: *angle,
                }
            },
        )
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new(0..self.len as usize)
        }
    }
}

impl Default for SceneTextMark {
    fn default() -> Self {
        Self {
            interactive: true,
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: ScalarOrArray::new_scalar(String::new()),
            text_syntax: TextSyntaxMode::Plain,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(TextAlign::Left),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Alphabetic),
            angle: ScalarOrArray::new_scalar(0.0),
            color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            font: ScalarOrArray::new_scalar("sans-serif".to_string()),
            font_size: ScalarOrArray::new_scalar(10.0),
            font_weight: ScalarOrArray::new_scalar(FontWeight::NORMAL),
            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
            width: ScalarOrArray::new_scalar(LabelWidth::Auto),
            wrap: true,
            max_lines: None,
            ellipsis: false,
            hanging_signs: false,
            line_height: ScalarOrArray::new_scalar(LabelLineHeight::Auto),
            line_align: ScalarOrArray::new_scalar(LabelAlign::Start),
            indices: None,
            zindex: None,
            opacity: ScalarOrArray::new_scalar(1.0),
        }
    }
}

/// One label of a text mark: what it shows, and where it lies.
#[derive(Debug, Clone)]
pub struct TextLabel<'a> {
    pub label: Label<'a>,
    /// The position that `align` and `baseline` anchor, before rotation.
    pub position: [f32; 2],
    pub align: TextAlign,
    pub baseline: TextBaseline,
    /// The rotation about the position, in degrees.
    pub angle: f32,
}

impl From<SceneTextMark> for SceneMark {
    fn from(mark: SceneTextMark) -> Self {
        SceneMark::Text(Arc::new(mark))
    }
}

/// The label crate's text style for a font, size, weight, style and fill.
pub fn text_style(
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    color: [f32; 4],
) -> TextStyle {
    TextStyle {
        font_family: font.to_string(),
        font_size,
        fill: AbsoluteColor::from_rgba(color),
        font_weight: avenger_typst_label::FontWeight::from_number(font_weight.0),
        font_style: match font_style {
            FontStyle::Normal => avenger_typst_label::FontStyle::Normal,
            FontStyle::Italic => avenger_typst_label::FontStyle::Italic,
            FontStyle::Oblique => avenger_typst_label::FontStyle::Oblique,
        },
        ..Default::default()
    }
}

/// A label of a source that reads by a syntax: as literal text, whose newlines are spaces or
/// end lines, or as markup.
pub fn text_label(source: &str, syntax: TextSyntaxMode, options: LabelOptions) -> Label<'_> {
    let (source, newline_breaks) = match syntax {
        TextSyntaxMode::Plain => (LabelSource::Text(source), false),
        TextSyntaxMode::PlainLines => (LabelSource::Text(source), true),
        TextSyntaxMode::TypstMarkup => (LabelSource::Markup(source), false),
    };
    Label {
        source,
        options: LabelOptions {
            newline_breaks,
            ..options
        },
    }
}

/// The top left of a label's box, for a position that the alignment and baseline anchor.
///
/// Top, Middle and Bottom place the box, LineTop and LineBottom its line box, and Alphabetic
/// the first line's baseline.
pub fn text_origin(
    bounds: &TextBounds,
    position: [f32; 2],
    align: TextAlign,
    baseline: TextBaseline,
) -> [f32; 2] {
    let x = match align {
        TextAlign::Left => position[0],
        TextAlign::Center => position[0] - bounds.width / 2.0,
        TextAlign::Right => position[0] - bounds.width,
    };
    let y = match baseline {
        TextBaseline::Alphabetic => position[1] - bounds.ascent,
        TextBaseline::Top => position[1],
        TextBaseline::Middle => position[1] - bounds.height / 2.0,
        TextBaseline::Bottom => position[1] - bounds.height,
        TextBaseline::LineTop => position[1] + bounds.leading / 2.0,
        TextBaseline::LineBottom => position[1] - bounds.height - bounds.leading / 2.0,
    };
    [x, y]
}

/// A width the label crate accepts: one it would reject means none.
fn valid_width(width: LabelWidth) -> LabelWidth {
    match width {
        LabelWidth::Max(width) | LabelWidth::Fixed(width) if !valid(width) => LabelWidth::Auto,
        width => width,
    }
}

/// A line height the label crate accepts: one it would reject means none.
fn valid_line_height(line_height: LabelLineHeight) -> LabelLineHeight {
    match line_height {
        LabelLineHeight::Fixed(value) | LabelLineHeight::Relative(value) if !valid(value) => {
            LabelLineHeight::Auto
        }
        line_height => line_height,
    }
}

fn valid(distance: f32) -> bool {
    distance.is_finite() && distance >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_take_the_layout_and_syntax() {
        let mark = SceneTextMark {
            text: "a\nb".into(),
            text_syntax: TextSyntaxMode::PlainLines,
            width: LabelWidth::Fixed(80.0).into(),
            wrap: false,
            max_lines: NonZeroUsize::new(2),
            ellipsis: true,
            hanging_signs: true,
            line_height: LabelLineHeight::Relative(1.1).into(),
            line_align: LabelAlign::Center.into(),
            ..Default::default()
        };
        let label = mark.labels().next().unwrap().label;
        assert_eq!(label.source, LabelSource::Text("a\nb"));
        let options = &label.options;
        assert_eq!(
            (
                options.width,
                options.wrap,
                options.max_lines,
                options.ellipsis,
                options.hanging_signs
            ),
            (
                LabelWidth::Fixed(80.0),
                false,
                NonZeroUsize::new(2),
                true,
                true
            )
        );
        assert_eq!(
            (options.line_height, options.align, options.newline_breaks),
            (LabelLineHeight::Relative(1.1), LabelAlign::Center, true)
        );
        for (syntax, source, newline_breaks) in [
            (TextSyntaxMode::Plain, LabelSource::Text("x"), false),
            (TextSyntaxMode::TypstMarkup, LabelSource::Markup("x"), false),
        ] {
            let label = text_label("x", syntax, LabelOptions::default());
            assert_eq!(
                (label.source, label.options.newline_breaks),
                (source, newline_breaks)
            );
        }

        // Distances the label crate would reject mean none.
        let invalid = SceneTextMark {
            width: LabelWidth::Max(f32::NAN).into(),
            line_height: LabelLineHeight::Fixed(-1.0).into(),
            ..Default::default()
        };
        let options = invalid.labels().next().unwrap().label.options;
        assert_eq!(
            (options.width, options.line_height),
            (LabelWidth::Auto, LabelLineHeight::Auto)
        );
    }

    #[test]
    fn text_origins_place_the_box_line_box_or_first_baseline() {
        let bounds = TextBounds {
            width: 100.0,
            height: 20.0,
            ascent: 15.0,
            leading: 4.0,
        };
        let origin = |align, baseline| text_origin(&bounds, [10.0, 10.0], align, baseline);
        assert_eq!(origin(TextAlign::Left, TextBaseline::Top), [10.0, 10.0]);
        assert_eq!(
            origin(TextAlign::Center, TextBaseline::Middle),
            [-40.0, 0.0]
        );
        assert_eq!(
            origin(TextAlign::Right, TextBaseline::Bottom),
            [-90.0, -10.0]
        );
        assert_eq!(
            origin(TextAlign::Left, TextBaseline::Alphabetic),
            [10.0, -5.0]
        );
        // The line box has half the leading above the box and half below.
        assert_eq!(origin(TextAlign::Left, TextBaseline::LineTop), [10.0, 12.0]);
        assert_eq!(
            origin(TextAlign::Left, TextBaseline::LineBottom),
            [10.0, -12.0]
        );
    }
}
