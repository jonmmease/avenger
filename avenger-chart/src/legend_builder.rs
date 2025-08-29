use crate::legend::{Legend, LegendOrientation, LegendPosition};
use crate::legend_renderer::LegendRenderer;
use std::sync::Arc;

/// Base trait for legend builders - just for common functionality
/// Each concrete builder implements this for shared methods
pub trait LegendBuilder {
    /// Get mutable access to the inner legend
    fn legend_mut(&mut self) -> &mut Legend;

    /// Build the final legend
    fn build(self) -> Legend;
}

// Concrete builder for color channels (fill, stroke)
pub struct ColorLegendBuilder {
    legend: Legend,
}

impl Default for ColorLegendBuilder {
    fn default() -> Self {
        Self {
            legend: Legend::default(),
        }
    }
}

impl ColorLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.legend.title = Some(title.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.legend.visible = visible;
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.legend.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.legend.orientation = Some(orientation);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.legend.order = Some(order);
        self
    }

    // Color-specific methods
    pub fn gradient_length(mut self, length: f64) -> Self {
        self.legend.gradient_length = Some(length);
        self
    }

    pub fn gradient_thickness(mut self, thickness: f64) -> Self {
        self.legend.gradient_thickness = Some(thickness);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.legend.columns = Some(columns);
        self
    }

    pub fn symbol_size(mut self, size: f64) -> Self {
        self.legend.symbol_size = Some(size);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.legend.label_limit = Some(limit);
        self
    }

    pub fn format_number(mut self, format: impl Into<String>) -> Self {
        self.legend.format_number = Some(format.into());
        self
    }

    pub fn background_fill(mut self, fill: impl Into<String>) -> Self {
        self.legend.background_fill = Some(fill.into());
        self
    }

    pub fn background_stroke(mut self, stroke: impl Into<String>) -> Self {
        self.legend.background_stroke = Some(stroke.into());
        self
    }

    pub fn background_corner_radius(mut self, radius: f32) -> Self {
        self.legend.background_corner_radius = Some(radius);
        self
    }

    pub fn background_padding(mut self, padding: f32) -> Self {
        self.legend.background_padding = Some(padding);
        self
    }

    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.legend.renderer = Some(Arc::new(renderer));
        self
    }
}

impl LegendBuilder for ColorLegendBuilder {
    fn legend_mut(&mut self) -> &mut Legend {
        &mut self.legend
    }

    fn build(self) -> Legend {
        self.legend
    }
}

// Concrete builder for size channels
pub struct SizeLegendBuilder {
    legend: Legend,
}

impl Default for SizeLegendBuilder {
    fn default() -> Self {
        Self {
            legend: Legend::default(),
        }
    }
}

impl SizeLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.legend.title = Some(title.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.legend.visible = visible;
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.legend.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.legend.orientation = Some(orientation);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.legend.order = Some(order);
        self
    }

    // Size-specific methods
    pub fn symbol_size(mut self, size: f64) -> Self {
        self.legend.symbol_size = Some(size);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.legend.columns = Some(columns);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.legend.label_limit = Some(limit);
        self
    }

    pub fn format_number(mut self, format: impl Into<String>) -> Self {
        self.legend.format_number = Some(format.into());
        self
    }

    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.legend.renderer = Some(Arc::new(renderer));
        self
    }
}

impl LegendBuilder for SizeLegendBuilder {
    fn legend_mut(&mut self) -> &mut Legend {
        &mut self.legend
    }

    fn build(self) -> Legend {
        self.legend
    }
}

// Concrete builder for shape channels
pub struct ShapeLegendBuilder {
    legend: Legend,
}

impl Default for ShapeLegendBuilder {
    fn default() -> Self {
        Self {
            legend: Legend::default(),
        }
    }
}

impl ShapeLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.legend.title = Some(title.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.legend.visible = visible;
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.legend.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.legend.orientation = Some(orientation);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.legend.order = Some(order);
        self
    }

    // Shape-specific methods
    pub fn columns(mut self, columns: usize) -> Self {
        self.legend.columns = Some(columns);
        self
    }

    pub fn symbol_size(mut self, size: f64) -> Self {
        self.legend.symbol_size = Some(size);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.legend.label_limit = Some(limit);
        self
    }

    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.legend.renderer = Some(Arc::new(renderer));
        self
    }
}

impl LegendBuilder for ShapeLegendBuilder {
    fn legend_mut(&mut self) -> &mut Legend {
        &mut self.legend
    }

    fn build(self) -> Legend {
        self.legend
    }
}

