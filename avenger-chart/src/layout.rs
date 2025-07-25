//! Layout types for composing multiple charts

use crate::coords::CoordinateSystem;
use crate::plot::Plot;
use crate::scales::Scale;
use std::collections::HashMap;
use taffy::prelude::*;

/// Unique identifier for plots within a layout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlotId(usize);

/// Trait that all plots must implement to participate in layouts
pub trait PlotTrait: Send + Sync {
    /// Get the preferred size of this plot
    fn preferred_size(&self) -> Option<(f32, f32)>;

    /// Get the aspect ratio constraint
    fn aspect_ratio(&self) -> Option<f32>;

    /// Render the plot at the given position and size
    fn render(&self, x: f32, y: f32, width: f32, height: f32);
}

/// Type-erased wrapper for plots
struct AnyPlot(Box<dyn PlotTrait>);

impl<C: CoordinateSystem + 'static> PlotTrait for Plot<C> {
    fn preferred_size(&self) -> Option<(f32, f32)> {
        // TODO: Implement based on Plot's width/height
        None
    }

    fn aspect_ratio(&self) -> Option<f32> {
        // TODO: Implement if Plot has aspect ratio constraint
        None
    }

    fn render(&self, _x: f32, _y: f32, _width: f32, _height: f32) {
        // TODO: Implement rendering at specified position/size
    }
}

/// A layout that arranges multiple plots
pub struct Layout {
    /// Taffy tree for layout computation
    tree: TaffyTree<PlotId>,

    /// Plots stored by ID
    plots: HashMap<PlotId, AnyPlot>,

    /// Shared scales accessible by name
    shared_scales: HashMap<String, Scale>,

    /// Root node of the layout tree
    root: Option<NodeId>,

    /// Next plot ID
    next_id: usize,
}

impl Layout {
    /// Create a new empty layout
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
            plots: HashMap::new(),
            shared_scales: HashMap::new(),
            root: None,
            next_id: 0,
        }
    }

    /// Add a named scale that can be referenced by plots
    pub fn scale<S: Into<String>>(mut self, name: S, scale: Scale) -> Self {
        self.shared_scales.insert(name.into(), scale);
        self
    }

    /// Get a shared scale by name
    pub fn get_scale(&self, name: &str) -> Option<&Scale> {
        self.shared_scales.get(name)
    }

    /// Create a horizontal concatenation layout
    pub fn hconcat<P: PlotTrait + 'static>(plots: Vec<P>) -> LayoutBuilder {
        LayoutBuilder::new(LayoutType::HConcat).plots(plots)
    }

    /// Create a vertical concatenation layout
    pub fn vconcat<P: PlotTrait + 'static>(plots: Vec<P>) -> LayoutBuilder {
        LayoutBuilder::new(LayoutType::VConcat).plots(plots)
    }

    /// Create a grid layout
    pub fn grid(rows: usize, cols: usize) -> GridBuilder {
        GridBuilder::new(rows, cols)
    }

    /// Compute the layout with the given available space
    pub fn compute(&mut self, width: f32, height: f32) -> Result<(), taffy::TaffyError> {
        if let Some(root) = self.root {
            self.tree.compute_layout(
                root,
                Size {
                    width: AvailableSpace::Definite(width),
                    height: AvailableSpace::Definite(height),
                },
            )?;
        }
        Ok(())
    }

    /// Get the computed layout for a plot
    pub fn get_layout(&self, _plot_id: PlotId) -> Option<taffy::Layout> {
        // TODO: Need to track node_id -> plot_id mapping
        None
    }

    /// Internal: Generate next plot ID
    fn next_plot_id(&mut self) -> PlotId {
        let id = PlotId(self.next_id);
        self.next_id += 1;
        id
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self::new()
    }
}

/// Types of layout arrangements
#[derive(Debug, Clone, Copy)]
pub enum LayoutType {
    HConcat,
    VConcat,
    Grid { rows: usize, cols: usize },
}

/// Builder for creating layouts
pub struct LayoutBuilder {
    layout_type: LayoutType,
    style: Style,
    children: Vec<LayoutNode>,
}

/// A node in the layout tree
enum LayoutNode {
    Plot(Box<dyn PlotTrait>),
    Layout(LayoutBuilder),
}

impl LayoutBuilder {
    /// Create a new layout builder
    pub fn new(layout_type: LayoutType) -> Self {
        let style = match layout_type {
            LayoutType::HConcat => Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                ..Default::default()
            },
            LayoutType::VConcat => Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
            LayoutType::Grid { .. } => Style {
                display: Display::Grid,
                ..Default::default()
            },
        };

