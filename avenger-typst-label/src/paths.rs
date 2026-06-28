use crate::style::Color;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathTransform {
    pub xx: f32,
    pub yx: f32,
    pub xy: f32,
    pub yy: f32,
    pub dx: f32,
    pub dy: f32,
}

impl MathTransform {
    pub const IDENTITY: Self = Self {
        xx: 1.0,
        yx: 0.0,
        xy: 0.0,
        yy: 1.0,
        dx: 0.0,
        dy: 0.0,
    };
}

impl Default for MathTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathPathCommand {
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
pub struct MathPathData {
    pub commands: Vec<MathPathCommand>,
}

impl MathPathData {
    pub fn rect(width: f32, height: f32) -> Self {
        Self {
            commands: vec![
                MathPathCommand::MoveTo { x: 0.0, y: 0.0 },
                MathPathCommand::LineTo { x: width, y: 0.0 },
                MathPathCommand::LineTo {
                    x: width,
                    y: height,
                },
                MathPathCommand::LineTo { x: 0.0, y: height },
                MathPathCommand::Close,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathStroke {
    pub color: Color,
    pub width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathPathKind {
    GlyphOutline {
        glyph_run: usize,
        glyph_index: usize,
    },
    MathShape,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathPathItem {
    pub path: MathPathData,
    pub kind: MathPathKind,
    pub fill: Option<Color>,
    pub stroke: Option<MathStroke>,
    pub transform: MathTransform,
    pub clip: Option<MathPathData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathImageFormat {
    Png,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathImageItem {
    pub data: Vec<u8>,
    pub format: MathImageFormat,
    pub width: f32,
    pub height: f32,
    pub transform: MathTransform,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathPathArtifact {
    pub logical_width: f32,
    pub logical_height: f32,
    pub items: Vec<MathPathItem>,
    pub images: Vec<MathImageItem>,
}
