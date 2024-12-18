#[derive(Debug, Clone)]
pub struct Axis {
    pub scale: String,
    pub orientation: AxisOrientation,
    pub ticks: Option<bool>,
    pub grid: Option<bool>,
}

impl Axis {
    pub fn new(scale: String, orientation: AxisOrientation) -> Self {
        Self {
            scale,
            orientation,
            ticks: None,
            grid: None,
        }
    }

    pub fn ticks(self, ticks: bool) -> Self {
        Self {
            ticks: Some(ticks),
            ..self
        }
    }

    pub fn get_ticks(&self) -> Option<bool> {
        self.ticks
    }

    pub fn grid(self, grid: bool) -> Self {
        Self {
            grid: Some(grid),
            ..self
        }
    }

    pub fn get_grid(&self) -> Option<bool> {
        self.grid
    }
}

#[derive(Debug, Clone)]
pub enum AxisOrientation {
    Top,
    Bottom,
    Left,
    Right,
}
