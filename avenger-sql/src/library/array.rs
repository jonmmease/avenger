//! The array functions, and the subscripts of arrays.

use datafusion::{
    arrow::datatypes::DataType,
    functions_nested::expr_fn as nested,
    logical_expr::{expr::ScalarFunction, Expr},
};

use super::{cast, lit_i64, Function};

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::scalar("ARRAY_LENGTH", "Returns the number of elements of an array.")
            .description("Returns `NULL` for a `NULL` array.")
            .param("value", "The array.")
            .form("(value: ARRAY<T>) -> INT64 where T: ANY", |call| {
                Ok(cast(nested::cardinality(call.arg(0)), DataType::Int64))
            })
            .safe()
            .example("ARRAY_LENGTH([1, 2, 3])", "3"),
        Function::scalar("ARRAY_CONCAT", "Joins arrays into one.")
            .param("arrays", "The arrays, of one element type, in order.")
            .form("(arrays: ARRAY<T>...) -> ARRAY<T> where T: ANY", |call| {
                Ok(nested::array_concat(call.rest()))
            })
            .safe()
            .example("ARRAY_CONCAT([1, 2], [3])", "[1, 2, 3]"),
        Function::scalar("ARRAY_REVERSE", "Returns the elements of an array in reverse order.")
            .param("value", "The array.")
            .form("(value: ARRAY<T>) -> ARRAY<T> where T: ANY", |call| {
                Ok(nested::array_reverse(call.arg(0)))
            })
            .safe()
            .example("ARRAY_REVERSE([1, 2, 3])", "[3, 2, 1]"),
        Function::scalar("ARRAY_TO_STRING", "Joins the strings of an array into one, with a delimiter between them.")
            .param("array_expression", "The array of strings.")
            .param("delimiter", "The string between two elements.")
            .param("null_text", "The string in the place of a `NULL` element. When omitted, a `NULL` element is left out.")
            .form("(array_expression: ARRAY<STRING>, delimiter: STRING[, null_text: STRING]) -> STRING", |call| {
                let mut args = vec![call.arg(0), call.arg(1)];
                args.extend(call.value(2).cloned());
                Ok(Expr::ScalarFunction(ScalarFunction::new_udf(
                    datafusion::functions_nested::string::array_to_string_udf(),
                    args,
                )))
            })
            .safe()
            .example("ARRAY_TO_STRING(['a', NULL, 'c'], ',')", "'a,c'")
            .example("ARRAY_TO_STRING(['a', NULL, 'c'], ',', 'N')", "'a,N,c'"),
        Function::scalar("GENERATE_ARRAY", "Returns an array of the numbers from a start to an end in steps.")
            .description("The array holds the end when a whole number of steps reaches it, and is empty when the steps lead away from the end.")
            .param("start_value", "The first number.")
            .param("end_value", "The last number that the array can hold.")
            .param("step", "The difference between two numbers, which is not 0. When omitted, 1.")
            .form("(start_value: INT64, end_value: INT64[, step: INT64]) -> ARRAY<INT64>", |call| {
                let step = call.value(2).cloned().unwrap_or_else(|| lit_i64(1));
                Ok(nested::gen_series(call.arg(0), call.arg(1), step))
            })
            .safe()
            .example("GENERATE_ARRAY(1, 5)", "[1, 2, 3, 4, 5]")
            .example("GENERATE_ARRAY(1, 10, 3)", "[1, 4, 7, 10]")
            .example("GENERATE_ARRAY(5, 1)", "[]"),
    ]
}

/// How a subscript counts the elements of an array: `array[OFFSET(i)]` from 0, and
/// `array[ORDINAL(i)]` from 1. An index out of the array is `NULL` for the `SAFE_` forms, and an
/// error for the others, as in GoogleSQL.
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
            Self::Offset | Self::SafeOffset => cast(index.clone(), DataType::Int64) + lit_i64(1),
            Self::Ordinal | Self::SafeOrdinal => cast(index.clone(), DataType::Int64),
        };
        let element = nested::array_element(array.clone(), ordinal.clone());
        if matches!(self, Self::SafeOffset | Self::SafeOrdinal) {
            return element;
        }
        let length = cast(nested::cardinality(array), DataType::Int64);
        let outside = ordinal.clone().lt(lit_i64(1)).or(ordinal.gt(length));
        super::error::error_when(
            outside,
            element,
            "Array index {} is out of bounds",
            vec![index],
        )
    }
}
