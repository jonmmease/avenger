//! Main ChartLayout struct and core implementation

use super::grid::{GridBuilder, GridLayout};
use super::legend::measure_legend_size;
use super::sizing::{LayoutSpec, SizeMode};
use super::types::{ComponentType, LayoutBounds, LayoutResult, OVERFLOW_THRESHOLD, OverflowSide};
use crate::cartesian::axis::AxisPosition;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
use crate::marks::Mark;
use crate::plot::{PlotSubtitle, PlotTitle, TitleAlign};
use avenger_scales::scales::ConfiguredScale;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use taffy::prelude::*;
use taffy::{NodeId, TaffyTree};
use tracing::debug;

/// Dynamic grid-based layout manager for data visualization charts.
///
/// `ChartLayout` orchestrates the spatial arrangement of all visual components in a chart,
/// including the plot area, axes/guides, legends, titles, and subtitles. It uses the Taffy
/// flexbox/grid layout engine to create a responsive layout that adapts to:
///
/// - Variable content sizes (e.g., legend entries, axis labels)
/// - Overflow requirements from coordinate system guides
/// - Available canvas dimensions
///
/// # Architecture
///
/// The layout system works in two phases:
///
/// 1. **Grid Construction**: Dynamically builds a CSS Grid template based on which
///    components are present and their positioning requirements. Components are
///    arranged in a grid where:
///    - margin rows and columns are added around the edges
///    - The plot area occupies the center
///    - Axes are positioned on edges as overflow regions
///    - Legends are grouped in containers at their specified positions
///    - Titles and subtitles are positioned above the plot area
///
/// 2. **Layout Computation**: Uses Taffy to calculate precise pixel positions and
///    dimensions for each component, respecting:
///    - Minimum size requirements
///    - Flexible sizing for components like colorbars
///    - Overflow space for axes and grids
///    - Text measurement for accurate title/legend sizing
///
/// # Example Grid Structure
///
/// ```text
///   ↓ margin column
/// ┌────┬───────────────────────────────────────────────┬────┐
/// │    │                                               │    │ ← margin row
/// ├────┼───────────────────────────────────────────────┼────┤
/// │    │              Title (optional)                 │    │
/// │    ├───────────────────────────────────────────────┤    │
/// │    │            Subtitle (optional)                │    │
/// │    ├────────────┬─────────────────┬────────┬───────┤    │
/// │    │            │ Overflow-top    │        │       │    │
/// │    │            │ (axes/guide)    │        │       │    │
/// │    ├────────────┼─────────────────┼────────┼───────┤    │
/// │    │ Overflow-  │                 │Overflow│Right  │    │
/// │    │ left       │   Plot Area     │-right  │Legend │    │
/// │    │(axes/guide)│                 │(axes)  │       │    │
/// │    ├────────────┼─────────────────┼────────┼───────┤    │
/// │    │            │ Overflow-bottom │        │       │    │
/// │    │            │ (axes/guide)    │        │       │    │
/// ├────├────────────┴─────────────────┴────────┴───────┼────┤
/// │    │                                               │    │ ← margin row
/// └────┴───────────────────────────────────────────────┴────┘
///    ↑ margin column                                     ↑ margin column
/// ```
///
/// The layout adapts based on which components are actually present,
/// collapsing empty rows and columns automatically. The outer margin
/// is implemented as fixed-size rows/columns in the Taffy grid.
#[derive(Debug)]
pub struct ChartLayout {
    taffy: TaffyTree,
    root_node: NodeId,

    // Component nodes
    plot_area_node: Option<NodeId>,
    guide_overflow_nodes: HashMap<AxisPosition, NodeId>, // Guide overflow regions
    legend_container_nodes: HashMap<LegendPosition, NodeId>, // Flex containers for each position
    legend_nodes: HashMap<String, NodeId>,               // Individual legend nodes keyed by channel
    legend_sizes: HashMap<String, Size<f32>>,            // Store measured sizes
    legend_flexible: HashMap<String, bool>, // Store whether legend prefers flexible layout
    title_node: Option<NodeId>,
    subtitle_node: Option<NodeId>,

    // Grid configuration
    grid_layout: GridLayout,
}

