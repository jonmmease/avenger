//! Core types for chart layout

use crate::cartesian::axis::AxisPosition;
use crate::legend::LegendPosition;
use std::collections::HashMap;

/// Result of layout computation
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub plot_area: LayoutBounds,
    pub axes: HashMap<AxisPosition, LayoutBounds>,
    pub legends: HashMap<String, LayoutBounds>,
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

/// Constants for layout
pub(crate) const OVERFLOW_THRESHOLD: f32 = 2.0;
pub(crate) const EDGE_MARGIN: f32 = 10.0;
