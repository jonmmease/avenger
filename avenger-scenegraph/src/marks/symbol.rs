use crate::error::AvengerSceneGraphError;
use crate::marks::path::PathTransform;
use avenger_common::value::{ColorOrGradient, Gradient, ScalarOrArray};
use avenger_geometry::geo_types::Geometry;
use avenger_geometry::lyon_to_geo::IntoGeoType;
use avenger_geometry::GeometryInstance;
use geo::{Rotate as GeoRotate, Scale as GeoScale, Translate as GeoTranslate};
use itertools::izip;
use lyon_extra::euclid::Vector2D;
use lyon_extra::parser::{ParserOptions, Source};
use lyon_path::geom::euclid::Point2D;
use lyon_path::geom::{Angle, Box2D, Point, Scale};
use lyon_path::{Path, Winding};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use super::mark::SceneMark;

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
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
}

impl SceneSymbolMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn x_vec(&self) -> Vec<f32> {
        self.x.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn y_vec(&self) -> Vec<f32> {
        self.y.as_vec(self.len as usize, self.indices.as_ref())
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
            Box::new((0..self.len as usize).into_iter())
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
                    .then_translate(Vector2D::new(*x + origin[0], *y + origin[1]));

                paths[*shape_idx].as_ref().clone().transformed(&transform)
            }),
        )
    }

    pub fn geometry_iter(
        &self,
        mark_index: usize,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        let symbol_geometries: Vec<_> = self.shapes.iter().map(|symbol| symbol.as_geo()).collect();
        let half_stroke_width = self.stroke_width.unwrap_or(0.0) / 2.0;
        Box::new(
            izip!(
                self.indices_iter(),
                self.x_iter(),
                self.y_iter(),
                self.size_iter(),
                self.angle_iter(),
                self.shape_index_iter()
            )
            .map(move |(instance_idx, x, y, size, angle, shape_idx)| {
                let geometry = symbol_geometries[*shape_idx]
                    .clone()
                    .scale(size.sqrt())
                    .rotate_around_point(angle.to_radians(), geo::Point::new(0.0, 0.0))
                    .translate(*x, *y);

                GeometryInstance {
                    mark_index,
                    instance_idx: Some(instance_idx),
                    geometry,
                    half_stroke_width,
                }
            }),
        )
    }

    pub fn max_size(&self) -> f32 {
        match &self.size {
            ScalarOrArray::Scalar(size) => *size,
            ScalarOrArray::Array(values) => *values
                .iter()
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(&1.0),
        }
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
            x: ScalarOrArray::Scalar(0.0),
            y: ScalarOrArray::Scalar(0.0),
            shape_index: ScalarOrArray::Scalar(0),
            fill: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            size: ScalarOrArray::Scalar(20.0),
            stroke: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            angle: ScalarOrArray::Scalar(0.0),
            indices: None,
            gradients: vec![],
            zindex: None,
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

    pub fn as_geo(&self) -> Geometry<f32> {
        let path = self.as_path();
        path.as_geo_type(0.1, true)
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
