use super::mark::SceneMark;
use crate::error::AvengerSceneGraphError;
use avenger_common::types::{ColorOrGradient, Gradient, LinearScaleAdjustment, PathTransform};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use itertools::izip;
use lyon_extra::euclid::{UnknownUnit, Vector2D};
use lyon_extra::parser::{ParserOptions, Source};
use lyon_path::geom::euclid::Point2D;
use lyon_path::geom::{Angle, Box2D, Point, Scale};
use lyon_path::PathEvent;
use lyon_path::{Path, Winding};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneSymbolMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub shapes: Vec<SymbolShape>,
    pub stroke_width: Option<f32>,
    pub shape_index: ScalarOrArray<usize>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub size: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub angle: ScalarOrArray<f32>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
    pub x_adjustment: Option<LinearScaleAdjustment>,
    pub y_adjustment: Option<LinearScaleAdjustment>,
}

impl SceneSymbolMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(adjustment) = self.x_adjustment {
            let scale = adjustment.scale;
            let offset = adjustment.offset;
            Box::new(
                self.x
                    .as_iter(self.len as usize, self.indices.as_ref())
                    .map(move |x| scale * x + offset),
            )
        } else {
            self.x
                .as_iter_owned(self.len as usize, self.indices.as_ref())
        }
    }

    pub fn x_vec(&self) -> Vec<f32> {
        self.x_iter().collect()
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(adjustment) = self.y_adjustment {
            let scale = adjustment.scale;
            let offset = adjustment.offset;
            Box::new(
                self.y
                    .as_iter(self.len as usize, self.indices.as_ref())
                    .map(move |y| scale * y + offset),
            )
        } else {
            self.y
                .as_iter_owned(self.len as usize, self.indices.as_ref())
        }
    }

    pub fn y_vec(&self) -> Vec<f32> {
        self.y_iter().collect()
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_vec(&self) -> Vec<ColorOrGradient> {
        self.fill.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.size.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn size_vec(&self) -> Vec<f32> {
        self.size.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_vec(&self) -> Vec<ColorOrGradient> {
        self.stroke.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.angle.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn angle_vec(&self) -> Vec<f32> {
        self.angle.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn shape_index_iter(&self) -> Box<dyn Iterator<Item = &usize> + '_> {
        self.shape_index
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn shape_index_vec(&self) -> Vec<usize> {
        self.shape_index
            .as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new((0..self.len as usize))
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        let paths = self.shapes.iter().map(|s| s.as_path()).collect::<Vec<_>>();
        Box::new(
            izip!(
                self.x_iter(),
                self.y_iter(),
                self.size_iter(),
                self.angle_iter(),
                self.shape_index_iter()
            )
            .map(move |(x, y, size, angle, shape_idx)| {
                let scale = size.sqrt();
                let angle = Angle::degrees(*angle);
                let transform = PathTransform::scale(scale, scale)
                    .then_rotate(angle)
                    .then_translate(Vector2D::new(x + origin[0], y + origin[1]));

                paths[*shape_idx].as_ref().clone().transformed(&transform)
            }),
        )
    }

    pub fn max_size(&self) -> f32 {
        match self.size.value() {
            ScalarOrArrayValue::Scalar(size) => *size,
            ScalarOrArrayValue::Array(values) => values
                .iter()
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .cloned()
                .unwrap_or(1.0),
        }
    }

    pub fn single_symbol_mark(&self, index: usize) -> SceneSymbolMark {
        let mut mark = self.clone();
        mark.len = 1;
        mark.indices = Some(Arc::new(vec![index]));
        mark
    }
}

impl Default for SceneSymbolMark {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            clip: true,
            shapes: vec![Default::default()],
            stroke_width: None,
            len: 1,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            shape_index: ScalarOrArray::new_scalar(0),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            size: ScalarOrArray::new_scalar(20.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            angle: ScalarOrArray::new_scalar(0.0),
            indices: None,
            gradients: vec![],
            zindex: None,
            x_adjustment: None,
            y_adjustment: None,
        }
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolShape {
    #[default]
    Circle,
    /// Path with origin top-left
    Path(lyon_path::Path),
}

impl Hash for SymbolShape {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            SymbolShape::Circle => state.write_u8(0),
            SymbolShape::Path(path) => hash_lyon_path(path, state),
        }
    }
}

impl SymbolShape {
    pub fn from_vega_str(shape: &str) -> Result<SymbolShape, AvengerSceneGraphError> {
        let tan30: f32 = (30.0 * std::f32::consts::PI / 180.0).tan();
        let sqrt3: f32 = 3.0f32.sqrt();

        // See https://github.com/vega/vega/blob/main/packages/vega-scenegraph/src/path/symbols.js
        Ok(match shape.to_ascii_lowercase().as_str() {
            "circle" => SymbolShape::Circle,
            "square" => {
                let mut builder = lyon_path::Path::builder();
                builder.add_rectangle(
                    &Box2D::new(Point2D::new(-0.5, -0.5), Point2D::new(0.5, 0.5)),
                    Winding::Negative,
                );
                let path = builder.build();
                SymbolShape::Path(path)
            }
            "cross" => {
                let r = 0.5;
                let s = r / 2.5;

                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(-r, -s));
                builder.line_to(Point::new(-r, s));
                builder.line_to(Point::new(-s, s));
                builder.line_to(Point::new(-s, r));
                builder.line_to(Point::new(s, r));
                builder.line_to(Point::new(s, s));
                builder.line_to(Point::new(r, s));
                builder.line_to(Point::new(r, -s));
                builder.line_to(Point::new(s, -s));
                builder.line_to(Point::new(s, -r));
                builder.line_to(Point::new(-s, -r));
                builder.line_to(Point::new(-s, -s));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "diamond" => {
                let r = 0.5;
                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(-r, 0.0));
                builder.line_to(Point::new(0.0, -r));
                builder.line_to(Point::new(r, 0.0));
                builder.line_to(Point::new(0.0, r));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "triangle-up" => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(0.0, -h));
                builder.line_to(Point::new(-r, h));
                builder.line_to(Point::new(r, h));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "triangle-down" => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(0.0, h));
                builder.line_to(Point::new(-r, -h));
                builder.line_to(Point::new(r, -h));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "triangle-right" => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(h, 0.0));
                builder.line_to(Point::new(-h, -r));
                builder.line_to(Point::new(-h, r));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "triangle-left" => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(-h, 0.0));
                builder.line_to(Point::new(h, -r));
                builder.line_to(Point::new(h, r));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "arrow" => {
                let r = 0.5;
                let s = r / 7.0;
                let t = r / 2.5;
                let v = r / 8.0;

                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(-s, r));
                builder.line_to(Point::new(s, r));
                builder.line_to(Point::new(s, -v));
                builder.line_to(Point::new(t, -v));
                builder.line_to(Point::new(0.0, -r));
                builder.line_to(Point::new(-t, -v));
                builder.line_to(Point::new(-s, -v));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "wedge" => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let o = h - r * tan30;
                let b = r / 4.0;

                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(0.0, -h - o));
                builder.line_to(Point::new(-b, h - o));
                builder.line_to(Point::new(b, h - o));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "triangle" => {
                let r = 0.5;
                let h = r * sqrt3 / 2.0;
                let o = h - r * tan30;
                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(0.0, -h - o));
                builder.line_to(Point::new(-r, h - o));
                builder.line_to(Point::new(r, h - o));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            _ => {
                // General SVG string
                let path = parse_svg_path(shape)?;

                // - Coordinates are divided by 2 to match Vega
                let path = path.transformed(&Scale::new(0.5));

                SymbolShape::Path(path)
            }
        })
    }

    pub fn as_path(&self) -> Cow<lyon_path::Path> {
        match self {
            SymbolShape::Circle => {
                let mut builder = lyon_path::Path::builder();
                builder.add_circle(lyon_path::geom::point(0.0, 0.0), 0.5, Winding::Positive);
                Cow::Owned(builder.build())
            }
            SymbolShape::Path(path) => Cow::Borrowed(path),
        }
    }
}

