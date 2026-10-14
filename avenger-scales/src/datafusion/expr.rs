use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, StructArray};
use arrow::datatypes::Field;
use datafusion::common::{exec_err, plan_err, Result, ScalarValue};
use datafusion::logical_expr::{lit, AggregateUDF, Expr, ScalarUDF};

use super::scale::ScaleUdf;
use super::summary::SummaryUdaf;
use crate::scalar::Scalar;
use crate::scales::domain_summary::encoding::one_row_list;
use crate::scales::ScaleImpl;

/// `values` as one list-valued literal, keeping their type: the form of a scale's domain and range.
pub fn list_literal(values: ArrayRef) -> Expr {
    lit(ScalarValue::List(Arc::new(one_row_list(values))))
}

/// Scale options as one struct-valued literal, with fields in name order.
pub fn options_literal(options: &HashMap<String, Scalar>) -> Result<Expr> {
    let mut entries: Vec<_> = options.iter().collect();
    entries.sort_by_key(|(name, _)| *name);
    let mut fields = Vec::new();
    let mut arrays = Vec::new();
    for (name, value) in entries {
        if value.0.len() != 1 {
            return plan_err!("scale option {name} must hold one value");
        }
        fields.push(Arc::new(Field::new(name, value.data_type().clone(), true)));
        arrays.push(value.to_array());
    }
    let options = if fields.is_empty() {
        StructArray::new_empty_fields(1, None)
    } else {
        StructArray::try_new(fields.into(), arrays, None)?
    };
    Ok(lit(ScalarValue::Struct(Arc::new(options))))
}

/// `scale` applied to `values`, with its `domain` and `range` as list-valued expressions and its
/// `options` as a struct-valued expression, which each have one value in a batch. The result has
/// the scale's [`output_type`](ScaleImpl::output_type).
pub fn scale_expr(
    scale: &dyn ScaleImpl,
    domain: Expr,
    range: Expr,
    options: Expr,
    values: Expr,
) -> Result<Expr> {
    let udf = ScalarUDF::new_from_impl(ScaleUdf::new(scale)?);
    Ok(udf.call(vec![domain, range, options, values]))
}

/// An aggregate that summarizes `values` for fitting the domain of `scale`, with `options` as a
/// struct-valued literal. `extents` are the lower and upper pixel extents of the marks drawn at the
/// values. Missing values are skipped with their extents, and a missing extent of a value is an
/// error. The result is the summary in its Arrow form, and no values give the summary of no data.
pub fn summary_expr(
    scale: &dyn ScaleImpl,
    options: Expr,
    values: Expr,
    extents: Option<(Expr, Expr)>,
) -> Result<Expr> {
    let udaf = AggregateUDF::new_from_impl(SummaryUdaf::new(scale)?);
    let mut args = vec![options, values];
    if let Some((lower, upper)) = extents {
        args.extend([lower, upper]);
    }
    Ok(udaf.call(args))
}

/// The options in a struct of one row.
pub(crate) fn options_from_struct(options: &StructArray) -> Result<HashMap<String, Scalar>> {
    if options.len() != 1 || options.is_null(0) {
        return exec_err!("scale options must be one non-null struct");
    }
    let mut names = HashSet::new();
    let mut map = HashMap::new();
    for (field, array) in options.fields().iter().zip(options.columns()) {
        if !names.insert(field.name()) {
            return exec_err!("duplicate scale option {}", field.name());
        }
        map.insert(field.name().clone(), Scalar(array.clone()));
    }
    Ok(map)
}
