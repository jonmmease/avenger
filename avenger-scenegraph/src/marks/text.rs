use std::sync::Arc;

use avenger_common::value::ScalarOrArray;
use avenger_geometry::GeometryInstance;
use avenger_text::{
    error::AvengerTextError,
    measurement::{TextMeasurementConfig, TextMeasurer},
    types::{FontStyleSpec, FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec},
};
use geo::{Geometry, Rotate};
use itertools::izip;
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTextMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub text: ScalarOrArray<String>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub align: ScalarOrArray<TextAlignSpec>,
    pub baseline: ScalarOrArray<TextBaselineSpec>,
    pub angle: ScalarOrArray<f32>,
    pub color: ScalarOrArray<[f32; 4]>,
    pub font: ScalarOrArray<String>,
    pub font_size: ScalarOrArray<f32>,
    pub font_weight: ScalarOrArray<FontWeightSpec>,
    pub font_style: ScalarOrArray<FontStyleSpec>,
    pub limit: ScalarOrArray<f32>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
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
    pub fn align_iter(&self) -> Box<dyn Iterator<Item = &TextAlignSpec> + '_> {
        self.align.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn baseline_iter(&self) -> Box<dyn Iterator<Item = &TextBaselineSpec> + '_> {
        self.baseline
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn color_iter(&self) -> Box<dyn Iterator<Item = &[f32; 4]> + '_> {
        self.color.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        self.font.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.font_size
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_weight_iter(&self) -> Box<dyn Iterator<Item = &FontWeightSpec> + '_> {
        self.font_weight
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn font_style_iter(&self) -> Box<dyn Iterator<Item = &FontStyleSpec> + '_> {
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
            Box::new((0..self.len as usize).into_iter())
        }
    }

    pub fn geometry_iter(
        &self,
        measurer: Arc<dyn TextMeasurer>,
        dimensions: &[f32; 2],
    ) -> Result<Box<dyn Iterator<Item = GeometryInstance> + '_>, AvengerTextError> {
        // Simple case where we don't need to build lyon paths first
        let dimensions = *dimensions;
        Ok(Box::new(
            izip!(
                self.indices_iter(),
                self.text_iter(),
                self.x_iter(),
                self.y_iter(),
                self.angle_iter(),
                self.font_iter(),
                self.font_size_iter(),
                self.font_weight_iter(),
                self.font_style_iter(),
                self.align_iter(),
                self.baseline_iter()
            )
            .map(
                move |(
                    id,
                    text,
                    x,
                    y,
                    angle,
                    font,
                    font_size,
                    font_weight,
                    font_style,
                    align,
                    baseline,
                )| {
                    let config = TextMeasurementConfig {
                        text: text,
                        font: font,
                        font_size: *font_size,
                        font_weight: font_weight,
                        font_style: font_style,
                    };

                    let bounds = measurer.measure_text_bounds(&config, &dimensions);
                    let origin = bounds.calculate_origin([*x, *y], align, baseline);

                    let text_rect = Geometry::Rect(geo::Rect::<f32>::new(
                        geo::Coord {
                            x: origin[0],
                            y: origin[1],
                        },
                        geo::Coord {
                            x: origin[0] + bounds.width,
                            y: origin[1] + bounds.height,
                        },
                    ));

                    // Rotate around x/y position
                    let geometry = text_rect
                        .rotate_around_point((*angle).to_radians(), geo::Point::new(*x, *y));

                    GeometryInstance {
                        id,
                        geometry,
                        half_stroke_width: 0.0,
                    }
                },
            ),
        ))
    }
}

impl Default for SceneTextMark {
    fn default() -> Self {
        Self {
            name: "text_mark".to_string(),
            clip: true,
            len: 1,
            text: ScalarOrArray::Scalar(String::new()),
            x: ScalarOrArray::Scalar(0.0),
            y: ScalarOrArray::Scalar(0.0),
            align: ScalarOrArray::Scalar(TextAlignSpec::Left),
            baseline: ScalarOrArray::Scalar(TextBaselineSpec::Alphabetic),
            angle: ScalarOrArray::Scalar(0.0),
            color: ScalarOrArray::Scalar([0.0, 0.0, 0.0, 1.0]),
            font: ScalarOrArray::Scalar("sans serif".to_string()),
            font_size: ScalarOrArray::Scalar(10.0),
            font_weight: ScalarOrArray::Scalar(FontWeightSpec::Name(FontWeightNameSpec::Normal)),
            font_style: ScalarOrArray::Scalar(FontStyleSpec::Normal),
            limit: ScalarOrArray::Scalar(0.0),
            indices: None,
            zindex: None,
        }
    }
}

impl From<SceneTextMark> for SceneMark {
    fn from(mark: SceneTextMark) -> Self {
        SceneMark::Text(Box::new(mark))
    }
}
