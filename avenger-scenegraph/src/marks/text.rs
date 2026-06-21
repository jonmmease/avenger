use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{SceneTextLeaderArrow, SceneTextLeaderShape, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use serde::{Deserialize, Serialize};

use super::mark::{default_interactive, SceneMark};

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    pub name: String,
    #[serde(default = "default_interactive")]
    pub interactive: bool,
    pub clip: bool,
    pub len: u32,
    pub text: ScalarOrArray<String>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    #[serde(default = "default_zero_f32_channel")]
    pub dx: ScalarOrArray<f32>,
    #[serde(default = "default_zero_f32_channel")]
    pub dy: ScalarOrArray<f32>,
    pub align: ScalarOrArray<TextAlign>,
    pub baseline: ScalarOrArray<TextBaseline>,
    pub angle: ScalarOrArray<f32>,
    pub color: ScalarOrArray<ColorOrGradient>,
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
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            dx: ScalarOrArray::new_scalar(0.0),
            dy: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(TextAlign::Left),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Alphabetic),
            angle: ScalarOrArray::new_scalar(0.0),
            color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
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
