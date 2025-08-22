use std::any::Any;

/// Trait for all axis types
///
/// The clone_box, as_any, and into_any methods enable storing different
/// axis types in a type-erased collection while preserving the ability
/// to downcast back to concrete types when needed.
pub trait AxisTrait: Send + Sync {
    fn clone_box(&self) -> Box<dyn AxisTrait>;
    fn as_any(&self) -> &dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Position for Cartesian axes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AxisPosition {
    Top,
    Right,
    Bottom,
    Left,
}

/// Axis for Cartesian coordinates
#[derive(Clone, Debug)]
pub struct CartesianAxis {
    pub visible: bool,
    pub position: Option<AxisPosition>,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub label_angle: Option<f64>,
    pub format_number: Option<String>,
}

impl CartesianAxis {
    pub fn new() -> Self {
        Self {
            visible: true,
            position: None,
            title: None,
            grid: false,
            tick_count: None,
            label_angle: None,
            format_number: None,
        }
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn position(mut self, position: AxisPosition) -> Self {
        self.position = Some(position);
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    pub fn tick_count(mut self, count: usize) -> Self {
        self.tick_count = Some(count);
        self
    }

    pub fn label_angle(mut self, angle: f64) -> Self {
        self.label_angle = Some(angle);
        self
    }

    /// Set a numeric formatting string (e.g., "$,.2f", ".1%", ".2s").
    /// This passes directly to the number formatter used by scales.
    pub fn format_number(mut self, pattern: impl Into<String>) -> Self {
        self.format_number = Some(pattern.into());
        self
    }
}

impl Default for CartesianAxis {
    fn default() -> Self {
        Self::new()
    }
}

impl AxisTrait for CartesianAxis {
    fn clone_box(&self) -> Box<dyn AxisTrait> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// Type of polar axis
#[derive(Clone, Debug, PartialEq)]
pub enum PolarAxisType {
    Radial,  // r axis
    Angular, // theta axis
}

/// Direction for angular axis
#[derive(Clone, Debug, PartialEq)]
pub enum PolarDirection {
    Clockwise,
    CounterClockwise,
}

/// Axis for Polar coordinates
#[derive(Clone, Debug)]
pub struct PolarAxis {
    pub visible: bool,
    pub axis_type: PolarAxisType,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub format_number: Option<String>,
    // Polar-specific properties
    pub grid_levels: Option<usize>, // For radial axis concentric circles
    pub start_angle: f64,           // For angular axis (default 0)
    pub direction: PolarDirection,  // Clockwise or CounterClockwise
}

impl PolarAxis {
    pub fn new(axis_type: PolarAxisType) -> Self {
        Self {
            visible: true,
            axis_type,
            title: None,
            grid: false,
            tick_count: None,
            format_number: None,
            grid_levels: None,
            start_angle: 0.0,
            direction: PolarDirection::CounterClockwise,
        }
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn grid(mut self, grid: bool) -> Self {
        self.grid = grid;
        self
    }

    pub fn tick_count(mut self, count: usize) -> Self {
        self.tick_count = Some(count);
        self
    }

    pub fn format_number(mut self, pattern: impl Into<String>) -> Self {
        self.format_number = Some(pattern.into());
        self
    }

    pub fn grid_levels(mut self, levels: usize) -> Self {
        self.grid_levels = Some(levels);
        self
    }

    pub fn start_angle(mut self, angle: f64) -> Self {
        self.start_angle = angle;
        self
    }

    pub fn direction(mut self, direction: PolarDirection) -> Self {
        self.direction = direction;
        self
    }
}

impl Default for PolarAxis {
    fn default() -> Self {
        Self::new(PolarAxisType::Radial)
    }
}

impl AxisTrait for PolarAxis {
    fn clone_box(&self) -> Box<dyn AxisTrait> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::coords::Cartesian;
    use crate::plot::Plot;

    #[test]
    fn test_axis_x_with_configuration() {
        let plot = Plot::new(Cartesian)
            .scale_x(|scale| scale)
            .axis_x(|axis| axis.title("Temperature").grid(true));

        // Should have axis spec configured
        assert!(plot.axis_specs.contains_key("x"));
    }

    #[test]
    fn test_axis_x_with_visible_false() {
        let plot = Plot::new(Cartesian)
            .scale_x(|scale| scale)
            .axis_x(|axis| axis.visible(false));

        // Axis spec still exists (visibility will be applied during rendering)
        assert!(plot.axis_specs.contains_key("x"));
    }

    #[test]
    fn test_axis_x_with_defaults() {
        let plot = Plot::new(Cartesian)
            .scale_x(|scale| scale)
            .axis_x(|axis| axis.title("Modified"));

        // Should have axis spec configured
        assert!(plot.axis_specs.contains_key("x"));
    }
}
