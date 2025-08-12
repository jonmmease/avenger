use crate::axis::{AxisPosition, CartesianAxis};
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
use crate::plot::{PlotSubtitle, PlotTitle};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
};
use avenger_scales::scales::ConfiguredScale;
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
    // Text properties for calculating heights
    #[allow(dead_code)]
    title_font_size: Option<f32>,
    #[allow(dead_code)]
    title_font_family: Option<String>,
    #[allow(dead_code)]
    subtitle_font_size: Option<f32>,
    #[allow(dead_code)]
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

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum ComponentType {
    PlotArea,
    Axis(AxisPosition),
    Legend(String),                  // Channel name
    LegendContainer(LegendPosition), // Container for legends at a position
    Title,
    Subtitle,
    Padding, // Empty space
}

/// Result of layout computation
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub plot_area: LayoutBounds,
    pub axes: HashMap<AxisPosition, LayoutBounds>,
    pub legends: HashMap<String, LayoutBounds>,
    pub title: Option<LayoutBounds>,
    pub subtitle: Option<LayoutBounds>,
    #[allow(dead_code)]
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
    legends_by_position: HashMap<LegendPosition, Vec<(String, Legend)>>,
}

#[derive(Debug)]
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
    pub fn new<C: crate::coords::CoordinateSystem>(
        axes: &HashMap<String, CartesianAxis>,
        legends: &HashMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        preferred_size: Option<(f32, f32)>,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        marks: &[Box<dyn crate::marks::Mark<C>>],
    ) -> Result<Self, AvengerChartError> {
        let mut taffy = TaffyTree::new();
        let mut builder = GridBuilder::new();

        // Analyze component positions
        let axes_by_position = Self::group_axes_by_position(axes);

        // Build grid structure dynamically
        builder.add_plot_area(); // Always present

        // Add optional title and subtitle
        if title.is_some() {
            builder.add_title();
        }
        if subtitle.is_some() {
            builder.add_subtitle();
        }

        // Add axes by position
        for (position, axis_channels) in &axes_by_position {
            builder.add_axes_at_position(*position, axis_channels.len());
        }

        // Add legends
        for (channel, legend) in legends.iter() {
            builder.add_legend(channel.clone(), legend);
        }

        // Finalize legend containers after all legends are added
        builder.finalize_legend_containers();

        // Measure components and generate optimal grid template
        let available_size = preferred_size.unwrap_or((400.0, 300.0));
        let (grid_template, component_map) = builder.build_with_measurements(
            axes,
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

        // Measure legend sizes first
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

        // Create nodes for each component
        layout.create_component_nodes(&axes_by_position, legends, scales)?;

        Ok(layout)
    }

    /// Group axes by their position
    fn group_axes_by_position(
        axes: &HashMap<String, CartesianAxis>,
    ) -> HashMap<AxisPosition, Vec<String>> {
        let mut by_position = HashMap::new();

        for (channel, axis) in axes.iter() {
            if let Some(position) = axis.position {
                by_position
                    .entry(position)
                    .or_insert_with(Vec::new)
                    .push(channel.clone());
            }
        }

        by_position
    }

    /// Create taffy nodes for each component
    fn create_component_nodes(
        &mut self,
        axes_by_position: &HashMap<AxisPosition, Vec<String>>,
        legends: &HashMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
    ) -> Result<(), AvengerChartError> {
        // With the new dynamic grid, we need to find the plot area position from the component map
        let mut plot_row = 0;
        let mut plot_col = 0;

        // Find plot area position in the component map
        for ((row, col), component) in &self.component_map.cells {
            if matches!(component, ComponentType::PlotArea) {
                plot_row = *row + 1; // Convert to 1-based grid line
                plot_col = *col + 1; // Convert to 1-based grid line
                break;
            }
        }

        // Create plot area node (always at the center of the grid)
        // Plot area should flex to fill available space
        let plot_style = Style {
            grid_row: line(plot_row as i16),
            grid_column: line(plot_col as i16),
            flex_grow: 1.0,   // Allow plot area to grow
            flex_shrink: 1.0, // Allow plot area to shrink
            min_size: Size {
                width: length(50.0),  // Minimum width
                height: length(50.0), // Minimum height
            },
            ..Default::default()
        };
        self.plot_area_node = Some(self.taffy.new_leaf(plot_style)?);

        // Create axis nodes - we need to map each axis to its grid position
        for position in axes_by_position.keys() {
            let axis_style = self.create_axis_style_from_grid(
                *position,
                &self.component_map,
                plot_row as i16,
                plot_col as i16,
            )?;
            let node = self.taffy.new_leaf(axis_style)?;
            self.axis_nodes.insert(*position, node);
        }

        // Create legend container nodes and their children
        // First, group legends by position
        let mut legends_by_position: HashMap<LegendPosition, Vec<(String, Legend)>> =
            HashMap::new();
        for (channel, legend) in legends.iter() {
            let position = legend.position.unwrap_or(LegendPosition::Right);
            legends_by_position
                .entry(position)
                .or_default()
                .push((channel.clone(), legend.clone()));
        }

        // Create container nodes for each position with legends
        for ((row, col), component) in &self.component_map.cells {
            if let ComponentType::LegendContainer(position) = component {
                // Create flex container for this position
                if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "Legend container {:?} at grid position: row={}, col={}",
                        position,
                        *row + 1,
                        *col + 1
                    );
                }
                let container_style = Style {
                    grid_row: line((*row + 1) as i16),
                    grid_column: line((*col + 1) as i16),
                    display: Display::Flex,
                    flex_direction: match position {
                        LegendPosition::Left | LegendPosition::Right => FlexDirection::Column,
                        LegendPosition::Top | LegendPosition::Bottom => FlexDirection::Row,
                    },
                    align_items: Some(AlignItems::FlexStart),
                    gap: Size {
                        width: length(10.0),
                        height: length(10.0),
                    },
                    ..Default::default()
                };

                let container_node = self.taffy.new_leaf(container_style)?;
                self.legend_container_nodes
                    .insert(*position, container_node);

                // Create individual legend nodes as children of this container
                if let Some(legends_at_position) = legends_by_position.get(position) {
                    // Sort legends by order field and channel name
                    let mut sorted_legends = legends_at_position.clone();
                    sorted_legends.sort_by(|(channel_a, legend_a), (channel_b, legend_b)| {
                        match (legend_a.order, legend_b.order) {
                            (Some(o1), Some(o2)) => o1.cmp(&o2),
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => channel_a.cmp(channel_b),
                        }
                    });

                    let mut legend_children = vec![];
                    for (channel, legend) in sorted_legends {
                        // Determine legend type and configure flex style accordingly
                        let legend_style =
                            self.configure_legend_flex_style(&legend, &channel, position, scales)?;
                        let legend_node = self.taffy.new_leaf(legend_style)?;
                        self.legend_nodes.insert(channel.clone(), legend_node);
                        legend_children.push(legend_node);
                    }

                    // Set children of container
                    self.taffy.set_children(container_node, &legend_children)?;
                }
            }
        }

        // Create title node if present in component map
        for ((row, col), component) in &self.component_map.cells {
            if matches!(component, ComponentType::Title) {
                let style = Style {
                    grid_row: line((*row + 1) as i16),
                    grid_column: line((*col + 1) as i16),
                    justify_content: Some(JustifyContent::FlexStart), // Left align
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.title_node = Some(node);
                break;
            }
        }

        // Create subtitle node if present in component map
        for ((row, col), component) in &self.component_map.cells {
            if matches!(component, ComponentType::Subtitle) {
                let style = Style {
                    grid_row: line((*row + 1) as i16),
                    grid_column: line((*col + 1) as i16),
                    justify_content: Some(JustifyContent::FlexStart), // Left align
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                };
                let node = self.taffy.new_leaf(style)?;
                self.subtitle_node = Some(node);
                break;
            }
        }

        // Set children of root node
        let mut children = vec![];
        if let Some(plot_node) = self.plot_area_node {
            children.push(plot_node);
        }
        children.extend(self.axis_nodes.values());
        // Add legend containers instead of individual legends
        children.extend(self.legend_container_nodes.values());
        if let Some(title_node) = self.title_node {
            children.push(title_node);
        }
        if let Some(subtitle_node) = self.subtitle_node {
            children.push(subtitle_node);
        }

        self.taffy.set_children(self.root_node, &children)?;

        Ok(())
    }

    /// Create axis style based on grid component map
    fn create_axis_style_from_grid(
        &self,
        position: AxisPosition,
        component_map: &ComponentGridMap,
        plot_row: i16,
        plot_col: i16,
    ) -> Result<Style, AvengerChartError> {
        // Find the grid position for this axis
        for ((row, col), component) in &component_map.cells {
            if let ComponentType::Axis(axis_pos) = component {
                if *axis_pos == position {
                    return Ok(Style {
                        grid_row: line((*row + 1) as i16),    // Convert to 1-based
                        grid_column: line((*col + 1) as i16), // Convert to 1-based
                        align_items: Some(match position {
                            AxisPosition::Left | AxisPosition::Right => AlignItems::Center,
                            AxisPosition::Top => AlignItems::FlexEnd,
                            AxisPosition::Bottom => AlignItems::FlexStart,
                        }),
                        justify_content: Some(match position {
                            AxisPosition::Left => JustifyContent::FlexEnd,
                            AxisPosition::Right => JustifyContent::FlexStart,
                            AxisPosition::Top | AxisPosition::Bottom => JustifyContent::Center,
                        }),
                        padding: Rect {
                            left: length(if position == AxisPosition::Right {
                                5.0
                            } else {
                                0.0
                            }),
                            right: length(if position == AxisPosition::Left {
                                5.0
                            } else {
                                0.0
                            }),
                            top: length(if position == AxisPosition::Bottom {
                                5.0
                            } else {
                                0.0
                            }),
                            bottom: length(if position == AxisPosition::Top {
                                5.0
                            } else {
                                0.0
                            }),
                        },
                        ..Default::default()
                    });
                }
            }
        }

        // Fallback to plot-adjacent position if not found in map
        Ok(Style {
            grid_row: line(plot_row),
            grid_column: line(plot_col),
            ..Default::default()
        })
    }

    /// Configure flex style for individual legend based on type
    fn configure_legend_flex_style(
        &self,
        _legend: &Legend,
        channel: &str,
        position: &LegendPosition,
        scales: &HashMap<String, ConfiguredScale>,
    ) -> Result<Style, AvengerChartError> {
        // Get the scale to determine legend type
        let scale = scales
            .get(channel)
            .ok_or_else(|| AvengerChartError::ScaleNotFound(channel.to_string()))?;

        // Determine if this is a colorbar legend
        let scale_type = scale.scale_impl.scale_type();
        let is_continuous = matches!(scale_type, "linear" | "log" | "pow" | "sqrt");
        let is_colorbar = matches!(channel, "fill" | "stroke" | "color") && is_continuous;

        // Get measured size if available
        let measured_size = self.legend_sizes.get(channel);

        let mut style = Style {
            display: Display::Flex,
            ..Default::default()
        };

        if is_colorbar {
            // Colorbar legends can grow to fill space and shrink if needed
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.align_self = Some(AlignSelf::Stretch);

            // Set preferred size from measurement, but allow flexibility
            if let Some(size) = measured_size {
                if matches!(position, LegendPosition::Right | LegendPosition::Left) {
                    // For vertical legends, width is fixed, height is flexible
                    style.size.width = length(size.width);
                    style.min_size.height = length(50.0); // Minimum height
                // Don't set max height - let colorbar stretch to fill available space
                } else {
                    // For horizontal legends, height is fixed, width is flexible
                    style.size.height = length(size.height);
                    style.min_size.width = length(50.0); // Minimum width
                    style.max_size.width = length(size.width); // Maximum from measurement
                }
            } else {
                // Default size if measurement failed
                if matches!(position, LegendPosition::Right | LegendPosition::Left) {
                    style.size.width = length(80.0);
                    style.min_size.height = length(50.0);
                    // Don't set max height - let colorbar stretch to fill available space
                } else {
                    style.size.height = length(80.0);
                    style.min_size.width = length(50.0);
                    style.max_size.width = length(170.0);
                }
            }
        } else {
            // Symbol and Line legends have fixed size
            style.flex_grow = 0.0;
            style.flex_shrink = 0.0;
            style.align_self = Some(AlignSelf::FlexStart);

            // Use measured size if available
            if let Some(size) = measured_size {
                style.size = Size {
                    width: length(size.width),
                    height: length(size.height),
                };
            } else {
                // Default size
                style.size = Size {
                    width: length(120.0),
                    height: length(100.0),
                };
            }
        }

        Ok(style)
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
            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "Plot area bounds: x={}, y={}, w={}, h={}",
                    layout.location.x, layout.location.y, layout.size.width, layout.size.height
                );
            }
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
            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "Axis {:?} bounds: x={}, y={}, w={}, h={}",
                    position,
                    layout.location.x,
                    layout.location.y,
                    layout.size.width,
                    layout.size.height
                );
            }
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
            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                eprintln!(
                    "Legend container {:?} bounds: x={}, y={}, w={}, h={}",
                    position,
                    container_layout.location.x,
                    container_layout.location.y,
                    container_layout.size.width,
                    container_layout.size.height
                );
            }
            container_positions.insert(
                position,
                (container_layout.location.x, container_layout.location.y),
            );
        }

        // Now get legend bounds relative to their containers
        for (channel, legend_node) in &self.legend_nodes {
            let legend_layout = self.taffy.layout(*legend_node)?;
            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() && channel == "stroke" {
                eprintln!(
                    "Individual legend '{}' node bounds: x={}, y={}, w={}, h={}",
                    channel,
                    legend_layout.location.x,
                    legend_layout.location.y,
                    legend_layout.size.width,
                    legend_layout.size.height
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
        if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() && channel == "stroke" {
            eprintln!(
                "  measure_legend_size: checking {} marks for line type",
                marks.len()
            );
        }
        for mark in marks {
            let mark_type = mark.mark_type();
            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() && channel == "stroke" {
                eprintln!("    Mark type: {}", mark_type);
            }

            // Check for line marks
            if mark_type == "line" {
                has_line_mark = true;
            }

            // Extract encodings from symbol or rect marks
            let is_relevant = mark_type == "symbol" || mark_type == "rect";
            if is_relevant {
                let encodings = mark.data_context().encodings();
                for (channel_name, value) in encodings {
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
        let domain_values = scale.domain();
        let domain_len = domain_values.len();
        if domain_len == 0 {
            return Ok(Size {
                width: 0.0,
                height: 0.0,
            });
        }

        // Create text labels from domain values
        // Try to extract actual string values from the domain array
        use datafusion::arrow::array::Array;
        use datafusion::arrow::compute::cast;
        use datafusion::arrow::datatypes::DataType;

        // Debug: Show domain array type
        // eprintln!("Domain array data type: {:?}", domain_values.data_type());

        // Try to cast to Utf8 to handle various string types (LargeUtf8, Dictionary, etc.)
        let text_values: Vec<String> = if let Ok(string_array) =
            cast(&domain_values, &DataType::Utf8)
        {
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
            } else if let Some(int_array) = domain_values.as_any().downcast_ref::<Int64Array>() {
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

                if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Checking size scale: domain len = {}, our domain len = {}",
                        size_domain.len(),
                        our_domain.len()
                    );
                }

                // Compare domains - if they're the same, the channels share the same data
                if size_domain.len() == our_domain.len() {
                    // Map our domain values through the size scale to get the sizes
                    // The size scale should map the same domain values to the appropriate sizes
                    match size_scale.scale(our_domain) {
                        Ok(scaled_array) => {
                            use datafusion::arrow::array::{Array, Float32Array, Float64Array};
                            use datafusion::arrow::compute::cast;
                            use datafusion::arrow::datatypes::DataType;

                            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                                eprintln!(
                                    "  Size scale output type: {:?}",
                                    scaled_array.data_type()
                                );
                            }

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

                                    if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                                        eprintln!(
                                            "  Using size scale values (with buffer): {:?}",
                                            sizes
                                        );
                                    }
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
                                if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "  Using size scale values (from f64, with buffer): {:?}",
                                        sizes
                                    );
                                }
                                ScalarOrArray::new_array(sizes)
                            } else {
                                if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "  Size scale output could not be converted to float"
                                    );
                                }
                                ScalarOrArray::new_scalar(
                                    legend.symbol_size.unwrap_or(64.0) as f32 * 1.1,
                                )
                            }
                        }
                        Err(e) => {
                            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                                eprintln!("  Error scaling through size scale: {:?}", e);
                            }
                            ScalarOrArray::new_scalar(legend.symbol_size.unwrap_or(64.0) as f32)
                        }
                    }
                } else {
                    if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                        eprintln!("  Domain lengths don't match, using default size");
                    }
                    ScalarOrArray::new_scalar(legend.symbol_size.unwrap_or(64.0) as f32)
                }
            } else {
                if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                    eprintln!("  No size scale found, using default size");
                }
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
            let fill_values = if let Some(fill_scale) = scales.get("fill") {
                let fill_domain = fill_scale.domain();
                if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "  Fill scale domain len: {}, legend scale domain len: {}",
                        fill_domain.len(),
                        scale.domain().len()
                    );
                }
                if fill_domain.len() == scale.domain().len() {
                    // Map domain through fill scale to get colors
                    match fill_scale.scale(scale.domain()) {
                        Ok(scaled_array) => {
                            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                                eprintln!(
                                    "  Scaled array len: {}, dtype: {:?}",
                                    scaled_array.len(),
                                    scaled_array.data_type()
                                );
                            }

                            // Use Coercer to handle color conversion
                            use avenger_scales::scales::coerce::Coercer;
                            let coercer = Coercer::default();

                            if let Ok(colors) = coercer.to_color(&scaled_array, None) {
                                if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                                    eprintln!("  Converted to {} colors", colors.len());
                                }
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
                } else {
                    ScalarOrArray::new_scalar(ColorOrGradient::Color([0.5, 0.5, 0.5, 1.0]))
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

            if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
                eprintln!("MEASUREMENT: Creating symbol legend '{}' with:", channel);
                eprintln!("  padding: {:?}", legend.background_padding);
                eprintln!("  text_values: {:?}", text_values);
                eprintln!("  inner_width: 0.0, inner_height: 100.0");
                eprintln!("  outer_margin: 0.0, text_padding: 2.0");
                eprintln!("  stroke_width: Some({})", stroke_width);
            }
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
        if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "\n=== LEGEND MEASUREMENT (channel: {}, type: {}) ===",
                channel,
                if should_use_line_legend {
                    "Line"
                } else {
                    "Symbol"
                }
            );
            eprintln!("  Legend config: {:?}", legend);
            eprintln!("  Legend group clip: {:?}", legend_group.clip);
            eprintln!("  Legend group marks count: {}", legend_group.marks.len());
            eprintln!("  Text labels: {:?}", text_values);
        }

        // Measure the actual bounding box
        let bbox = legend_group.bounding_box();
        let width = bbox.upper()[0] - bbox.lower()[0];
        let height = bbox.upper()[1] - bbox.lower()[1];

        // Always show debug for legend measurement
        if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "  Bounding box: x={}, y={}, width={}, height={}",
                bbox.lower()[0],
                bbox.lower()[1],
                width,
                height
            );
            eprintln!("=== END LEGEND MEASUREMENT ===\n");
        }

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
            legends_by_position: HashMap::new(),
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
    fn build_with_measurements<C: crate::coords::CoordinateSystem>(
        &self,
        axes: &HashMap<String, CartesianAxis>,
        legends: &HashMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        available_space: Size<f32>,
        marks: &[Box<dyn crate::marks::Mark<C>>],
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
    ) -> Result<(GridTemplate, ComponentGridMap), AvengerChartError> {
        // Use edge margins from constants to ensure consistent padding
        // This provides proper spacing even when there's no title
        use crate::constants::EDGE_MARGIN;

        // Use the same margin on all sides for consistent appearance
        let left_margin = EDGE_MARGIN;
        let right_margin = EDGE_MARGIN;
        let top_margin = EDGE_MARGIN;
        let bottom_margin = EDGE_MARGIN;

        // Build dynamic grid based on which components are present
        let mut rows = vec![];
        let mut cols = vec![];
        let mut map = ComponentGridMap {
            cells: HashMap::new(),
            row_count: 0,
            col_count: 0,
        };

        // === Build Column Template ===
        // Start with left margin (doubled if no left components)
        cols.push(length(left_margin));
        let mut col_index = 1;

        // Add left components
        for component in &self.left_components {
            let width = self.measure_component_width(
                component,
                axes,
                legends,
                scales,
                available_space,
                marks,
            )?;
            cols.push(length(width));

            // Don't track component position here - will do it after we know plot row
            col_index += 1;
        }

        // Plot area column (flexible)
        let plot_col_index = col_index;
        cols.push(fr(1.0));
        col_index += 1;

        // Add right components
        for component in &self.right_components {
            let width = self.measure_component_width(
                component,
                axes,
                legends,
                scales,
                available_space,
                marks,
            )?;
            cols.push(length(width));

            // Don't track component position here - will do it after we know plot row
            col_index += 1;
        }

        // End with right margin (doubled if no right components)
        cols.push(length(right_margin));

        if std::env::var("AVENGER_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "Grid columns count: {}, right margin: {}",
                cols.len(),
                right_margin
            );
        }

        // === Build Row Template ===
        // Start with top margin (doubled if no top components)
        rows.push(length(top_margin));
        let mut row_index = 1;

        // Add top components
        for component in &self.top_components {
            let height = match component {
                ComponentType::Title => {
                    // Measure actual title text
                    if let Some(t) = title {
                        let (height, _width) =
                            ChartLayout::measure_text(&t.text, t.font_size, &t.font_family);
                        height * 1.15 // Reduced spacing below title
                    } else {
                        28.0 // Fallback
                    }
                }
                ComponentType::Subtitle => {
                    // Measure actual subtitle text
                    if let Some(s) = subtitle {
                        let (height, _width) =
                            ChartLayout::measure_text(&s.text, s.font_size, &s.font_family);
                        height * 1.5 // More spacing below subtitle
                    } else {
                        20.0 // Fallback
                    }
                }
                _ => self.measure_component_height(
                    component,
                    axes,
                    legends,
                    scales,
                    available_space,
                )?,
            };
            rows.push(length(height));

            // Track component position
            match component {
                ComponentType::Axis(_pos) => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                ComponentType::Title => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                ComponentType::Subtitle => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                ComponentType::Legend(_channel) => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                ComponentType::LegendContainer(_position) => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                _ => {}
            }
            row_index += 1;
        }

        // Plot area row (flexible)
        let plot_row_index = row_index;
        rows.push(fr(1.0));
        row_index += 1;

        // Add bottom components
        for component in &self.bottom_components {
            let height =
                self.measure_component_height(component, axes, legends, scales, available_space)?;
            rows.push(length(height));

            // Track component position
            match component {
                ComponentType::Axis(_pos) => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                ComponentType::Legend(_channel) => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                ComponentType::LegendContainer(_position) => {
                    map.cells
                        .insert((row_index, plot_col_index), component.clone());
                }
                _ => {}
            }
            row_index += 1;
        }

        // End with bottom margin
        rows.push(length(bottom_margin));

        // Store plot area position
        map.cells
            .insert((plot_row_index, plot_col_index), ComponentType::PlotArea);

        // Now add left and right components at the plot row
        let mut left_col_index = 1;
        for component in &self.left_components {
            map.cells
                .insert((plot_row_index, left_col_index), component.clone());
            left_col_index += 1;
        }

        let mut right_col_index = plot_col_index + 1;
        for component in &self.right_components {
            map.cells
                .insert((plot_row_index, right_col_index), component.clone());
            right_col_index += 1;
        }

        // Update component map dimensions
        map.row_count = rows.len();
        map.col_count = cols.len();

        Ok((GridTemplate { rows, cols }, map))
    }

    fn measure_component_width<C: crate::coords::CoordinateSystem>(
        &self,
        component: &ComponentType,
        axes: &HashMap<String, CartesianAxis>,
        legends: &HashMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        available_space: Size<f32>,
        marks: &[Box<dyn crate::marks::Mark<C>>],
    ) -> Result<f32, AvengerChartError> {
        match component {
            ComponentType::Axis(position) => {
                // Find axis with this position
                for (channel, axis) in axes {
                    if axis.position == Some(*position) {
                        if let Some(scale) = scales.get(channel) {
                            let size =
                                ChartLayout::measure_axis_size(axis, scale, available_space)?;
                            return Ok(size.width);
                        }
                    }
                }
                Ok(60.0) // Default width
            }
            ComponentType::Legend(channel) => {
                if let Some(legend) = legends.get(channel) {
                    if let Some(scale) = scales.get(channel) {
                        let size = ChartLayout::measure_legend_size_impl(
                            channel,
                            legend,
                            scale,
                            scales,
                            available_space,
                            None,  // No mark encodings available in this context
                            false, // No line mark information in this context
                        )?;
                        // No padding compensation needed since we removed all padding
                        return Ok(size.width);
                    }
                }
                Ok(120.0) // Default width
            }
            ComponentType::LegendContainer(position) => {
                // Measure the width needed for all legends at this position
                // For left/right positions, use the maximum width of all legends
                // For top/bottom positions, this would be the sum of widths (for horizontal layout)
                let mut max_width = 0.0f32;

                if let Some(legends_at_position) = self.legends_by_position.get(position) {
                    for (channel, legend) in legends_at_position {
                        if let Some(scale) = scales.get(channel) {
                            let size = ChartLayout::measure_legend_size(
                                channel,
                                legend,
                                scale,
                                scales,
                                available_space,
                                marks,
                            )?;
                            max_width = max_width.max(size.width);
                        }
                    }
                }

                if max_width > 0.0 {
                    Ok(max_width)
                } else {
                    Ok(120.0) // Default width
                }
            }
            _ => Ok(0.0),
        }
    }

    fn measure_component_height(
        &self,
        component: &ComponentType,
        axes: &HashMap<String, CartesianAxis>,
        legends: &HashMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        available_space: Size<f32>,
    ) -> Result<f32, AvengerChartError> {
        match component {
            ComponentType::Axis(position) => {
                // Find axis with this position
                for (channel, axis) in axes {
                    if axis.position == Some(*position) {
                        if let Some(scale) = scales.get(channel) {
                            let size =
                                ChartLayout::measure_axis_size(axis, scale, available_space)?;
                            return Ok(size.height);
                        }
                    }
                }
                Ok(50.0) // Default height
            }
            ComponentType::Legend(channel) => {
                if let Some(legend) = legends.get(channel) {
                    if let Some(scale) = scales.get(channel) {
                        let size = ChartLayout::measure_legend_size_impl(
                            channel,
                            legend,
                            scale,
                            scales,
                            available_space,
                            None,  // No mark encodings available in this context
                            false, // No line mark information in this context
                        )?;
                        // No padding compensation needed since we removed all padding
                        return Ok(size.height);
                    }
                }
                Ok(100.0) // Default height
            }
            ComponentType::LegendContainer(position) => {
                // Measure the height needed for all legends at this position
                // Taffy will handle gaps, so we just sum the content heights

                if let Some(legends_at_position) = self.legends_by_position.get(position) {
                    match position {
                        LegendPosition::Left | LegendPosition::Right => {
                            // Vertical layout - sum heights (Taffy handles gaps)
                            let mut total_height = 0.0f32;
                            let mut legend_count = 0;
                            for (channel, legend) in legends_at_position {
                                if let Some(scale) = scales.get(channel) {
                                    let size = ChartLayout::measure_legend_size_impl(
                                        channel,
                                        legend,
                                        scale,
                                        scales,
                                        available_space,
                                        None,  // No mark encodings available in this context
                                        false, // No line mark information in this context
                                    )?;
                                    total_height += size.height;
                                    legend_count += 1;
                                }
                            }

                            // Add gap space that Taffy will include (n-1 gaps of 10px each)
                            if legend_count > 1 {
                                total_height += (legend_count - 1) as f32 * 10.0;
                            }

                            if total_height > 0.0 {
                                Ok(total_height)
                            } else {
                                Ok(100.0) // Default height
                            }
                        }
                        LegendPosition::Top | LegendPosition::Bottom => {
                            // Horizontal layout - use max height
                            let mut max_height = 0.0f32;
                            for (channel, legend) in legends_at_position {
                                if let Some(scale) = scales.get(channel) {
                                    let size = ChartLayout::measure_legend_size_impl(
                                        channel,
                                        legend,
                                        scale,
                                        scales,
                                        available_space,
                                        None,  // No mark encodings available in this context
                                        false, // No line mark information in this context
                                    )?;
                                    max_height = max_height.max(size.height);
                                }
                            }
                            if max_height > 0.0 {
                                Ok(max_height)
                            } else {
                                Ok(100.0) // Default height
                            }
                        }
                    }
                } else {
                    Ok(100.0) // Default height
                }
            }
            _ => Ok(0.0),
        }
    }

    #[allow(dead_code)]
    fn build(&self) -> (GridTemplate, ComponentGridMap) {
        // Calculate grid dimensions
        let mut rows = vec![];
        let mut cols = vec![];
        let mut map = ComponentGridMap {
            cells: HashMap::new(),
            row_count: 0,
            col_count: 0,
        };

        // Build column template
        if !self.left_components.is_empty() {
            cols.push(length(60.0)); // Y-axis width
        }
        cols.push(fr(1.0)); // Plot area
        if !self.right_components.is_empty() {
            cols.push(length(120.0)); // Legend width
        }

        // Build row template
        if !self.top_components.is_empty() {
            rows.push(length(30.0)); // Title/top axis height
        }
        rows.push(fr(1.0)); // Plot area
        if !self.bottom_components.is_empty() {
            rows.push(length(50.0)); // X-axis height
        }

        // Map components to grid cells
        for (component, placement) in &self.components {
            for row in placement.row..placement.row + placement.row_span {
                for col in placement.col..placement.col + placement.col_span {
                    map.cells.insert((row, col), component.clone());
                }
            }
        }

        map.row_count = rows.len();
        map.col_count = cols.len();

        (GridTemplate { rows, cols }, map)
    }
}

impl ComponentGridMap {
    #[allow(dead_code)]
    fn new() -> Self {
        ComponentGridMap {
            cells: HashMap::new(),
            row_count: 0,
            col_count: 0,
        }
    }
}
