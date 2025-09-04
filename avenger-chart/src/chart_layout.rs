use crate::axis::AxisPosition;
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
use crate::plot::{PlotSubtitle, PlotTitle, TitleAlign};
use avenger_scales::scales::ConfiguredScale;
use indexmap::IndexMap;
use tracing::debug;
// Use stable ordering by iterating sorted keys, not map type
use std::collections::HashMap;
use taffy::prelude::*;
use taffy::{NodeId, TaffyTree};

const OVERFLOW_THRESHOLD: f32 = 2.0;
const EDGE_MARGIN: f32 = 10.0;

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

/// Maps components to their grid positions
#[derive(Debug)]
struct ComponentGridMap {
    // Track which grid cells are occupied by which components
    cells: HashMap<(usize, usize), ComponentType>,

    // Dynamic grid dimensions
    row_count: usize,
    col_count: usize,
}

impl ComponentGridMap {
    fn new() -> Self {
        ComponentGridMap {
            cells: HashMap::new(),
            row_count: 0,
            col_count: 0,
        }
    }

    fn add_component(&mut self, component: ComponentType, row: usize, col: usize) {
        self.cells.insert((row, col), component);
        self.row_count = self.row_count.max(row + 1);
        self.col_count = self.col_count.max(col + 1);
    }
}

#[derive(Debug, Clone)]
enum ComponentType {
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

#[derive(Debug, Clone, Copy)]
pub struct LayoutBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Builder for dynamically constructing grid layout
struct GridBuilder {
    components: Vec<(ComponentType, GridPlacement)>,

    // Track components by position for dynamic grid building
    left_components: Vec<ComponentType>, // Order: axis first, then legend container
    right_components: Vec<ComponentType>, // Order: axis first, then legend container
    top_components: Vec<ComponentType>,  // Order: axis first, then legend container, then title
    bottom_components: Vec<ComponentType>, // Order: axis first, then legend container

    // Track legends by position for container creation
    legends_by_position: IndexMap<LegendPosition, Vec<(String, Legend)>>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct GridPlacement {
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
}

#[derive(Debug)]
struct GridTemplate {
    rows: Vec<TrackSizingFunction>,
    cols: Vec<TrackSizingFunction>,
}

impl ChartLayout {
    /// Measure the actual text to get accurate bounds
    /// Returns (height, width) for the measured text
    fn measure_text(text: &str, font_size: f32, font_family: &str) -> (f32, f32) {
        use avenger_text::measurement::cosmic::CosmicTextMeasurer;
        use avenger_text::measurement::{TextMeasurementConfig, TextMeasurer};
        use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec};

        let measurer = CosmicTextMeasurer::new();

        let config = TextMeasurementConfig {
            text,
            font: font_family,
            font_size,
            font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
            font_style: &FontStyle::Normal,
        };

        let bounds = measurer.measure_text_bounds(&config);

        // Return height with padding and width
        // Add 10% padding to line height for visual breathing room
        // This matches the default line spacing in most typography systems
        (bounds.line_height * 1.1, bounds.width)
    }

    /// Create a new chart layout with default axes and legends included
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
            title_node: None,
            subtitle_node: None,
            grid_template,
            component_map,
            title_font_size: title.map(|t| t.font_size),
            title_font_family: title.map(|t| t.font_family.clone()),
            subtitle_font_size: subtitle.map(|s| s.font_size),
            subtitle_font_family: subtitle.map(|s| s.font_family.clone()),
        };

        // Measure legend sizes
        let mut legend_sizes = HashMap::new();
        for (channel, legend) in legends.iter() {
            if let Some(scale) = scales.get(channel) {
                let size = ChartLayout::measure_legend_size(
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
            }
        }
        layout.legend_sizes = legend_sizes;

        // Create nodes for each component in the grid
        layout.create_component_nodes_with_overflow(overflow, legends, scales, title, subtitle)?;

        Ok(layout)
    }

    /// Create taffy nodes for each component
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
        scales: &HashMap<String, ConfiguredScale>,
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

        const OVERFLOW_THRESHOLD: f32 = 2.0;
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
                            // Check if this is a colorbar legend (continuous color scale)
                            let is_colorbar = if let Some(scale) = scales.get(channel) {
                                let is_color_channel =
                                    matches!(channel.as_str(), "fill" | "stroke" | "color");
                                let scale_type = scale.scale_impl.scale_type();
                                let is_continuous = matches!(
                                    scale_type,
                                    "linear" | "log" | "pow" | "sqrt" | "symlog"
                                );
                                is_color_channel && is_continuous
                            } else {
                                false
                            };

