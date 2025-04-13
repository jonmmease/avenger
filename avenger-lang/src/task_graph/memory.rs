use arrow_schema::{DataType, Field, FieldRef, Schema};
use datafusion::{arrow::array::{ArrayRef, ListArray, RecordBatch}, prelude::Expr};
use datafusion_common::ScalarValue;
use std::mem::{size_of, size_of_val};

use super::value::ArrowTable;

/// Get the size of a Field value, including any inner heap-allocated data
fn size_of_field(field: &FieldRef) -> usize {
    size_of::<Field>() + inner_size_of_dtype(field.data_type())
}

/// Get the size of inner heap-allocated data associated with a DataType value
fn inner_size_of_dtype(value: &DataType) -> usize {
    match value {
        DataType::Map(field, _) => size_of_field(field),
        DataType::Timestamp(_, Some(tz)) => size_of::<String>() + size_of_val(tz.as_bytes()),
        DataType::List(field) => size_of_field(field),
        DataType::LargeList(field) => size_of_field(field),
        DataType::FixedSizeList(field, _) => size_of_field(field),
        DataType::Struct(fields) => {
            size_of::<Vec<Field>>() + fields.iter().map(size_of_field).sum::<usize>()
        }
        DataType::Union(fields, _) => {
            size_of::<Vec<Field>>()
                + fields
                    .iter()
                    .map(|(_, field)| size_of_field(field))
                    .sum::<usize>()
        }
        DataType::Dictionary(key_dtype, value_dtype) => {
            2 * size_of::<DataType>()
                + inner_size_of_dtype(key_dtype)
                + inner_size_of_dtype(value_dtype)
        }
        _ => {
            // No inner heap-allocated data
            0
        }
    }
}

/// Get the size of inner heap-allocated data associated with a ScalarValue value
pub fn inner_size_of_scalar(value: &ScalarValue) -> usize {
    match value {
        ScalarValue::Utf8(Some(s)) => size_of_val(s.as_bytes()) + size_of::<String>(),
        ScalarValue::LargeUtf8(Some(s)) => size_of_val(s.as_bytes()) + size_of::<String>(),
        ScalarValue::Binary(Some(b)) => size_of_val(b.as_slice()) + size_of::<Vec<u8>>(),
        ScalarValue::LargeBinary(Some(b)) => size_of_val(b.as_slice()) + size_of::<Vec<u8>>(),
        ScalarValue::List(array) => size_of::<Vec<ScalarValue>>() + size_of_list_array(array),
        ScalarValue::Struct(sa) => {
            let fields = sa.fields();
            let fields_bytes: usize =
                size_of::<Vec<DataType>>() + fields.iter().map(size_of_field).sum::<usize>();
            let values_bytes: usize = sa
                .columns()
                .iter()
                .map(|col| col.get_array_memory_size())
                .sum();
            values_bytes + fields_bytes
        }
        _ => {
            // No inner heap-allocated data
            0
        }
    }
}

pub fn size_of_list_array(array: &ListArray) -> usize {
    array
        .iter()
        .map(|el| el.map(|el| size_of_array_ref(&el)).unwrap_or(0))
        .sum()
}

pub fn size_of_array_ref(array: &ArrayRef) -> usize {
    array.get_array_memory_size() + inner_size_of_dtype(array.data_type()) + size_of::<ArrayRef>()
}

pub fn size_of_schema(schema: &Schema) -> usize {
    size_of::<Schema>() + schema.fields().iter().map(size_of_field).sum::<usize>()
}

pub fn size_of_record_batch(batch: &RecordBatch) -> usize {
    let schema = batch.schema();
    let schema_size: usize = size_of_schema(schema.as_ref());
    let arrays_size: usize = batch.columns().iter().map(size_of_array_ref).sum();
    size_of::<RecordBatch>() + schema_size + arrays_size
}

pub fn inner_size_of_table(value: &ArrowTable) -> usize {
    let schema_size: usize = size_of_schema(&value.schema);
    let size_of_batches: usize = value.batches.iter().map(size_of_record_batch).sum();
    schema_size + size_of_batches
}
