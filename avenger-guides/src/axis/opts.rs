pub enum AxisOrientation {
    Top,
    Bottom { height: f32 },
    Left,
    Right { width: f32 },
}

pub struct AxisConfig {
    pub orientation: AxisOrientation,
}
