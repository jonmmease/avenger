pub mod colorbar;
pub mod line;
pub mod symbol;

use std::collections::HashSet;

use avenger_scenegraph::marks::group::SceneGroup;

use crate::error::AvengerGuidesError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideLegendItem {
    pub index: usize,
    pub label: String,
    pub group_path: Vec<usize>,
    pub hit_rect_path: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideLegendSurfaceKind {
    Colorbar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideLegendContinuousOrientation {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuideLegendContinuousSurface {
    pub kind: GuideLegendSurfaceKind,
    pub orientation: GuideLegendContinuousOrientation,
    pub surface_group_path: Vec<usize>,
    pub gradient_rect_path: Vec<usize>,
    pub hit_rect_path: Vec<usize>,
    /// [x, y, width, height] relative to the returned legend group.
    pub bounds: [f32; 4],
    pub value_channel: String,
    pub band_channel: String,
}

#[derive(Debug, Clone)]
pub struct GuideLegendOutput {
    pub group: SceneGroup,
    pub items: Vec<GuideLegendItem>,
    pub continuous_surfaces: Vec<GuideLegendContinuousSurface>,
}

fn compute_encoding_length(lengths: &[usize]) -> Result<usize, AvengerGuidesError> {
    let lengths = lengths
        .iter()
        .cloned()
        .filter(|&len| len > 1)
        .collect::<HashSet<_>>();

    let len = if lengths.len() > 1 {
        return Err(AvengerGuidesError::InvalidLegendLength(lengths));
    } else if lengths.is_empty() {
        1
    } else {
        // Only one unique length greater than 1
        lengths.into_iter().next().unwrap_or(1)
    };

    Ok(len)
}
