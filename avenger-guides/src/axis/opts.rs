use avenger_format::PreparedNumberFormatter;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub enum AxisOrientation {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct AxisConfig {
    pub orientation: AxisOrientation,
    pub dimensions: [f32; 2],
    pub grid: bool,
    /// Labels numeric ticks together with `format_ticks`. Band and point axes label numeric
    /// categories with it, each by its own digits.
    pub format: Arc<dyn PreparedNumberFormatter>,
}
