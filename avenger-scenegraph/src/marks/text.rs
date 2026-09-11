use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{SceneTextLeaderArrow, SceneTextLeaderShape, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_text::types::{
    FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline, TextSyntaxMode,
};
use serde::{Deserialize, Serialize};

use super::mark::{default_interactive, SceneMark};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    pub name: String,
    #[serde(default = "default_interactive")]
    pub interactive: bool,
    pub clip: bool,
    pub len: u32,
    pub text: ScalarOrArray<String>,
    #[serde(default)]
    pub text_syntax: TextSyntaxMode,
    #[serde(default, skip_serializing_if = "text_params_is_empty")]
    pub text_params: avenger_text::LabelParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_locale: Option<String>,
    #[serde(default, skip_serializing_if = "number_locale_specs_is_empty")]
    pub number_locale_specs: avenger_text::NumberLocaleSpecs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime_timezone: Option<String>,
    #[serde(default, skip_serializing_if = "datetime_locale_specs_is_empty")]
    pub datetime_locale_specs: avenger_text::DateTimeLocaleSpecs,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    #[serde(default = "default_true_bool_channel")]
    pub defined: ScalarOrArray<bool>,
    #[serde(default = "default_zero_f32_channel")]
    pub dx: ScalarOrArray<f32>,
    #[serde(default = "default_zero_f32_channel")]
    pub dy: ScalarOrArray<f32>,
    pub align: ScalarOrArray<TextAlign>,
    pub baseline: ScalarOrArray<TextBaseline>,
    pub angle: ScalarOrArray<f32>,
    pub color: ScalarOrArray<ColorOrGradient>,
    /// Logical text opacity retained for chart adjustment pipelines. Renderers
    /// expect text opacity to already be baked into `color`.
    #[serde(default = "default_one_f32_channel")]
    pub opacity: ScalarOrArray<f32>,
    pub font: ScalarOrArray<String>,
    pub font_size: ScalarOrArray<f32>,
    pub font_weight: ScalarOrArray<FontWeight>,
    pub font_style: ScalarOrArray<FontStyle>,
    pub limit: ScalarOrArray<f32>,
    #[serde(default = "default_false_bool_channel")]
    pub leader: ScalarOrArray<bool>,
    #[serde(default = "default_leader_stroke_channel")]
    pub leader_stroke: ScalarOrArray<ColorOrGradient>,
    #[serde(default = "default_one_f32_channel")]
    pub leader_stroke_width: ScalarOrArray<f32>,
    #[serde(default = "default_round_stroke_cap_channel")]
    pub leader_stroke_cap: ScalarOrArray<StrokeCap>,
    #[serde(default = "default_round_stroke_join_channel")]
    pub leader_stroke_join: ScalarOrArray<StrokeJoin>,
    #[serde(default)]
    pub leader_stroke_dash: Option<ScalarOrArray<Vec<f32>>>,
    #[serde(default = "default_two_f32_channel")]
    pub leader_label_padding: ScalarOrArray<f32>,
    #[serde(default = "default_zero_f32_channel")]
    pub leader_target_radius: ScalarOrArray<f32>,
    #[serde(default = "default_one_f32_channel")]
    pub leader_min_length: ScalarOrArray<f32>,
    #[serde(default = "default_text_leader_shape_channel")]
    pub leader_shape: ScalarOrArray<SceneTextLeaderShape>,
    #[serde(default = "default_text_leader_arrow_channel")]
    pub leader_arrow: ScalarOrArray<SceneTextLeaderArrow>,
    #[serde(default = "default_six_f32_channel")]
    pub leader_arrow_length: ScalarOrArray<f32>,
    #[serde(default = "default_five_f32_channel")]
    pub leader_arrow_width: ScalarOrArray<f32>,
    pub indices: Option<Arc<Vec<usize>>>,
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
        avenger_text::label_params_fingerprint(&self.text_params).hash(state);
        self.number_locale.hash(state);
        avenger_text::number_locale_specs_fingerprint(&self.number_locale_specs).hash(state);
        self.datetime_locale.hash(state);
        self.datetime_timezone.hash(state);
        avenger_text::datetime_locale_specs_fingerprint(&self.datetime_locale_specs).hash(state);
        self.x.hash(state);
        self.y.hash(state);
        self.defined.hash(state);
        self.dx.hash(state);
        self.dy.hash(state);
        self.align.hash(state);
        self.baseline.hash(state);
        self.angle.hash(state);
        self.color.hash(state);
        self.opacity.hash(state);
        self.font.hash(state);
        self.font_size.hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        self.limit.hash(state);
        self.leader.hash(state);
        self.leader_stroke.hash(state);
        self.leader_stroke_width.hash(state);
        self.leader_stroke_cap.hash(state);
        self.leader_stroke_join.hash(state);
        self.leader_stroke_dash.hash(state);
        self.leader_label_padding.hash(state);
        self.leader_target_radius.hash(state);
        self.leader_min_length.hash(state);
        self.leader_shape.hash(state);
        self.leader_arrow.hash(state);
        self.leader_arrow_length.hash(state);
        self.leader_arrow_width.hash(state);
        self.indices.hash(state);
        self.zindex.hash(state);
    }
}

