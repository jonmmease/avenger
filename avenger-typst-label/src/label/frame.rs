//! The frame a label lays out to: upstream's frame model, in `f32` points.
//!
//! Positions are in points, with y growing downwards. A text item's origin lies on its
//! baseline. Glyph advances and offsets are in ems of the item's size, and glyph offsets and
//! vertical advances are y-up, as upstream's. Frames carry no outlines or PDF data; the SVG,
//! PDF and raster lowerers derive them.

use std::fmt::{self, Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use avenger_color::AbsoluteColor;

use crate::typst_library::text::FontInstance;

/// A point, in points.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    /// The origin.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// Creates a point.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A size, in points.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub x: f32,
    pub y: f32,
}

impl Size {
    /// Creates a size.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An affine transformation, which maps `(x, y)` to
/// `(sx·x + kx·y + tx, ky·x + sy·y + ty)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub sx: f32,
    pub ky: f32,
    pub kx: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Transform {
    /// The identity transformation.
    pub const IDENTITY: Self = Self {
        sx: 1.0,
        ky: 0.0,
        kx: 0.0,
        sy: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// A translation.
    pub const fn translate(x: f32, y: f32) -> Self {
        Self { tx: x, ty: y, ..Self::IDENTITY }
    }

    /// A scale.
    pub const fn scale(x: f32, y: f32) -> Self {
        Self { sx: x, sy: y, ..Self::IDENTITY }
    }

    /// This transformation after `inner`: applying the result is applying `inner`, then
    /// `self`.
    pub fn pre_concat(self, inner: Self) -> Self {
        Self {
            sx: self.sx * inner.sx + self.kx * inner.ky,
            ky: self.ky * inner.sx + self.sy * inner.ky,
            kx: self.sx * inner.kx + self.kx * inner.sy,
            sy: self.ky * inner.kx + self.sy * inner.sy,
            tx: self.sx * inner.tx + self.kx * inner.ty + self.tx,
            ty: self.ky * inner.tx + self.sy * inner.ty + self.ty,
        }
    }

    /// Applies the transformation to a point.
    pub fn apply(self, point: Point) -> Point {
        Point {
            x: self.sx * point.x + self.kx * point.y + self.tx,
            y: self.ky * point.x + self.sy * point.y + self.ty,
        }
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A laid-out label line.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelFrame {
    /// The frame's size.
    pub size: Size,
    /// The baseline's distance from the top of the frame.
    pub baseline: f32,
    /// The items, positioned relative to the frame's top left, in drawing order.
    pub items: Vec<(Point, FrameItem)>,
}

impl LabelFrame {
    /// The text items at any depth, in drawing order, each with the transform from its
    /// coordinates to the frame's: the item's origin is the transform applied to the origin.
    pub fn text_items(&self) -> Vec<(Transform, &TextItem)> {
        let mut items = vec![];
        self.visit(Transform::IDENTITY, &mut |ts, item| {
            if let FrameItem::Text(text) = item {
                items.push((ts, text));
            }
        });
        items
    }

    /// Calls `f` with each item at any depth, in drawing order, and the transform from the
    /// item's coordinates to the frame's. Groups are visited before their items.
    pub fn visit<'a>(
        &'a self,
        ts: Transform,
        f: &mut impl FnMut(Transform, &'a FrameItem),
    ) {
        for (pos, item) in &self.items {
            let ts = ts.pre_concat(Transform::translate(pos.x, pos.y));
            f(ts, item);
            if let FrameItem::Group(group) = item {
                group.frame.visit(ts.pre_concat(group.transform), f);
            }
        }
    }
}

/// An item in a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameItem {
    /// A subframe with a transformation.
    Group(GroupItem),
    /// A run of shaped text.
    Text(TextItem),
    /// A geometric shape with optional fill and stroke.
    Shape(Shape),
}

/// A subframe with a transformation.
// avenger: no clip, since labels don't clip.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupItem {
    /// The group's frame.
    pub frame: LabelFrame,
    /// The transformation applied to the frame, around its position.
    pub transform: Transform,
}

/// A run of shaped text in one font, size and fill.
#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    /// The font the glyphs are from.
    pub font: FontRef,
    /// The font size, in points.
    pub size: f32,
    /// The fill of the glyphs.
    pub fill: AbsoluteColor,
    /// The text the glyphs show.
    pub text: String,
    /// The glyphs, in visual order.
    pub glyphs: Vec<Glyph>,
    /// The byte range in the label source that the glyphs come from: the union of the
    /// glyphs' ranges.
    pub source: Range<usize>,
}

impl TextItem {
    /// The glyphs with their origins relative to the item's origin, as upstream's renderers
    /// place them: the pen advances by each glyph's advances, and the glyph sits at the pen
    /// plus its offset.
    pub fn positioned_glyphs(&self) -> impl Iterator<Item = (Point, &Glyph)> + '_ {
        let mut x = 0.0;
        let mut y = 0.0;
        self.glyphs.iter().map(move |glyph| {
            let point = Point {
                x: x + glyph.x_offset * self.size,
                y: -(y + glyph.y_offset * self.size),
            };
            x += glyph.x_advance * self.size;
            y += glyph.y_advance * self.size;
            (point, glyph)
        })
    }

