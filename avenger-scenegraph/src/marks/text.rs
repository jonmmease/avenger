use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use super::mark::SceneMark;
use avenger_color::ColorOrGradient;

use avenger_common::value::ScalarOrArray;
use avenger_text::types::{
    FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline, TextSyntaxMode,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    pub name: String,
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
    pub align: ScalarOrArray<TextAlign>,
    pub baseline: ScalarOrArray<TextBaseline>,
    pub angle: ScalarOrArray<f32>,
    pub color: ScalarOrArray<ColorOrGradient>,
    pub font: ScalarOrArray<String>,
    pub font_size: ScalarOrArray<f32>,
    pub font_weight: ScalarOrArray<FontWeight>,
    pub font_style: ScalarOrArray<FontStyle>,
    pub limit: ScalarOrArray<f32>,
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
        avenger_text::label_params_fingerprint(&self.text_params).hash(state);
        self.number_locale.hash(state);
        avenger_text::number_locale_specs_fingerprint(&self.number_locale_specs).hash(state);
        self.datetime_locale.hash(state);
        self.datetime_timezone.hash(state);
        avenger_text::datetime_locale_specs_fingerprint(&self.datetime_locale_specs).hash(state);
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
        self.limit.hash(state);
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
    pub fn limit_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.limit.as_iter(self.len as usize, self.indices.as_ref())
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
            number_locale: None,
            number_locale_specs: avenger_text::NumberLocaleSpecs::default(),
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: avenger_text::DateTimeLocaleSpecs::default(),
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
            limit: ScalarOrArray::new_scalar(0.0),
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

fn text_params_is_empty(params: &avenger_text::LabelParams) -> bool {
    params.is_empty()
}

fn number_locale_specs_is_empty(specs: &avenger_text::NumberLocaleSpecs) -> bool {
    specs.is_empty()
}

fn datetime_locale_specs_is_empty(specs: &avenger_text::DateTimeLocaleSpecs) -> bool {
    specs.is_empty()
}
