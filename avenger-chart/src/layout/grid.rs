//! Grid layout building logic
use crate::theme::Theme;

use super::sizing::LayoutSpec;
use super::types::{ComponentType, MIN_GUIDE_OVERFLOW_SIZE, OverflowSide};
use crate::error::AvengerChartError;
use crate::legend::LegendPosition;
use crate::plot::{PlotSubtitle, PlotTitle};
use avenger_text::measurement::{TextMeasurementConfig, TextMeasurer, default_text_measurer};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec};
use indexmap::IndexMap;
use std::collections::HashMap;
use taffy::Size;
use taffy::prelude::*;

/// Spacing multipliers for title and subtitle rows
const TITLE_ROW_HEIGHT_MULTIPLIER: f32 = 1.15;
const SUBTITLE_ROW_HEIGHT_MULTIPLIER: f32 = 1.1;

/// Dynamic grid builder for chart layouts
///
/// `GridBuilder` constructs CSS Grid layouts for charts by dynamically positioning components
/// based on their semantic roles and spatial requirements. The builder is order-independent -
/// components can be added in any order and will be positioned deterministically based on
/// their types and the overflow measurements
///
/// ## Component Positioning
///
/// Components are positioned in layers moving outward from the plot area:
/// - **Innermost**: Plot area (where data marks are rendered)
/// - **Next layer**: Guide overflows (directly adjacent to plot area, in overflow regions)
/// - **Outer layers**: Legends (farther from plot, after guide overflows)
/// - **Outermost**: Titles/subtitles (top only, before any guide overflows/legends)
///
/// ## Grid Structure
///
/// The builder creates a grid with the following structure:
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
/// │    │            │ (guide)         │        │       │    │
/// │    ├────────────┼─────────────────┼────────┼───────┤    │
/// │    │ Overflow-  │                 │Overflow│Right  │    │
/// │    │ left       │   Plot Area     │-right  │Legend │    │
/// │    │(guide)     │                 │(guide) │       │    │
/// │    ├────────────┼─────────────────┼────────┼───────┤    │
/// │    │            │ Overflow-bottom │        │       │    │
/// │    │            │ (guide)         │        │       │    │
/// ├────├────────────┴─────────────────┴────────┴───────┼────┤
/// │    │                                               │    │ ← margin row
/// └────┴───────────────────────────────────────────────┴────┘
///    ↑ margin column                                     ↑ margin column
/// ```
///
/// ## Dynamic Columns and Rows
///
/// The builder dynamically adds columns and rows based on:
/// - **Overflow requirements**: Space needed for guide elements beyond plot area
/// - **Legend containers**: Each legend position gets its own column/row
/// - **Titles**: Title and subtitle each get their own row
///
/// Only overflow regions larger than `MIN_GUIDE_OVERFLOW_SIZE` are created to avoid
/// unnecessary grid complexity for minimal overflows.
///
/// ## Component Positioning Rules
///
/// Components are positioned deterministically based on their types:
/// - **Margins**: Always outermost rows/columns
/// - **Title/Subtitle**: Top rows, before any guide overflows
/// - **Guide overflows**: Adjacent to plot area, created based on overflow measurements
/// - **Plot area**: Center, flexible sizing
/// - **Legend containers**: Outside guide overflows, preserving insertion order within each position
///
/// ## Legend Containers
///
/// Legends are grouped by position into containers:
/// - Multiple legends at the same position share a container
/// - The container manages internal spacing and alignment
/// - Container width is determined by the widest legend it contains
/// - Only channel names are stored, preserving insertion order per position
/// - Containers are created automatically during build phase
///
/// ## Build Process
///
/// The `build_with_overflow()` method:
/// 1. Creates margin columns/rows from `layout_spec.margins`
/// 2. Adds title/subtitle rows with measured heights
/// 3. Adds guide overflow columns/rows based on measured requirements
/// 4. Places the plot area in the center (flexible sizing with `fr(1.0)`)
/// 5. Adds legend container columns with measured widths
/// 6. Returns a `GridLayout` with track sizing functions and component positions
///    mapping components to their grid positions
pub(crate) struct GridBuilder {
    // Simple flags for component presence
    pub has_title: bool,
    pub has_subtitle: bool,

    // Track legend channels by position, preserving insertion order
    // This is the only place where insertion order matters (for stacking)
    pub legends_by_position: IndexMap<LegendPosition, Vec<String>>,
}

#[derive(Debug)]
pub(crate) struct GridLayout {
    /// CSS Grid row track sizing functions
    pub rows: Vec<TrackSizingFunction>,
    /// CSS Grid column track sizing functions
    pub cols: Vec<TrackSizingFunction>,
    /// Maps grid cells (row, col) to their component types
    pub component_cells: HashMap<(usize, usize), ComponentType>,
}

impl GridLayout {
    /// Create a new empty grid layout
    pub fn new() -> Self {
        GridLayout {
            rows: Vec::new(),
            cols: Vec::new(),
            component_cells: HashMap::new(),
        }
    }

