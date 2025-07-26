//! Layout types for composing multiple charts

use crate::coords::CoordinateSystem;
use crate::plot::Plot;
use crate::scales::Scale;
use std::collections::HashMap;

/// Unique identifier for plots within a layout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlotId(usize);

/// Represents a rectangular area with position and size
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Padding around a plot area
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Padding {
    pub fn uniform(value: f32) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }
}

impl Default for Padding {
    fn default() -> Self {
        Self {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        }
    }
}

/// Size preferences for layout
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeConstraints {
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
    pub preferred_width: Option<f32>,
    pub preferred_height: Option<f32>,
    pub aspect_ratio: Option<f32>,
}

impl Default for SizeConstraints {
    fn default() -> Self {
        Self {
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            preferred_width: None,
            preferred_height: None,
            aspect_ratio: None,
        }
    }
}

/// Alignment options for layout
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Alignment {
    Start,
    Center,
    End,
    /// Align plot areas (not container bounds)
    PlotArea,
}

/// Trait that all plots must implement to participate in layouts
pub trait PlotTrait: Send + Sync {
    /// Get the size constraints for this plot
    fn size_constraints(&self) -> SizeConstraints;

    /// Measure components (labels, titles, etc) for the given plot area size
    /// Returns the required padding to accommodate all components
    fn measure_padding(&self, plot_width: f32, plot_height: f32) -> Padding;

    /// Render the plot at the given position and size
    fn render(&self, bounds: Rect);
}

/// Type-erased wrapper for plots
struct AnyPlot(Box<dyn PlotTrait>);

impl<C: CoordinateSystem + 'static> PlotTrait for Plot<C> {
    fn size_constraints(&self) -> SizeConstraints {
        SizeConstraints {
            preferred_width: self.get_preferred_size().map(|(w, _)| w as f32),
            preferred_height: self.get_preferred_size().map(|(_, h)| h as f32),
            aspect_ratio: self.get_aspect_ratio().map(|r| r as f32),
            ..Default::default()
        }
    }

    fn measure_padding(&self, _plot_width: f32, _plot_height: f32) -> Padding {
        // TODO: Calculate padding dynamically based on plot components
        // For now, return default padding
        // In the future, this should:
        // 1. Measure axis label and tick sizes for each configured axis
        // 2. Measure title height if present
        // 3. Measure legend dimensions if present
        // 4. Calculate required padding to accommodate all components
        Padding {
            left: 60.0,   // Space for y-axis labels, ticks, and title
            right: 20.0,  // Space for right-side elements
            top: 20.0,    // Space for title
            bottom: 45.0, // Space for x-axis labels, ticks, and title
        }
    }

    fn render(&self, _bounds: Rect) {
        // TODO: Implement rendering at specified bounds
        // This will use bounds.width and bounds.height instead of self.width/height
    }
}

/// Layout arrangement types
#[derive(Debug, Clone, Copy)]
pub enum LayoutMode {
    /// Horizontal concatenation
    Horizontal,
    /// Vertical concatenation  
    Vertical,
    /// Grid layout
    Grid { rows: usize, cols: usize },
}

/// Computed layout information for a plot
#[derive(Debug, Clone)]
pub struct PlotLayout {
    /// The plot ID
    pub id: PlotId,
    /// Total bounds including padding
    pub bounds: Rect,
    /// Plot area bounds (excluding padding)
    pub plot_area: Rect,
    /// Applied padding
    pub padding: Padding,
}

/// A layout that arranges multiple plots
pub struct Layout {
    /// Layout mode
    mode: LayoutMode,

    /// Plots stored by ID
    plots: HashMap<PlotId, AnyPlot>,

    /// Computed layouts
    computed_layouts: Vec<PlotLayout>,

    /// Shared scales accessible by name
    shared_scales: HashMap<String, Scale>,

    /// Layout properties
    gap: f32,
    container_padding: Padding,
    alignment: Alignment,

    /// Next plot ID
    next_id: usize,
}

impl Layout {
    /// Create a new layout with the specified mode
    pub fn new(mode: LayoutMode) -> Self {
        Self {
            mode,
            plots: HashMap::new(),
            computed_layouts: Vec::new(),
            shared_scales: HashMap::new(),
            gap: 0.0,
            container_padding: Padding::default(),
            alignment: Alignment::PlotArea,
            next_id: 0,
        }
    }

    /// Create a horizontal concatenation layout
    pub fn horizontal() -> Self {
        Self::new(LayoutMode::Horizontal)
    }

    /// Create a vertical concatenation layout
    pub fn vertical() -> Self {
        Self::new(LayoutMode::Vertical)
    }

    /// Create a grid layout
    pub fn grid(rows: usize, cols: usize) -> Self {
        Self::new(LayoutMode::Grid { rows, cols })
    }