impl ChartLayout {
    /// Create a new ChartLayout with overflow space requirements
    /// This is the unified layout method for all coordinate systems
    pub fn new_with_overflow<C: CoordinateSystem>(
        overflow: &crate::coords::OverflowSpaceRequirement,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        layout_spec: &LayoutSpec,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        marks: &[Arc<dyn Mark<C>>],
        theme: &dyn crate::theme::Theme,
    ) -> Result<Self, AvengerChartError> {
        let mut taffy = TaffyTree::new();
        let mut builder = GridBuilder::new();

        // Build grid structure dynamically
        // Plot area is always present and positioned dynamically in the grid

        // Add optional title and subtitle
        if title.is_some() {
            builder.add_title();
        }
        if subtitle.is_some() {
            builder.add_subtitle();
        }

        // Note: Guide overflow regions are determined directly from the overflow
        // measurements in build_with_overflow(), so we don't need to add them explicitly

        // Add legends with their positions
        for (channel, legend) in legends.iter() {
            let position = legend.position.unwrap_or(LegendPosition::Right);
            builder.add_legend(channel.clone(), position);
        }

        // Generate grid template with overflow measurements
        // For initial layout, we need to estimate a size for legend measurements
        let available_size = match &layout_spec.canvas {
            SizeMode::Fixed { width, height } => (*width, *height),
            _ => (400.0, 300.0), // Default for Auto or other modes
        };
        let grid_layout = builder.build_with_overflow(
            overflow,
            legends,
            scales,
            marks,
            title,
            subtitle,
            theme,
            layout_spec,
        )?;

        // Create root node with grid layout
        // Initial size for root will be set during compute based on layout mode
        let root_style = Style {
            display: Display::Grid,
            grid_template_columns: grid_layout.cols.clone(),
            grid_template_rows: grid_layout.rows.clone(),
            size: Size {
                width: auto(),
                height: auto(),
            },
            ..Default::default()
        };

        let root_node = taffy.new_leaf(root_style)?;

        let mut layout = ChartLayout {
            taffy,
            root_node,
            plot_area_node: None,
            guide_overflow_nodes: HashMap::new(),
            legend_container_nodes: HashMap::new(),
            legend_nodes: HashMap::new(),
            legend_sizes: HashMap::new(),
            legend_flexible: HashMap::new(),
            title_node: None,
            subtitle_node: None,
            grid_layout,
        };

        // Measure legend sizes and flexibility preferences
        let mut legend_sizes = HashMap::new();
        let mut legend_flexible = HashMap::new();
        for (channel, legend) in legends.iter() {
            if let Some(scale) = scales.get(channel) {
                let (size, flexible) = measure_legend_size(
                    channel,
                    legend,
                    scale,
                    scales,
                    Size {
                        width: available_size.0,
                        height: available_size.1,
                    },
                    marks,
                )?;
                legend_sizes.insert(channel.clone(), size);
                legend_flexible.insert(channel.clone(), flexible);
            }
        }
        layout.legend_sizes = legend_sizes;
        layout.legend_flexible = legend_flexible;

        // Create nodes for each component in the grid
        layout.create_component_nodes_with_overflow(overflow, legends, scales, title, subtitle)?;

        Ok(layout)
    }

    /// Find component position in the grid
    fn find_component_position(&self, component: &ComponentType) -> Option<(usize, usize)> {
        self.grid_layout.find_component_position(component)
    }

