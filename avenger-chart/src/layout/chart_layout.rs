//! Main ChartLayout struct and core implementation

use crate::cartesian::axis::AxisPosition;
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
use crate::plot::{PlotSubtitle, PlotTitle, TitleAlign};
use avenger_scales::scales::ConfiguredScale;
use indexmap::IndexMap;
use std::collections::HashMap;
use taffy::prelude::*;
use taffy::{NodeId, TaffyTree};
use tracing::debug;

use super::grid::{GridBuilder, GridTemplate};
use super::legend::measure_legend_size;
use super::types::{
    ComponentGridMap, ComponentType, LayoutBounds, LayoutResult, OVERFLOW_THRESHOLD,
};

/// Main chart layout manager
#[derive(Debug)]
pub struct ChartLayout {
    taffy: TaffyTree,
    root_node: NodeId,

    // Component nodes
    plot_area_node: Option<NodeId>,
    axis_nodes: HashMap<AxisPosition, NodeId>,
    legend_container_nodes: HashMap<LegendPosition, NodeId>, // Flex containers for each position
    legend_nodes: HashMap<String, NodeId>, // Individual legend nodes keyed by channel
    legend_sizes: HashMap<String, Size<f32>>, // Store measured sizes
    legend_flexible: HashMap<String, bool>, // Store whether legend prefers flexible layout
    title_node: Option<NodeId>,
    subtitle_node: Option<NodeId>,

    // Grid configuration
    grid_template: GridTemplate,
    component_map: ComponentGridMap,
    // Text properties reserved for future font customization
    #[allow(dead_code)] // Will be used when custom font support is added
    title_font_size: Option<f32>,
    #[allow(dead_code)] // Will be used when custom font support is added
    title_font_family: Option<String>,
    #[allow(dead_code)] // Will be used when custom font support is added
    subtitle_font_size: Option<f32>,
    #[allow(dead_code)] // Will be used when custom font support is added
    subtitle_font_family: Option<String>,
}

