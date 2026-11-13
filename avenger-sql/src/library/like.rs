//! `value [NOT] LIKE {ANY | ALL} UNNEST(patterns)`: whether a string matches any or all of the
//! patterns of an array, as GoogleSQL defines it. A `NULL` array has no patterns, so `ANY` is
//! `FALSE` and `ALL` is `TRUE` for it, and a `NULL` value or pattern gives `NULL` unless another
//! pattern decides the result.
//!
//! An internal function matches each row's value against the patterns of its array, through
//! Arrow's `like` kernel, so the form needs no correlated subquery.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, BooleanArray, Scalar, StringArray},
        compute::{cast, kernels::comparison},
        datatypes::DataType,
    },
    common::{Result, ScalarValue},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

/// The match of `value` against the patterns of the array `patterns`: of any of them, or of all
/// when `all`, and of `NOT LIKE` when `not`.
pub(crate) fn like_unnest(value: Expr, patterns: Expr, all: bool, not: bool) -> Expr {
    let flag = |b: bool| Expr::Literal(ScalarValue::Boolean(Some(b)), None);
    LIKE_UNNEST.call(vec![value, patterns, flag(all), flag(not)])
}

static LIKE_UNNEST: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| {
    Arc::new(ScalarUDF::new_from_impl(LikeUnnest {
        signature: Signature::any(4, Volatility::Immutable),
    }))
});

#[derive(Debug, PartialEq, Eq, Hash)]
struct LikeUnnest {
    signature: Signature,
}

/// The flag at `index` of `args`, a literal.
fn flag(args: &ScalarFunctionArgs, index: usize) -> bool {
    matches!(
        &args.args[index],
        ColumnarValue::Scalar(ScalarValue::Boolean(Some(true)))
    )
}

impl ScalarUDFImpl for LikeUnnest {
    fn name(&self) -> &str {
        "$like_unnest"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let (all, not) = (flag(&args, 2), flag(&args, 3));
        let scalar = matches!(
            (&args.args[0], &args.args[1]),
            (ColumnarValue::Scalar(_), ColumnarValue::Scalar(_))
        );
        let values = cast(&args.args[0].clone().into_array(rows)?, &DataType::Utf8)?;
        let values = values.as_string::<i32>();
        let lists = args.args[1].clone().into_array(rows)?;
        let lists = cast(
            &lists,
            &DataType::List(Arc::new(datafusion::arrow::datatypes::Field::new(
                "item",
                DataType::Utf8,
                true,
            ))),
        )?;
        let lists = lists.as_list::<i32>();
        let mut out = Vec::with_capacity(values.len());
        for row in 0..values.len() {
            let patterns: ArrayRef = match lists.is_null(row) {
                true => Arc::new(StringArray::from(Vec::<Option<&str>>::new())),
                false => lists.value(row),
            };
            let value = match values.is_null(row) {
                true => Scalar::new(StringArray::from(vec![None::<&str>])),
                false => Scalar::new(StringArray::from(vec![values.value(row)])),
            };
            let matched: BooleanArray = match not {
                true => comparison::nlike(&value, &patterns)?,
                false => comparison::like(&value, &patterns)?,
            };
            let known: Vec<bool> = matched.iter().flatten().collect();
            let unknown = matched.null_count() > 0;
            // Three-valued logic: a deciding match wins over an unknown one.
            out.push(match all {
                false if known.contains(&true) => Some(true),
                true if known.contains(&false) => Some(false),
                _ if unknown => None,
                false => Some(false),
                true => Some(true),
            });
        }
        let out: ArrayRef = Arc::new(BooleanArray::from(out));
        match scalar {
            true => Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(&out, 0)?)),
            false => Ok(ColumnarValue::Array(out)),
        }
    }
}