    fn create_component_nodes_with_overflow(
        &mut self,
        overflow: &crate::coords::OverflowSpaceRequirement,
        legends: &IndexMap<String, Legend>,
        _scales: &HashMap<String, ConfiguredScale>,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
    ) -> Result<(), AvengerChartError> {
        // Find plot area position in the component map
        let mut plot_row = 0;
        let mut plot_col = 0;

        for ((row, col), comp_type) in &self.grid_layout.component_cells {
            if matches!(comp_type, ComponentType::PlotArea) {
                plot_row = *row;
                plot_col = *col;
                break;
            }
        }

        // Create plot area node
        let plot_style = Style {
            display: Display::Block,
            grid_row: line((plot_row + 1) as i16), // Convert to 1-based grid line
            grid_column: line((plot_col + 1) as i16), // Convert to 1-based grid line
            flex_grow: 1.0,                        // Allow plot area to grow
            flex_shrink: 1.0,                      // Allow plot area to shrink
            min_size: Size {
                width: length(50.0),  // Minimum width
                height: length(50.0), // Minimum height
            },
            ..Default::default()
        };
        self.plot_area_node = Some(self.taffy.new_leaf(plot_style)?);

        // Create overflow region nodes if they exist
        if overflow.left > OVERFLOW_THRESHOLD {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::GuideOverflow(OverflowSide::Left))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.guide_overflow_nodes.insert(AxisPosition::Left, node);
            }
        }

        if overflow.right > OVERFLOW_THRESHOLD {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::GuideOverflow(OverflowSide::Right))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.guide_overflow_nodes.insert(AxisPosition::Right, node);
            }
        }

        if overflow.top > OVERFLOW_THRESHOLD {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::GuideOverflow(OverflowSide::Top))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.guide_overflow_nodes.insert(AxisPosition::Top, node);
            }
        }

        if overflow.bottom > OVERFLOW_THRESHOLD {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::GuideOverflow(OverflowSide::Bottom))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.guide_overflow_nodes.insert(AxisPosition::Bottom, node);
            }
        }

        // Helper function to calculate grid column based on TitleAlign
        let calculate_grid_column = |col: usize, align: TitleAlign| {
            match align {
                TitleAlign::PlotAreaOnly => {
                    // Only span the plot area column
                    let mut plot_col = col;
                    for ((_, c), comp) in &self.grid_layout.component_cells {
                        if matches!(comp, ComponentType::PlotArea) {
                            plot_col = *c;
                            break;
                        }
                    }
                    line((plot_col + 1) as i16)
                }
                TitleAlign::FullWidth => {
                    // Find the rightmost column with a component
                    let mut end_col = col;
                    for ((_, c), _comp) in &self.grid_layout.component_cells {
                        if *c > end_col {
                            end_col = *c;
                        }
                    }

                    let span_count = (end_col - col + 1) as u16;
                    if span_count == 1 {
                        line((col + 1) as i16)
                    } else {
                        Line {
                            start: line((col + 1) as i16),
                            end: line((end_col + 2) as i16),
                        }
                    }
                }
            }
        };

        // Create title node if present
        if let Some((row, col)) = self.find_component_position(&ComponentType::Title) {
            let grid_col = if let Some(t) = title {
                calculate_grid_column(col, t.align)
            } else {
                // Default to full width if no title config (shouldn't happen)
                line((col + 1) as i16)
            };

            let style = Style {
                display: Display::Block,
                grid_row: line((row + 1) as i16), // Convert to 1-based
                grid_column: grid_col,
                ..Default::default()
            };
            self.title_node = Some(self.taffy.new_leaf(style)?);
        }

        // Create subtitle node if present
        if let Some((row, col)) = self.find_component_position(&ComponentType::Subtitle) {
            let grid_col = if let Some(s) = subtitle {
                calculate_grid_column(col, s.align)
            } else {
                // Default to full width if no subtitle config (shouldn't happen)
                line((col + 1) as i16)
            };

            let style = Style {
                display: Display::Block,
                grid_row: line((row + 1) as i16), // Convert to 1-based
                grid_column: grid_col,
                ..Default::default()
            };
            self.subtitle_node = Some(self.taffy.new_leaf(style)?);
        }

        // Create legend nodes and group by container
        let mut legend_nodes_by_container: HashMap<LegendPosition, Vec<taffy::NodeId>> =
            HashMap::new();

        for (channel, legend) in legends {
            // Find which container this legend belongs to based on legend position
            let legend_position = legend.position.unwrap_or(LegendPosition::Right);

            for ((row, col), comp_type) in &self.grid_layout.component_cells {
                if let ComponentType::LegendContainer(pos) = comp_type {
                    if *pos == legend_position {
                        // Create container node if it doesn't exist
                        if !self.legend_container_nodes.contains_key(pos) {
                            let container_style = Style {
                                display: Display::Flex,
                                flex_direction: FlexDirection::Column,
                                grid_row: line((*row + 1) as i16), // Convert to 1-based
                                grid_column: line((*col + 1) as i16), // Convert to 1-based
                                ..Default::default()
                            };
                            let container_node = self.taffy.new_leaf(container_style)?;
                            self.legend_container_nodes.insert(*pos, container_node);
                        }

                        // Create legend node
                        if let Some(size) = self.legend_sizes.get(channel) {
                            // Check if this legend prefers flexible layout
                            let is_flexible =
                                self.legend_flexible.get(channel).copied().unwrap_or(false);

                            let legend_style = if is_flexible {
                                // Colorbar should stretch vertically
                                Style {
                                    display: Display::Block,
                                    size: Size {
                                        width: length(size.width),
                                        height: auto(), // Let it stretch
                                    },
                                    flex_grow: 1.0,   // Allow it to grow
                                    flex_shrink: 1.0, // Allow it to shrink
                                    min_size: Size {
                                        width: length(size.width),
                                        height: length(50.0), // Minimum height
                                    },
                                    ..Default::default()
                                }
                            } else {
                                // Regular legends have fixed size
                                Style {
                                    display: Display::Block,
                                    size: Size {
                                        width: length(size.width),
                                        height: length(size.height),
                                    },
                                    ..Default::default()
                                }
                            };

                            let legend_node = self.taffy.new_leaf(legend_style)?;
                            self.legend_nodes.insert(channel.clone(), legend_node);

                            // Track this legend node for its container
                            legend_nodes_by_container
                                .entry(*pos)
                                .or_default()
                                .push(legend_node);
                        }
                        break;
                    }
                }
            }
        }

        // Set children for each legend container
        for (position, legend_children) in legend_nodes_by_container {
            if let Some(container_node) = self.legend_container_nodes.get(&position) {
                self.taffy.set_children(*container_node, &legend_children)?;
            }
        }

        // Set all children on root
        let mut children = Vec::new();
        if let Some(node) = self.plot_area_node {
            children.push(node);
        }
        for node in self.guide_overflow_nodes.values() {
            children.push(*node);
        }
        for node in self.legend_container_nodes.values() {
            children.push(*node);
        }
        if let Some(node) = self.title_node {
            children.push(node);
        }
        if let Some(node) = self.subtitle_node {
            children.push(node);
        }
        self.taffy.set_children(self.root_node, &children)?;

        Ok(())
    }

    /// Compute layout with flexible sizing based on LayoutSpec
    pub fn compute_with_spec(
        &mut self,
        layout_spec: &LayoutSpec,
    ) -> Result<super::sizing::ComputeResult, AvengerChartError> {
        // Determine available space based on layout spec
        let (available_width, available_height) =
            match (&layout_spec.canvas, &layout_spec.plot_area) {
                // Case 1: Fixed canvas size (traditional mode)
                (SizeMode::Fixed { width, height }, _) => {
                    // Update root to fixed size
                    let root_style = Style {
                        display: Display::Grid,
                        grid_template_columns: self.grid_layout.cols.clone(),
                        grid_template_rows: self.grid_layout.rows.clone(),
                        size: Size {
                            width: length(*width),
                            height: length(*height),
                        },
                        ..Default::default()
                    };
                    self.taffy.set_style(self.root_node, root_style)?;

                    (
                        AvailableSpace::Definite(*width),
                        AvailableSpace::Definite(*height),
                    )
                }
                // Case 2: Plot area drives layout
                (SizeMode::Auto, SizeMode::Fixed { width, height }) => {
                    // Set plot area to fixed size
                    if let Some(plot_node) = self.plot_area_node {
                        // Get current plot node style to preserve grid position
                        let current_style = self.taffy.style(plot_node)?;
                        let plot_style = Style {
                            display: Display::Block,
                            size: Size {
                                width: length(*width),
                                height: length(*height),
                            },
                            grid_row: current_style.grid_row,
                            grid_column: current_style.grid_column,
                            min_size: Size {
                                width: length(*width),
                                height: length(*height),
                            },
                            ..Default::default()
                        };
                        self.taffy.set_style(plot_node, plot_style)?;
                    }

                    // Update root node for content-based sizing
                    let root_style = Style {
                        display: Display::Grid,
                        grid_template_columns: self.grid_layout.cols.clone(),
                        grid_template_rows: self.grid_layout.rows.clone(),
                        size: Size {
                            width: auto(),
                            height: auto(),
                        },
                        ..Default::default()
                    };
                    self.taffy.set_style(self.root_node, root_style)?;

                    // Let canvas size be content-driven
                    (AvailableSpace::MinContent, AvailableSpace::MinContent)
                }
                // Default: Use sensible defaults
                _ => {
                    let root_style = Style {
                        display: Display::Grid,
                        grid_template_columns: self.grid_layout.cols.clone(),
                        grid_template_rows: self.grid_layout.rows.clone(),
                        size: Size {
                            width: length(400.0),
                            height: length(300.0),
                        },
                        ..Default::default()
                    };
                    self.taffy.set_style(self.root_node, root_style)?;
                    (
                        AvailableSpace::Definite(400.0),
                        AvailableSpace::Definite(300.0),
                    )
                }
            };

        // Compute layout
        self.taffy.compute_layout(
            self.root_node,
            Size {
                width: available_width,
                height: available_height,
            },
        )?;

        // Get the actual canvas size after layout
        let root_layout = self.taffy.layout(self.root_node)?;
        let canvas_size = (root_layout.size.width, root_layout.size.height);

        // Extract layout result
        let layout = self.extract_layout_result()?;

        Ok(super::sizing::ComputeResult {
            layout,
            canvas_size,
        })
    }

    /// Compute layout for given dimensions (legacy method)
    pub fn compute(&mut self, width: f32, height: f32) -> Result<LayoutResult, AvengerChartError> {
        // Use the new method with a fixed canvas size spec
        let spec = LayoutSpec {
            canvas: SizeMode::Fixed { width, height },
            plot_area: SizeMode::Auto,
            margins: super::sizing::Margins::default(),
        };
        let result = self.compute_with_spec(&spec)?;
        Ok(result.layout)
    }

    /// Extract computed positions from Taffy layout
    fn extract_layout_result(&self) -> Result<LayoutResult, AvengerChartError> {
        let mut result = LayoutResult {
            plot_area: LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            axes: HashMap::new(),
            legends: HashMap::new(),
            title: None,
            subtitle: None,
        };

        // Get plot area bounds
        if let Some(plot_node) = self.plot_area_node {
            let layout = self.taffy.layout(plot_node)?;
            debug!(
                x = layout.location.x,
                y = layout.location.y,
                width = layout.size.width,
                height = layout.size.height,
                "Plot area bounds"
            );
            result.plot_area = LayoutBounds {
                x: layout.location.x.round(),
                y: layout.location.y.round(),
                width: layout.size.width.round(),
                height: layout.size.height.round(),
            };
        }

        // Get guide overflow bounds (stored as axes for backward compatibility)
        for (position, node) in &self.guide_overflow_nodes {
            let layout = self.taffy.layout(*node)?;
            debug!(
                position = ?position,
                x = layout.location.x,
                y = layout.location.y,
                width = layout.size.width,
                height = layout.size.height,
                "Guide overflow bounds"
            );
            result.axes.insert(
                *position,
                LayoutBounds {
                    x: layout.location.x.round(),
                    y: layout.location.y.round(),
                    width: layout.size.width.round(),
                    height: layout.size.height.round(),
                },
            );
        }

        // Get legend bounds
        // Legends are children of containers, so we need to add container position to get absolute position
        // First, get container positions
        let mut container_positions = HashMap::new();
        for (position, container_node) in &self.legend_container_nodes {
            let container_layout = self.taffy.layout(*container_node)?;
            debug!(
                position = ?position,
                x = container_layout.location.x,
                y = container_layout.location.y,
                width = container_layout.size.width,
                height = container_layout.size.height,
                "Legend container bounds"
            );
            container_positions.insert(
                position,
                (container_layout.location.x, container_layout.location.y),
            );
        }

        // Now get legend bounds relative to their containers
        for (channel, legend_node) in &self.legend_nodes {
            let legend_layout = self.taffy.layout(*legend_node)?;
            if channel == "stroke" {
                debug!(
                    channel = channel,
                    x = legend_layout.location.x,
                    y = legend_layout.location.y,
                    width = legend_layout.size.width,
                    height = legend_layout.size.height,
                    "Individual legend node bounds"
                );
            }

            // Find which container this legend belongs to by checking the legend configuration
            // We need to determine the legend's position to know its container
            // For now, we'll need to iterate through containers to find the parent
            let mut absolute_x = legend_layout.location.x;
            let mut absolute_y = legend_layout.location.y;

            // Check if this legend is a child of any container
            for (position, container_node) in &self.legend_container_nodes {
                let children = self.taffy.children(*container_node)?;
                if children.contains(legend_node) {
                    // Found the parent container
                    if let Some((container_x, container_y)) = container_positions.get(position) {
                        absolute_x += container_x;
                        absolute_y += container_y;
                    }
                    break;
                }
            }

            result.legends.insert(
                channel.clone(),
                LayoutBounds {
                    x: absolute_x.round(),
                    y: absolute_y.round(),
                    width: legend_layout.size.width.round(),
                    height: legend_layout.size.height.round(),
                },
            );
        }

        // Get title bounds
        if let Some(title_node) = self.title_node {
            let layout = self.taffy.layout(title_node)?;
            result.title = Some(LayoutBounds {
                x: layout.location.x.round(),
                y: layout.location.y.round(),
                width: layout.size.width.round(),
                height: layout.size.height.round(),
            });
        }

        // Get subtitle bounds
        if let Some(subtitle_node) = self.subtitle_node {
            let layout = self.taffy.layout(subtitle_node)?;
            result.subtitle = Some(LayoutBounds {
                x: layout.location.x.round(),
                y: layout.location.y.round(),
                width: layout.size.width.round(),
                height: layout.size.height.round(),
            });
        }

        Ok(result)
    }

    /// Measure legend size with mark encodings and return flexibility preference
    pub fn measure_legend_size<C: CoordinateSystem>(
        channel: &str,
        legend: &Legend,
        scale: &ConfiguredScale,
        scales: &HashMap<String, ConfiguredScale>,
        available_space: Size<f32>,
        marks: &[Arc<dyn Mark<C>>],
    ) -> Result<(Size<f32>, bool), AvengerChartError> {
        measure_legend_size(channel, legend, scale, scales, available_space, marks)
    }
}
