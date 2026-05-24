use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegendRendererKind {
    Symbol,
    Line,
    Rect,
    Colorbar,
}

impl LegendRendererKind {
    pub fn theme_selector(self) -> &'static str {
        match self {
            LegendRendererKind::Symbol => "symbol",
            LegendRendererKind::Line => "line",
            LegendRendererKind::Rect => "rect",
            LegendRendererKind::Colorbar => "colorbar",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LegendPosition {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LegendOrientation {
    Horizontal,
    Vertical,
}
