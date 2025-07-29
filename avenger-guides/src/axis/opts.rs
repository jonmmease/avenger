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
}

impl AxisConfig {
    /// Get axis-specific dimensions (0 width/height for the axis line dimension)
    pub fn axis_dimensions(&self) -> [f32; 2] {
        match self.orientation {
            AxisOrientation::Bottom | AxisOrientation::Top => {
                [self.dimensions[0], 0.0] // Full width, zero height
            }
            AxisOrientation::Left | AxisOrientation::Right => {
                [0.0, self.dimensions[1]] // Zero width, full height
            }
        }
    }
}
