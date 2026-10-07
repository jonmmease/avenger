use std::{
    hash::{Hash, Hasher},
    num::NonZeroUsize,
    sync::Arc,
};

use super::mark::SceneMark;
use avenger_color::ColorOrGradient;

use avenger_common::value::ScalarOrArray;
use avenger_text::types::{
    FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline, TextConfig, TextLayout,
    TextSyntaxMode,
};
use avenger_text::{
    DateTimeFormatProvider, LabelAlign, LabelLineHeight, LabelWidth, NumberFormatProvider,
};
use itertools::izip;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub text: ScalarOrArray<String>,
    /// How the text reads: as plain text, as plain text whose newlines end lines, or as Typst
    /// markup.
    #[serde(default)]
    pub text_syntax: TextSyntaxMode,
    #[serde(default, skip_serializing_if = "text_params_is_empty")]
    pub text_params: avenger_text::LabelParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<avenger_format_config::NumberFormatConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime_format: Option<avenger_format_config::DateTimeFormatConfig>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    /// Where the label's position lies across its box. Its lines align within the box by
    /// `line_align`.
    pub align: ScalarOrArray<TextAlign>,
    /// Where the label's position lies down it: on the top, middle or bottom of its box, which
    /// pads its lines to at least the font size, on the top or bottom of its line box, which
    /// adds half the gap between lines, or on its first line's baseline.
    pub baseline: ScalarOrArray<TextBaseline>,
    pub angle: ScalarOrArray<f32>,
    pub color: ScalarOrArray<ColorOrGradient>,
    pub font: ScalarOrArray<String>,
    pub font_size: ScalarOrArray<f32>,
    pub font_weight: ScalarOrArray<FontWeight>,
    pub font_style: ScalarOrArray<FontStyle>,
    /// How wide each label is. Its lines wrap at the width or, without `wrap`, end only at
    /// explicit breaks.
    pub width: ScalarOrArray<LabelWidth>,
    /// Whether lines wrap at the width.
    pub wrap: bool,
    /// The most lines each label keeps, or all of them.
    pub max_lines: Option<NonZeroUsize>,
    /// Whether "…" marks text that the width or `max_lines` cuts. Without it, a line wider than
    /// the width overflows the label's box.
    pub ellipsis: bool,
    /// The distance between each label's baselines.
    pub line_height: ScalarOrArray<LabelLineHeight>,
    /// How each label's lines align within its box.
    pub line_align: ScalarOrArray<LabelAlign>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl Hash for SceneTextMark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.text.hash(state);
        self.text_syntax.hash(state);
        for (name, value) in &self.text_params {
            name.hash(state);
            value.hash(state);
        }
        self.number_format.hash(state);
        self.datetime_format.hash(state);
        self.x.hash(state);
        self.y.hash(state);
        self.align.hash(state);
        self.baseline.hash(state);
        self.angle.hash(state);
        self.color.hash(state);
        self.font.hash(state);
        self.font_size.hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        self.width.hash(state);
        self.wrap.hash(state);
        self.max_lines.hash(state);
        self.ellipsis.hash(state);
        self.line_height.hash(state);
        self.line_align.hash(state);
        self.indices.hash(state);
        self.zindex.hash(state);
    }
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
    /// Each label's layout of its lines.
    pub fn layout_iter(&self) -> Box<dyn Iterator<Item = TextLayout> + '_> {
        Box::new(
            self.width_iter()
                .zip(self.line_height_iter())
                .zip(self.line_align_iter())
                .map(|((width, line_height), align)| TextLayout {
                    width: *width,
                    wrap: self.wrap,
                    max_lines: self.max_lines,
                    ellipsis: self.ellipsis,
                    line_height: *line_height,
                    align: *align,
                }),
        )
    }

    /// The mark's formatting providers, for `labels`.
    pub fn formatters(&self) -> TextFormatters {
        TextFormatters {
            number: self.number_format.as_ref().map(|config| config.provider()),
            datetime: self
                .datetime_format
                .as_ref()
                .map(|config| config.provider()),
        }
    }

    /// The mark's labels, formatted with its providers.
    pub fn labels<'a>(
        &'a self,
        formatters: &'a TextFormatters,
    ) -> impl Iterator<Item = TextLabel<'a>> + 'a {
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
            self.layout_iter(),
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
                layout,
            )| TextLabel {
                config: TextConfig {
                    text,
                    syntax_mode: self.text_syntax,
                    font,
                    font_size: *font_size,
                    font_weight: *font_weight,
                    font_style: *font_style,
                    color: color.color_or_transparent(),
                    layout,
                    params: &self.text_params,
                    number_format: formatters.number.as_ref(),
                    datetime_format: formatters.datetime.as_ref(),
                },
                position: [*x, *y],
                align: *align,
                baseline: *baseline,
                angle: *angle,
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
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: ScalarOrArray::new_scalar(String::new()),
            text_syntax: TextSyntaxMode::Plain,
            text_params: avenger_text::LabelParams::default(),
            number_format: Default::default(),
            datetime_format: None,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(TextAlign::Left),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Alphabetic),
            angle: ScalarOrArray::new_scalar(0.0),
            color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            font: ScalarOrArray::new_scalar("sans serif".to_string()),
            font_size: ScalarOrArray::new_scalar(10.0),
            font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Normal)),
            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
            width: ScalarOrArray::new_scalar(LabelWidth::Auto),
            wrap: true,
            max_lines: None,
            ellipsis: false,
            line_height: ScalarOrArray::new_scalar(LabelLineHeight::Auto),
            line_align: ScalarOrArray::new_scalar(LabelAlign::Start),
            indices: None,
            zindex: None,
        }
    }
}

/// A text mark's formatting providers.
#[derive(Debug, Clone, Default)]
pub struct TextFormatters {
    pub number: Option<Arc<dyn NumberFormatProvider>>,
    pub datetime: Option<Arc<dyn DateTimeFormatProvider>>,
}

/// One label of a text mark: what it shows, and where it lies.
#[derive(Debug, Clone)]
pub struct TextLabel<'a> {
    pub config: TextConfig<'a>,
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

fn text_params_is_empty(params: &avenger_text::LabelParams) -> bool {
    params.is_empty()
}
