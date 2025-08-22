use crate::axis::{AxisPosition, CartesianAxis};
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
use crate::plot::{PlotSubtitle, PlotTitle, TitleAlign};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
};
use avenger_scales::scales::ConfiguredScale;
use indexmap::IndexMap;
use tracing::{debug, trace};
// Use stable ordering by iterating sorted keys, not map type
use std::collections::HashMap;
use taffy::prelude::*;
use taffy::{NodeId, TaffyTree};

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
        // Add a small padding (10%) for visual breathing room on height
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

        // Add overflow regions as needed
        if overflow.top > 0.0 {
            builder.add_axes_at_position(AxisPosition::Top, 1);
        }
        if overflow.bottom > 0.0 {
            builder.add_axes_at_position(AxisPosition::Bottom, 1);
        }
        if overflow.left > 0.0 {
            builder.add_axes_at_position(AxisPosition::Left, 1);
        }
        if overflow.right > 0.0 {
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

        if overflow.top > 0.0 {
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

        if overflow.bottom > 0.0 {
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

    /// Measure component size using actual rendered marks
    /// Currently unused but kept for potential future layout improvements
    #[allow(dead_code)]
    pub fn measure_axis_size(
        axis: &CartesianAxis,
        scale: &ConfiguredScale,
        available_space: Size<f32>,
    ) -> Result<Size<f32>, AvengerChartError> {
        // Skip invisible axes
        if !axis.visible {
            return Ok(Size {
                width: 0.0,
                height: 0.0,
            });
        }

        // Special handling for polar pseudo axes - dimension is stored in tick_count
        if axis.tick_count.is_some() && axis.title.is_none() && !axis.grid {
            let dimension = axis.tick_count.unwrap() as f32;
            return match axis.position {
                Some(AxisPosition::Top) | Some(AxisPosition::Bottom) => Ok(Size {
                    width: available_space.width,
                    height: dimension,
                }),
                Some(AxisPosition::Left) | Some(AxisPosition::Right) => Ok(Size {
                    width: dimension,
                    height: available_space.height,
                }),
                None => Ok(Size {
                    width: 0.0,
                    height: 0.0,
                }),
            };
        }

        // Create axis configuration
        let orientation = match axis.position {
            Some(AxisPosition::Left) => AxisOrientation::Left,
            Some(AxisPosition::Right) => AxisOrientation::Right,
            Some(AxisPosition::Top) => AxisOrientation::Top,
            Some(AxisPosition::Bottom) => AxisOrientation::Bottom,
            None => {
                return Ok(Size {
                    width: 0.0,
                    height: 0.0,
                });
            }
        };

        // For axes, we want to measure their natural size, not constrain them
        // The dimensions here affect where gridlines and ticks are placed
        // For vertical axes, we care about the vertical range (plot height)
        // For horizontal axes, we care about the horizontal range (plot width)
        let dimensions = match orientation {
            AxisOrientation::Left | AxisOrientation::Right => {
                // Vertical axis - height matters for tick placement
                // Width should be minimal (will be determined by text)
                [0.0, available_space.height]
            }
            AxisOrientation::Top | AxisOrientation::Bottom => {
                // Horizontal axis - width matters for tick placement
                // Height should be minimal (will be determined by text)
                [available_space.width, 0.0]
            }
        };

        let config = AxisConfig {
            orientation,
            dimensions,
            grid: axis.grid,
            format_number: axis.format_number.clone(),
            title_font_size: None, // Use default for regular axes
        };

        // Create axis marks
        let title = axis.title.as_deref().unwrap_or("");
        let origin = [0.0, 0.0];

        // Generate axis marks based on scale type
        let scale_type = scale.scale_impl.scale_type();

        let axis_group = match scale_type {
            "band" | "point" => make_band_axis_marks(scale, title, origin, &config)
                .map_err(|e| AvengerChartError::InternalError(e.to_string()))?,
            _ => {
                // Default to numeric axis for linear and other continuous scales
                make_numeric_axis_marks(scale, title, origin, &config)
                    .map_err(|e| AvengerChartError::InternalError(e.to_string()))?
            }
        };

        // Measure the bounding box
        let bbox = axis_group.bounding_box();
        let width = (bbox.upper()[0] - bbox.lower()[0]).abs();
        let height = (bbox.upper()[1] - bbox.lower()[1]).abs();

        // Debug: Show axis measurement

        // Return exact size without extra padding - the edge margins handle clipping
        Ok(Size { width, height })
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
        use std::collections::HashMap;

        // Extract mark encodings from provided marks and check for line marks
        let mut mark_encodings = HashMap::new();
        let mut has_line_mark = false;
        if channel == "stroke" {
            trace!(
                mark_count = marks.len(),
                "measure_legend_size: checking marks for line type"
            );
        }
        for mark in marks {
            let mark_type = mark.mark_type();
            if channel == "stroke" {
                trace!(mark_type = mark_type, "Mark type");
            }

            // Check for line marks
            if mark_type == "line" {
                has_line_mark = true;
            }

            // Extract encodings from symbol or rect marks
            let is_relevant = mark_type == "symbol" || mark_type == "rect";
            if is_relevant {
                let channels = mark.data_context().channels();
                for (channel_name, value) in channels {
                    mark_encodings.insert(channel_name.clone(), value.clone());
                }
            }
        }

        Self::measure_legend_size_impl(
            channel,
            legend,
            scale,
            scales,
            available_space,
            Some(&mark_encodings),
            has_line_mark,
        )
    }

    /// Measure legend size by creating the actual SceneGroup and measuring its bounding box
    /// Currently unused but kept for potential future layout improvements
    #[allow(dead_code)]
    fn measure_legend_size_impl(
        channel: &str,
        legend: &Legend,
        scale: &ConfiguredScale,
        scales: &HashMap<String, ConfiguredScale>,
        _available_space: Size<f32>,
        mark_encodings: Option<&HashMap<String, crate::marks::ChannelValue>>,
        has_line_mark: bool,
    ) -> Result<Size<f32>, AvengerChartError> {
        // Helper function to parse color strings
        fn parse_color_string(color_str: &str) -> Option<avenger_common::types::ColorOrGradient> {
            use avenger_scales::scales::coerce::Coercer;
            use datafusion_common::ScalarValue;

            let coercer = Coercer::default();
            let array = ScalarValue::iter_to_array(
                [ScalarValue::Utf8(Some(color_str.to_string()))]
                    .iter()
                    .cloned(),
            )
            .ok()?;
            coercer
                .to_color(&array, None)
                .ok()
                .and_then(|colors| colors.as_vec(1, None).first().cloned())
        }
        // Skip invisible legends
        if !legend.visible {
            return Ok(Size {
                width: 0.0,
                height: 0.0,
            });
        }

        // Extract domain values from scale - similar to create_symbol_legend
        // For threshold scales, we need to account for n+1 intervals for n thresholds
        let domain_values = scale.domain();
        let domain_len = if scale.scale_impl.scale_type() == "threshold" {
            // Threshold scales have n+1 intervals for n thresholds
            domain_values.len() + 1
        } else {
            domain_values.len()
        };

        if domain_len == 0 {
            return Ok(Size {
                width: 0.0,
                height: 0.0,
            });
        }

        // Create text labels - use domain_labels() for proper threshold scale handling
        use crate::scales::ConfiguredScaleLegendExt;
        let text_values: Vec<String> = if scale.scale_impl.scale_type() == "threshold" {
            // Use domain_labels() for threshold scales to get interval labels
            scale.domain_labels().unwrap_or_else(|_| {
                // Fallback labels if domain_labels() fails
                (0..domain_len).map(|i| format!("Interval {}", i)).collect()
            })
        } else {
            // For other scale types, extract from domain values
            use datafusion::arrow::array::Array;
            use datafusion::arrow::compute::cast;
            use datafusion::arrow::datatypes::DataType;

            // Try to cast to Utf8 to handle various string types (LargeUtf8, Dictionary, etc.)
            if let Ok(string_array) = cast(&domain_values, &DataType::Utf8) {
                // Successfully cast to Utf8 - extract the values
                use datafusion::arrow::array::StringArray;
                if let Some(string_array) = string_array.as_any().downcast_ref::<StringArray>() {
                    let values: Vec<String> = (0..domain_len)
                        .map(|i| string_array.value(i).to_string())
                        .collect();
                    values
                } else {
                    // Fallback if downcast fails
                    (0..domain_len)
                        .map(|i| format!("Type {}", (b'A' + (i as u8 % 26)) as char))
                        .collect()
                }
            } else {
                // Cast failed - try to handle numeric arrays
                use datafusion::arrow::array::{Float64Array, Int64Array};

                if let Some(float_array) = domain_values.as_any().downcast_ref::<Float64Array>() {
                    // Handle Float64 arrays
                    (0..domain_len)
                        .map(|i| {
                            let value = float_array.value(i);
                            if value.fract() == 0.0 && value.abs() < 1e10 {
                                format!("{:.0}", value)
                            } else {
                                format!("{}", value)
                            }
                        })
                        .collect()
                } else if let Some(int_array) = domain_values.as_any().downcast_ref::<Int64Array>()
                {
                    // Handle Int64 arrays
                    (0..domain_len)
                        .map(|i| format!("{}", int_array.value(i)))
                        .collect()
                } else {
                    // Fallback if we can't handle the type
                    (0..domain_len)
                        .map(|i| format!("Type {}", (b'A' + (i as u8 % 26)) as char))
                        .collect()
                }
            }
        };

        // Determine legend type based on the channel it represents
        use avenger_common::types::ColorOrGradient;
        use avenger_common::value::ScalarOrArray;

        // Check if scale is continuous (for colorbar)
        let scale_type = scale.scale_impl.scale_type();
        let is_continuous = matches!(scale_type, "linear" | "log" | "pow" | "sqrt");
        let is_color_channel = matches!(channel, "fill" | "stroke" | "color");

        // Use colorbar for continuous color scales
        let should_use_colorbar = is_color_channel && is_continuous;

        // Determine if this should be a line legend based on channel name and mark type
        // Use line legend for stroke properties on line marks (matching render.rs logic)
        let should_use_line_legend = !should_use_colorbar
            && has_line_mark
            && matches!(channel, "stroke" | "stroke_dash" | "stroke_width");

        let legend_group = if should_use_colorbar {
            // Create a colorbar for continuous color scales
            use avenger_guides::legend::colorbar::{
                ColorbarConfig, ColorbarOrientation, make_colorbar_marks,
            };

            // For measurement, use a reasonable size
            // colorbar_height now refers to total height including padding
            let padding = legend.background_padding.unwrap_or(8.0);
            let total_height = 150.0 + 2.0 * padding; // 166px with default 8px padding
            let config = ColorbarConfig {
                orientation: ColorbarOrientation::Right,
                dimensions: [100.0, 200.0], // Available space for measurement
                colorbar_width: Some(15.0),
                colorbar_height: Some(total_height),
                colorbar_margin: Some(0.0),
                format_number: legend.format_number.clone(),
                background_fill: legend
                    .background_fill
                    .as_ref()
                    .and_then(|s| parse_color_string(s)),
                background_stroke: legend
                    .background_stroke
                    .as_ref()
                    .and_then(|s| parse_color_string(s)),
                background_corner_radius: legend.background_corner_radius,
                background_padding: legend.background_padding,
            };

            // Use the scale that's already configured (passed to this function)
            make_colorbar_marks(scale, "", [0.0, 0.0], &config)
                .map_err(|e| AvengerChartError::InternalError(e.to_string()))?
        } else if should_use_line_legend {
            // Create a line legend for line-based channels
            use avenger_guides::legend::line::{LineLegendConfig, make_line_legend};

            // For line legends, check if we're measuring a stroke_dash or stroke_width channel
            // These need special handling for proper measurement
            let (stroke_widths, stroke_dashes) = if channel == "stroke_dash" {
                // Vary stroke dash if that's the legend channel
                // Use common dash patterns for measurement
                let dash_patterns = [
                    None,                 // Solid
                    Some(vec![4.0, 4.0]), // Dashed
                    Some(vec![1.0, 3.0]), // Dotted
                ];
                let patterns = (0..text_values.len())
                    .map(|i| dash_patterns[i % dash_patterns.len()].clone())
                    .collect();
                (
                    ScalarOrArray::new_scalar(2.0),
                    ScalarOrArray::new_array(patterns),
                )
            } else if channel == "stroke_width" {
                // Vary stroke width if that's the legend channel
                // Use a range of widths
                let widths: Vec<f32> = (0..text_values.len())
                    .map(|i| 1.0 + (i as f32) * 2.0) // 1, 3, 5, etc.
                    .collect();
                (
                    ScalarOrArray::new_array(widths),
                    ScalarOrArray::new_scalar(None),
                )
            } else {
                // Default: solid lines with standard width
                (
                    ScalarOrArray::new_scalar(2.0),
                    ScalarOrArray::new_scalar(None),
                )
            };

            let config = LineLegendConfig {
                title: legend.title.clone(),
                text: ScalarOrArray::new_array(text_values.clone()),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0])),
                stroke_width: stroke_widths,
                stroke_dash: stroke_dashes,
                stroke_cap: avenger_common::types::StrokeCap::Round,
                stroke_join: Some(avenger_common::types::StrokeJoin::Round),
                font_size: ScalarOrArray::new_scalar(10.0),
                font_family: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                inner_width: 0.0,
                inner_height: 100.0,
                outer_margin: 0.0,
                entry_margin: 2.0,
                text_padding: 4.0, // Consistent with symbol legend spacing
                line_length: ScalarOrArray::new_scalar(16.0), // Similar to symbol size, enough for dash patterns
                background_fill: legend
                    .background_fill
                    .as_ref()
                    .and_then(|s| parse_color_string(s)),
                background_stroke: legend
                    .background_stroke
                    .as_ref()
                    .and_then(|s| parse_color_string(s)),
                background_corner_radius: legend.background_corner_radius,
                background_padding: legend.background_padding,
            };

            make_line_legend(&config)
                .map_err(|e| AvengerChartError::InternalError(e.to_string()))?
        } else {
            // Default to symbol legend for other channels
            use avenger_common::types::SymbolShape;
            use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};

            // Check if we have a size scale that might affect the legend
            // If the domain values are the same as our legend scale, use the size values
            let size_values = if let Some(size_scale) = scales.get("size") {
                // Check if the size scale has the same domain as our legend scale
                let size_domain = size_scale.domain();
                let our_domain = scale.domain();

                trace!(
                    size_domain_len = size_domain.len(),
                    our_domain_len = our_domain.len(),
                    "Checking size scale"
                );

                // Compare domains - if they're the same, the channels share the same data
                if size_domain.len() == our_domain.len() {
                    // Map our domain values through the size scale to get the sizes
                    // The size scale should map the same domain values to the appropriate sizes
                    match size_scale.scale(our_domain) {
                        Ok(scaled_array) => {
                            use datafusion::arrow::array::{Array, Float32Array, Float64Array};
                            use datafusion::arrow::compute::cast;
                            use datafusion::arrow::datatypes::DataType;

                            trace!(
                                data_type = ?scaled_array.data_type(),
                                "Size scale output type"
                            );

                            // Try casting to Float32
                            if let Ok(float_array) = cast(&scaled_array, &DataType::Float32) {
                                if let Some(f32_array) =
                                    float_array.as_any().downcast_ref::<Float32Array>()
                                {
                                    let mut sizes: Vec<f32> =
                                        (0..f32_array.len()).map(|i| f32_array.value(i)).collect();

                                    // Add a 10% buffer to each size for conservative measurement
                                    for size in &mut sizes {
                                        *size *= 1.1;
                                    }

                                    trace!(
                                        sizes = ?sizes,
                                        "Using size scale values (with buffer)"
                                    );
                                    ScalarOrArray::new_array(sizes)
                                } else {
                                    ScalarOrArray::new_scalar(
                                        legend.symbol_size.unwrap_or(64.0) as f32 * 1.1,
                                    )
                                }
                            } else if let Some(float_array) =
                                scaled_array.as_any().downcast_ref::<Float64Array>()
                            {
                                let sizes: Vec<f32> = (0..float_array.len())
                                    .map(|i| (float_array.value(i) * 1.1) as f32)
                                    .collect();
                                trace!(
                                    sizes = ?sizes,
                                    "Using size scale values (from f64, with buffer)"
                                );
                                ScalarOrArray::new_array(sizes)
                            } else {
                                trace!("Size scale output could not be converted to float");
                                ScalarOrArray::new_scalar(
                                    legend.symbol_size.unwrap_or(64.0) as f32 * 1.1,
                                )
                            }
                        }
                        Err(e) => {
                            trace!(error = ?e, "Error scaling through size scale");
                            ScalarOrArray::new_scalar(legend.symbol_size.unwrap_or(64.0) as f32)
                        }
                    }
                } else {
                    trace!("Domain lengths don't match, using default size");
                    ScalarOrArray::new_scalar(legend.symbol_size.unwrap_or(64.0) as f32)
                }
            } else {
                trace!("No size scale found, using default size");
                ScalarOrArray::new_scalar(legend.symbol_size.unwrap_or(64.0) as f32)
            };

            // Check if mark encodings have a scalar size value (like .size(100.0) in the test)
            // This must override the default size to match what's used in final legend creation
            let size_values = if let Some(mark_encodings) = mark_encodings {
                if let Some(size_encoding) = mark_encodings.get("size") {
                    // Check if this is a scalar expression (not referencing columns)
                    use crate::utils::ScalarValueHelpers;
                    use datafusion::logical_expr::Expr;
                    match size_encoding.expr() {
                        Expr::Literal(scalar_value, _) => {
                            // It's a literal value - try to extract as f32
                            if let Ok(f_val) = scalar_value.as_f32() {
                                ScalarOrArray::new_scalar(f_val)
                            } else {
                                size_values
                            }
                        }
                        _ => size_values,
                    }
                } else {
                    size_values
                }
            } else {
                size_values
            };

            // Check for shape scale
            let shape_values = if let Some(shape_scale) = scales.get("shape") {
                let shape_domain = shape_scale.domain();
                if shape_domain.len() == scale.domain().len() {
                    // Map domain through shape scale to get shapes
                    match shape_scale.scale(scale.domain()) {
                        Ok(scaled_array) => {
                            use avenger_common::types::SymbolShape;
                            use datafusion::arrow::array::{Array, StringArray};

                            if let Some(string_array) =
                                scaled_array.as_any().downcast_ref::<StringArray>()
                            {
                                let shapes: Vec<SymbolShape> = (0..string_array.len())
                                    .map(|i| {
                                        SymbolShape::from_vega_str(string_array.value(i))
                                            .unwrap_or(SymbolShape::Circle)
                                    })
                                    .collect();
                                ScalarOrArray::new_array(shapes)
                            } else {
                                ScalarOrArray::new_scalar(SymbolShape::Circle)
                            }
                        }
                        Err(_) => ScalarOrArray::new_scalar(SymbolShape::Circle),
                    }
                } else {
                    ScalarOrArray::new_scalar(SymbolShape::Circle)
                }
            } else {
                // Check if this is for a rect mark - use square shape
                // We can't easily detect mark type here, but we can use square as default for better rect legends
                // For now, use Circle as default (will be improved later)
                ScalarOrArray::new_scalar(SymbolShape::Circle)
            };

            // Check for fill scale
            let fill_values = if channel == "fill" {
                // This legend is for the fill channel - use the threshold scale colors
                if scale.scale_impl.scale_type() == "threshold" {
                    // For threshold scales, get colors directly from the range
                    // There are n+1 colors for n thresholds
                    use crate::scales::ConfiguredScaleLegendExt;
                    match scale.range_colors() {
                        Ok(colors) => ScalarOrArray::new_array(
                            colors.into_iter().map(ColorOrGradient::Color).collect(),
                        ),
                        Err(_) => {
                            ScalarOrArray::new_scalar(ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0]))
                        }
                    }
                } else {
                    // For other scales, map domain values through the scale
                    match scale.scale(scale.domain()) {
                        Ok(scaled_array) => {
                            trace!(
                                array_len = scaled_array.len(),
                                data_type = ?scaled_array.data_type(),
                                "Scaled array info"
                            );

                            // Use Coercer to handle color conversion
                            use avenger_scales::scales::coerce::Coercer;
                            let coercer = Coercer::default();

                            if let Ok(colors) = coercer.to_color(&scaled_array, None) {
                                trace!(color_count = colors.len(), "Converted to colors");
                                colors
                            } else {
                                // Fallback: try the old Float32 approach for backwards compatibility
                                use datafusion::arrow::array::{Array, Float32Array};
                                use datafusion::arrow::compute::cast;
                                use datafusion::arrow::datatypes::DataType;

                                if let Ok(color_array) = cast(&scaled_array, &DataType::Float32) {
                                    if let Some(float_array) =
                                        color_array.as_any().downcast_ref::<Float32Array>()
                                    {
                                        // Group into RGBA colors (4 values per color)
                                        let mut colors = Vec::new();
                                        let mut i = 0;
                                        while i + 4 <= float_array.len() {
                                            colors.push(ColorOrGradient::Color([
                                                float_array.value(i),
                                                float_array.value(i + 1),
                                                float_array.value(i + 2),
                                                float_array.value(i + 3),
                                            ]));
                                            i += 4;
                                        }
                                        if !colors.is_empty() {
                                            ScalarOrArray::new_array(colors)
                                        } else {
                                            ScalarOrArray::new_scalar(ColorOrGradient::Color([
                                                0.5, 0.5, 0.5, 1.0,
                                            ]))
                                        }
                                    } else {
                                        ScalarOrArray::new_scalar(ColorOrGradient::Color([
                                            0.5, 0.5, 0.5, 1.0,
                                        ]))
                                    }
                                } else {
                                    ScalarOrArray::new_scalar(ColorOrGradient::Color([
                                        0.5, 0.5, 0.5, 1.0,
                                    ]))
                                }
                            }
                        }
                        Err(_) => {
                            ScalarOrArray::new_scalar(ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0]))
                        }
                    }
                }
            } else {
                ScalarOrArray::new_scalar(ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0]))
            };

            // Extract stroke_width from mark encodings if available
            let stroke_width = if let Some(mark_encodings) = mark_encodings {
                if let Some(stroke_width_encoding) = mark_encodings.get("stroke_width") {
                    // Check if this is a scalar expression (not referencing columns)
                    use crate::utils::ScalarValueHelpers;
                    use datafusion::logical_expr::Expr;
                    match stroke_width_encoding.expr() {
                        Expr::Literal(scalar_value, _) => {
                            // It's a literal value - try to extract as f32
                            scalar_value.as_f32().unwrap_or(1.0)
                        }
                        _ => 1.0, // Default if expression references columns
                    }
                } else {
                    1.0 // Default if no stroke_width encoding
                }
            } else {
                1.0 // Default if no mark encodings
            };

            debug!(
                channel = channel,
                padding = ?legend.background_padding,
                text_values = ?text_values,
                stroke_width = stroke_width,
                "Creating symbol legend with inner_width: 0.0, inner_height: 100.0, outer_margin: 0.0, text_padding: 2.0"
            );
            let config = SymbolLegendConfig {
                title: legend.title.clone(),
                text: ScalarOrArray::new_array(text_values.clone()),
                shape: shape_values,
                size: size_values,
                fill: fill_values,
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: Some(stroke_width),
                angle: ScalarOrArray::new_scalar(0.0),
                inner_width: 0.0,
                inner_height: 100.0, // Match the render config
                outer_margin: 0.0,
                text_padding: 2.0, // Match the default from legend config
                background_fill: legend
                    .background_fill
                    .as_ref()
                    .and_then(|s| parse_color_string(s)),
                background_stroke: legend
                    .background_stroke
                    .as_ref()
                    .and_then(|s| parse_color_string(s)),
                background_corner_radius: legend.background_corner_radius,
                background_padding: legend.background_padding,
            };

            make_symbol_legend(&config)
                .map_err(|e| AvengerChartError::InternalError(e.to_string()))?
        };

        // Debug: Print the scene group structure
        debug!(
            channel = channel,
            legend_type = if should_use_line_legend { "Line" } else { "Symbol" },
            legend_config = ?legend,
            legend_group_clip = ?legend_group.clip,
            legend_group_marks_count = legend_group.marks.len(),
            text_labels = ?text_values,
            "Legend measurement"
        );

        // Measure the actual bounding box
        let bbox = legend_group.bounding_box();
        let width = bbox.upper()[0] - bbox.lower()[0];
        let height = bbox.upper()[1] - bbox.lower()[1];

        // Always show debug for legend measurement
        debug!(
            x = bbox.lower()[0],
            y = bbox.lower()[1],
            width = width,
            height = height,
            "Legend bounding box"
        );

        Ok(Size { width, height })
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
        use crate::constants::EDGE_MARGIN;

        let mut cols = Vec::new();
        let mut rows = Vec::new();
        let mut component_map = ComponentGridMap::new();

        // === Build Column Template ===
        // Start with left margin
        cols.push(length(EDGE_MARGIN));
        let mut col_index = 1;

        // Track column index for left overflow
        let left_overflow_col = if overflow.left > 0.0 {
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
        let right_overflow_col = if overflow.right > 0.0 {
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
                    // Get channels for this position from legends_by_position
                    let channels: Vec<String> = self
                        .legends_by_position
                        .get(position)
                        .map(|legends| legends.iter().map(|(ch, _)| ch.clone()).collect())
                        .unwrap_or_default();
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
        if overflow.top > 0.0 {
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
        if overflow.bottom > 0.0 {
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