    /// Add a plot to the layout
    pub fn add_plot<P: PlotTrait + 'static>(&mut self, plot: P) -> PlotId {
        let id = self.next_plot_id();
        self.plots.insert(id, AnyPlot(Box::new(plot)));
        id
    }

    /// Add a named scale that can be referenced by plots
    pub fn add_scale<S: Into<String>>(&mut self, name: S, scale: Scale) {
        self.shared_scales.insert(name.into(), scale);
    }

    /// Get a shared scale by name
    pub fn get_scale(&self, name: &str) -> Option<&Scale> {
        self.shared_scales.get(name)
    }

    /// Set the gap between items
    pub fn set_gap(&mut self, gap: f32) {
        self.gap = gap;
    }

    /// Set container padding
    pub fn set_padding(&mut self, padding: Padding) {
        self.container_padding = padding;
    }

    /// Set alignment mode
    pub fn set_alignment(&mut self, alignment: Alignment) {
        self.alignment = alignment;
    }

    /// Compute the layout with the given available space
    pub fn compute(&mut self, width: f32, height: f32) -> Result<(), String> {
        // This is where we'll implement the constraint-based layout
        // For now, we'll implement a simple version

        let available_width = width - self.container_padding.left - self.container_padding.right;
        let available_height = height - self.container_padding.top - self.container_padding.bottom;

        match self.mode {
            LayoutMode::Horizontal => {
                self.compute_horizontal_layout(available_width, available_height)
            }
            LayoutMode::Vertical => self.compute_vertical_layout(available_width, available_height),
            LayoutMode::Grid { rows, cols } => {
                self.compute_grid_layout(available_width, available_height, rows, cols)
            }
        }
    }

    /// Get the computed layouts
    pub fn get_layouts(&self) -> &[PlotLayout] {
        &self.computed_layouts
    }

    /// Get the computed layout for a specific plot
    pub fn get_plot_layout(&self, plot_id: PlotId) -> Option<&PlotLayout> {
        self.computed_layouts.iter().find(|l| l.id == plot_id)
    }

    /// Internal: Generate next plot ID
    fn next_plot_id(&mut self) -> PlotId {
        let id = PlotId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Compute horizontal layout
    fn compute_horizontal_layout(&mut self, width: f32, height: f32) -> Result<(), String> {
        // TODO: Implement proper constraint-based layout
        // For now, simple equal distribution
        let plot_count = self.plots.len();
        if plot_count == 0 {
            return Ok(());
        }

        let total_gap = self.gap * (plot_count - 1) as f32;
        let plot_width = (width - total_gap) / plot_count as f32;

        self.computed_layouts.clear();
        let mut x = self.container_padding.left;

        for (id, plot) in &self.plots {
            let padding = plot.0.measure_padding(plot_width, height);
            let plot_area = Rect::new(
                x + padding.left,
                self.container_padding.top + padding.top,
                plot_width - padding.left - padding.right,
                height - padding.top - padding.bottom,
            );

            self.computed_layouts.push(PlotLayout {
                id: *id,
                bounds: Rect::new(x, self.container_padding.top, plot_width, height),
                plot_area,
                padding,
            });

            x += plot_width + self.gap;
        }

        Ok(())
    }

    /// Compute vertical layout
    fn compute_vertical_layout(&mut self, width: f32, height: f32) -> Result<(), String> {
        // TODO: Implement proper constraint-based layout
        // For now, simple equal distribution
        let plot_count = self.plots.len();
        if plot_count == 0 {
            return Ok(());
        }

        let total_gap = self.gap * (plot_count - 1) as f32;
        let plot_height = (height - total_gap) / plot_count as f32;

        self.computed_layouts.clear();
        let mut y = self.container_padding.top;

        for (id, plot) in &self.plots {
            let padding = plot.0.measure_padding(width, plot_height);
            let plot_area = Rect::new(
                self.container_padding.left + padding.left,
                y + padding.top,
                width - padding.left - padding.right,
                plot_height - padding.top - padding.bottom,
            );

            self.computed_layouts.push(PlotLayout {
                id: *id,
                bounds: Rect::new(self.container_padding.left, y, width, plot_height),
                plot_area,
                padding,
            });

            y += plot_height + self.gap;
        }

        Ok(())
    }

    /// Compute grid layout
    fn compute_grid_layout(
        &mut self,
        width: f32,
        height: f32,
        rows: usize,
        cols: usize,
    ) -> Result<(), String> {
        // TODO: Implement proper constraint-based layout
        // For now, simple equal distribution
        if rows == 0 || cols == 0 {
            return Ok(());
        }

        let h_gap_total = self.gap * (cols - 1) as f32;
        let v_gap_total = self.gap * (rows - 1) as f32;
        let cell_width = (width - h_gap_total) / cols as f32;
        let cell_height = (height - v_gap_total) / rows as f32;

        self.computed_layouts.clear();

        for (idx, (id, plot)) in self.plots.iter().enumerate() {
            let row = idx / cols;
            let col = idx % cols;

            if row >= rows {
                break; // More plots than grid cells
            }

            let x = self.container_padding.left + col as f32 * (cell_width + self.gap);
            let y = self.container_padding.top + row as f32 * (cell_height + self.gap);

            let padding = plot.0.measure_padding(cell_width, cell_height);
            let plot_area = Rect::new(
                x + padding.left,
                y + padding.top,
                cell_width - padding.left - padding.right,
                cell_height - padding.top - padding.bottom,
            );

            self.computed_layouts.push(PlotLayout {
                id: *id,
                bounds: Rect::new(x, y, cell_width, cell_height),
                plot_area,
                padding,
            });
        }

        Ok(())
    }
}

