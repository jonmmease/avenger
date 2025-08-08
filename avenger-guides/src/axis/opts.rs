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
    pub format_number: Option<String>,
}
