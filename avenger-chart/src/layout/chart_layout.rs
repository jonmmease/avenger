//! Main ChartLayout struct and core implementation

use super::grid::{GridBuilder, GridLayout};
use super::sizing::{LayoutSpec, SizeMode};
use super::types::{ComponentType, LayoutBounds, LayoutResult, OVERFLOW_THRESHOLD, OverflowSide};
use crate::cartesian::axis::AxisPosition;
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
use crate::plot::{PlotSubtitle, PlotTitle, TitleAlign};
use crate::render::LegendMeasurements;
use indexmap::IndexMap;
use std::collections::HashMap;
use taffy::prelude::*;
use taffy::{NodeId, TaffyTree};
use tracing::debug;

/// Structure to hold all the node IDs from the TaffyTree
#[derive(Debug, Clone)]
pub(crate) struct TaffyNodes {
    pub root_node: NodeId,
    pub plot_area_node: Option<NodeId>,
    pub guide_overflow_nodes: HashMap<AxisPosition, NodeId>,
    pub legend_container_nodes: HashMap<LegendPosition, NodeId>,
    pub legend_nodes: HashMap<String, NodeId>,
    pub title_node: Option<NodeId>,
    pub subtitle_node: Option<NodeId>,
}

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
    nodes: TaffyNodes,
    grid_layout: GridLayout,
}

impl ChartLayout {
    /// Compute layout with flexible sizing based on LayoutSpec
    pub(crate) fn compute(
        &mut self,
        layout_spec: &LayoutSpec,
    ) -> Result<crate::render::LayoutSolution, AvengerChartError> {
        // Normalize the layout spec to handle special cases
        // The layout spec should already have canvas and plot_area fields set from the mode
        let normalized_spec = match (&layout_spec.canvas, &layout_spec.plot_area) {
            // Case 1: Both canvas and plot area are Auto - use default 400x300
            (SizeMode::Auto, SizeMode::Auto) => {
                let mut spec = layout_spec.clone();
                spec.canvas = SizeMode::Fixed {
                    width: 400.0,
                    height: 300.0,
                };
                spec
            }
            // Case 2: Canvas aspect ratio with Auto plot area - compute canvas size
            (SizeMode::AspectRatio(ratio), SizeMode::Auto) => {
                let width = 400.0;
                let height = width / ratio;
                let mut spec = layout_spec.clone();
                spec.canvas = SizeMode::Fixed { width, height };
                spec
            }
            // Case 3: Canvas aspect ratio with fixed plot area
            // Keep as-is - Taffy should handle this with proper constraints
            (SizeMode::AspectRatio(_), SizeMode::Fixed { .. }) => layout_spec.clone(),
            // All other cases: use as-is
            _ => layout_spec.clone(),
        };

        // Step 1: Configure canvas (root) dimensions
        let (canvas_width, canvas_height, canvas_aspect) = match &normalized_spec.canvas {
            SizeMode::Fixed { width, height } => (length(*width), length(*height), None),
            SizeMode::Width(w) => (length(*w), auto(), None),
            SizeMode::Height(h) => (auto(), length(*h), None),
            SizeMode::AspectRatio(ratio) => (auto(), auto(), Some(*ratio)),
            SizeMode::Auto => (auto(), auto(), None),
        };

        let root_style = Style {
            display: Display::Grid,
            grid_template_columns: self.grid_layout.cols.clone(),
            grid_template_rows: self.grid_layout.rows.clone(),
            size: Size {
                width: canvas_width,
                height: canvas_height,
            },
            aspect_ratio: canvas_aspect,
            ..Default::default()
        };
        self.taffy.set_style(self.nodes.root_node, root_style)?;

        // Step 2: Configure plot area dimensions
        if let Some(plot_node) = self.nodes.plot_area_node {
            let current_style = self.taffy.style(plot_node)?;

            let (plot_width, plot_height, aspect_ratio) = match &normalized_spec.plot_area {
                SizeMode::Fixed { width, height } => (length(*width), length(*height), None),
                SizeMode::Width(w) => (length(*w), auto(), None),
                SizeMode::Height(h) => (auto(), length(*h), None),
                // For aspect ratio, set width to 100% and let height be determined by ratio
                SizeMode::AspectRatio(ratio) => (auto(), auto(), Some(*ratio)),
                SizeMode::Auto => (auto(), auto(), None),
            };

            let plot_style = Style {
                display: Display::Block,
                size: Size {
                    width: plot_width,
                    height: plot_height,
                },
                aspect_ratio,
                grid_row: current_style.grid_row,
                grid_column: current_style.grid_column,
                flex_grow: 1.0,
                flex_shrink: 1.0,
                // Ensure minimum size for fixed dimensions
                // Use the same value as size if it's fixed, otherwise use default minimum
                min_size: Size {
                    width: match &normalized_spec.plot_area {
                        SizeMode::Fixed { width, .. } | SizeMode::Width(width) => length(*width),
                        _ => length(50.0),
                    },
                    height: match &normalized_spec.plot_area {
                        SizeMode::Fixed { height, .. } | SizeMode::Height(height) => {
                            length(*height)
                        }
                        _ => length(50.0),
                    },
                },
                ..Default::default()
            };
            self.taffy.set_style(plot_node, plot_style)?;
        }

        // Step 3: Determine available space for layout computation
        // If dimension is fixed, use Definite; otherwise use MaxContent for aspect ratio, MinContent otherwise
        let available_width = match &normalized_spec.canvas {
            SizeMode::Fixed { width, .. } | SizeMode::Width(width) => {
                AvailableSpace::Definite(*width)
            }
            SizeMode::AspectRatio(_) => AvailableSpace::MaxContent,
            _ => AvailableSpace::MinContent,
        };

        let available_height = match &normalized_spec.canvas {
            SizeMode::Fixed { height, .. } | SizeMode::Height(height) => {
                AvailableSpace::Definite(*height)
            }
            SizeMode::AspectRatio(_) => AvailableSpace::MaxContent,
            _ => AvailableSpace::MinContent,
        };

        // Compute layout
        self.taffy.compute_layout(
            self.nodes.root_node,
            Size {
                width: available_width,
                height: available_height,
            },
        )?;

        // Get the actual canvas size after layout
        let root_layout = self.taffy.layout(self.nodes.root_node)?;
        let canvas_size = (root_layout.size.width, root_layout.size.height);

        // Extract layout result
        let taffy_layout = self.extract_layout_result()?;

        Ok(crate::render::LayoutSolution {
            taffy_layout,
            canvas_size,
        })
    }