impl ChartLayout {
    /// Create a new ChartLayout with overflow space requirements
    /// This is the unified layout method for all coordinate systems
    pub fn new_with_overflow<C: crate::coords::CoordinateSystem>(
        overflow: &crate::coords::OverflowSpaceRequirement,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        preferred_size: Option<(f32, f32)>,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        marks: &[Box<dyn crate::marks::Mark<C>>],
        theme: &dyn crate::theme::Theme,
    ) -> Result<Self, AvengerChartError> {
        let mut taffy = TaffyTree::new();
        let mut builder = GridBuilder::new();

        // Build grid structure dynamically
        builder.add_plot_area(); // Always present

        // Add optional title and subtitle
        if title.is_some() {
            builder.add_title();
        }
        if subtitle.is_some() {
            builder.add_subtitle();
        }

        // Add overflow regions as needed (only if above threshold)
        if overflow.top > OVERFLOW_THRESHOLD {
            builder.add_axes_at_position(AxisPosition::Top, 1);
        }
        if overflow.bottom > OVERFLOW_THRESHOLD {
            builder.add_axes_at_position(AxisPosition::Bottom, 1);
        }
        if overflow.left > OVERFLOW_THRESHOLD {
            builder.add_axes_at_position(AxisPosition::Left, 1);
        }
        if overflow.right > OVERFLOW_THRESHOLD {
            builder.add_axes_at_position(AxisPosition::Right, 1);
        }

        // Add legends
        for (channel, legend) in legends.iter() {
            builder.add_legend(channel.clone(), legend);
        }

        // Finalize legend containers after all legends are added
        builder.finalize_legend_containers();

        // Generate grid template with overflow measurements
        let available_size = preferred_size.unwrap_or((400.0, 300.0));
        let (grid_template, component_map) = builder.build_with_overflow(
            overflow,
            legends,
            scales,
            Size {
                width: available_size.0,
                height: available_size.1,
            },
            marks,
            title,
            subtitle,
            theme,
        )?;

        // Create root node with grid layout
        let root_style = Style {
            display: Display::Grid,
            grid_template_columns: grid_template.cols.clone(),
            grid_template_rows: grid_template.rows.clone(),
            size: if let Some((width, height)) = preferred_size {
                Size {
                    width: length(width),
                    height: length(height),
                }
            } else {
                Size {
                    width: length(400.0),
                    height: length(300.0),
                }
            },
            ..Default::default()
        };

        let root_node = taffy.new_leaf(root_style)?;

        let mut layout = ChartLayout {
            taffy,
            root_node,
            plot_area_node: None,
            axis_nodes: HashMap::new(),
            legend_container_nodes: HashMap::new(),
            legend_nodes: HashMap::new(),
            legend_sizes: HashMap::new(),
            legend_flexible: HashMap::new(),
            title_node: None,
            subtitle_node: None,
            grid_template,
            component_map,
            title_font_size: title.and_then(|t| t.font_size),
            title_font_family: title.and_then(|t| t.font_family.clone()),
            subtitle_font_size: subtitle.and_then(|s| s.font_size),
            subtitle_font_family: subtitle.and_then(|s| s.font_family.clone()),
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
        for ((row, col), comp) in &self.component_map.cells {
            if std::mem::discriminant(comp) == std::mem::discriminant(component) {
                // For axis positions, check specific position match
                if let (ComponentType::Axis(pos1), ComponentType::Axis(pos2)) = (comp, component) {
                    if pos1 == pos2 {
                        return Some((*row, *col));
                    }
                } else {
                    return Some((*row, *col));
                }
            }
        }
        None
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

        for ((row, col), comp_type) in &self.component_map.cells {
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
        if overflow.left > 0.0 {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::Axis(AxisPosition::Left))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.axis_nodes.insert(AxisPosition::Left, node);
            }
        }

        if overflow.right > 0.0 {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::Axis(AxisPosition::Right))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.axis_nodes.insert(AxisPosition::Right, node);
            }
        }

        if overflow.top > OVERFLOW_THRESHOLD {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::Axis(AxisPosition::Top))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.axis_nodes.insert(AxisPosition::Top, node);
            }
        }

        if overflow.bottom > OVERFLOW_THRESHOLD {
            if let Some((row, col)) =
                self.find_component_position(&ComponentType::Axis(AxisPosition::Bottom))
            {
                let style = Style {
                    display: Display::Block,
                    grid_row: line((row + 1) as i16), // Convert to 1-based
                    grid_column: line((col + 1) as i16), // Convert to 1-based
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.axis_nodes.insert(AxisPosition::Bottom, node);
            }
        }

        // Helper function to calculate grid column based on TitleAlign
        let calculate_grid_column = |col: usize, align: TitleAlign| {
            match align {
                TitleAlign::PlotAreaOnly => {
                    // Only span the plot area column
                    let mut plot_col = col;
                    for ((_, c), comp) in &self.component_map.cells {
                        if matches!(comp, ComponentType::PlotArea) {
                            plot_col = *c;
                            break;
                        }
                    }
                    line((plot_col + 1) as i16)
                }
                TitleAlign::FullWidth => {
                    // Find the rightmost column that isn't padding
                    let mut end_col = col;
                    for ((_, c), comp) in &self.component_map.cells {
                        // Include all component types except padding
                        if !matches!(comp, ComponentType::Padding) && *c > end_col {
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

            for ((row, col), comp_type) in &self.component_map.cells {
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
        for node in self.axis_nodes.values() {
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

    /// Compute layout for given dimensions
    pub fn compute(&mut self, width: f32, height: f32) -> Result<LayoutResult, AvengerChartError> {
        // Update root node size
        let root_style = Style {
            display: Display::Grid,
            grid_template_columns: self.grid_template.cols.clone(),
            grid_template_rows: self.grid_template.rows.clone(),
            size: Size {
                width: length(width),
                height: length(height),
            },
            ..Default::default()
        };
        self.taffy.set_style(self.root_node, root_style)?;

        // Compute layout
        self.taffy.compute_layout(
            self.root_node,
            Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
        )?;

        // Extract computed positions
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
            total_bounds: LayoutBounds {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
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

        // Get axis bounds
        for (position, node) in &self.axis_nodes {
            let layout = self.taffy.layout(*node)?;
            debug!(
                position = ?position,
                x = layout.location.x,
                y = layout.location.y,
                width = layout.size.width,
                height = layout.size.height,
                "Axis bounds"
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
    pub fn measure_legend_size<C: crate::coords::CoordinateSystem>(
        channel: &str,
        legend: &Legend,
        scale: &ConfiguredScale,
        scales: &HashMap<String, ConfiguredScale>,
        available_space: Size<f32>,
        marks: &[Box<dyn crate::marks::Mark<C>>],
    ) -> Result<(Size<f32>, bool), AvengerChartError> {
        measure_legend_size(channel, legend, scale, scales, available_space, marks)
    }
}
