//! Grid layout building logic

use crate::cartesian::axis::AxisPosition;
use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendPosition};
use crate::plot::{PlotSubtitle, PlotTitle};
use avenger_scales::scales::ConfiguredScale;
use indexmap::IndexMap;
use std::collections::HashMap;
use taffy::prelude::*;

use super::chart_layout::ChartLayout;
use super::text::measure_text;
use super::types::{ComponentGridMap, ComponentType, EDGE_MARGIN, OVERFLOW_THRESHOLD};

/// Builder for dynamically constructing grid layout
pub(crate) struct GridBuilder {
    pub components: Vec<(ComponentType, GridPlacement)>,

    // Track components by position for dynamic grid building
    pub left_components: Vec<ComponentType>, // Order: axis first, then legend container
    pub right_components: Vec<ComponentType>, // Order: axis first, then legend container
    pub top_components: Vec<ComponentType>,  // Order: axis first, then legend container, then title
    pub bottom_components: Vec<ComponentType>, // Order: axis first, then legend container

    // Track legends by position for container creation
    pub legends_by_position: IndexMap<LegendPosition, Vec<(String, Legend)>>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct GridPlacement {
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
}

#[derive(Debug)]
pub(crate) struct GridTemplate {
    pub rows: Vec<TrackSizingFunction>,
    pub cols: Vec<TrackSizingFunction>,
}

impl GridBuilder {
    pub fn new() -> Self {
        GridBuilder {
            components: Vec::new(),
            left_components: Vec::new(),
            right_components: Vec::new(),
            top_components: Vec::new(),
            bottom_components: Vec::new(),
            legends_by_position: IndexMap::new(),
        }
    }

    pub fn add_plot_area(&mut self) {
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

    pub fn add_title(&mut self) {
        // Title sits at the top area before axes/legends
        self.top_components.insert(0, ComponentType::Title);
    }

    pub fn add_subtitle(&mut self) {
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

    pub fn add_axes_at_position(&mut self, position: AxisPosition, _count: usize) {
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

    pub fn add_legend(&mut self, channel: String, legend: &Legend) {
        let position = legend.position.unwrap_or(LegendPosition::Right);

        // Collect legends by position for later container creation
        self.legends_by_position
            .entry(position)
            .or_default()
            .push((channel, legend.clone()));
    }

    pub fn finalize_legend_containers(&mut self) {
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
    pub fn measure_legend_container_width<C: crate::coords::CoordinateSystem>(
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
                    let (size, _flexible) = ChartLayout::measure_legend_size(
                        channel, legend, scale, scales, available, marks,
                    )?;
                    max_width = max_width.max(size.width);
                }
            }
        }
        Ok(max_width)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_with_overflow<C: crate::coords::CoordinateSystem>(
        &self,
        overflow: &crate::coords::OverflowSpaceRequirement,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScale>,
        _available_space: Size<f32>,
        _marks: &[Box<dyn crate::marks::Mark<C>>],
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        theme: &dyn crate::theme::Theme,
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
            let font_size = t.font_size.unwrap_or(theme.title_font_size());
            let title_font_family = theme.title_font_family();
            let font_family = t.font_family.as_deref().unwrap_or(&title_font_family);
            let (height, _) = measure_text(&t.text, font_size, font_family);
            rows.push(length(height * 1.15));
            // Title starts from left overflow column (if present) or plot column
            let title_start_col = left_overflow_col.unwrap_or(plot_col_index);
            component_map.add_component(ComponentType::Title, row_index, title_start_col);
            row_index += 1;
        }

        // Add subtitle if present
        if let Some(s) = subtitle {
            let font_size = s.font_size.unwrap_or(theme.subtitle_font_size());
            let subtitle_font_family = theme.subtitle_font_family();
            let font_family = s.font_family.as_deref().unwrap_or(&subtitle_font_family);
            let (height, _) = measure_text(&s.text, font_size, font_family);
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