impl TryInto<SymbolShape> for &str {
    type Error = AvengerSceneGraphError;

    fn try_into(self) -> Result<SymbolShape, Self::Error> {
        SymbolShape::from_vega_str(self)
    }
}

pub fn parse_svg_path(path: &str) -> Result<Path, AvengerSceneGraphError> {
    let mut source = Source::new(path.chars());
    let mut parser = lyon_extra::parser::PathParser::new();
    let opts = ParserOptions::DEFAULT;
    let mut builder = lyon_path::Path::builder();
    parser.parse(&opts, &mut source, &mut builder)?;
    Ok(builder.build())
}

impl From<SceneSymbolMark> for SceneMark {
    fn from(mark: SceneSymbolMark) -> Self {
        SceneMark::Symbol(mark)
    }
}

pub fn hash_point<H: Hasher>(point: &Point2D<f32, UnknownUnit>, hasher: &mut H) {
    OrderedFloat::from(point.x).hash(hasher);
    OrderedFloat::from(point.y).hash(hasher);
}

pub fn hash_lyon_path<H: Hasher>(path: &Path, hasher: &mut H) {
    for evt in path.iter() {
        // hash enum variant
        let variant = std::mem::discriminant(&evt);
        variant.hash(hasher);

        // hash enum value
        match evt {
            PathEvent::Begin { at } => hash_point(&at, hasher),
            PathEvent::Line { from, to, .. } => {
                hash_point(&from, hasher);
                hash_point(&to, hasher);
            }
            PathEvent::End { last, first, close } => {
                hash_point(&last, hasher);
                hash_point(&first, hasher);
                close.hash(hasher);
            }
            PathEvent::Quadratic { from, ctrl, to, .. } => {
                hash_point(&from, hasher);
                hash_point(&ctrl, hasher);
                hash_point(&to, hasher);
            }
            PathEvent::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            } => {
                hash_point(&from, hasher);
                hash_point(&ctrl1, hasher);
                hash_point(&ctrl2, hasher);
                hash_point(&to, hasher);
            }
        }
    }
}
