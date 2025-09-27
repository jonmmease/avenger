use crate::serialization::{SerializableExpr, SerializableScalar};
use datafusion::logical_expr::Expr;
use datafusion_common::ScalarValue;
use palette::Srgba;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScaleRange {
    Numeric(SerializableExpr, Box<SerializableExpr>),
    Discrete(Vec<SerializableScalar>),
    Color(Vec<[f32; 4]>), // Store as RGBA arrays for serialization
}

impl ScaleRange {
    pub fn new_interval<E: Into<Expr>, F: Into<Expr>>(start: E, end: F) -> Self {
        let start_ser =
            SerializableExpr::from_expr(start.into()).expect("Failed to serialize start expr");
        let end_ser =
            SerializableExpr::from_expr(end.into()).expect("Failed to serialize end expr");
        Self::Numeric(start_ser, Box::new(end_ser))
    }

    pub fn new_color(colors: Vec<Srgba>) -> Self {
        Self::Color(
            colors
                .into_iter()
                .map(|c| [c.red, c.green, c.blue, c.alpha])
                .collect(),
        )
    }

    pub fn new_discrete<T: Into<ScalarValue>>(values: Vec<T>) -> Self {
        Self::Discrete(
            values
                .into_iter()
                .map(|v| {
                    SerializableScalar::new(v.into())
                })
                .collect(),
        )
    }

    /// Create a discrete range with `num` values linearly spaced between `start` and `end`
    pub fn new_linspace_discrete(start: f32, end: f32, num: usize) -> Self {
        let step = if num > 1 {
            (end - start) / (num - 1) as f32
        } else {
            0.0
        };
        let values: Vec<SerializableScalar> = (0..num)
            .map(|i| {
                SerializableScalar::new(ScalarValue::Float32(Some(start + i as f32 * step)))
            })
            .collect();
        Self::Discrete(values)
    }
}
