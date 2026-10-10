use crate::{
    impl_hash_for_scalar_or_array,
    lyon::{hash_lyon_path, parse_svg_path},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_color::{ColorOrGradient, Gradient};
use lyon_extra::{
    euclid::{Box2D, Point2D, Scale, Transform2D, UnknownUnit},
    parser::ParseError,
};
use lyon_path::{geom::Point, Winding};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    hash::{DefaultHasher, Hash, Hasher},
};
use strum::VariantNames;

/// Determines which regions of a compound path are filled.
#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl From<FillRule> for lyon_path::FillRule {
    fn from(rule: FillRule) -> Self {
        match rule {
            FillRule::NonZero => Self::NonZero,
            FillRule::EvenOdd => Self::EvenOdd,
        }
    }
}

/// Ordinary scene miter limit, measured as miter length / half stroke width.
pub const SCENE_MITER_LIMIT: f32 = 8.0;
/// Lyon 1.x measures the miter limit relative to the full stroke width.
pub const LYON_SCENE_MITER_LIMIT: f32 = SCENE_MITER_LIMIT / 2.0;

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

impl_hash_for_scalar_or_array!(ColorOrGradient);
impl_hash_for_scalar_or_array!(Gradient);

#[derive(Default, Debug, Copy, Clone, Hash, PartialEq, Serialize, Deserialize, VariantNames)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AreaOrientation {
    #[default]
    Vertical,
    Horizontal,
}
impl_hash_for_scalar_or_array!(AreaOrientation);

/// Where a text mark's position lies across each label's box.
#[derive(
    Debug, Default, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize, VariantNames,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}
impl_hash_for_scalar_or_array!(TextAlign);

/// Where a text mark's position lies down each label.
#[derive(
    Debug, Default, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize, VariantNames,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum TextBaseline {
    /// On the first line's baseline.
    Alphabetic,
    /// On the top of the label's box, which pads its lines to at least the font size.
    Top,
    /// On the middle of the box.
    Middle,
    /// On the bottom of the box.
    #[default]
    Bottom,
    /// On the top of the line box: the box with half the gap between lines above it.
    LineTop,
    /// On the bottom of the line box: the box with half the gap between lines below it.
    LineBottom,
}
impl_hash_for_scalar_or_array!(TextBaseline);

/// How a text mark's labels read. The syntaxes differ only in how they read a newline: all of
/// them wrap alike under the label's layout.
#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextSyntaxMode {
    /// Literal text, whose newlines are spaces.
    #[default]
    Plain,
    /// Literal text, whose newlines end lines.
    PlainLines,
    /// Typst markup, where a newline is a space and `\` ends a line.
    TypstMarkup,
}

/// A font weight, a number as in CSS: 400 is normal and 700 bold.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const NORMAL: Self = Self(400);
    pub const BOLD: Self = Self(700);
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl From<f32> for FontWeight {
    /// The weight nearest a number.
    fn from(weight: f32) -> Self {
        Self(weight.round() as u16)
    }
}
impl_hash_for_scalar_or_array!(FontWeight);

#[derive(
    Debug, Default, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize, VariantNames,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}