        Self {
            layout_type,
            style,
            children: Vec::new(),
        }
    }

    /// Add plots to this layout
    pub fn plots<P: PlotTrait + 'static>(mut self, plots: Vec<P>) -> Self {
        for plot in plots {
            self.children.push(LayoutNode::Plot(Box::new(plot)));
        }
        self
    }

    /// Set the gap between items
    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = Size {
            width: LengthPercentage::length(gap),
            height: LengthPercentage::length(gap),
        };
        self
    }

    /// Set the size of this layout
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.style.size = Size {
            width: length(width),
            height: length(height),
        };
        self
    }

    /// Set minimum size constraints
    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.style.min_size = Size {
            width: length(width),
            height: length(height),
        };
        self
    }

    /// Set maximum size constraints
    pub fn max_size(mut self, width: f32, height: f32) -> Self {
        self.style.max_size = Size {
            width: length(width),
            height: length(height),
        };
        self
    }

    /// Set padding
    pub fn padding(mut self, padding: f32) -> Self {
        self.style.padding = Rect {
            left: LengthPercentage::length(padding),
            right: LengthPercentage::length(padding),
            top: LengthPercentage::length(padding),
            bottom: LengthPercentage::length(padding),
        };
        self
    }

    /// Set alignment for flex layouts
    pub fn align_items(mut self, align: AlignItems) -> Self {
        self.style.align_items = Some(align);
        self
    }

    /// Set justification for flex layouts
    pub fn justify_content(mut self, justify: JustifyContent) -> Self {
        self.style.justify_content = Some(justify);
        self
    }

    /// Build the layout
    pub fn build(self) -> Layout {
        let layout = Layout::new();
        // TODO: Convert LayoutBuilder into Layout with proper Taffy tree
        layout
    }
}

/// Builder for grid layouts
pub struct GridBuilder {
    rows: usize,
    cols: usize,
    style: Style,
    cells: HashMap<(usize, usize), Box<dyn PlotTrait>>,
}

impl GridBuilder {
    /// Create a new grid builder
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            style: Style {
                display: Display::Grid,
                grid_template_columns: vec![fr(1.0); cols],
                grid_template_rows: vec![fr(1.0); rows],
                ..Default::default()
            },
            cells: HashMap::new(),
        }
    }

    /// Add a plot to a specific cell
    pub fn cell<P: PlotTrait + 'static>(mut self, row: usize, col: usize, plot: P) -> Self {
        self.cells.insert((row, col), Box::new(plot));
        self
    }

    /// Set the gap between cells
    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = Size {
            width: LengthPercentage::length(gap),
            height: LengthPercentage::length(gap),
        };
        self
    }

    /// Set explicit column widths
    pub fn column_widths(mut self, widths: Vec<TrackSizingFunction>) -> Self {
        self.style.grid_template_columns = widths;
        self
    }

    /// Set explicit row heights
    pub fn row_heights(mut self, heights: Vec<TrackSizingFunction>) -> Self {
        self.style.grid_template_rows = heights;
        self
    }

    /// Build the grid layout
    pub fn build(self) -> Layout {
        let layout = Layout::new();
        // TODO: Convert GridBuilder into Layout with proper Taffy tree
        layout
    }
}

/// Helper functions for creating Taffy dimensions
pub fn length(value: f32) -> Dimension {
    Dimension::length(value)
}

pub fn percent(value: f32) -> Dimension {
    Dimension::percent(value)
}

pub fn auto() -> Dimension {
    Dimension::auto()
}

pub fn fr(value: f32) -> TrackSizingFunction {
    taffy::style_helpers::fr(value)
}

pub fn minmax(min: f32, max: f32) -> TrackSizingFunction {
    taffy::style_helpers::minmax(
        taffy::MinTrackSizingFunction::length(min),
        taffy::MaxTrackSizingFunction::length(max),
    )
}

#[cfg(test)]
mod examples {
    use super::*;
    use crate::coords::Cartesian;
    use crate::marks::line::Line;
    use crate::marks::rect::Rect;
    use crate::marks::symbol::Symbol;
    use crate::plot::Plot;
    use crate::scales::Scale;
    use avenger_scales::scales::linear::LinearScale;

    #[allow(dead_code)]
    fn example_simple_layout() {
        // Example 1: Simple horizontal concatenation
        let plot1 = Plot::new(Cartesian)
            .width(400.0)
            .height(300.0)
            .scale_x(|s| s.domain((0.0, 100.0)))
            .mark(Symbol::new().x("x").y("y"));

        let plot2 = Plot::new(Cartesian)
            .width(400.0)
            .height(300.0)
            .scale_x(|s| s.domain((0.0, 50.0)))
            .mark(Line::new().x("x").y("y"));

        let _layout = Layout::hconcat(vec![plot1, plot2]).gap(20.0).build();

        // Example 2: Vertical layout with constraints
        // Note: Can't reuse plot1 and plot2 as they were moved
        let plot3 = Plot::new(Cartesian)
            .width(400.0)
            .height(300.0)
            .scale_x(|s| s.domain((0.0, 100.0)))
            .mark(Symbol::new().x("x").y("y"));

        let plot4 = Plot::new(Cartesian)
            .width(400.0)
            .height(300.0)
            .scale_x(|s| s.domain((0.0, 50.0)))
            .mark(Line::new().x("x").y("y"));

        let _layout2 = Layout::vconcat(vec![plot3, plot4])
            .gap(10.0)
            .min_size(800.0, 600.0)
            .padding(20.0)
            .build();
    }

