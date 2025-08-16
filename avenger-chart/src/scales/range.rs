use crate::error::AvengerChartError;
use datafusion::arrow::datatypes::DataType;
use datafusion::functions_array::expr_fn::make_array;
use datafusion::logical_expr::{Expr, lit};
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

    pub fn data_type(&self) -> Result<DataType, AvengerChartError> {
        match self {
            ScaleRange::Numeric(_, _) => Ok(DataType::Float32),
            ScaleRange::Enum(vals) => {
                vals.first()
                    .map(|v| v.data_type().clone())
                    .ok_or(AvengerChartError::InternalError(
                        "Enum range may not be empty".to_string(),
                    ))
            }
            ScaleRange::Color(_) => Ok(DataType::new_list(DataType::Float32, true)),
        }
    }

    /// Compile range to an expression that evaluates to a list
    pub fn compile(&self) -> Result<Expr, AvengerChartError> {
        match self {
            ScaleRange::Numeric(start, end) => {
                Ok(make_array(vec![start.clone(), end.as_ref().clone()]))
            }
            ScaleRange::Enum(values) => {
                let exprs = values.iter().map(|v| lit(v.clone())).collect::<Vec<_>>();
                Ok(make_array(exprs))
            }
            ScaleRange::Color(colors) => {
                // Convert colors to RGBA array expressions
                let color_exprs = colors
                    .iter()
                    .map(|c| {
                        // For now, create a simple array instead of struct
                        make_array(vec![lit(c.red), lit(c.green), lit(c.blue), lit(c.alpha)])
                    })
                    .collect::<Vec<_>>();
                Ok(make_array(color_exprs))
            }
        }
    }
}