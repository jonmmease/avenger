use datafusion::logical_expr::Expr;
use datafusion_common::ScalarValue;
use palette::Srgba;

#[derive(Debug, Clone)]
pub enum ScaleRange {
    Numeric(Expr, Box<Expr>),
    Discrete(Vec<ScalarValue>),
    Color(Vec<Srgba>),
}

impl ScaleRange {
    pub fn new_interval<E: Into<Expr>, F: Into<Expr>>(start: E, end: F) -> Self {
        Self::Numeric(start.into(), Box::new(end.into()))
    }

    pub fn new_color(colors: Vec<Srgba>) -> Self {
        Self::Color(colors)
    }

    pub fn new_discrete<T: Into<ScalarValue>>(values: Vec<T>) -> Self {
        Self::Discrete(values.into_iter().map(|v| v.into()).collect())
    }

    /// Create a discrete range with `num` values linearly spaced between `start` and `end`
    pub fn new_linspace_discrete(start: f32, end: f32, num: usize) -> Self {
        let step = if num > 1 {
            (end - start) / (num - 1) as f32
        } else {
            0.0
        };
        let values: Vec<ScalarValue> = (0..num)
            .map(|i| ScalarValue::Float32(Some(start + i as f32 * step)))
            .collect();
        Self::Discrete(values)
    }
}