    #[allow(dead_code)]
    fn example_shared_scales() {
        // Example: Two plots sharing the same y-scale
        let plot1 = Plot::new(Cartesian)
            .scale_x(|s| s.domain((0.0, 100.0)))
            .scale_y_ref("shared_y") // Reference shared scale
            .mark(Symbol::new().x("x").y("y"));

        let plot2 = Plot::new(Cartesian)
            .scale_x(|s| s.domain((0.0, 50.0)))
            .scale_y_ref("shared_y") // Same shared scale
            .mark(Line::new().x("x").y("y"));

        let layout = Layout::hconcat(vec![plot1, plot2]).gap(20.0).build();
        let _layout = layout.scale("shared_y", Scale::new(LinearScale).domain((0.0, 200.0)));
    }

    #[allow(dead_code)]
    fn example_grid_layout() {
        // Example: 2x2 grid layout
        let _layout = Layout::grid(2, 2)
            .cell(
                0,
                0,
                Plot::new(Cartesian)
                    .preferred_size(300.0, 300.0)
                    .mark(Symbol::new().x("x").y("y")),
            )
            .cell(
                0,
                1,
                Plot::new(Cartesian)
                    .preferred_size(300.0, 300.0)
                    .mark(Symbol::new().x("x").y("y")),
            )
            .cell(
                1,
                0,
                Plot::new(Cartesian)
                    .preferred_size(300.0, 300.0)
                    .mark(Symbol::new().x("x").y("y")),
            )
            .cell(
                1,
                1,
                Plot::new(Cartesian)
                    .preferred_size(300.0, 300.0)
                    .mark(Symbol::new().x("x").y("y")),
            )
            .gap(10.0)
            .build();

        // Grid with different column widths
        let _layout2 = Layout::grid(2, 3)
            .column_widths(vec![fr(2.0), fr(1.0), fr(1.0)]) // First column twice as wide
            .row_heights(vec![minmax(100.0, 100.0), fr(1.0)]) // Fixed height first row
            .build();
    }

    #[allow(dead_code)]
    fn example_splom() {
        // Example: Scatter Plot Matrix (SPLOM)
        let variables = vec!["sepal_length", "sepal_width", "petal_length", "petal_width"];
        let n = variables.len();

        // Create grid builder
        let mut grid = Layout::grid(n, n);

        // Create plots for each cell
        for (i, var_y) in variables.iter().enumerate() {
            for (j, var_x) in variables.iter().enumerate() {
                let plot = if i == j {
                    // Diagonal: histogram
                    Plot::new(Cartesian)
                        .scale_x_ref(*var_x)
                        .mark(Rect::new().x(*var_x))
                } else {
                    // Off-diagonal: scatter plot
                    Plot::new(Cartesian)
                        .scale_x_ref(*var_x)
                        .scale_y_ref(*var_y)
                        .mark(Symbol::new().x(*var_x).y(*var_y))
                };

                grid = grid.cell(i, j, plot);
            }
        }

        let mut splom = grid.gap(5.0).build();

        // Add shared scales for each variable
        for var in &variables {
            splom = splom.scale(
                *var,
                Scale::new(LinearScale).domain((0.0, 100.0)), // Example domain
            );
        }
    }

    #[allow(dead_code)]
    fn example_nested_layout() {
        // Example: Complex dashboard with nested layouts
        let _overview_plot = Plot::new(Cartesian)
            .preferred_size(800.0, 200.0)
            .mark(Line::new().x("date").y("value"));

        let detail_plot1 = Plot::new(Cartesian)
            .scale_x_ref("time") // Shared time scale
            .mark(Symbol::new().x("time").y("metric1"));

        let detail_plot2 = Plot::new(Cartesian)
            .scale_x_ref("time") // Same shared time scale
            .mark(Symbol::new().x("time").y("metric2"));

        let _detail_section = Layout::hconcat(vec![detail_plot1, detail_plot2]).gap(20.0);

        // Build the dashboard as a nested layout
        // (Note: nested layouts would require LayoutBuilder to implement PlotTrait)
        let _dashboard = Layout::new().scale("time", Scale::new(LinearScale).domain((0.0, 24.0)));
    }

    #[allow(dead_code)]
    fn example_responsive_layout() {
        // Example: Layout with responsive constraints
        let plot1 = Plot::new(Cartesian)
            .aspect_ratio(16.0 / 9.0) // Maintain aspect ratio
            .mark(Line::new().x("x").y("y"));

        let _plot2 = Plot::new(Cartesian).mark(Symbol::new().x("x").y("y"));

        let sidebar = Plot::new(Cartesian).mark(Rect::new().x("category").y("count"));

        // Responsive layout with flexible sizing
        let _layout = Layout::hconcat(vec![sidebar, plot1])
            .gap(20.0)
            .min_size(800.0, 600.0)
            .max_size(1920.0, 1080.0)
            .build();
    }
}
