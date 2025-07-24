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
#[derive(Clone, Copy, Debug)]
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

#[cfg(test)]
mod tests {
    use crate::coords::Cartesian;
    use crate::plot::Plot;

    #[test]
    fn test_axis_x_with_configuration() {
        let plot = Plot::new(Cartesian)
            .scale_x(|scale| scale)
            .axis_x(|axis| axis.title("Temperature").grid(true));

        // Should have axis configured
        assert!(plot.axes.contains_key("x"));
    }

    #[test]
    fn test_axis_x_with_visible_false() {
        let plot = Plot::new(Cartesian)
            .scale_x(|scale| scale)
            .axis_x(|axis| axis.visible(false));

        // Axis still exists but is marked invisible
        assert!(plot.axes.contains_key("x"));
    }

    #[test]
    fn test_axis_x_with_defaults() {
        let plot = Plot::new(Cartesian)
            .scale_x(|scale| scale)
            .axis_x(|axis| axis.title("Modified"));

        // Should have axis with defaults modified
        assert!(plot.axes.contains_key("x"));
    }
}
