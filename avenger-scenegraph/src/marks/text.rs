use std::{collections::HashMap, hash::Hash, sync::Arc};

use avenger_common::{canvas::CanvasDimensions, value::ScalarOrArray};
use avenger_geometry::{lyon_to_geo::IntoGeoType, GeometryInstance};
use avenger_text::{
    error::AvengerTextError,
    measurement::TextMeasurementConfig,
    rasterization::{TextRasterizationConfig, TextRasterizer},
    types::{FontStyleSpec, FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec},
};
use geo::{BooleanOps, BoundingRect, Geometry, MultiPolygon, Rotate, Translate};
use itertools::izip;
use lyon_path::{
    geom::{Box2D, Point},
    traits::PathBuilder,
};
use serde::{Deserialize, Serialize};

use super::{mark::SceneMark, path::PathTransform};

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

    pub fn geometry_iter<CacheKey, CacheValue>(
        &self,
        mark_index: usize,
        rasterizer: Arc<dyn TextRasterizer<CacheKey = CacheKey, CacheValue = CacheValue>>,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_>
    where
        CacheKey: Hash + Eq + Clone + 'static,
        CacheValue: Clone + 'static,
    {
        // For measurement, use scale 1.0 and a large canvas.
        let dimensions = CanvasDimensions {
            size: [1024.0, 512.0],
            scale: 1.0,
        };
        Box::new(
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
            .enumerate()
            .map(
                move |(
                    z_index,
                    (
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
                    ),
                )| {
                    let config = TextRasterizationConfig {
                        text: text,
                        font: font,
                        font_size: *font_size,
                        font_weight: font_weight,
                        font_style: font_style,
                        color: &[0.0, 0.0, 0.0, 1.0],
                        limit: 0.0,
                    };

                    let text_buffer = rasterizer
                        .rasterize(&config, 1.0, &Default::default())
                        .unwrap();

                    let origin =
                        text_buffer
                            .text_bounds
                            .calculate_origin([*x, *y], align, baseline);

                    // Build up the text polygon by unioning the glyph bounding boxes
                    let mut text_poly = geo::MultiPolygon::<f32>::new(vec![]);

                    for (glyph_data, phys_pos) in text_buffer.glyphs {
                        let glyph_bbox = glyph_data.bbox;

                        // let path = glyph_data.path;
                        let path: Option<lyon_path::Path> = None;
                        let glyph_bbox_poly = if let Some(path) = path {
                            // We have vector path info, so we can use it to build a polygon
                            match path.as_geo_type(0.0, true) {
                                geo::Geometry::Polygon(poly) => geo::MultiPolygon::new(vec![poly]),
                                geo::Geometry::MultiPolygon(mpoly) => mpoly,
                                g => panic!("Expected polygon or multipolygon: {:?}", g),
                            }
                        } else {
                            // No vector path info, so we use the bounding box of the glyph image to build a polygon
                            geo::MultiPolygon::new(vec![geo::Polygon::new(
                                geo::LineString::new(vec![
                                    geo::Coord {
                                        x: glyph_bbox.left as f32,
                                        y: -glyph_bbox.top as f32,
                                    },
                                    geo::Coord {
                                        x: glyph_bbox.left as f32 + glyph_bbox.width as f32,
                                        y: -glyph_bbox.top as f32,
                                    },
                                    geo::Coord {
                                        x: glyph_bbox.left as f32 + glyph_bbox.width as f32,
                                        y: -glyph_bbox.top as f32 + glyph_bbox.height as f32,
                                    },
                                    geo::Coord {
                                        x: glyph_bbox.left as f32,
                                        y: -glyph_bbox.top as f32 + glyph_bbox.height as f32,
                                    },
                                    geo::Coord {
                                        x: glyph_bbox.left as f32,
                                        y: -glyph_bbox.top as f32,
                                    },
                                ]),
                                vec![],
                            )])
                        }
                        .translate(
                            phys_pos.x + origin[0],
                            phys_pos.y + origin[1] + text_buffer.text_bounds.height,
                        );

                        text_poly = text_poly.union(&glyph_bbox_poly);
                    }

                    let geometry = Geometry::MultiPolygon(text_poly)
                        .rotate_around_point(*angle, geo::Point::new(*x, *y));

                    GeometryInstance {
                        mark_index,
                        instance_index: Some(id),
                        z_index,
                        geometry,
                        half_stroke_width: 0.0,
                    }
                },
            ),
        )
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
