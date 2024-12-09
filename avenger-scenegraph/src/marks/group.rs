use crate::error::AvengerSceneGraphError;
use crate::marks::mark::SceneMark;
use crate::marks::path::{PathTransform, ScenePathMark};
use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::{ColorOrGradient, Gradient};
use avenger_common::value::ScalarOrArray;
use avenger_geometry::rtree::MarkRTree;
use avenger_geometry::GeometryInstance;
use avenger_text::rasterization::TextRasterizer;
use lyon_path::geom::euclid::Point2D;
use lyon_path::geom::Box2D;
use lyon_path::Winding;
use serde::{Deserialize, Serialize};
use std::hash::Hash;
use std::sync::Arc;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Clip {
    None,
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    Path(lyon_path::Path),
}

impl Default for Clip {
    fn default() -> Self {
        Self::None
    }
}

impl Clip {
    pub fn maybe_clip(&self, should_clip: bool) -> Self {
        if !should_clip {
            Self::None
        } else {
            self.clone()
        }
    }

    pub fn translate(&self, translate_x: f32, translate_y: f32) -> Self {
        match self {
            Clip::None => Clip::None,
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => Clip::Rect {
                x: *x + translate_x,
                y: *y + translate_y,
                width: *width,
                height: *height,
            },
            Clip::Path(path) => Clip::Path(
                path.clone()
                    .transformed(&PathTransform::translation(translate_x, translate_y)),
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneGroup {
    pub name: String,
    pub origin: [f32; 2],
    pub clip: Clip,
    pub marks: Vec<SceneMark>,
    pub gradients: Vec<Gradient>,
    pub fill: Option<ColorOrGradient>,
    pub stroke: Option<ColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub stroke_offset: Option<f32>,
    pub zindex: Option<i32>,
}

impl SceneGroup {
    pub fn make_path_mark(&self) -> Option<ScenePathMark> {
        if self.fill.is_none() && self.stroke.is_none() {
            return None;
        }
        let stroke_width =
            self.stroke_width
                .unwrap_or(if self.stroke.is_some() { 1.0 } else { 0.0 });
        let stroke_offset = if let Some(stroke_offset) = self.stroke_offset {
            stroke_offset
        } else {
            // From Vega's default stroke offset logic
            if self.stroke.is_some() && stroke_width > 0.5 && stroke_width < 1.5 {
                0.5 - (stroke_width - 1.0).abs()
            } else {
                0.0
            }
        };

        // Convert clip region to path
        let path = match &self.clip {
            Clip::None => return None,
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => {
                let mut builder = lyon_path::Path::builder();
                let x = self.origin[0] + *x + stroke_offset;
                let y = self.origin[1] + *y + stroke_offset;
                builder.add_rectangle(
                    &Box2D::new(Point2D::new(x, y), Point2D::new(x + width, y + height)),
                    Winding::Positive,
                );
                builder.build()
            }
            Clip::Path(path) => path.clone().transformed(&PathTransform::translation(
                self.origin[0] + stroke_offset,
                self.origin[1] + stroke_offset,
            )),
        };

        Some(ScenePathMark {
            name: format!("path_{}", self.name),
            clip: false,
            len: 1,
            gradients: self.gradients.clone(),
            path: ScalarOrArray::Scalar(path),
            fill: ScalarOrArray::Scalar(
                self.fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            ),
            stroke: ScalarOrArray::Scalar(
                self.stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            ),
            stroke_width: Some(stroke_width),
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            transform: ScalarOrArray::Scalar(PathTransform::identity()),
            indices: None,
            zindex: self.zindex,
        })
    }

    pub fn make_rtree<CacheKey, CacheValue>(
        &self,
        rasterizer: Arc<dyn TextRasterizer<CacheKey = CacheKey, CacheValue = CacheValue>>,
        dimensions: &CanvasDimensions,
    ) -> MarkRTree
    where
        CacheKey: Hash + Eq + Clone + 'static,
        CacheValue: Clone + 'static,
    {
        let mut geometry_instances: Vec<GeometryInstance> = vec![];
        for (mark_index, mark) in self.marks.iter().enumerate() {
            match mark {
                SceneMark::Arc(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Area(mark) => {
                    geometry_instances.push(mark.geometry(mark_index));
                }
                SceneMark::Path(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Symbol(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Line(mark) => {
                    geometry_instances.push(mark.geometry(mark_index));
                }
                SceneMark::Trail(mark) => {
                    geometry_instances.push(mark.geometry(mark_index));
                }
                SceneMark::Rect(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Rule(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Text(mark) => {
                    geometry_instances.extend(mark.geometry_iter(
                        mark_index,
                        rasterizer.clone(),
                        dimensions,
                    ));
                }
                SceneMark::Image(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Group(_scene_group) => {
                    // Consider whether to recurse into group marks
                }
            }
        }
        MarkRTree::new(geometry_instances)
    }
}

impl Default for SceneGroup {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            origin: [0.0, 0.0],
            clip: Default::default(),
            marks: vec![],
            gradients: vec![],
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        }
    }
}

impl From<SceneGroup> for SceneMark {
    fn from(mark: SceneGroup) -> Self {
        SceneMark::Group(mark)
    }
}