    /// Create a new ChartLayout with overflow space requirements
    /// This is the unified layout method for all coordinate systems
    pub fn new_with_overflow(
        overflow: &crate::coords::OverflowSpaceRequirement,
        legends: &IndexMap<String, Legend>,
        layout_spec: &LayoutSpec,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        theme: &dyn crate::theme::Theme,
        legend_measurements: &LegendMeasurements,
    ) -> Result<Self, AvengerChartError> {
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

        // Add legends with their positions
        for (channel, legend) in legends.iter() {
            let position = legend.position.unwrap_or(LegendPosition::Right);
            builder.add_legend(channel.clone(), position);
        }

        // Extract sizes for grid building
        let legend_sizes: HashMap<String, Size<f32>> = legend_measurements
            .iter()
            .map(|(k, (size, _))| (k.clone(), *size))
            .collect();

        // Generate grid template with overflow measurements
        let grid_layout = builder.build_with_overflow(
            overflow,
            title,
            subtitle,
            theme,
            layout_spec,
            &legend_sizes,
        )?;

        // Extract flexible flags for taffy tree building
        let legend_flexible: HashMap<String, bool> = legend_measurements
            .iter()
            .map(|(k, (_, flexible))| (k.clone(), *flexible))
            .collect();

        // Build the TaffyTree and get all the nodes using the pure function
        let (taffy, nodes) = build_taffy_tree(
            &grid_layout,
            overflow,
            legends,
            &legend_sizes,
            &legend_flexible,
            title,
            subtitle,
        )?;

        // Create the ChartLayout with the built tree and nodes
        let layout = ChartLayout {
            taffy,
            nodes,
            grid_layout,
        };

        Ok(layout)
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
        if let Some(plot_node) = self.nodes.plot_area_node {
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
        for (position, node) in &self.nodes.guide_overflow_nodes {
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
        for (position, container_node) in &self.nodes.legend_container_nodes {
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
        for (channel, legend_node) in &self.nodes.legend_nodes {
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
            for (position, container_node) in &self.nodes.legend_container_nodes {
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
        if let Some(title_node) = self.nodes.title_node {
            let layout = self.taffy.layout(title_node)?;
            result.title = Some(LayoutBounds {
                x: layout.location.x.round(),
                y: layout.location.y.round(),
                width: layout.size.width.round(),
                height: layout.size.height.round(),
            });
        }

        // Get subtitle bounds
        if let Some(subtitle_node) = self.nodes.subtitle_node {
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
}

/// Pure function to build a TaffyTree with all component nodes
fn build_taffy_tree(
    grid_layout: &GridLayout,
    overflow: &crate::coords::OverflowSpaceRequirement,
    legends: &IndexMap<String, Legend>,
    legend_sizes: &HashMap<String, Size<f32>>,
    legend_flexible: &HashMap<String, bool>,
    title: Option<&PlotTitle>,
    subtitle: Option<&PlotSubtitle>,
) -> Result<(TaffyTree, TaffyNodes), AvengerChartError> {
    let mut taffy = TaffyTree::new();

    // Create root node with grid layout
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

    // Initialize node tracking structure
    let mut nodes = TaffyNodes {
        root_node,
        plot_area_node: None,
        guide_overflow_nodes: HashMap::new(),
        legend_container_nodes: HashMap::new(),
        legend_nodes: HashMap::new(),
        title_node: None,
        subtitle_node: None,
    };

    // Find plot area position and create its node
    let mut plot_row = 0;
    let mut plot_col = 0;
    for ((row, col), comp_type) in &grid_layout.component_cells {
        if matches!(comp_type, ComponentType::PlotArea) {
            plot_row = *row;
            plot_col = *col;
            break;
        }
    }

    let plot_style = Style {
        display: Display::Block,
        grid_row: line((plot_row + 1) as i16),
        grid_column: line((plot_col + 1) as i16),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        min_size: Size {
            width: length(50.0),
            height: length(50.0),
        },
        ..Default::default()
    };
    nodes.plot_area_node = Some(taffy.new_leaf(plot_style)?);

    // Create overflow nodes
    create_overflow_nodes(&mut taffy, &mut nodes, grid_layout, overflow)?;

    // Create title and subtitle nodes
    create_title_nodes(&mut taffy, &mut nodes, grid_layout, title, subtitle)?;

    // Create legend nodes
    create_legend_nodes(
        &mut taffy,
        &mut nodes,
        grid_layout,
        legends,
        legend_sizes,
        legend_flexible,
    )?;

    // Set all children on root
    let mut children = Vec::new();
    if let Some(node) = nodes.plot_area_node {
        children.push(node);
    }
    for node in nodes.guide_overflow_nodes.values() {
        children.push(*node);
    }
    for node in nodes.legend_container_nodes.values() {
        children.push(*node);
    }
    if let Some(node) = nodes.title_node {
        children.push(node);
    }
    if let Some(node) = nodes.subtitle_node {
        children.push(node);
    }
    taffy.set_children(root_node, &children)?;

    Ok((taffy, nodes))
}

/// Helper function to create overflow nodes
fn create_overflow_nodes(
    taffy: &mut TaffyTree,
    nodes: &mut TaffyNodes,
    grid_layout: &GridLayout,
    overflow: &crate::coords::OverflowSpaceRequirement,
) -> Result<(), AvengerChartError> {
    // Helper to create a single overflow node
    let mut create_node = |overflow_value: f32,
                           overflow_side: OverflowSide,
                           axis_position: AxisPosition|
     -> Result<(), AvengerChartError> {
        if overflow_value > OVERFLOW_THRESHOLD {
            if let Some((row, col)) =
                grid_layout.find_component_position(&ComponentType::GuideOverflow(overflow_side))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16),
                    grid_column: line((col + 1) as i16),
                    ..Default::default()
                };
                let node = taffy.new_leaf(style)?;
                nodes.guide_overflow_nodes.insert(axis_position, node);
            }
        }
        Ok(())
    };

    create_node(overflow.left, OverflowSide::Left, AxisPosition::Left)?;
    create_node(overflow.right, OverflowSide::Right, AxisPosition::Right)?;
    create_node(overflow.top, OverflowSide::Top, AxisPosition::Top)?;
    create_node(overflow.bottom, OverflowSide::Bottom, AxisPosition::Bottom)?;

    Ok(())
}

/// Helper function to create title and subtitle nodes
fn create_title_nodes(
    taffy: &mut TaffyTree,
    nodes: &mut TaffyNodes,
    grid_layout: &GridLayout,
    title: Option<&PlotTitle>,
    subtitle: Option<&PlotSubtitle>,
) -> Result<(), AvengerChartError> {
    // Helper function to calculate grid column based on TitleAlign
    let calculate_grid_column =
        |col: usize, align: TitleAlign, grid_layout: &GridLayout| match align {
            TitleAlign::PlotAreaOnly => {
                let mut plot_col = col;
                for ((_, c), comp) in &grid_layout.component_cells {
                    if matches!(comp, ComponentType::PlotArea) {
                        plot_col = *c;
                        break;
                    }
                }
                line((plot_col + 1) as i16)
            }
            TitleAlign::FullWidth => {
                let mut end_col = col;
                for ((_, c), _comp) in &grid_layout.component_cells {
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
        };

    // Create title node
    if let Some((row, col)) = grid_layout.find_component_position(&ComponentType::Title) {
        let grid_col = if let Some(t) = title {
            calculate_grid_column(col, t.align, grid_layout)
        } else {
            line((col + 1) as i16)
        };

        let style = Style {
            display: Display::Block,
            grid_row: line((row + 1) as i16),
            grid_column: grid_col,
            ..Default::default()
        };
        nodes.title_node = Some(taffy.new_leaf(style)?);
    }

    // Create subtitle node
    if let Some((row, col)) = grid_layout.find_component_position(&ComponentType::Subtitle) {
        let grid_col = if let Some(s) = subtitle {
            calculate_grid_column(col, s.align, grid_layout)
        } else {
            line((col + 1) as i16)
        };

        let style = Style {
            display: Display::Block,
            grid_row: line((row + 1) as i16),
            grid_column: grid_col,
            ..Default::default()
        };
        nodes.subtitle_node = Some(taffy.new_leaf(style)?);
    }

    Ok(())
}

/// Helper function to create legend nodes
fn create_legend_nodes(
    taffy: &mut TaffyTree,
    nodes: &mut TaffyNodes,
    grid_layout: &GridLayout,
    legends: &IndexMap<String, Legend>,
    legend_sizes: &HashMap<String, Size<f32>>,
    legend_flexible: &HashMap<String, bool>,
) -> Result<(), AvengerChartError> {
    let mut legend_nodes_by_container: HashMap<LegendPosition, Vec<NodeId>> = HashMap::new();

    for (channel, legend) in legends {
        let legend_position = legend.position.unwrap_or(LegendPosition::Right);

        for ((row, col), comp_type) in &grid_layout.component_cells {
            if let ComponentType::LegendContainer(pos) = comp_type {
                if *pos == legend_position {
                    // Create container node if it doesn't exist
                    if !nodes.legend_container_nodes.contains_key(pos) {
                        let container_style = Style {
                            display: Display::Flex,
                            flex_direction: FlexDirection::Column,
                            grid_row: line((*row + 1) as i16),
                            grid_column: line((*col + 1) as i16),
                            ..Default::default()
                        };
                        let container_node = taffy.new_leaf(container_style)?;
                        nodes.legend_container_nodes.insert(*pos, container_node);
                    }

                    // Create legend node
                    if let Some(size) = legend_sizes.get(channel) {
                        let is_flexible = legend_flexible.get(channel).copied().unwrap_or(false);

                        let legend_style = if is_flexible {
                            Style {
                                display: Display::Block,
                                size: Size {
                                    width: length(size.width),
                                    height: auto(),
                                },
                                flex_grow: 1.0,
                                flex_shrink: 1.0,
                                min_size: Size {
                                    width: length(size.width),
                                    height: length(50.0),
                                },
                                ..Default::default()
                            }
                        } else {
                            Style {
                                display: Display::Block,
                                size: Size {
                                    width: length(size.width),
                                    height: length(size.height),
                                },
                                ..Default::default()
                            }
                        };

                        let legend_node = taffy.new_leaf(legend_style)?;
                        nodes.legend_nodes.insert(channel.clone(), legend_node);

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
        if let Some(container_node) = nodes.legend_container_nodes.get(&position) {
            taffy.set_children(*container_node, &legend_children)?;
        }
    }

    Ok(())
}
