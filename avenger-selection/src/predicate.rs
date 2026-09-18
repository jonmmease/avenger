use crate::{
    EmptySelection, Resolution, ResolvedContribution, ResolvedFilter, SelectionStatus,
    SelectionValue, ValueTest,
};
use datafusion::{
    arrow::{
        array::{Array, BooleanArray, Float16Array, Float32Array, Float64Array},
        datatypes::DataType,
    },
    common::{Result as DFResult, ScalarValue},
    logical_expr::{
        expr::BinaryExpr, lit, ColumnarValue, Expr, Operator, ScalarFunctionArgs, ScalarUDF,
        ScalarUDFImpl, Signature, Volatility,
    },
};
use std::{
    ops::{Bound, Not},
    sync::{Arc, LazyLock},
};

pub(crate) fn resolved(filter: &ResolvedFilter) -> Expr {
    match filter {
        ResolvedFilter::All(filters) => combine(filters.iter().map(resolved), true),
        ResolvedFilter::Any(filters) => combine(filters.iter().map(resolved), false),
        ResolvedFilter::Not(filter) => resolved(filter).not(),
        ResolvedFilter::Selection(selection) => match selection.status() {
            SelectionStatus::Inactive => lit(selection.usage().empty == EmptySelection::MatchAll),
            SelectionStatus::AllExcluded => lit(true),
            SelectionStatus::Active => combine(
                selection.contributions().iter().map(contribution),
                selection.definition().resolution() == Resolution::Intersect,
            ),
        },
    }
}
pub(crate) fn combine(exprs: impl IntoIterator<Item = Expr>, and: bool) -> Expr {
    exprs
        .into_iter()
        .reduce(|a, b| if and { a.and(b) } else { a.or(b) })
        .unwrap_or_else(|| lit(and))
}
pub(crate) fn contribution(c: &ResolvedContribution) -> Expr {
    match c.contribution().effective_value() {
        SelectionValue::Tuples(tuples) => tuples_predicate(
            tuples,
            &c.projections()
                .iter()
                .map(|p| p.expr().clone())
                .collect::<Vec<_>>(),
        ),
        SelectionValue::RowIds(ids) => combine(
            ids.values().iter().map(|value| {
                equality(
                    c.identity_expr().expect("resolved row lineage").clone(),
                    value,
                )
            }),
            false,
        ),
    }
}

// Both native row predicates and predicates over retained interaction keys use
// these comparisons. This keeps tuple correlation and pixel bounds identical.
pub(crate) fn tuples_predicate(tuples: &[crate::SelectionTuple], projections: &[Expr]) -> Expr {
    combine(
        tuples.iter().map(|tuple| {
            combine(
                tuple
                    .terms
                    .iter()
                    .zip(projections)
                    .map(|(term, expr)| comparison(expr.clone(), &term.test)),
                true,
            )
        }),
        false,
    )
}
fn equality(expr: Expr, value: &ScalarValue) -> Expr {
    let neg_zero = match value {
        ScalarValue::Float16(Some(v)) if v.is_nan() => return classify(expr, NumberClass::Nan),
        ScalarValue::Float32(Some(v)) if v.is_nan() => return classify(expr, NumberClass::Nan),
        ScalarValue::Float64(Some(v)) if v.is_nan() => return classify(expr, NumberClass::Nan),
        ScalarValue::Float16(Some(v)) if *v == half::f16::ZERO => {
            Some(ScalarValue::Float16(Some(half::f16::NEG_ZERO)))
        }
        ScalarValue::Float32(Some(v)) if *v == 0.0 => Some(ScalarValue::Float32(Some(-0.0))),
        ScalarValue::Float64(Some(v)) if *v == 0.0 => Some(ScalarValue::Float64(Some(-0.0))),
        _ => None,
    };
    let equal = |v| {
        Expr::BinaryExpr(BinaryExpr::new(
            Box::new(expr.clone()),
            Operator::IsNotDistinctFrom,
            Box::new(lit(v)),
        ))
    };
    let result = equal(value.clone());
    if let Some(zero) = neg_zero {
        result.or(equal(zero))
    } else {
        result
    }
}
fn comparison(expr: Expr, test: &ValueTest) -> Expr {
    match test {
        ValueTest::Equal(value) => equality(expr, value),
        ValueTest::OneOf(values) => {
            combine(values.iter().map(|v| equality(expr.clone(), v)), false)
        }
        ValueTest::Range { lower, upper } => {
            let mut result = classify(expr.clone(), NumberClass::Finite);
            match lower {
                Bound::Included(v) => result = result.and(expr.clone().gt_eq(lit(v.clone()))),
                Bound::Excluded(v) => result = result.and(expr.clone().gt(lit(v.clone()))),
                Bound::Unbounded => (),
            }
            match upper {
                Bound::Included(v) => result = result.and(expr.lt_eq(lit(v.clone()))),
                Bound::Excluded(v) => result = result.and(expr.lt(lit(v.clone()))),
                Bound::Unbounded => (),
            }
            result.is_true()
        }
    }
}