                            let legend_style = if is_colorbar {
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
                x: layout.location.x,
                y: layout.location.y,
                width: layout.size.width,
                height: layout.size.height,
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
                    x: layout.location.x,
                    y: layout.location.y,
                    width: layout.size.width,
                    height: layout.size.height,
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
                    x: absolute_x,
                    y: absolute_y,
                    width: legend_layout.size.width,
                    height: legend_layout.size.height,
                },
            );
        }

        // Get title bounds
        if let Some(title_node) = self.title_node {
            let layout = self.taffy.layout(title_node)?;
            result.title = Some(LayoutBounds {
                x: layout.location.x,
                y: layout.location.y,
                width: layout.size.width,
                height: layout.size.height,
            });
        }

        // Get subtitle bounds
        if let Some(subtitle_node) = self.subtitle_node {
            let layout = self.taffy.layout(subtitle_node)?;
            result.subtitle = Some(LayoutBounds {
                x: layout.location.x,
                y: layout.location.y,
                width: layout.size.width,
                height: layout.size.height,
            });
        }

        Ok(result)
    }

    /// Measure legend size with mark encodings
    pub fn measure_legend_size<C: crate::coords::CoordinateSystem>(
        channel: &str,
        legend: &Legend,
        scale: &ConfiguredScale,
        scales: &HashMap<String, ConfiguredScale>,
        available_space: Size<f32>,
        marks: &[Box<dyn crate::marks::Mark<C>>],
    ) -> Result<Size<f32>, AvengerChartError> {
        use crate::legend_renderer::LegendChannel;

        // Skip invisible legends
        if !legend.visible {
            return Ok(Size {
                width: 0.0,
                height: 0.0,
            });
        }

        // Find the mark that has this channel
        let mark_with_channel = marks
            .iter()
            .find(|m| m.data_context().channels().contains_key(channel))
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Channel '{}' not found in any mark",
                    channel
                ))
            })?;

        // Get the renderer - either explicitly configured or from the mark
        let renderer = if let Some(ref renderer) = legend.renderer {
            // Use explicitly configured renderer
            renderer.clone()
        } else {
            // Get the mark's preferred renderer for this channel
            mark_with_channel
                .preferred_legend_renderer(channel, scale)
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "No legend renderer available for channel '{}'",
                        channel
                    ))
                })?
        };

        // Collect related channels from the mark (needed for correct size constants)
        use crate::legend_renderer::ChannelInfo;
        let mut related_channels = HashMap::new();
        for (other_name, other_value) in mark_with_channel.data_context().channels() {
            if other_name != channel {
                // Check if this channel has a scale or is constant
                let channel_info = if let Some(other_scale) = scales.get(other_name) {
                    // Channel has a scale
                    ChannelInfo::Scaled {
                        expr: other_value.expr().cloned(),
                        scale: other_scale.clone(),
                    }
                } else if let Some(expr) = other_value.expr() {
                    // Channel has a constant expression
                    ChannelInfo::Constant { expr: expr.clone() }
                } else {
                    // Skip channels without expressions
                    continue;
                };
                related_channels.insert(other_name.clone(), channel_info);
            }
        }

        // Create legend channels for all merged channels, not just the primary one
        let mut legend_channels = Vec::new();

        // Always add the primary channel
        legend_channels.push(LegendChannel {
            name: channel.to_string(),
            expression: mark_with_channel
                .data_context()
                .channels()
                .get(channel)
                .and_then(|v| v.expr().cloned()),
            scale: scale.clone(),
            channel_type: channel.to_string(),
            mark_type: mark_with_channel.mark_type().to_string(),
            mark_id: mark_with_channel.mark_id(),
            related_channels: related_channels.clone(),
        });

        // Add any merged channels
        for merged_channel_name in &legend.merged_channels {
            if merged_channel_name != channel {
                // Only add if this channel exists in the mark
                if let Some(channel_value) = mark_with_channel
                    .data_context()
                    .channels()
                    .get(merged_channel_name)
                {
                    if let Some(merged_scale) = scales.get(merged_channel_name) {
                        legend_channels.push(LegendChannel {
                            name: merged_channel_name.clone(),
                            expression: channel_value.expr().cloned(),
                            scale: merged_scale.clone(),
                            channel_type: merged_channel_name.clone(),
                            mark_type: mark_with_channel.mark_type().to_string(),
                            mark_id: mark_with_channel.mark_id(),
                            related_channels: related_channels.clone(),
                        });
                    }
                }
            }
        }

        // Ask the renderer to measure itself with all merged channels
        renderer.measure(&legend_channels, legend, available_space)
    }
}

