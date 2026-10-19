//! What every guide shares: the side it stands on, the answer that sizes one of its parts, and
//! the background that a composed legend or colorbar draws behind its content.

use std::ops::RangeInclusive;

use avenger_color::ColorOrGradient;

/// The side of a plot that a guide stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuideSide {
    Top,
    Bottom,
    Left,
    Right,
}

impl GuideSide {
    /// Whether the guide runs down the plot's height, on its left or right, so that a layout
    /// measures its width at a height.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// The sign of the direction away from the plot, across the guide.
    pub(crate) fn outward(self) -> f32 {
        match self {
            Self::Left | Self::Top => -1.0,
            Self::Right | Self::Bottom => 1.0,
        }
    }

    /// The point `along` the guide from its start and `across` it, outward from its inner edge.
    pub(crate) fn point(self, along: f32, across: f32) -> [f32; 2] {
        let across = self.outward() * across;
        if self.is_vertical() {
            [across, along]
        } else {
            [along, across]
        }
    }

    /// How far `point`, in the frame of a part on this side, lies along the guide from its start
    /// and outward across it from its inner edge.
    pub fn along_across(self, point: [f32; 2]) -> (f32, f32) {
        if self.is_vertical() {
            (point[1], self.outward() * point[0])
        } else {
            (point[0], self.outward() * point[1])
        }
    }

    /// The origin of a part that a slab with rectangle `slot`, `[x, y, width, height]`, holds:
    /// where the slab's inner edge, nearest the plot, meets its start.
    pub fn slot_origin(self, slot: [f32; 4]) -> [f32; 2] {
        let [x, y, width, height] = slot;
        match self {
            Self::Bottom | Self::Right => [x, y],
            Self::Top => [x, y + height],
            Self::Left => [x + width, y],
        }
    }

    /// The origin of a composed legend or colorbar `thickness` thick, whose top left corner is
    /// its origin, in the slab with rectangle `slot`, `[x, y, width, height]`: its inner edge,
    /// nearest the plot, lies on the slab's inner edge, at the slab's start.
    pub(crate) fn box_origin(self, slot: [f32; 4], thickness: f32) -> [f32; 2] {
        let [x, y, width, height] = slot;
        match self {
            Self::Bottom | Self::Right => [x, y],
            Self::Top => [x, y + height - thickness],
            Self::Left => [x + width - thickness, y],
        }
    }
}

/// How large one part of a guide is at one length.
#[derive(Debug, Clone, PartialEq)]
pub struct PartExtent {
    /// How far the part reaches outward from its inner edge.
    pub thickness: f32,
    /// How far the part reaches past the start and the end of its length, along it.
    pub overhang: [f32; 2],
    /// The lengths around the asked length where `thickness` and `overhang` stay the same.
    pub holds: RangeInclusive<f32>,
}

impl PartExtent {
    /// A part with no size at any length.
    pub(crate) fn empty() -> Self {
        Self {
            thickness: 0.0,
            overhang: [0.0; 2],
            holds: 0.0..=f32::INFINITY,
        }
    }
}

/// The farthest a set of shapes reaches outward from a part's inner edge, and past the start
/// and end of its length.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Reach {
    pub(crate) across: f32,
    pub(crate) start: f32,
    pub(crate) end: f32,
}

impl Reach {
    /// Extends the reach to `point`, for a part of `length` on `side`.
    pub(crate) fn add(&mut self, side: GuideSide, length: f32, point: [f32; 2]) {
        let (along, across) = side.along_across(point);
        self.across = self.across.max(across);
        self.start = self.start.max(-along);
        self.end = self.end.max(along - length);
    }

    pub(crate) fn overhang(&self) -> [f32; 2] {
        [self.start, self.end]
    }
}

/// The box behind a composed legend or colorbar. `BackgroundStyle::default()` is transparent,
/// with 4 pixels of padding, so a legend's size is the same whether its background shows or not.
#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundStyle {
    /// The background's fill, transparent by default.
    pub fill: ColorOrGradient,
    /// Without it, the background has no outline.
    pub stroke: Option<ColorOrGradient>,
    /// The radius of the background's corners.
    pub corner_radius: f32,
    /// Space between the background's edges and the content.
    pub padding: f32,
}

impl Default for BackgroundStyle {
    fn default() -> Self {
        Self {
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]),
            stroke: None,
            corner_radius: 0.0,
            padding: 4.0,
        }
    }
}
