use crate::marks::mark::SceneMark;
use crate::marks::path::{PathMark, PathTransform};
use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use lyon_path::geom::euclid::Point2D;
use lyon_path::geom::Box2D;
use lyon_path::Winding;
use serde::{Deserialize, Serialize};

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
    pub fn make_path_mark(&self) -> Option<PathMark> {
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

        Some(PathMark {
            name: format!("path_{}", self.name),
            clip: false,
            len: 1,
            gradients: self.gradients.clone(),
            path: EncodingValue::Scalar { value: path },
            fill: EncodingValue::Scalar {
                value: self
                    .fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            },
            stroke: EncodingValue::Scalar {
                value: self
                    .stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            },
            stroke_width: Some(stroke_width),
            stroke_cap: Default::default(),
            stroke_join: Default::default(),
            transform: EncodingValue::Scalar {
                value: PathTransform::identity(),
            },
            indices: None,
            zindex: self.zindex,
        })
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
