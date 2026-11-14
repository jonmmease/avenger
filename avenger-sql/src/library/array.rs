//! The array functions, and the subscripts of arrays.

use datafusion::{
    arrow::datatypes::DataType, functions_nested::expr_fn as nested, logical_expr::Expr,
};

use super::{cast, lit_i64, Function, Param};

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar("ARRAY_LENGTH", vec![Param::required("value")], |call| {
            Ok(cast(nested::cardinality(call.arg(0)), DataType::Int64))
        })
        .safe(),
        Function::scalar("ARRAY_CONCAT", vec![Param::repeated("arrays")], |call| {
            Ok(nested::array_concat(call.rest()))
        })
        .safe(),
        Function::scalar("ARRAY_REVERSE", vec![Param::required("value")], |call| {
            Ok(nested::array_reverse(call.arg(0)))
        })
        .safe(),
        Function::scalar(
            "GENERATE_ARRAY",
            vec![
                Param::required("start_value"),
                Param::required("end_value"),
                Param::optional("step"),
            ],
            |call| {
                let step = call.value(2).cloned().unwrap_or_else(|| lit_i64(1));
                Ok(nested::gen_series(call.arg(0), call.arg(1), step))
            },
        )
        .safe(),
    ]
}

/// How a subscript counts the elements of an array: `array[OFFSET(i)]` from 0, and
/// `array[ORDINAL(i)]` from 1. An index out of the array is `NULL` for the `SAFE_` forms, and
/// for the others too, where GoogleSQL fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subscript {
    Offset,
    Ordinal,
    SafeOffset,
    SafeOrdinal,
}

impl Subscript {
    /// The subscript of the function named `name` in `array[name(i)]`, without regard to case.
    pub fn parse(name: &str) -> Option<Self> {
        let subscript = match name.to_ascii_uppercase().as_str() {
            "OFFSET" => Self::Offset,
            "ORDINAL" => Self::Ordinal,
            "SAFE_OFFSET" => Self::SafeOffset,
            "SAFE_ORDINAL" => Self::SafeOrdinal,
            _ => return None,
        };
        Some(subscript)
    }

    /// The element of `array` at `index`. `array[i]` is `array[OFFSET(i)]`.
    pub fn element(self, array: Expr, index: Expr) -> Expr {
        let ordinal = match self {
            Self::Offset | Self::SafeOffset => cast(index, DataType::Int64) + lit_i64(1),
            Self::Ordinal | Self::SafeOrdinal => cast(index, DataType::Int64),
        };
        nested::array_element(array, ordinal)
    }
}
