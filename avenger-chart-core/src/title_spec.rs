//! Shared title/subtitle spec values.

use datafusion::prelude::{Expr, lit};
use serde::{Deserialize, Serialize};

use crate::IntoExpr;

/// Controls the width that the title/subtitle spans.
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleSpan {
    /// Title/subtitle spans entire canvas width.
    #[default]
    Canvas,
    /// Title/subtitle only spans the plot area width.
    PlotArea,
}

/// Controls the text alignment within the title/subtitle span.
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleAlign {
    /// Text aligned to the left.
    #[default]
    Left,
    /// Text aligned to the center.
    Center,
    /// Text aligned to the right.
    Right,
}

impl TitleSpan {
    /// Convert TitleSpan to a string literal.
    pub fn to_str(&self) -> &'static str {
        match self {
            TitleSpan::Canvas => "canvas",
            TitleSpan::PlotArea => "plot_area",
        }
    }
}

impl TitleAlign {
    /// Convert TitleAlign to a string literal.
    pub fn to_str(&self) -> &'static str {
        match self {
            TitleAlign::Left => "left",
            TitleAlign::Center => "center",
            TitleAlign::Right => "right",
        }
    }
}

impl IntoExpr for TitleSpan {
    fn into_expr(self) -> Expr {
        lit(self.to_str())
    }
}

impl IntoExpr for TitleAlign {
    fn into_expr(self) -> Expr {
        lit(self.to_str())
    }
}
