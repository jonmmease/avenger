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
    #[allow(dead_code)] // Reserved for future bounding box calculations
    pub total_bounds: LayoutBounds,
}

/// Bounding box for a layout component
#[derive(Debug, Clone, Copy)]
pub struct LayoutBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Maps components to their grid positions
#[derive(Debug)]
pub(crate) struct ComponentGridMap {
    // Track which grid cells are occupied by which components
    pub cells: HashMap<(usize, usize), ComponentType>,

    // Dynamic grid dimensions
    pub row_count: usize,
    pub col_count: usize,
}

impl ComponentGridMap {
    pub fn new() -> Self {
        ComponentGridMap {
            cells: HashMap::new(),
            row_count: 0,
            col_count: 0,
        }
    }

    pub fn add_component(&mut self, component: ComponentType, row: usize, col: usize) {
        self.cells.insert((row, col), component);
        self.row_count = self.row_count.max(row + 1);
        self.col_count = self.col_count.max(col + 1);
    }
}

/// Types of components that can be laid out
#[derive(Debug, Clone)]
pub(crate) enum ComponentType {
    PlotArea,
    Axis(AxisPosition),
    #[allow(dead_code)]
    Legend(String), // Channel name - may be used in future
    LegendContainer(LegendPosition), // Container for legends at a position
    Title,
    Subtitle,
    #[allow(dead_code)]
    Padding, // Empty space - may be used for layout padding in future
}

/// Constants for layout
pub(crate) const OVERFLOW_THRESHOLD: f32 = 2.0;
pub(crate) const EDGE_MARGIN: f32 = 10.0;