impl GridBuilder {
    fn new() -> Self {
        GridBuilder {
            components: Vec::new(),
            left_components: Vec::new(),
            right_components: Vec::new(),
            top_components: Vec::new(),
            bottom_components: Vec::new(),
            legends_by_position: IndexMap::new(),
        }
    }

    fn add_plot_area(&mut self) {
        // Plot area will be positioned dynamically based on components
        self.components.push((
            ComponentType::PlotArea,
            GridPlacement {
                row: 0, // Will be calculated dynamically
                col: 0, // Will be calculated dynamically
                row_span: 1,
                col_span: 1,
            },
        ));
    }

    fn add_title(&mut self) {
        // Title sits at the top area before axes/legends
        self.top_components.insert(0, ComponentType::Title);
    }

    fn add_subtitle(&mut self) {
        // Subtitle sits directly after the title
        // Find the position after the title if it exists, otherwise at the beginning
        let insert_pos = self
            .top_components
            .iter()
            .position(|c| matches!(c, ComponentType::Title))
            .map(|pos| pos + 1)
            .unwrap_or(0);
        self.top_components
            .insert(insert_pos, ComponentType::Subtitle);
    }

    fn add_axes_at_position(&mut self, position: AxisPosition, _count: usize) {
        let component = ComponentType::Axis(position);
        match position {
            AxisPosition::Left => {
                // Insert axis at beginning (closest to plot)
                self.left_components.insert(0, component);
            }
            AxisPosition::Right => {
                // Insert axis at beginning (closest to plot)
                self.right_components.insert(0, component);
            }
            AxisPosition::Top => {
                // Insert axis at end (closest to plot)
                self.top_components.push(component);
            }
            AxisPosition::Bottom => {
                // Insert axis at beginning (closest to plot)
                self.bottom_components.insert(0, component);
            }
        }
    }

    fn add_legend(&mut self, channel: String, legend: &Legend) {
        let position = legend.position.unwrap_or(LegendPosition::Right);

        // Collect legends by position for later container creation
        self.legends_by_position
            .entry(position)
            .or_default()
            .push((channel, legend.clone()));
    }

