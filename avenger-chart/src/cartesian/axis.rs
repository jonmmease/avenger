use crate::axis::{AxisPosition, AxisTrait};
use std::any::Any;

/// Axis for Cartesian coordinates
#[derive(Clone, Debug)]
pub struct CartesianAxis {
    pub visible: bool,
    pub position: Option<AxisPosition>,
    pub title: Option<String>,
    pub grid: bool,
    pub tick_count: Option<usize>,
    pub label_angle: f32,
    pub format_number: Option<String>,
}

impl CartesianAxis {
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

impl Default for CartesianAxis {
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
