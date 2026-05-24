use datafusion::prelude::{Expr, lit};

use crate::{AxisPosition, LegendOrientation, LegendPosition, Param};

/// Trait for values that can be converted to DataFusion expressions.
pub trait IntoExpr {
    fn into_expr(self) -> Expr;
}

impl IntoExpr for Expr {
    fn into_expr(self) -> Expr {
        self
    }
}

impl IntoExpr for f32 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for f64 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for i32 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for i64 {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for String {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for &str {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for bool {
    fn into_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoExpr for usize {
    fn into_expr(self) -> Expr {
        lit(self as i64)
    }
}

impl IntoExpr for AxisPosition {
    fn into_expr(self) -> Expr {
        let s = match self {
            AxisPosition::Top => "top",
            AxisPosition::Bottom => "bottom",
            AxisPosition::Left => "left",
            AxisPosition::Right => "right",
        };
        lit(s)
    }
}

impl IntoExpr for Param {
    fn into_expr(self) -> Expr {
        self.expr()
    }
}

impl IntoExpr for &Param {
    fn into_expr(self) -> Expr {
        self.expr()
    }
}

impl IntoExpr for LegendPosition {
    fn into_expr(self) -> Expr {
        let s = match self {
            LegendPosition::Top => "top",
            LegendPosition::Right => "right",
            LegendPosition::Bottom => "bottom",
            LegendPosition::Left => "left",
        };
        lit(s)
    }
}

impl IntoExpr for LegendOrientation {
    fn into_expr(self) -> Expr {
        let s = match self {
            LegendOrientation::Horizontal => "horizontal",
            LegendOrientation::Vertical => "vertical",
        };
        lit(s)
    }
}
