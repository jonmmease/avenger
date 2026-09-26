use std::{collections::HashMap, sync::Arc};

use avenger_scales::scalar::Scalar;
use datafusion::{
    arrow::{
        array::{ArrayRef, ListArray, StructArray},
        buffer::OffsetBuffer,
        datatypes::Field,
    },
    common::{Result, ScalarValue},
    logical_expr::{lit, Expr},
};

use crate::{create_scale_udf, ScaleSpec};

/// Wrap an Arrow array as one list-valued expression, preserving its type.
pub fn list_literal(values: ArrayRef) -> Result<Expr> {
    let list = ListArray::try_new(
        Arc::new(Field::new_list_field(values.data_type().clone(), true)),
        OffsetBuffer::from_lengths([values.len()]),
        values,
        None,
    )?;
    Ok(lit(ScalarValue::List(Arc::new(list))))
}

/// Build a scalar options struct in deterministic field order.
pub fn options_literal(options: &HashMap<String, Scalar>) -> Result<Expr> {
    let mut entries: Vec<_> = options.iter().collect();
    entries.sort_by_key(|(name, _)| *name);
    let mut fields = Vec::new();
    let mut arrays = Vec::new();
    for (name, value) in entries {
        if value.0.len() != 1 {
            return datafusion::common::plan_err!(
                "Scale option {name} must contain exactly one value"
            );
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

/// Apply a scale with expression-valued domain, range, options, and values.
pub fn scale_expr(
    spec: impl ScaleSpec + 'static,
    domain: Expr,
    range: Expr,
    options: Expr,
    values: Expr,
) -> Result<Expr> {
    Ok(create_scale_udf(spec)?.call(vec![domain, range, options, values]))
}