// Concrete builder for opacity channels
pub struct OpacityLegendBuilder {
    legend: Legend,
}

impl Default for OpacityLegendBuilder {
    fn default() -> Self {
        Self {
            legend: Legend::default(),
        }
    }
}

impl OpacityLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.legend.title = Some(title.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.legend.visible = visible;
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.legend.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.legend.orientation = Some(orientation);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.legend.order = Some(order);
        self
    }

    // Opacity-specific methods
    pub fn gradient_length(mut self, length: f64) -> Self {
        self.legend.gradient_length = Some(length);
        self
    }

    pub fn gradient_thickness(mut self, thickness: f64) -> Self {
        self.legend.gradient_thickness = Some(thickness);
        self
    }

    pub fn symbol_size(mut self, size: f64) -> Self {
        self.legend.symbol_size = Some(size);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.legend.label_limit = Some(limit);
        self
    }

    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.legend.renderer = Some(Arc::new(renderer));
        self
    }
}

impl LegendBuilder for OpacityLegendBuilder {
    fn legend_mut(&mut self) -> &mut Legend {
        &mut self.legend
    }

    fn build(self) -> Legend {
        self.legend
    }
}

// Concrete builder for angle channels
pub struct AngleLegendBuilder {
    legend: Legend,
}

impl Default for AngleLegendBuilder {
    fn default() -> Self {
        Self {
            legend: Legend::default(),
        }
    }
}

impl AngleLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.legend.title = Some(title.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.legend.visible = visible;
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.legend.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.legend.orientation = Some(orientation);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.legend.order = Some(order);
        self
    }

    // Angle-specific methods
    pub fn symbol_size(mut self, size: f64) -> Self {
        self.legend.symbol_size = Some(size);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.legend.columns = Some(columns);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.legend.label_limit = Some(limit);
        self
    }

    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.legend.renderer = Some(Arc::new(renderer));
        self
    }
}

impl LegendBuilder for AngleLegendBuilder {
    fn legend_mut(&mut self) -> &mut Legend {
        &mut self.legend
    }

    fn build(self) -> Legend {
        self.legend
    }
}

// Concrete builder for stroke width channels
pub struct StrokeWidthLegendBuilder {
    legend: Legend,
}

impl Default for StrokeWidthLegendBuilder {
    fn default() -> Self {
        Self {
            legend: Legend::default(),
        }
    }
}

impl StrokeWidthLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.legend.title = Some(title.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.legend.visible = visible;
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.legend.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.legend.orientation = Some(orientation);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.legend.order = Some(order);
        self
    }

    // Stroke width-specific methods
    pub fn symbol_size(mut self, size: f64) -> Self {
        self.legend.symbol_size = Some(size);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.legend.columns = Some(columns);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.legend.label_limit = Some(limit);
        self
    }

    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.legend.renderer = Some(Arc::new(renderer));
        self
    }
}

impl LegendBuilder for StrokeWidthLegendBuilder {
    fn legend_mut(&mut self) -> &mut Legend {
        &mut self.legend
    }

    fn build(self) -> Legend {
        self.legend
    }
}

// Concrete builder for stroke dash channels
pub struct StrokeDashLegendBuilder {
    legend: Legend,
}

impl Default for StrokeDashLegendBuilder {
    fn default() -> Self {
        Self {
            legend: Legend::default(),
        }
    }
}

impl StrokeDashLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.legend.title = Some(title.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.legend.visible = visible;
        self
    }

    pub fn position(mut self, position: LegendPosition) -> Self {
        self.legend.position = Some(position);
        self
    }

    pub fn orientation(mut self, orientation: LegendOrientation) -> Self {
        self.legend.orientation = Some(orientation);
        self
    }

    pub fn order(mut self, order: i32) -> Self {
        self.legend.order = Some(order);
        self
    }

    // Stroke dash-specific methods
    pub fn symbol_size(mut self, size: f64) -> Self {
        self.legend.symbol_size = Some(size);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.legend.columns = Some(columns);
        self
    }

    pub fn label_limit(mut self, limit: f64) -> Self {
        self.legend.label_limit = Some(limit);
        self
    }

    pub fn renderer(mut self, renderer: impl LegendRenderer + 'static) -> Self {
        self.legend.renderer = Some(Arc::new(renderer));
        self
    }
}

impl LegendBuilder for StrokeDashLegendBuilder {
    fn legend_mut(&mut self) -> &mut Legend {
        &mut self.legend
    }

    fn build(self) -> Legend {
        self.legend
    }
}
