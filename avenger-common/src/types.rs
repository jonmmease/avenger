use crate::{impl_hash_for_scalar_or_array, lyon::{hash_lyon_path, parse_svg_path}, value::{ScalarOrArray, ScalarOrArrayValue}};
use lyon_extra::{euclid::{Box2D, Point2D, Scale, Transform2D, UnknownUnit}, parser::ParseError};
use lyon_path::{geom::Point, Winding};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, hash::{DefaultHasher, Hash, Hasher}};
use strum::VariantNames;

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize, VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StrokeCap {
    #[default]
    Butt,
    Round,
    Square,
}
impl_hash_for_scalar_or_array!(StrokeCap);

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize, VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StrokeJoin {
    Bevel,
    #[default]
    Miter,
    Round,
}
impl_hash_for_scalar_or_array!(StrokeJoin);

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize, VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ImageAlign {
    #[default]
    Left,
    Center,
    Right,
}
impl_hash_for_scalar_or_array!(ImageAlign);

#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Serialize, Deserialize, VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ImageBaseline {
    #[default]
    Top,
    Middle,
    Bottom,
}
impl_hash_for_scalar_or_array!(ImageBaseline);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorOrGradient {
    Color([f32; 4]),
    GradientIndex(u32),
}
impl Hash for ColorOrGradient {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            ColorOrGradient::Color(c) => [
                OrderedFloat::from(c[0]),
                OrderedFloat::from(c[1]),
                OrderedFloat::from(c[2]),
                OrderedFloat::from(c[3]),
            ]
            .hash(state),
            ColorOrGradient::GradientIndex(i) => i.hash(state),
        }
    }
}
impl_hash_for_scalar_or_array!(ColorOrGradient);

impl ColorOrGradient {
    pub fn transparent() -> Self {
        ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])
    }
    pub fn color_or_transparent(&self) -> [f32; 4] {
        match self {
            ColorOrGradient::Color(c) => *c,
            _ => [0.0, 0.0, 0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Gradient {
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
}
impl_hash_for_scalar_or_array!(Gradient);

impl Gradient {
    pub fn stops(&self) -> &[GradientStop] {
        match self {
            Gradient::LinearGradient(grad) => grad.stops.as_slice(),
            Gradient::RadialGradient(grad) => grad.stops.as_slice(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stops: Vec<GradientStop>,
}
impl Hash for LinearGradient {
    fn hash<H: Hasher>(&self, state: &mut H) {
        [self.x0, self.y0, self.x1, self.y1]
            .iter()
            .for_each(|v| OrderedFloat::from(*v).hash(state));

        self.stops.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadialGradient {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub r0: f32,
    pub r1: f32,
    pub stops: Vec<GradientStop>,
}

impl Hash for RadialGradient {
    fn hash<H: Hasher>(&self, state: &mut H) {
        [self.x0, self.y0, self.x1, self.y1, self.r0, self.r1]
            .iter()
            .for_each(|v| OrderedFloat::from(*v).hash(state));

        self.stops.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub offset: f32,
    pub color: [f32; 4],
}

impl Hash for GradientStop {
    fn hash<H: Hasher>(&self, state: &mut H) {
        OrderedFloat::from(self.offset).hash(state);
        self.color
            .iter()
            .for_each(|v| OrderedFloat::from(*v).hash(state));
    }
}

#[derive(Default, Debug, Copy, Clone, Hash, PartialEq, Serialize, Deserialize, VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AreaOrientation {
    #[default]
    Vertical,
    Horizontal,
}
impl_hash_for_scalar_or_array!(AreaOrientation);

#[derive(Clone, Debug, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearScaleAdjustment {
    pub scale: f32,
    pub offset: f32,
}

impl Default for LinearScaleAdjustment {
    fn default() -> Self {
        LinearScaleAdjustment {
            scale: 1.0,
            offset: 0.0,
        }
    }
}

impl Hash for LinearScaleAdjustment {
    fn hash<H: Hasher>(&self, state: &mut H) {
        [self.scale, self.offset]
            .iter()
            .for_each(|v| OrderedFloat::from(*v).hash(state));
    }
}

pub type PathTransform = Transform2D<f32, UnknownUnit, UnknownUnit>;

pub fn hash_path_transform(transform: &PathTransform, state: &mut impl Hasher) {
    OrderedFloat(transform.m11).hash(state);
    OrderedFloat(transform.m12).hash(state);
    OrderedFloat(transform.m21).hash(state);
    OrderedFloat(transform.m22).hash(state);
    OrderedFloat(transform.m31).hash(state);
    OrderedFloat(transform.m32).hash(state);
}

impl Hash for ScalarOrArray<PathTransform> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.value {
            ScalarOrArrayValue::Scalar(transform) => hash_path_transform(transform, state),
            ScalarOrArrayValue::Array(transforms) => {
                transforms.iter().for_each(|transform| hash_path_transform(transform, state));
            }
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

impl PartialEq for SymbolShape {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Circle, Self::Circle) => true,
            (Self::Path(a), Self::Path(b)) => {
                let mut hash_a = DefaultHasher::new();
                let mut hash_b = DefaultHasher::new();
                hash_lyon_path(a, &mut hash_a);
                hash_lyon_path(b, &mut hash_b);
                hash_a.finish() == hash_b.finish()
            }
            _ => false,
        }
    }
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
    pub fn from_vega_str(shape: &str) -> Result<SymbolShape, ParseError> {
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
    type Error = ParseError;

    fn try_into(self) -> Result<SymbolShape, Self::Error> {
        SymbolShape::from_vega_str(self)
    }
}

