//! Core types for chart layout

use crate::cartesian::axis::AxisPosition;
use crate::legend::LegendPosition;
use indexmap::IndexMap;
use std::collections::HashMap;

/// Result of layout computation
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub plot_area: LayoutBounds,
    pub guide_overflows: HashMap<AxisPosition, LayoutBounds>,
    pub legends: IndexMap<String, LayoutBounds>,
    pub title: Option<LayoutBounds>,
    pub subtitle: Option<LayoutBounds>,
}

/// Bounding box for a layout component
#[derive(Debug, Clone, Copy)]
pub struct LayoutBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverflowSide {
    Top,
    Right,
    Bottom,
    Left,
}

/// Types of components that can be laid out
#[derive(Debug, Clone)]
pub(crate) enum ComponentType {
    PlotArea,
    GuideOverflow(OverflowSide),
    LegendContainer(LegendPosition),
    Title,
    Subtitle,
}

/// Minimum size in pixels for creating guide overflow regions
/// Overflow regions smaller than this are ignored to avoid unnecessary grid complexity
pub(crate) const MIN_GUIDE_OVERFLOW_SIZE: f32 = 2.0;
