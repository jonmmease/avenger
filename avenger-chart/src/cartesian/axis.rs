use crate::axis::{Axis as AxisBase, AxisPosition};
use std::any::Any;

/// Trait for axes that can be used with Cartesian coordinates
pub trait CartesianAxis: AxisBase + Clone + Send + Sync + 'static {
    /// Get axis visibility
    fn visible(&self) -> bool;

    /// Get axis position (Top, Right, Bottom, Left)
    fn position(&self) -> Option<AxisPosition>;

    /// Get axis title
    fn title(&self) -> Option<&str>;

    /// Whether to show grid lines
    fn grid(&self) -> bool;

    /// Get tick count hint
    fn tick_count(&self) -> Option<usize>;

    /// Get label angle in degrees
    fn label_angle(&self) -> f32;

    /// Get number format pattern
    fn format_number(&self) -> Option<&str>;
}

/// Default implementation of CartesianAxis
#[derive(Clone, Debug)]
pub struct DefaultCartesianAxis {
    pub visible: bool,
    pub position: Option<AxisPosition>,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub label_angle: f32,
    pub format_number: Option<String>,
}

impl DefaultCartesianAxis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn position(mut self, position: AxisPosition) -> Self {
        self.position = Some(position);
        self
    }

    pub fn title<S: Into<String>>(mut self, title: S) -> Self {
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

    pub fn label_angle(mut self, angle: f32) -> Self {
        self.label_angle = angle;
        self
    }

    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format_number = Some(format.into());
        self
    }
}

impl Default for DefaultCartesianAxis {
    fn default() -> Self {
        Self {
            visible: true,
            position: None,
            title: None,
            grid: false,
            tick_count: None,
            label_angle: 0.0,
            format_number: None,
        }
    }
}

impl AxisBase for DefaultCartesianAxis {
    fn clone_box(&self) -> Box<dyn AxisBase> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl CartesianAxis for DefaultCartesianAxis {
    fn visible(&self) -> bool {
        self.visible
    }

    fn position(&self) -> Option<AxisPosition> {
        self.position
    }

    fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    fn grid(&self) -> bool {
        self.grid
    }

    fn tick_count(&self) -> Option<usize> {
        self.tick_count
    }

    fn label_angle(&self) -> f32 {
        self.label_angle
    }

    fn format_number(&self) -> Option<&str> {
        self.format_number.as_deref()
    }
}
