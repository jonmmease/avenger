use datafusion::logical_expr::Expr;
use datafusion_common::ScalarValue;
use palette::Srgba;

#[derive(Debug, Clone)]
pub enum ScaleRange {
    Numeric(Expr, Box<Expr>),
    Enum(Vec<ScalarValue>),
    Color(Vec<Srgba>),
}

impl ScaleRange {
    pub fn new_interval<E: Into<Expr>, F: Into<Expr>>(start: E, end: F) -> Self {
        Self::Numeric(start.into(), Box::new(end.into()))
    }

    pub fn new_color(colors: Vec<Srgba>) -> Self {
        Self::Color(colors)
    }

    pub fn new_enum<T: Into<ScalarValue>>(values: Vec<T>) -> Self {
        Self::Enum(values.into_iter().map(|v| v.into()).collect())
    }

    /// Alias for new_enum (backward compat)
    pub fn new_discrete<T: Into<ScalarValue>>(values: Vec<T>) -> Self {
        Self::new_enum(values)
    }
}
