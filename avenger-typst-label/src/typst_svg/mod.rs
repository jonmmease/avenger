//! SVG/vector-oriented label artifacts.
//!
//! This mirrors the frame-to-vector role of upstream `typst-svg`: path data,
//! transforms, solid paints/strokes, image items, and native-text-friendly
//! metadata. Actual SVG document emission is owned by `avenger-svg` through
//! `avenger-text`.

use crate::typst_library::Color;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Transform {
    pub sx: f32,
    pub ky: f32,
    pub kx: f32,
    pub sy: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        sx: 1.0,
        ky: 0.0,
        kx: 0.0,
        sy: 1.0,
        tx: 0.0,
        ty: 0.0,
    };
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PathCommand {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    QuadTo {
        x1: f32,
        y1: f32,
        x: f32,
        y: f32,
    },
    CubicTo {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        x: f32,
        y: f32,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PathData {
    pub commands: Vec<PathCommand>,
}

impl PathData {
    pub fn rect(width: f32, height: f32) -> Self {
        Self {
            // Match upstream typst-svg rectangle winding.
            commands: vec![
                PathCommand::MoveTo { x: 0.0, y: 0.0 },
                PathCommand::LineTo { x: 0.0, y: height },
                PathCommand::LineTo {
                    x: width,
                    y: height,
                },
                PathCommand::LineTo { x: width, y: 0.0 },
                PathCommand::Close,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PathCommand, PathData};

    #[test]
    fn rect_matches_upstream_typst_svg_winding() {
        let commands = PathData::rect(12.0, 8.0).commands;
        assert!(matches!(
            commands.as_slice(),
            [
                PathCommand::MoveTo { x: 0.0, y: 0.0 },
                PathCommand::LineTo { x: 0.0, y: 8.0 },
                PathCommand::LineTo { x: 12.0, y: 8.0 },
                PathCommand::LineTo { x: 12.0, y: 0.0 },
                PathCommand::Close,
            ]
        ));
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DashPattern {
    pub array: Vec<f32>,
    pub phase: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub dash: Option<DashPattern>,
    pub miter_limit: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LineJoin {
    Bevel,
    #[default]
    Miter,
    Round,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PathKind {
    GlyphOutline {
        glyph_run: usize,
        glyph_index: usize,
    },
    MathShape,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PathItem {
    pub path: PathData,
    pub kind: PathKind,
    pub fill: Option<Color>,
    pub stroke: Option<Stroke>,
    pub transform: Transform,
    pub clip: Option<PathData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PathImageFormat {
    Png,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PathImageItem {
    pub data: Vec<u8>,
    pub format: PathImageFormat,
    pub width: f32,
    pub height: f32,
    pub transform: Transform,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PathArtifact {
    pub logical_width: f32,
    pub logical_height: f32,
    pub items: Vec<PathItem>,
    pub images: Vec<PathImageItem>,
}