// The input schema is known only at the usage site. This immutable kernel
// preserves the original numeric/temporal type while checking unbounded ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum NumberClass {
    Finite,
    Nan,
}
fn classify(expr: Expr, class: NumberClass) -> Expr {
    ScalarUDF::from(class).call(vec![expr]).is_true()
}
impl ScalarUDFImpl for NumberClass {
    fn name(&self) -> &str {
        match self {
            Self::Finite => "avenger_selection_finite",
            Self::Nan => "avenger_selection_nan",
        }
    }
    fn signature(&self) -> &Signature {
        static SIGNATURE: LazyLock<Signature> =
            LazyLock::new(|| Signature::any(1, Volatility::Immutable));
        &SIGNATURE
    }
    fn return_type(&self, args: &[DataType]) -> DFResult<DataType> {
        crate::values::validate_type(&args[0])
            .map_err(|e| datafusion::common::DataFusionError::Plan(e.to_string()))?;
        if *self == Self::Nan && !args[0].is_numeric() && args[0] != DataType::Null {
            return datafusion::common::plan_err!(
                "NaN membership requires a numeric projection, received {}",
                args[0]
            );
        }
        Ok(DataType::Boolean)
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        let scalar = matches!(&args.args[0], ColumnarValue::Scalar(_));
        let array = args.args[0]
            .clone()
            .into_array(if scalar { 1 } else { args.number_rows })?;
        let result = match array.data_type() {
            DataType::Float16 => {
                let a = array.as_any().downcast_ref::<Float16Array>().unwrap();
                BooleanArray::from_iter(a.iter().map(|v| {
                    Some(v.is_some_and(|v| {
                        if *self == Self::Finite {
                            v.is_finite()
                        } else {
                            v.is_nan()
                        }
                    }))
                }))
            }
            DataType::Float32 => {
                let a = array.as_any().downcast_ref::<Float32Array>().unwrap();
                BooleanArray::from_iter(a.iter().map(|v| {
                    Some(v.is_some_and(|v| {
                        if *self == Self::Finite {
                            v.is_finite()
                        } else {
                            v.is_nan()
                        }
                    }))
                }))
            }
            DataType::Float64 => {
                let a = array.as_any().downcast_ref::<Float64Array>().unwrap();
                BooleanArray::from_iter(a.iter().map(|v| {
                    Some(v.is_some_and(|v| {
                        if *self == Self::Finite {
                            v.is_finite()
                        } else {
                            v.is_nan()
                        }
                    }))
                }))
            }
            _ => BooleanArray::from_iter(
                (0..array.len()).map(|i| Some(*self == Self::Finite && array.is_valid(i))),
            ),
        };
        if scalar {
            Ok(ColumnarValue::Scalar(ScalarValue::Boolean(Some(
                result.value(0),
            ))))
        } else {
            Ok(ColumnarValue::Array(Arc::new(result)))
        }
    }
}
