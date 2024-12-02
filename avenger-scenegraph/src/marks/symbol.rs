use crate::error::AvengerError;
use crate::marks::value::{ColorOrGradient, EncodingValue, Gradient};
use lyon_extra::parser::{ParserOptions, Source};
use lyon_path::geom::euclid::Point2D;
use lyon_path::geom::{Box2D, Point, Scale};
use lyon_path::{Path, Winding};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneSymbolMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub shapes: Vec<SymbolShape>,
    pub stroke_width: Option<f32>,
    pub shape_index: EncodingValue<usize>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub fill: EncodingValue<ColorOrGradient>,
    pub size: EncodingValue<f32>,
    pub stroke: EncodingValue<ColorOrGradient>,
    pub angle: EncodingValue<f32>,
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

    pub fn max_size(&self) -> f32 {
        match &self.size {
            EncodingValue::Scalar { value: size } => *size,
            EncodingValue::Array { values } => *values
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
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            shape_index: EncodingValue::Scalar { value: 0 },
            fill: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            size: EncodingValue::Scalar { value: 20.0 },
            stroke: EncodingValue::Scalar {
                value: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            },
            angle: EncodingValue::Scalar { value: 0.0 },
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
    pub fn from_vega_str(shape: &str) -> Result<SymbolShape, AvengerError> {
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

pub fn parse_svg_path(path: &str) -> Result<Path, AvengerError> {
    let mut source = Source::new(path.chars());
    let mut parser = lyon_extra::parser::PathParser::new();
    let opts = ParserOptions::DEFAULT;
    let mut builder = lyon_path::Path::builder();
    parser.parse(&opts, &mut source, &mut builder)?;
    Ok(builder.build())
}