    /// Add a component at the specified grid position
    pub fn add_component(&mut self, component: ComponentType, row: usize, col: usize) {
        self.component_cells.insert((row, col), component);
    }

    /// Find the grid position of a component
    pub fn find_component_position(&self, component: &ComponentType) -> Option<(usize, usize)> {
        for ((row, col), comp) in &self.component_cells {
            if std::mem::discriminant(comp) == std::mem::discriminant(component) {
                // For guide overflow positions, check specific position match
                if let (ComponentType::GuideOverflow(pos1), ComponentType::GuideOverflow(pos2)) =
                    (comp, component)
                {
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
}

impl GridBuilder {
    pub fn new() -> Self {
        GridBuilder {
            has_title: false,
            has_subtitle: false,
            legends_by_position: IndexMap::new(),
        }
    }

    pub fn add_title(&mut self) {
        self.has_title = true;
    }

    pub fn add_subtitle(&mut self) {
        self.has_subtitle = true;
    }

    pub fn add_legend(&mut self, channel: String, position: LegendPosition) {
        // Only store channel names, preserving insertion order per position
        self.legends_by_position
            .entry(position)
            .or_default()
            .push(channel);
    }

    /// Helper to get the starting column for title/subtitle content
    /// Returns the left overflow column if present, otherwise the plot column
    fn get_content_start_col(left_overflow_col: Option<usize>, plot_col_index: usize) -> usize {
        left_overflow_col.unwrap_or(plot_col_index)
    }

    pub fn measure_legend_container_width(
        &self,
        channels: &[String],
        legend_sizes: &HashMap<String, Size<f32>>,
    ) -> f32 {
        let mut max_width: f32 = 0.0;
        for channel in channels {
            if let Some(size) = legend_sizes.get(channel) {
                max_width = max_width.max(size.width);
            }
        }
        max_width
    }

    /// Build the final grid template based on collected components and overflow requirements.
    ///
    /// Returns a `GridLayout` containing both the track sizing functions and component positions
    pub fn build_with_overflow(
        &self,
        overflow: &crate::coords::OverflowSpaceRequirement,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        theme: &Theme,
        layout_spec: &LayoutSpec,
        legend_sizes: &HashMap<String, Size<f32>>,
    ) -> Result<GridLayout, AvengerChartError> {
        // Use margins from layout spec
        let margins = &layout_spec.margins;

        // Determine if margins should be expandable
        let expand_horizontal = layout_spec.should_expand_margins_horizontal();
        let expand_vertical = layout_spec.should_expand_margins_vertical();

        let mut grid = GridLayout::new();

        // === Build Column Template ===
        // Columns are built left-to-right:
        // [margin] [overflow-left?] [plot-area] [overflow-right?] [legends*] [margin]

        // 1. Start with left margin
        // Use fr(1.0) if margins should expand horizontally, otherwise use fixed length
        if expand_horizontal {
            grid.cols.push(fr(1.0));
        } else {
            grid.cols.push(length(margins.left));
        }
        let mut col_index = 1;

        // 2. Add left overflow column if needed for guide overflow (e.g., axis labels extending left)
        let left_overflow_col = if overflow.left > MIN_GUIDE_OVERFLOW_SIZE {
            grid.cols.push(length(overflow.left)); // Exact overflow size
            let idx = col_index;
            col_index += 1;
            Some(idx)
        } else {
            None
        };

        // 3. Add plot area column
        // Use fixed width if plot width is specified, otherwise flexible (fr)
        let plot_col_index = col_index;
        let plot_col_size = match &layout_spec.plot_area {
            crate::layout::SizeMode::Fixed { width, .. }
            | crate::layout::SizeMode::Width(width) => length(*width),
            _ => fr(1.0), // Flexible - takes remaining space
        };
        grid.cols.push(plot_col_size);
        col_index += 1;

        // 4. Add right overflow column if needed for guide overflow (e.g., axis labels extending right)
        let right_overflow_col = if overflow.right > MIN_GUIDE_OVERFLOW_SIZE {
            grid.cols.push(length(overflow.right)); // Exact overflow size
            let idx = col_index;
            col_index += 1;
            Some(idx)
        } else {
            None
        };

        // 5. Add columns for right-positioned legend containers
        // Each container gets its own column with measured width
        let mut right_legend_cols = Vec::new();
        if let Some(channels) = self.legends_by_position.get(&LegendPosition::Right) {
            let width = self.measure_legend_container_width(channels, legend_sizes);
            grid.cols.push(length(width));
            right_legend_cols.push(col_index);
            // col_index would be incremented here if we had more legend positions
            let _ = col_index + 1;
        }

        // 6. End with right margin
        // Use fr(1.0) if margins should expand horizontally, otherwise use fixed length
        if expand_horizontal {
            grid.cols.push(fr(1.0));
        } else {
            grid.cols.push(length(margins.right));
        }

        // === Build Row Template ===
        // Rows are built top-to-bottom:
        // [margin] [title?] [subtitle?] [overflow-top?] [plot-area] [overflow-bottom?] [margin]

        // 1. Start with top margin
        // Use fr(1.0) if margins should expand vertically, otherwise use fixed length
        if expand_vertical {
            grid.rows.push(fr(1.0));
        } else {
            grid.rows.push(length(margins.top));
        }
        let mut row_index = 1;

        // 2. Add title row if present
        if self.has_title {
            if let Some(t) = title {
                let font_size = t
                    .font_size
                    .or_else(|| theme.title_font_size())
                    .unwrap_or(16.0);
                let title_font_family = theme.title_font_family();
                let font_family = t.font_family.as_deref().unwrap_or(&title_font_family);

                // Measure text for layout (using Normal weight/style as approximation)
                let measurer = default_text_measurer();
                let config = TextMeasurementConfig {
                    text: &t.text,
                    font: font_family,
                    font_size,
                    font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
                    font_style: &FontStyle::Normal,
                };
                let bounds = measurer.measure_text_bounds(&config);
                grid.rows
                    .push(length(bounds.line_height * TITLE_ROW_HEIGHT_MULTIPLIER));
                // Title spans from left overflow (if present) or plot area to the end
                let start_col = Self::get_content_start_col(left_overflow_col, plot_col_index);
                grid.add_component(ComponentType::Title, row_index, start_col);
                row_index += 1;
            }
        }

        // 3. Add subtitle row if present
        if self.has_subtitle {
            if let Some(s) = subtitle {
                let font_size = s
                    .font_size
                    .or_else(|| theme.subtitle_font_size())
                    .unwrap_or(14.0);
                let subtitle_font_family = theme.subtitle_font_family();
                let font_family = s.font_family.as_deref().unwrap_or(&subtitle_font_family);

                // Measure text for layout (using Normal weight/style as approximation)
                let measurer = default_text_measurer();
                let config = TextMeasurementConfig {
                    text: &s.text,
                    font: font_family,
                    font_size,
                    font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
                    font_style: &FontStyle::Normal,
                };
                let bounds = measurer.measure_text_bounds(&config);
                grid.rows
                    .push(length(bounds.line_height * SUBTITLE_ROW_HEIGHT_MULTIPLIER));
                // Subtitle spans from left overflow (if present) or plot area to the end
                let start_col = Self::get_content_start_col(left_overflow_col, plot_col_index);
                grid.add_component(ComponentType::Subtitle, row_index, start_col);
                row_index += 1;
            }
        }

        // 4. Add top overflow row if needed for guide overflow (e.g., axis labels extending upward)
        if overflow.top > MIN_GUIDE_OVERFLOW_SIZE {
            grid.rows.push(length(overflow.top)); // Exact overflow size
            grid.add_component(
                ComponentType::GuideOverflow(OverflowSide::Top),
                row_index,
                plot_col_index,
            );
            row_index += 1;
        }

        // 5. Add plot area row
        // Use fixed height if plot height is specified, otherwise flexible (fr)
        let plot_row_index = row_index;
        let plot_row_size = match &layout_spec.plot_area {
            crate::layout::SizeMode::Fixed { height, .. }
            | crate::layout::SizeMode::Height(height) => length(*height),
            _ => fr(1.0), // Flexible - takes remaining vertical space
        };
        grid.rows.push(plot_row_size);
        grid.add_component(ComponentType::PlotArea, plot_row_index, plot_col_index);

        // 6. Position left/right guide overflows in their columns at the plot row
        if let Some(col) = left_overflow_col {
            grid.add_component(
                ComponentType::GuideOverflow(OverflowSide::Left),
                plot_row_index,
                col,
            );
        }

        if let Some(col) = right_overflow_col {
            grid.add_component(
                ComponentType::GuideOverflow(OverflowSide::Right),
                plot_row_index,
                col,
            );
        }

        // 7. Position right legend containers at the plot row
        if self
            .legends_by_position
            .contains_key(&LegendPosition::Right)
            && !right_legend_cols.is_empty()
        {
            grid.add_component(
                ComponentType::LegendContainer(LegendPosition::Right),
                plot_row_index,
                right_legend_cols[0],
            );
        }

        row_index += 1;

        // 8. Add bottom overflow row if needed for guide overflow (e.g., axis labels extending downward)
        if overflow.bottom > MIN_GUIDE_OVERFLOW_SIZE {
            grid.rows.push(length(overflow.bottom));
            grid.add_component(
                ComponentType::GuideOverflow(OverflowSide::Bottom),
                row_index,
                plot_col_index,
            );
        }

        // 9. End with bottom margin
        // Use fr(1.0) if margins should expand vertically, otherwise use fixed length
        if expand_vertical {
            grid.rows.push(fr(1.0));
        } else {
            grid.rows.push(length(margins.bottom));
        }

        Ok(grid)
    }
}