impl_hash_for_scalar_or_array!(FontStyle);

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
                transforms
                    .iter()
                    .for_each(|transform| hash_path_transform(transform, state));
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
            "star" => {
                // https://github.com/d3/d3-shape/blob/main/src/symbol/star.js, with the outer
                // radius that makes the star one unit wide, as the other shapes are.
                let kr =
                    (std::f32::consts::PI / 10.0).sin() / (7.0 * std::f32::consts::PI / 10.0).sin();
                let kx = (std::f32::consts::TAU / 10.0).sin() * kr;
                let ky = -(std::f32::consts::TAU / 10.0).cos() * kr;

                let r = 0.5 / (std::f32::consts::TAU / 5.0).sin();
                let x = kx * r;
                let y = ky * r;

                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(0.0, -r));
                builder.line_to(Point::new(x, y));

                for i in 1..5 {
                    let a = std::f32::consts::TAU * i as f32 / 5.0;
                    let c = a.cos();
                    let s = a.sin();
                    builder.line_to(Point::new(s * r, -c * r));
                    builder.line_to(Point::new(c * x - s * y, s * x + c * y));
                }
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "wye" => {
                // https://github.com/d3/d3-shape/blob/main/src/symbol/wye.js, with the arm length
                // that makes the wye one unit wide, as the other shapes are.
                let c = -0.5;
                let s = sqrt3 / 2.0;
                let k = 1.0 / 12.0f32.sqrt();

                let r = 1.0 / (1.0 + sqrt3);
                let x0 = r / 2.0;
                let y0 = r * k;
                let x1 = x0;
                let y1 = r * k + r;
                let x2 = -x1;
                let y2 = y1;

                let mut builder = lyon_path::Path::builder().with_svg();
                builder.move_to(Point::new(x0, y0));
                builder.line_to(Point::new(x1, y1));
                builder.line_to(Point::new(x2, y2));
                builder.line_to(Point::new(c * x0 - s * y0, s * x0 + c * y0));
                builder.line_to(Point::new(c * x1 - s * y1, s * x1 + c * y1));
                builder.line_to(Point::new(c * x2 - s * y2, s * x2 + c * y2));
                builder.line_to(Point::new(c * x0 + s * y0, c * y0 - s * x0));
                builder.line_to(Point::new(c * x1 + s * y1, c * y1 - s * x1));
                builder.line_to(Point::new(c * x2 + s * y2, c * y2 - s * x2));
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "pentagon" => {
                // The circumradius that makes the pentagon one unit wide, as the other shapes are.
                let r = 0.5 / (std::f32::consts::TAU / 5.0).sin();
                let n = 5;
                let mut builder = lyon_path::Path::builder().with_svg();

                for i in 0..n {
                    let angle = (2.0 * std::f32::consts::PI * i as f32 / n as f32)
                        - std::f32::consts::PI / 2.0;
                    let x = r * angle.cos();
                    let y = r * angle.sin();
                    if i == 0 {
                        builder.move_to(Point::new(x, y));
                    } else {
                        builder.line_to(Point::new(x, y));
                    }
                }
                builder.close();
                SymbolShape::Path(builder.build())
            }
            "cushion" => {
                let r = 0.5;
                let curve_depth = 0.6;

                let mut builder = lyon_path::Path::builder().with_svg();

                builder.move_to(Point::new(-r, -r));

                builder
                    .quadratic_bezier_to(Point::new(0.0, -r + curve_depth * r), Point::new(r, -r));

                builder.quadratic_bezier_to(Point::new(r - curve_depth * r, 0.0), Point::new(r, r));

                builder
                    .quadratic_bezier_to(Point::new(0.0, r - curve_depth * r), Point::new(-r, r));

                builder
                    .quadratic_bezier_to(Point::new(-r + curve_depth * r, 0.0), Point::new(-r, -r));

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

    pub fn as_path(&self) -> Cow<'_, lyon_path::Path> {
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

#[cfg(test)]
mod symbol_tests {
    use super::SymbolShape;
    use lyon_path::{iterator::PathIterator, Event};

    /// The width and height of a named shape at size 1.
    fn extent(name: &str) -> [f32; 2] {
        let shape = SymbolShape::from_vega_str(name).unwrap();
        let points: Vec<_> = shape
            .as_path()
            .iter()
            .flattened(1e-4)
            .filter_map(|event| match event {
                Event::Begin { at } => Some(at),
                Event::Line { to, .. } => Some(to),
                _ => None,
            })
            .collect();
        let span = |coordinate: fn(&lyon_path::math::Point) -> f32| {
            let values = points.iter().map(coordinate);
            values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)
        };
        [span(|p| p.x), span(|p| p.y)]
    }

    /// A named shape fits a box one unit on a side, so `size` sets the area of the box it fits.
    #[test]
    fn named_shapes_fit_a_unit_box() {
        for name in [
            "circle",
            "square",
            "cross",
            "diamond",
            "triangle-up",
            "triangle-down",
            "triangle-right",
            "triangle-left",
            "arrow",
            "wedge",
            "triangle",
            "star",
            "wye",
            "pentagon",
            "cushion",
        ] {
            let [width, height] = extent(name);
            assert!(
                width.max(height) <= 1.0 + 1e-4,
                "{name}: {width} × {height}"
            );
        }
        for name in ["star", "wye", "pentagon", "cushion"] {
            let [width, _] = extent(name);
            assert!((width - 1.0).abs() < 1e-4, "{name}: {width}");
        }
    }
}