fn text_params_is_empty(params: &avenger_text::LabelParams) -> bool {
    params.is_empty()
}

fn number_locale_specs_is_empty(specs: &avenger_text::NumberLocaleSpecs) -> bool {
    specs.is_empty()
}

fn datetime_locale_specs_is_empty(specs: &avenger_text::DateTimeLocaleSpecs) -> bool {
    specs.is_empty()
}

fn default_zero_f32_channel() -> ScalarOrArray<f32> {
    ScalarOrArray::new_scalar(0.0)
}

fn default_one_f32_channel() -> ScalarOrArray<f32> {
    ScalarOrArray::new_scalar(1.0)
}

fn default_two_f32_channel() -> ScalarOrArray<f32> {
    ScalarOrArray::new_scalar(2.0)
}

fn default_five_f32_channel() -> ScalarOrArray<f32> {
    ScalarOrArray::new_scalar(5.0)
}

fn default_six_f32_channel() -> ScalarOrArray<f32> {
    ScalarOrArray::new_scalar(6.0)
}

fn default_false_bool_channel() -> ScalarOrArray<bool> {
    ScalarOrArray::new_scalar(false)
}

fn default_true_bool_channel() -> ScalarOrArray<bool> {
    ScalarOrArray::new_scalar(true)
}

fn default_leader_stroke_channel() -> ScalarOrArray<ColorOrGradient> {
    ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.7]))
}

fn default_round_stroke_cap_channel() -> ScalarOrArray<StrokeCap> {
    ScalarOrArray::new_scalar(StrokeCap::Round)
}

fn default_round_stroke_join_channel() -> ScalarOrArray<StrokeJoin> {
    ScalarOrArray::new_scalar(StrokeJoin::Round)
}

fn default_text_leader_shape_channel() -> ScalarOrArray<SceneTextLeaderShape> {
    ScalarOrArray::new_scalar(SceneTextLeaderShape::Straight)
}

