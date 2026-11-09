//! Lumps: a column of the structs that the `lump` transform writes, of a kept category or of the
//! Other group that holds the rest, which a categorical channel draws with the Other group last.
//!
//! A lump is a struct of exactly a `value`, an `other` of Utf8, a `weight` of Float64, and a
//! `category_count` of Int64, in that order. A kept category has its category in `value`, a NULL
//! `other`, its weight, and a count of 1. The Other group has a NULL `value`, its label in
//! `other`, the weight of all of its categories, and their number. A kept NULL category has both
//! `value` and `other` NULL, so `other` alone tells the Other group apart.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, BooleanArray, StructArray},
    datatypes::{DataType, Field, Fields},
};

/// The names of a lump's fields, in their order.
const FIELDS: [&str; 4] = ["value", "other", "weight", "category_count"];

/// Whether `data_type` is the type of a lump.
pub fn is_lump(data_type: &DataType) -> bool {
    let DataType::Struct(fields) = data_type else {
        return false;
    };
    let types = [
        None,
        Some(DataType::Utf8),
        Some(DataType::Float64),
        Some(DataType::Int64),
    ];
    fields.len() == FIELDS.len()
        && (fields.iter().zip(FIELDS).zip(&types)).all(|((field, name), data_type)| {
            field.name() == name
                && data_type
                    .as_ref()
                    .is_none_or(|data_type| field.data_type() == data_type)
        })
}

/// The type of the categories of a column of `data_type`: the type of a lump's `value`, or
/// `data_type` itself. Explicit domains, preferred categories, and the values of ticks and legend
/// entries of a lump's scale have this type.
pub fn values_type(data_type: &DataType) -> DataType {
    match data_type {
        DataType::Struct(fields) if is_lump(data_type) => fields[0].data_type().clone(),
        _ => data_type.clone(),
    }
}

/// The type of the keys of a lump of `data_type`: a struct of its `value` and its `other`.
pub fn key_type(data_type: &DataType) -> Option<DataType> {
    let DataType::Struct(fields) = data_type else {
        return None;
    };
    is_lump(data_type).then(|| DataType::Struct(Fields::from(fields[..2].to_vec())))
}

/// The keys of the lumps of `column`: a struct of each lump's `value` and `other`, which a
/// categorical scale takes as its categories, so the Other group is one category whatever the type
/// of the kept ones. `None` for a column that is not a lump.
pub fn keys(column: &ArrayRef) -> Option<ArrayRef> {
    if !is_lump(column.data_type()) {
        return None;
    }
    let lumps = column.as_struct();
    let fields: Vec<Arc<Field>> = lumps.fields()[..2].to_vec();
    let keys = StructArray::new(
        Fields::from(fields),
        lumps.columns()[..2].to_vec(),
        lumps.nulls().cloned(),
    );
    Some(Arc::new(keys))
}

/// Whether each row of a lump column is in the Other group: its `other` is not NULL. `None` for a
/// column that is not a lump.
pub fn others(column: &ArrayRef) -> Option<BooleanArray> {
    if !is_lump(column.data_type()) {
        return None;
    }
    let other = column.as_struct().column(1);
    Some(
        (0..other.len())
            .map(|row| Some(other.is_valid(row)))
            .collect(),
    )
}

/// The key of the Other group with the label `label`, for a lump of `data_type`: a NULL `value`
/// and the label.
pub fn other_key(data_type: &DataType, label: &str) -> Option<ArrayRef> {
    let DataType::Struct(fields) = key_type(data_type)? else {
        return None;
    };
    let value = arrow::array::new_null_array(fields[0].data_type(), 1);
    let other: ArrayRef = Arc::new(arrow::array::StringArray::from(vec![label]));
    Some(Arc::new(StructArray::new(fields, vec![value, other], None)))
}
