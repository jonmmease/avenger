use super::{invalid, spec::*};
use crate::{Result, TableSnapshot};
use datafusion::{
    arrow::{
        array::new_empty_array,
        datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit},
        record_batch::{RecordBatch, RecordBatchOptions},
    },
    common::ScalarValue,
};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};
impl TypeSpec {
    pub fn arrow_type(&self) -> DataType {
        match self {
            Self::Null => DataType::Null,
            Self::Boolean => DataType::Boolean,
            Self::Int8 => DataType::Int8,
            Self::Int16 => DataType::Int16,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::Uint8 => DataType::UInt8,
            Self::Uint16 => DataType::UInt16,
            Self::Uint32 => DataType::UInt32,
            Self::Uint64 => DataType::UInt64,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::Utf8 => DataType::Utf8,
            Self::LargeUtf8 => DataType::LargeUtf8,
            Self::Date32 => DataType::Date32,
            Self::Date64 => DataType::Date64,
            Self::Timestamp(t) => DataType::Timestamp(
                match t.unit {
                    TimeUnitSpec::S => TimeUnit::Second,
                    TimeUnitSpec::Ms => TimeUnit::Millisecond,
                    TimeUnitSpec::Us => TimeUnit::Microsecond,
                    TimeUnitSpec::Ns => TimeUnit::Nanosecond,
                },
                t.timezone.as_deref().map(Into::into),
            ),
        }
    }
}
pub(super) fn schema(fields: &[FieldSpec]) -> Result<SchemaRef> {
    let mut names = HashSet::new();
    for field in fields {
        if field.name.is_empty() || !names.insert(&field.name) {
            return Err(invalid("schema fields must have unique nonempty names"));
        }
    }
    Ok(Arc::new(Schema::new(
        fields
            .iter()
            .map(|f| Field::new(&f.name, f.data_type.arrow_type(), f.nullable))
            .collect::<Vec<_>>(),
    )))
}
pub(super) fn scalar(value: &Value, data_type: &DataType) -> Result<ScalarValue> {
    if value.is_null() {
        return Ok(ScalarValue::try_from(data_type)?);
    }
    let bad = || invalid(format!("value {value} does not match {data_type}"));
    let signed = || {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(bad)
    };
    let unsigned = || {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(bad)
    };
    macro_rules! signed_value {
        ($variant:ident) => {
            ScalarValue::$variant(Some(signed()?.try_into().map_err(|_| bad())?))
        };
    }
    macro_rules! unsigned_value {
        ($variant:ident) => {
            ScalarValue::$variant(Some(unsigned()?.try_into().map_err(|_| bad())?))
        };
    }
    Ok(match data_type {
        DataType::Boolean => ScalarValue::Boolean(Some(value.as_bool().ok_or_else(bad)?)),
        DataType::Int8 => signed_value!(Int8),
        DataType::Int16 => signed_value!(Int16),
        DataType::Int32 => signed_value!(Int32),
        DataType::Int64 => ScalarValue::Int64(Some(signed()?)),
        DataType::UInt8 => unsigned_value!(UInt8),
        DataType::UInt16 => unsigned_value!(UInt16),
        DataType::UInt32 => unsigned_value!(UInt32),
        DataType::UInt64 => ScalarValue::UInt64(Some(unsigned()?)),
        DataType::Float64 => ScalarValue::Float64(Some(value.as_f64().ok_or_else(bad)?)),
        DataType::Float32 => {
            let f = value.as_f64().ok_or_else(bad)? as f32;
            if !f.is_finite() {
                return Err(bad());
            }
            ScalarValue::Float32(Some(f))
        }
        DataType::Utf8 => ScalarValue::Utf8(Some(value.as_str().ok_or_else(bad)?.into())),
        DataType::LargeUtf8 => ScalarValue::LargeUtf8(Some(value.as_str().ok_or_else(bad)?.into())),
        DataType::Date32 | DataType::Date64 => {
            let date =
                chrono::NaiveDate::parse_from_str(value.as_str().ok_or_else(bad)?, "%Y-%m-%d")
                    .map_err(|_| bad())?;
            let days = date
                .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
                .num_days();
            if matches!(data_type, DataType::Date32) {
                ScalarValue::Date32(Some(days.try_into().map_err(|_| bad())?))
            } else {
                ScalarValue::Date64(Some(days * 86_400_000))
            }
        }
        DataType::Timestamp(unit, zone) => {
            let timestamp = match signed() {
                Ok(v) => v,
                Err(_) => {
                    let dt = chrono::DateTime::parse_from_rfc3339(value.as_str().ok_or_else(bad)?)
                        .map_err(|_| bad())?;
                    match unit {
                        TimeUnit::Second => dt.timestamp(),
                        TimeUnit::Millisecond => dt.timestamp_millis(),
                        TimeUnit::Microsecond => dt.timestamp_micros(),
                        TimeUnit::Nanosecond => dt.timestamp_nanos_opt().ok_or_else(bad)?,
                    }
                }
            };
            match unit {
                TimeUnit::Second => ScalarValue::TimestampSecond(Some(timestamp), zone.clone()),
                TimeUnit::Millisecond => {
                    ScalarValue::TimestampMillisecond(Some(timestamp), zone.clone())
                }
                TimeUnit::Microsecond => {
                    ScalarValue::TimestampMicrosecond(Some(timestamp), zone.clone())
                }
                TimeUnit::Nanosecond => {
                    ScalarValue::TimestampNanosecond(Some(timestamp), zone.clone())
                }
            }
        }
        _ => return Err(bad()),
    })
}
pub(super) fn table(schema: SchemaRef, rows: &[Row]) -> Result<TableSnapshot> {
    let mut columns = vec![Vec::with_capacity(rows.len()); schema.fields().len()];
    for (row_index, row) in rows.iter().enumerate() {
        for name in row.0.keys() {
            if schema.index_of(name).is_err() {
                return Err(invalid(format!(
                    "values[{row_index}]: undeclared field {name}"
                )));
            }
        }
        for (index, field) in schema.fields().iter().enumerate() {
            let value = row.0.get(field.name()).unwrap_or(&Value::Null);
            if value.is_null() && !field.is_nullable() {
                return Err(invalid(format!(
                    "values[{row_index}].{} cannot be null or missing",
                    field.name()
                )));
            }
            columns[index].push(
                scalar(value, field.data_type())
                    .map_err(|e| invalid(format!("values[{row_index}].{}: {e}", field.name())))?,
            );
        }
    }
    let arrays = columns
        .into_iter()
        .zip(schema.fields())
        .map(|(values, field)| {
            if values.is_empty() {
                Ok(new_empty_array(field.data_type()))
            } else {
                ScalarValue::iter_to_array(values)
            }
        })
        .collect::<datafusion::common::Result<Vec<_>>>()?;
    let batch = RecordBatch::try_new_with_options(
        schema.clone(),
        arrays,
        &RecordBatchOptions::new().with_row_count(Some(rows.len())),
    )?;
    TableSnapshot::from_batches(schema, vec![batch])
}