fn default_text_leader_arrow_channel() -> ScalarOrArray<SceneTextLeaderArrow> {
    ScalarOrArray::new_scalar(SceneTextLeaderArrow::None)
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
    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn dx_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.dx.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn dy_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.dy.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn target_position_iter(&self) -> Box<dyn Iterator<Item = [f32; 2]> + '_> {
        Box::new(self.x_iter().zip(self.y_iter()).map(|(x, y)| [*x, *y]))
    }
    pub fn label_position_iter(&self) -> Box<dyn Iterator<Item = [f32; 2]> + '_> {
        Box::new(
            self.x_iter()
                .zip(self.y_iter())
                .zip(self.dx_iter())
                .zip(self.dy_iter())
                .map(|(((x, y), dx), dy)| [*x + *dx, *y + *dy]),
        )
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
    pub fn limit_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.limit.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.leader
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.leader_stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.leader_stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_stroke_cap_iter(&self) -> Box<dyn Iterator<Item = &StrokeCap> + '_> {
        self.leader_stroke_cap
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_stroke_join_iter(&self) -> Box<dyn Iterator<Item = &StrokeJoin> + '_> {
        self.leader_stroke_join
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_stroke_dash_iter(&self) -> Option<Box<dyn Iterator<Item = &Vec<f32>> + '_>> {
        self.leader_stroke_dash
            .as_ref()
            .map(|dash| dash.as_iter(self.len as usize, self.indices.as_ref()))
    }
    pub fn leader_label_padding_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.leader_label_padding
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_target_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.leader_target_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_min_length_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.leader_min_length
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_shape_iter(&self) -> Box<dyn Iterator<Item = &SceneTextLeaderShape> + '_> {
        self.leader_shape
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_arrow_iter(&self) -> Box<dyn Iterator<Item = &SceneTextLeaderArrow> + '_> {
        self.leader_arrow
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_arrow_length_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.leader_arrow_length
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn leader_arrow_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.leader_arrow_width
            .as_iter(self.len as usize, self.indices.as_ref())
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
            interactive: true,
            clip: true,
            len: 1,
            text: ScalarOrArray::new_scalar(String::new()),
            text_syntax: TextSyntaxMode::Plain,
            text_params: avenger_text::LabelParams::default(),
            number_locale: None,
            number_locale_specs: avenger_text::NumberLocaleSpecs::default(),
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: avenger_text::DateTimeLocaleSpecs::default(),
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            defined: ScalarOrArray::new_scalar(true),
            dx: ScalarOrArray::new_scalar(0.0),
            dy: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(TextAlign::Left),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Alphabetic),
            angle: ScalarOrArray::new_scalar(0.0),
            color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            opacity: ScalarOrArray::new_scalar(1.0),
            font: ScalarOrArray::new_scalar("sans-serif".to_string()),
            font_size: ScalarOrArray::new_scalar(10.0),
            font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Normal)),
            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
            limit: ScalarOrArray::new_scalar(0.0),
            leader: ScalarOrArray::new_scalar(false),
            leader_stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.7])),
            leader_stroke_width: ScalarOrArray::new_scalar(1.0),
            leader_stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Round),
            leader_stroke_join: ScalarOrArray::new_scalar(StrokeJoin::Round),
            leader_stroke_dash: None,
            leader_label_padding: ScalarOrArray::new_scalar(2.0),
            leader_target_radius: ScalarOrArray::new_scalar(0.0),
            leader_min_length: ScalarOrArray::new_scalar(1.0),
            leader_shape: ScalarOrArray::new_scalar(SceneTextLeaderShape::Straight),
            leader_arrow: ScalarOrArray::new_scalar(SceneTextLeaderArrow::None),
            leader_arrow_length: ScalarOrArray::new_scalar(6.0),
            leader_arrow_width: ScalarOrArray::new_scalar(5.0),
            indices: None,
            zindex: None,
        }
    }
}

impl From<SceneTextMark> for SceneMark {
    fn from(mark: SceneTextMark) -> Self {
        SceneMark::Text(Arc::new(mark))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_syntax_defaults_to_plain() {
        assert_eq!(SceneTextMark::default().text_syntax, TextSyntaxMode::Plain);
    }

    #[test]
    fn text_syntax_deserializes_missing_field_as_plain() {
        let mut value = serde_json::to_value(SceneTextMark::default()).unwrap();
        value.as_object_mut().unwrap().remove("text-syntax");

        let mark: SceneTextMark = serde_json::from_value(value).unwrap();
        assert_eq!(mark.text_syntax, TextSyntaxMode::Plain);
    }

    #[test]
    fn text_syntax_serializes_typst_markup_as_kebab_case() {
        let mark = SceneTextMark {
            text_syntax: TextSyntaxMode::TypstMarkup,
            ..Default::default()
        };

        let value = serde_json::to_value(mark).unwrap();
        assert_eq!(value["text-syntax"], "typst-markup");
    }

    #[test]
    fn number_locale_defaults_and_serializes_as_kebab_case() {
        let default_value = serde_json::to_value(SceneTextMark::default()).unwrap();
        assert!(default_value.get("number-locale").is_none());
        assert!(default_value.get("number-locale-specs").is_none());

        let mut mark = SceneTextMark {
            number_locale: Some("de-DE".to_string()),
            ..Default::default()
        };
        mark.number_locale_specs.insert(
            "custom".to_string(),
            avenger_text::NumberLocaleSpec {
                decimal: Some("~".to_string()),
                group: Some("_".to_string()),
                ..Default::default()
            },
        );

        let value = serde_json::to_value(mark).unwrap();
        assert_eq!(value["number-locale"], "de-DE");
        assert_eq!(value["number-locale-specs"]["custom"]["decimal"], "~");
        assert_eq!(value["number-locale-specs"]["custom"]["group"], "_");
    }
}