/// Builder for creating layouts with a fluent API
pub struct LayoutBuilder {
    layout: Layout,
}

impl LayoutBuilder {
    /// Create a horizontal layout builder
    pub fn horizontal() -> Self {
        Self {
            layout: Layout::horizontal(),
        }
    }

    /// Create a vertical layout builder
    pub fn vertical() -> Self {
        Self {
            layout: Layout::vertical(),
        }
    }

    /// Create a grid layout builder
    pub fn grid(rows: usize, cols: usize) -> Self {
        Self {
            layout: Layout::grid(rows, cols),
        }
    }

    /// Add a plot to the layout
    pub fn plot<P: PlotTrait + 'static>(mut self, plot: P) -> Self {
        self.layout.add_plot(plot);
        self
    }

    /// Add multiple plots
    pub fn plots<P: PlotTrait + 'static>(mut self, plots: Vec<P>) -> Self {
        for plot in plots {
            self.layout.add_plot(plot);
        }
        self
    }

    /// Set the gap between items
    pub fn gap(mut self, gap: f32) -> Self {
        self.layout.set_gap(gap);
        self
    }

    /// Set padding
    pub fn padding(mut self, padding: Padding) -> Self {
        self.layout.set_padding(padding);
        self
    }

    /// Set uniform padding
    pub fn padding_uniform(mut self, value: f32) -> Self {
        self.layout.set_padding(Padding::uniform(value));
        self
    }

    /// Set alignment
    pub fn align(mut self, alignment: Alignment) -> Self {
        self.layout.set_alignment(alignment);
        self
    }

    /// Add a shared scale
    pub fn scale<S: Into<String>>(mut self, name: S, scale: Scale) -> Self {
        self.layout.add_scale(name, scale);
        self
    }

    /// Build the layout
    pub fn build(self) -> Layout {
        self.layout
    }
}

#[cfg(test)]
mod examples {
    use super::*;
    use crate::coords::Cartesian;
    use crate::marks::line::Line;
    use crate::marks::symbol::Symbol;
    use crate::plot::Plot;
    use crate::scales::Scale;
    use avenger_scales::scales::linear::LinearScale;

    #[allow(dead_code)]
    fn example_simple_layout() {
        // Example 1: Simple horizontal concatenation
        let plot1 = Plot::new(Cartesian)
            .preferred_size(400.0, 300.0)
            .scale_x(|s| s.domain((0.0, 100.0)))
            .mark(Symbol::new().x("x").y("y"));

        let plot2 = Plot::new(Cartesian)
            .preferred_size(400.0, 300.0)
            .scale_x(|s| s.domain((0.0, 50.0)))
            .mark(Line::new().x("x").y("y"));

        let mut layout = LayoutBuilder::horizontal()
            .plot(plot1)
            .plot(plot2)
            .gap(20.0)
            .padding_uniform(10.0)
            .build();

        // Compute layout
        layout.compute(840.0, 320.0).unwrap();
    }

    #[allow(dead_code)]
    fn example_aspect_ratio_layout() {
        // Example: Plots with different aspect ratios
        let plot1 = Plot::new(Cartesian)
            .aspect_ratio(16.0 / 9.0)
            .mark(Line::new().x("time").y("value"));

        let plot2 = Plot::new(Cartesian)
            .aspect_ratio(4.0 / 3.0)
            .mark(Symbol::new().x("x").y("y"));

        let plot3 = Plot::new(Cartesian)
            .aspect_ratio(1.0)
            .mark(Symbol::new().x("x").y("y"));

        let mut layout = LayoutBuilder::horizontal()
            .plots(vec![plot1, plot2, plot3])
            .gap(10.0)
            .align(Alignment::PlotArea) // Align plot areas, not containers
            .build();

        // When computed with constraints, this would:
        // 1. Respect aspect ratios
        // 2. Align plot area bottoms
        // 3. Adjust padding as needed
        layout.compute(1200.0, 400.0).unwrap();
    }

    #[allow(dead_code)]
    fn example_grid_with_shared_scales() {
        // Example: Grid layout with shared scales
        let mut layout = LayoutBuilder::grid(2, 2)
            .gap(5.0)
            .scale("x_scale", Scale::new(LinearScale).domain((0.0, 100.0)))
            .scale("y_scale", Scale::new(LinearScale).domain((0.0, 50.0)))
            .build();

        // Add plots that reference shared scales
        for _i in 0..4 {
            let plot = Plot::new(Cartesian)
                .scale_x_ref("x_scale")
                .scale_y_ref("y_scale")
                .mark(Symbol::new().x("x").y("y"));

            layout.add_plot(plot);
        }

        layout.compute(800.0, 800.0).unwrap();
    }
}