    /// The item's width, the sum of its horizontal advances, in points.
    pub fn width(&self) -> f32 {
        self.glyphs.iter().map(|glyph| glyph.x_advance).sum::<f32>() * self.size
    }
}

/// A glyph in a run of shaped text.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// The glyph's index in the font.
    pub id: u16,
    /// The horizontal advance, in ems.
    pub x_advance: f32,
    /// The horizontal offset, in ems.
    pub x_offset: f32,
    /// The vertical advance (y-up), in ems.
    pub y_advance: f32,
    /// The vertical offset (y-up), in ems.
    pub y_offset: f32,
    /// The byte range of the glyph's cluster in its item's text.
    pub range: Range<usize>,
    /// The byte range in the label source that the glyph comes from: the cluster itself for
    /// text that appears verbatim in the source, else the whole markup or math node.
    pub source: Range<usize>,
}

/// A geometric shape with optional fill and stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// The shape's geometry.
    pub geometry: Geometry,
    /// The shape's fill.
    pub fill: Option<AbsoluteColor>,
    /// The fill rule.
    pub fill_rule: FillRule,
    /// The shape's stroke.
    pub stroke: Option<Stroke>,
}

/// A shape's geometry, relative to its position.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// A line to a point.
    Line(Point),
    /// A rectangle with its origin in the top left corner.
    Rect(Size),
    /// A curve of movements, lines and cubic Bézier segments.
    Curve(Curve),
}

/// A curve of movements, lines and cubic Bézier segments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curve(pub Vec<CurveItem>);

/// An item in a curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveItem {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Close,
}

/// How the inside of a self-intersecting shape is determined.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FillRule {
    /// Inside is where the signed edge crossings don't sum to zero.
    #[default]
    NonZero,
    /// Inside is where the number of edge crossings is odd.
    EvenOdd,
}

/// A stroke of a shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    /// The stroke's paint.
    pub paint: AbsoluteColor,
    /// The stroke's thickness, in points.
    pub thickness: f32,
    /// The line cap.
    pub cap: LineCap,
    /// The line join.
    pub join: LineJoin,
    /// The dash pattern.
    pub dash: Option<DashPattern>,
    /// The miter limit.
    pub miter_limit: f32,
}

/// A dash pattern, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct DashPattern {
    /// The lengths of alternating dashes and gaps.
    pub array: Vec<f32>,
    /// Where the pattern starts.
    pub phase: f32,
}

/// The shape at the ends of a stroked line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// The shape where stroked segments meet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// A font face at fixed variation coordinates, which a text item's glyphs index.
///
/// Cloning is cheap, and the face's data is shared, not copied. Two references are equal when
/// they reference the same face at the same coordinates.
#[derive(Clone)]
pub struct FontRef(pub(crate) FontInstance);

impl FontRef {
    /// The font's family name.
    pub fn family(&self) -> &str {
        &self.0.font().info().family
    }

    /// The face's PostScript name, if it has one.
    pub fn postscript_name(&self) -> Option<String> {
        self.0.font().post_script_name()
    }

    /// The face's index in its font collection, or zero for a single font.
    pub fn index(&self) -> u32 {
        self.0.font().index()
    }

    /// The font file's data.
    pub fn data(&self) -> &Arc<[u8]> {
        self.0.font().data()
    }

    /// The variation coordinates of a variable font, as OpenType axis tags and values.
    pub fn variations(&self) -> Vec<([u8; 4], f32)> {
        self.0
            .variations()
            .0
            .iter()
            .map(|&(tag, value)| (tag.to_bytes(), value.0))
            .collect()
    }

    /// The number of font units per em.
    pub fn units_per_em(&self) -> f32 {
        self.0.units_per_em() as f32
    }
}

impl Debug for FontRef {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "FontRef({:?}, {:?})", self.family(), self.0.font().info().variant)
    }
}

impl PartialEq for FontRef {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for FontRef {}

impl Hash for FontRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
