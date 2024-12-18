#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legend {
    pub direction: Option<LegendDirection>,
    pub orient: Option<LegendOrient>,
}

impl Legend {
    pub fn new() -> Self {
        Self {
            direction: None,
            orient: None,
        }
    }

    pub fn direction(self, direction: LegendDirection) -> Self {
        Self {
            direction: Some(direction),
            ..self
        }
    }

    pub fn get_direction(&self) -> Option<&LegendDirection> {
        self.direction.as_ref()
    }

    pub fn orient(self, orient: LegendOrient) -> Self {
        Self {
            orient: Some(orient),
            ..self
        }
    }

    pub fn get_orient(&self) -> Option<&LegendOrient> {
        self.orient.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegendDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegendOrient {
    Top,
    Bottom,
    Left,
    Right,
}