    fn finalize_legend_containers(&mut self) {
        // Create a container component for each position that has legends
        for position in self.legends_by_position.keys() {
            let component = ComponentType::LegendContainer(*position);

            match position {
                LegendPosition::Right => {
                    // Add container after axes (farther from plot)
                    self.right_components.push(component);
                }
                LegendPosition::Left => {
                    // Add container after axes (farther from plot)
                    self.left_components.push(component);
                }
                LegendPosition::Top => {
                    // Add container before axes (farther from plot)
                    self.top_components.insert(0, component);
                }
                LegendPosition::Bottom => {
                    // Add container after axes (farther from plot)
                    self.bottom_components.push(component);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn measure_legend_container_width<C: crate::coords::CoordinateSystem>(
        &self,
        channels: &[String],
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        marks: &[Box<dyn crate::marks::Mark<C>>],
    ) -> Result<f32, AvengerChartError> {
        let mut max_width: f32 = 0.0;
        for channel in channels {
            if let Some(legend) = legends.get(channel) {
                if let Some(scale) = scales.get(channel) {
                    // Use the actual measured legend size
                    // For now use a dummy available space - legends will adapt
                    let available = Size {
                        width: 200.0,
                        height: 400.0,
                    };
                    let size = ChartLayout::measure_legend_size(
                        channel, legend, scale, scales, available, marks,
                    )?;
                    max_width = max_width.max(size.width);
                }
            }
        }
        Ok(max_width)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_with_overflow<C: crate::coords::CoordinateSystem>(
        &self,
        overflow: &crate::coords::OverflowSpaceRequirement,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        _available_space: Size<f32>,
        _marks: &[Box<dyn crate::marks::Mark<C>>],
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
    ) -> Result<(GridTemplate, ComponentGridMap), AvengerChartError> {
        // Use edge margins from constants to ensure consistent padding
        let mut cols = Vec::new();
        let mut rows = Vec::new();
        let mut component_map = ComponentGridMap::new();

        // === Build Column Template ===
        // Start with left margin
        cols.push(length(EDGE_MARGIN));
        let mut col_index = 1;

        // Track column index for left overflow
        // Only create overflow column if it's more than a minimal threshold
        const OVERFLOW_THRESHOLD: f32 = 2.0; // Minimum pixels to create an overflow region
        let left_overflow_col = if overflow.left > OVERFLOW_THRESHOLD {
            cols.push(length(overflow.left)); // Exact overflow size
            let idx = col_index;
            col_index += 1;
            Some(idx)
        } else {
            None
        };

        // Plot area column (flexible)
        let plot_col_index = col_index;
        cols.push(fr(1.0));
        col_index += 1;

        // Track column index for right overflow
        let right_overflow_col = if overflow.right > OVERFLOW_THRESHOLD {
            cols.push(length(overflow.right)); // Exact overflow size
            let idx = col_index;
            col_index += 1;
            Some(idx)
        } else {
            None
        };

        // Add right legend containers and track their column indices
        let mut right_legend_cols = Vec::new();
        for component in &self.right_components {
            if let ComponentType::LegendContainer(position) = component {
                if *position == LegendPosition::Right {
                    // Get channels from the merged legends map (not legends_by_position which has unmerged channels)
                    // Filter legends by position to get only those in this container
                    let channels: Vec<String> = legends
                        .iter()
                        .filter(|(_, legend)| {
                            legend.position.unwrap_or(LegendPosition::Right) == *position
                        })
                        .map(|(ch, _)| ch.clone())
                        .collect();
                    let width =
                        self.measure_legend_container_width(&channels, legends, scales, _marks)?;
                    cols.push(length(width));
                    right_legend_cols.push(col_index);
                    col_index += 1;
                }
            }
        }

        // End with right margin
        cols.push(length(EDGE_MARGIN));

        // === Build Row Template ===
        rows.push(length(EDGE_MARGIN));
        let mut row_index = 1;

        // Add title if present
        if let Some(t) = title {
            let (height, _) = ChartLayout::measure_text(&t.text, t.font_size, &t.font_family);
            rows.push(length(height * 1.15));
            // Title starts from left overflow column (if present) or plot column
            let title_start_col = left_overflow_col.unwrap_or(plot_col_index);
            component_map.add_component(ComponentType::Title, row_index, title_start_col);
            row_index += 1;
        }

        // Add subtitle if present
        if let Some(s) = subtitle {
            let (height, _) = ChartLayout::measure_text(&s.text, s.font_size, &s.font_family);
            rows.push(length(height * 1.1));
            // Subtitle starts from left overflow column (if present) or plot column
            let subtitle_start_col = left_overflow_col.unwrap_or(plot_col_index);
            component_map.add_component(ComponentType::Subtitle, row_index, subtitle_start_col);
            row_index += 1;
        }

        // Add top overflow space if needed
        if overflow.top > OVERFLOW_THRESHOLD {
            rows.push(length(overflow.top)); // Exact overflow size
            component_map.add_component(
                ComponentType::Axis(AxisPosition::Top),
                row_index,
                plot_col_index,
            );
            row_index += 1;
        }

        // Plot area row (flexible)
        let plot_row_index = row_index;
        rows.push(fr(1.0));
        component_map.add_component(ComponentType::PlotArea, plot_row_index, plot_col_index);

        // Now add the left/right axis components at the plot row
        if let Some(col) = left_overflow_col {
            component_map.add_component(
                ComponentType::Axis(AxisPosition::Left),
                plot_row_index,
                col,
            );
        }

        if let Some(col) = right_overflow_col {
            component_map.add_component(
                ComponentType::Axis(AxisPosition::Right),
                plot_row_index,
                col,
            );
        }

        // Add right legend containers at the plot row
        let mut legend_idx = 0;
        for component in &self.right_components {
            if let ComponentType::LegendContainer(position) = component {
                if *position == LegendPosition::Right && legend_idx < right_legend_cols.len() {
                    component_map.add_component(
                        component.clone(),
                        plot_row_index,
                        right_legend_cols[legend_idx],
                    );
                    legend_idx += 1;
                }
            }
        }

        row_index += 1;

        // Add bottom overflow space if needed
        if overflow.bottom > OVERFLOW_THRESHOLD {
            rows.push(length(overflow.bottom)); // Exact overflow size
            component_map.add_component(
                ComponentType::Axis(AxisPosition::Bottom),
                row_index,
                plot_col_index,
            );
            // row_index += 1;
        }

        // End with bottom margin
        rows.push(length(EDGE_MARGIN));

        Ok((GridTemplate { cols, rows }, component_map))
    }
}
