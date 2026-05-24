use avenger_chart_core::{IntoExpr, Legend};

/// Base trait for legend builders - just for common functionality
/// Each concrete builder implements this for shared methods
pub trait LegendBuilder: Sized {
    /// Get mutable access to the inner legend
    fn legend_mut(&mut self) -> &mut Legend;

    /// Build the final legend
    fn build(self) -> Legend;

    /// Set visibility - available on all legend builders
    fn visible(mut self, visible: impl IntoExpr) -> Self {
        let legend = self.legend_mut().clone();
        *self.legend_mut() = legend.visible(visible);
        self
    }
}

// Concrete builder for color channels (fill, stroke)
#[derive(Default)]
pub struct ColorLegendBuilder {
    legend: Legend,
}

impl ColorLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.legend = self.legend.title(title);
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.legend = self.legend.visible(visible);
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        self.legend = self.legend.position(position);
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        self.legend = self.legend.orientation(orientation);
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
        self.legend = self.legend.order(order);
        self
    }

    // Color-specific methods
    pub fn gradient_thickness(mut self, thickness: impl IntoExpr) -> Self {
        self.legend = self.legend.gradient_thickness(thickness);
        self
    }

    pub fn columns(mut self, columns: impl IntoExpr) -> Self {
        self.legend = self.legend.columns(columns);
        self
    }

    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        self.legend = self.legend.symbol_size(size);
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        self.legend = self.legend.label_limit(limit);
        self
    }

    pub fn format_number(mut self, format: impl IntoExpr) -> Self {
        self.legend = self.legend.format_number(format);
        self
    }

    pub fn background_fill(mut self, fill: impl IntoExpr) -> Self {
        self.legend = self.legend.background_fill(fill);
        self
    }

    pub fn background_stroke(mut self, stroke: impl IntoExpr) -> Self {
        self.legend = self.legend.background_stroke(stroke);
        self
    }

    pub fn background_corner_radius(mut self, radius: impl IntoExpr) -> Self {
        self.legend = self.legend.background_corner_radius(radius);
        self
    }

    pub fn background_padding(mut self, padding: impl IntoExpr) -> Self {
        self.legend = self.legend.background_padding(padding);
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
#[derive(Default)]
pub struct SizeLegendBuilder {
    legend: Legend,
}

impl SizeLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.legend = self.legend.title(title);
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.legend = self.legend.visible(visible);
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        self.legend = self.legend.position(position);
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        self.legend = self.legend.orientation(orientation);
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
        self.legend = self.legend.order(order);
        self
    }

    // Size-specific methods
    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        self.legend = self.legend.symbol_size(size);
        self
    }

    pub fn columns(mut self, columns: impl IntoExpr) -> Self {
        self.legend = self.legend.columns(columns);
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        self.legend = self.legend.label_limit(limit);
        self
    }

    pub fn format_number(mut self, format: impl IntoExpr) -> Self {
        self.legend = self.legend.format_number(format);
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
#[derive(Default)]
pub struct ShapeLegendBuilder {
    legend: Legend,
}

impl ShapeLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.legend = self.legend.title(title);
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.legend = self.legend.visible(visible);
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        self.legend = self.legend.position(position);
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        self.legend = self.legend.orientation(orientation);
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
        self.legend = self.legend.order(order);
        self
    }

    // Shape-specific methods
    pub fn columns(mut self, columns: impl IntoExpr) -> Self {
        self.legend = self.legend.columns(columns);
        self
    }

    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        self.legend = self.legend.symbol_size(size);
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        self.legend = self.legend.label_limit(limit);
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
#[derive(Default)]
pub struct OpacityLegendBuilder {
    legend: Legend,
}

impl OpacityLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.legend = self.legend.title(title);
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.legend = self.legend.visible(visible);
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        self.legend = self.legend.position(position);
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        self.legend = self.legend.orientation(orientation);
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
        self.legend = self.legend.order(order);
        self
    }

    // Opacity-specific methods
    pub fn gradient_thickness(mut self, thickness: impl IntoExpr) -> Self {
        self.legend = self.legend.gradient_thickness(thickness);
        self
    }

    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        self.legend = self.legend.symbol_size(size);
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        self.legend = self.legend.label_limit(limit);
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
#[derive(Default)]
pub struct AngleLegendBuilder {
    legend: Legend,
}

impl AngleLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.legend = self.legend.title(title);
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.legend = self.legend.visible(visible);
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        self.legend = self.legend.position(position);
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        self.legend = self.legend.orientation(orientation);
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
        self.legend = self.legend.order(order);
        self
    }

    // Angle-specific methods
    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        self.legend = self.legend.symbol_size(size);
        self
    }

    pub fn columns(mut self, columns: impl IntoExpr) -> Self {
        self.legend = self.legend.columns(columns);
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        self.legend = self.legend.label_limit(limit);
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
#[derive(Default)]
pub struct StrokeWidthLegendBuilder {
    legend: Legend,
}

impl StrokeWidthLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.legend = self.legend.title(title);
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.legend = self.legend.visible(visible);
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        self.legend = self.legend.position(position);
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        self.legend = self.legend.orientation(orientation);
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
        self.legend = self.legend.order(order);
        self
    }

    // Stroke width-specific methods
    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        self.legend = self.legend.symbol_size(size);
        self
    }

    pub fn columns(mut self, columns: impl IntoExpr) -> Self {
        self.legend = self.legend.columns(columns);
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        self.legend = self.legend.label_limit(limit);
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
#[derive(Default)]
pub struct StrokeDashLegendBuilder {
    legend: Legend,
}

impl StrokeDashLegendBuilder {
    pub fn new() -> Self {
        Self {
            legend: Legend::new(),
        }
    }

    // Common legend methods
    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.legend = self.legend.title(title);
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.legend = self.legend.visible(visible);
        self
    }

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        self.legend = self.legend.position(position);
        self
    }

    pub fn orientation(mut self, orientation: impl IntoExpr) -> Self {
        self.legend = self.legend.orientation(orientation);
        self
    }

    pub fn order(mut self, order: impl IntoExpr) -> Self {
        self.legend = self.legend.order(order);
        self
    }

    // Stroke dash-specific methods
    pub fn symbol_size(mut self, size: impl IntoExpr) -> Self {
        self.legend = self.legend.symbol_size(size);
        self
    }

    pub fn columns(mut self, columns: impl IntoExpr) -> Self {
        self.legend = self.legend.columns(columns);
        self
    }

    pub fn label_limit(mut self, limit: impl IntoExpr) -> Self {
        self.legend = self.legend.label_limit(limit);
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
