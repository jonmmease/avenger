use crate::axis::AxisTrait;
use std::any::Any;

/// Type of polar axis
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolarAxisType {
    Radial,
    Angular,
}

/// Direction for angular axis
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub grid_levels: Option<usize>,
    pub start_angle: f32,
    pub direction: PolarDirection,
}

impl PolarAxis {
    pub fn new(axis_type: PolarAxisType) -> Self {
        Self {
            axis_type,
            ..Default::default()
        }
    }

    pub fn radial() -> Self {
        Self::new(PolarAxisType::Radial)
    }

    pub fn angular() -> Self {
        Self::new(PolarAxisType::Angular)
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
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

    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format_number = Some(format.into());
        self
    }

    pub fn grid_levels(mut self, levels: usize) -> Self {
        self.grid_levels = Some(levels);
        self
    }

    pub fn start_angle(mut self, angle: f32) -> Self {
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
        Self {
            visible: true,
            axis_type: PolarAxisType::Radial,
            title: None,
            grid: true,
            tick_count: None,
            format_number: None,
            grid_levels: None,
            start_angle: 0.0,
            direction: PolarDirection::Clockwise,
        }
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
