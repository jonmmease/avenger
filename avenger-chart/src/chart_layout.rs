use crate::axis::{AxisPosition, CartesianAxis};
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
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
    legend_nodes: HashMap<String, NodeId>, // Keyed by legend channel
    legend_sizes: HashMap<String, Size<f32>>, // Store measured sizes
    #[allow(dead_code)]
    title_node: Option<NodeId>,

    // Grid configuration
    grid_template: GridTemplate,
    #[allow(dead_code)]
    component_map: ComponentGridMap,
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
    Legend(String), // Channel name
    Title,
    Padding, // Empty space
}

/// Result of layout computation
#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub plot_area: LayoutBounds,
    pub axes: HashMap<AxisPosition, LayoutBounds>,
    pub legends: HashMap<String, LayoutBounds>,
    #[allow(dead_code)]
    pub title: Option<LayoutBounds>,
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
    left_components: Vec<ComponentType>, // Order: axis first, then legends
    right_components: Vec<ComponentType>, // Order: axis first, then legends
    top_components: Vec<ComponentType>,  // Order: axis first, then legends
    bottom_components: Vec<ComponentType>, // Order: axis first, then legends
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
    /// Create a new chart layout with default axes and legends included
    pub fn new(
        axes: &HashMap<String, CartesianAxis>,
        legends: &HashMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        preferred_size: Option<(f32, f32)>,
    ) -> Result<Self, AvengerChartError> {
        let mut taffy = TaffyTree::new();
        let mut builder = GridBuilder::new();

        // Analyze component positions
        let axes_by_position = Self::group_axes_by_position(axes);

        // Build grid structure dynamically
        builder.add_plot_area(); // Always present

        // Add axes by position
        for (position, axis_channels) in &axes_by_position {
            builder.add_axes_at_position(*position, axis_channels.len());
        }

        // Add legends
        for (channel, legend) in legends.iter() {
            builder.add_legend(channel.clone(), legend);
        }

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
            legend_nodes: HashMap::new(),
            legend_sizes: HashMap::new(),
            title_node: None,
            grid_template,
            component_map,
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
                )?;
                legend_sizes.insert(channel.clone(), size);
            }
        }
        layout.legend_sizes = legend_sizes;

        // Create nodes for each component
        layout.create_component_nodes(&axes_by_position, legends)?;

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
        for (position, _channels) in axes_by_position {
            let axis_style = self.create_axis_style_from_grid(
                *position,
                &self.component_map,
                plot_row as i16,
                plot_col as i16,
            )?;
            let node = self.taffy.new_leaf(axis_style)?;
            self.axis_nodes.insert(*position, node);
        }

        // Create legend nodes
        for (channel, legend) in legends.iter() {
            let legend_style = self.create_legend_style_from_grid(
                legend,
                channel,
                &self.component_map,
                plot_row as i16,
                plot_col as i16,
            )?;
            let node = self.taffy.new_leaf(legend_style)?;
            self.legend_nodes.insert(channel.clone(), node);
        }

        // Set children of root node
        let mut children = vec![];
        if let Some(plot_node) = self.plot_area_node {
            children.push(plot_node);
        }
        children.extend(self.axis_nodes.values());
        children.extend(self.legend_nodes.values());

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

    /// Create legend style based on grid component map
    fn create_legend_style_from_grid(
        &self,
        legend: &Legend,
        channel: &str,
        component_map: &ComponentGridMap,
        plot_row: i16,
        plot_col: i16,
    ) -> Result<Style, AvengerChartError> {
        // Find the grid position for this legend
        for ((row, col), component) in &component_map.cells {
            if let ComponentType::Legend(legend_channel) = component {
                if legend_channel == channel {
                    let _position = legend.position.unwrap_or(LegendPosition::Right);

                    // Use the measured legend size if available
                    let size = if let Some(measured_size) = self.legend_sizes.get(channel) {
                        Size {
                            width: length(measured_size.width),
                            height: length(measured_size.height),
                        }
                    } else {
                        // Fallback to default size
                        Size {
                            width: length(120.0),
                            height: length(100.0),
                        }
                    };

                    return Ok(Style {
                        grid_row: line((*row + 1) as i16),    // Convert to 1-based
                        grid_column: line((*col + 1) as i16), // Convert to 1-based
                        size,                                 // Set explicit size for the legend
                        padding: Rect {
                            left: length(0.0),
                            right: length(0.0),
                            top: length(0.0),
                            bottom: length(0.0),
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
        for (channel, node) in &self.legend_nodes {
            let layout = self.taffy.layout(*node)?;
            // Debug: Show legend computed bounds
            // eprintln!("Legend {} computed bounds: x={}, y={}, width={}, height={}",
            //          channel, layout.location.x, layout.location.y,
            //          layout.size.width, layout.size.height);
            result.legends.insert(
                channel.clone(),
                LayoutBounds {
                    x: layout.location.x,
                    y: layout.location.y,
                    width: layout.size.width,
                    height: layout.size.height,
                },
            );
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
        };

        // Create axis marks
        let title = axis.title.as_deref().unwrap_or("");
        let origin = [0.0, 0.0];

        // Generate axis marks based on scale type
        let scale_type = scale.scale_impl.scale_type();

        let axis_group = match &*scale_type {
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

    /// Measure legend size by creating the actual SceneGroup and measuring its bounding box
    #[allow(dead_code)]
    pub fn measure_legend_size(
        channel: &str,
        legend: &Legend,
        scale: &ConfiguredScale,
        scales: &HashMap<String, ConfiguredScale>,
        _available_space: Size<f32>,
    ) -> Result<Size<f32>, AvengerChartError> {
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
        let text_values: Vec<String> =
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
                // Cast failed - use fallback labels
                (0..domain_len)
                    .map(|i| format!("Type {}", (b'A' + (i as u8 % 26)) as char))
                    .collect()
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

        // Determine if this should be a line legend based on channel name
        // Line-specific channels: stroke_dash, stroke_width, stroke (when used with line marks)
        let should_use_line_legend =
            !should_use_colorbar && matches!(channel, "stroke" | "stroke_dash" | "stroke_width");

        let legend_group = if should_use_colorbar {
            // Create a colorbar for continuous color scales
            use avenger_guides::legend::colorbar::{
                ColorbarConfig, ColorbarOrientation, make_colorbar_marks,
            };

            // For measurement, use a reasonable size
            let config = ColorbarConfig {
                orientation: ColorbarOrientation::Right,
                dimensions: [100.0, 200.0], // Available space for measurement
                colorbar_width: Some(15.0),
                colorbar_height: Some(150.0),
                colorbar_margin: Some(0.0),
                left_padding: None,
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
                let dash_patterns = vec![
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
                stroke_cap: avenger_common::types::StrokeCap::Butt,
                font_size: ScalarOrArray::new_scalar(10.0),
                font_family: ScalarOrArray::new_scalar("sans-serif".to_string()),
                inner_width: 0.0,
                inner_height: 100.0,
                outer_margin: 0.0,
                entry_margin: 2.0,
                text_padding: 4.0, // Consistent with symbol legend spacing
                line_length: 16.0, // Similar to symbol size, enough for dash patterns
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
                if fill_domain.len() == scale.domain().len() {
                    // Map domain through fill scale to get colors
                    match fill_scale.scale(scale.domain()) {
                        Ok(scaled_array) => {
                            use datafusion::arrow::array::{Array, Float32Array};
                            use datafusion::arrow::compute::cast;
                            use datafusion::arrow::datatypes::DataType;

                            // Try to cast to Float32Array (colors are typically RGBA values)
                            if let Ok(color_array) = cast(&scaled_array, &DataType::Float32) {
                                if let Some(float_array) =
                                    color_array.as_any().downcast_ref::<Float32Array>()
                                {
                                    // Group into RGBA colors (4 values per color)
                                    let mut colors = Vec::new();
                                    let mut i = 0;
                                    while i + 3 < float_array.len() {
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

            let config = SymbolLegendConfig {
                title: legend.title.clone(),
                text: ScalarOrArray::new_array(text_values.clone()),
                shape: shape_values,
                size: size_values,
                fill: fill_values,
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: Some(1.0),
                angle: ScalarOrArray::new_scalar(0.0),
                inner_width: 0.0,
                inner_height: 100.0, // Match the render config
                outer_margin: 0.0,
                text_padding: 2.0, // Match the default from legend config
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
        let component = ComponentType::Legend(channel);
        let position = legend.position.unwrap_or(LegendPosition::Right);

        match position {
            LegendPosition::Right => {
                // Add legend after axes (farther from plot)
                self.right_components.push(component);
            }
            LegendPosition::Left => {
                // Add legend after axes (farther from plot)
                self.left_components.push(component);
            }
            LegendPosition::Top => {
                // Add legend before axes (farther from plot)
                self.top_components.insert(0, component);
            }
            LegendPosition::Bottom => {
                // Add legend after axes (farther from plot)
                self.bottom_components.push(component);
            }
        }
    }

    fn build_with_measurements(
        &self,
        axes: &HashMap<String, CartesianAxis>,
        legends: &HashMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        available_space: Size<f32>,
    ) -> Result<(GridTemplate, ComponentGridMap), AvengerChartError> {
        // Use minimal edge margins since components are measured with their own padding
        // Only add a small margin to ensure edges aren't clipped
        const EDGE_MARGIN: f32 = 5.0;

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
            let width =
                self.measure_component_width(component, axes, legends, scales, available_space)?;
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
            let width =
                self.measure_component_width(component, axes, legends, scales, available_space)?;
            cols.push(length(width));

            // Don't track component position here - will do it after we know plot row
            col_index += 1;
        }

        // End with right margin (doubled if no right components)
        cols.push(length(right_margin));

        // === Build Row Template ===
        // Start with top margin (doubled if no top components)
        rows.push(length(top_margin));
        let mut row_index = 1;

        // Add top components
        for component in &self.top_components {
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

    fn measure_component_width(
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
                            return Ok(size.width);
                        }
                    }
                }
                Ok(60.0) // Default width
            }
            ComponentType::Legend(channel) => {
                if let Some(legend) = legends.get(channel) {
                    if let Some(scale) = scales.get(channel) {
                        let size = ChartLayout::measure_legend_size(
                            channel,
                            legend,
                            scale,
                            scales,
                            available_space,
                        )?;
                        // No padding compensation needed since we removed all padding
                        return Ok(size.width);
                    }
                }
                Ok(120.0) // Default width
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
                        let size = ChartLayout::measure_legend_size(
                            channel,
                            legend,
                            scale,
                            scales,
                            available_space,
                        )?;
                        // No padding compensation needed since we removed all padding
                        return Ok(size.height);
                    }
                }
                Ok(100.0) // Default height
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
