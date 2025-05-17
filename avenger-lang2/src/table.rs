use std::hash::{Hash, Hasher};
use std::sync::Arc;
use arrow::array::{Array, ArrayData, ArrayRef, ArrowPrimitiveType, BinaryViewArray, GenericBinaryArray, GenericStringArray, NullArray, OffsetSizeTrait, PrimitiveArray, RecordBatch, StringViewArray};
use arrow::compute;
use arrow::datatypes::{Date32Type, Date64Type, Decimal128Type, Decimal256Type, Float16Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type, Time32MillisecondType, Time32SecondType, Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, ToByteSlice, UInt16Type, UInt32Type, UInt64Type, UInt8Type};
use arrow::util::pretty;
use arrow_schema::{DataType, Schema, SchemaRef, TimeUnit};
use datafusion::dataframe::DataFrame;
use crate::error::AvengerLangError;

#[derive(Debug, Clone)]
pub struct ArrowTable {
    pub schema: Arc<Schema>,
    pub batches: Vec<RecordBatch>,
}


impl ArrowTable {
    pub fn try_new(schema: SchemaRef, partitions: Vec<RecordBatch>) -> Result<Self, AvengerLangError> {
        // Make all columns nullable
        let schema_fields: Vec<_> = schema
            .fields
            .iter()
            .map(|f| f.as_ref().clone().with_nullable(true))
            .collect();
        let schema = Arc::new(Schema::new(schema_fields));
        if partitions.iter().all(|batch| {
            let batch_schema_fields: Vec<_> = batch
                .schema()
                .fields
                .iter()
                .map(|f| f.as_ref().clone().with_nullable(true))
                .collect();
            let batch_schema = Arc::new(Schema::new(batch_schema_fields));
            schema.fields.contains(&batch_schema.fields)
        }) {
            Ok(Self {
                schema,
                batches: partitions,
            })
        } else {
            Err(AvengerLangError::InternalError(
                "Mismatch between schema and batches".to_string(),
            ))
        }
    }

    pub async fn from_dataframe(df: DataFrame) -> Result<Self, AvengerLangError> {
        let schema = df.schema().clone();
        let partitions = df.collect().await?;
        Ok(Self::try_new(schema.inner().clone(), partitions)?)
    }

    pub fn show(&self) -> Result<(), AvengerLangError> {
        Ok(pretty::print_batches(&self.batches)?)
    }


    /// Get a column from the table as an Arrow array
    pub fn column(&self, name: &str) -> Result<ArrayRef, AvengerLangError> {
        let column = self.schema.index_of(name)?;
        let arrays = self.batches.iter().map(
            |batch| batch.column(column).as_ref()
        ).collect::<Vec<&dyn Array>>();
        Ok(compute::concat(arrays.as_slice())?)
    }

    /// Check if the table has a column
    pub fn has_column(&self, name: &str) -> bool {
        self.schema.index_of(name).is_ok()
    }

    pub fn num_rows(&self) -> usize {
        self.batches.iter().map(|batch| batch.num_rows()).sum()
    }
}


impl PartialEq for ArrowTable {
    fn eq(&self, other: &Self) -> bool {
        // Compare by computing and comparing hash values
        let mut self_hasher = std::collections::hash_map::DefaultHasher::new();
        let mut other_hasher = std::collections::hash_map::DefaultHasher::new();

        self.hash(&mut self_hasher);
        other.hash(&mut other_hasher);

        self_hasher.finish() == other_hasher.finish()
    }
}

impl Eq for ArrowTable {}

impl std::hash::Hash for ArrowTable {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash the schema
        self.schema.hash(state);

        // Hash each batch
        for batch in &self.batches {
            // Hash the number of rows in the batch
            batch.num_rows().hash(state);

            // Hash each column in the batch
            for column in batch.columns() {
                hash_array(column, state);
            }
        }
    }
}

fn hash_array<H: Hasher>(array: &ArrayRef, state: &mut H) {
    // Hash the array type
    let discriminant = std::mem::discriminant(array.data_type());
    discriminant.hash(state);

    // Hash the validity bitmap if present
    if let Some(nulls) = array.nulls() {
        let buffer = nulls.buffer();
        buffer.as_slice().hash(state);
    }

    match array.data_type() {
        DataType::Null => hash_null_array(array, state),
        DataType::Boolean => {
            let array = array.as_any().downcast_ref::<datafusion::arrow::array::BooleanArray>().unwrap();
            array.values().values().hash(state);
        },
        DataType::Int8 => hash_primitive_array::<Int8Type, H>(array, state),
        DataType::Int16 => hash_primitive_array::<Int16Type, H>(array, state),
        DataType::Int32 => hash_primitive_array::<Int32Type, H>(array, state),
        DataType::Int64 => hash_primitive_array::<Int64Type, H>(array, state),
        DataType::UInt8 => hash_primitive_array::<UInt8Type, H>(array, state),
        DataType::UInt16 => hash_primitive_array::<UInt16Type, H>(array, state),
        DataType::UInt32 => hash_primitive_array::<UInt32Type, H>(array, state),
        DataType::UInt64 => hash_primitive_array::<UInt64Type, H>(array, state),
        DataType::Float16 => hash_primitive_array::<Float16Type, H>(array, state),
        DataType::Float32 => hash_primitive_array::<Float32Type, H>(array, state),
        DataType::Float64 => hash_primitive_array::<Float64Type, H>(array, state),
        DataType::Date32 => hash_primitive_array::<Date32Type, H>(array, state),
        DataType::Date64 => hash_primitive_array::<Date64Type, H>(array, state),
        DataType::Time32(TimeUnit::Second) => {
            hash_primitive_array::<Time32SecondType, H>(array, state)
        }
        DataType::Time32(TimeUnit::Millisecond) => {
            hash_primitive_array::<Time32MillisecondType, H>(array, state)
        }
        DataType::Time64(TimeUnit::Microsecond) => {
            hash_primitive_array::<Time64MicrosecondType, H>(array, state)
        }
        DataType::Time64(TimeUnit::Nanosecond) => {
            hash_primitive_array::<Time64NanosecondType, H>(array, state)
        }
        DataType::Timestamp(time_unit, tz) => {
            match time_unit {
                TimeUnit::Second => hash_primitive_array::<TimestampSecondType, H>(array, state),
                TimeUnit::Millisecond => {
                    hash_primitive_array::<TimestampMillisecondType, H>(array, state)
                }
                TimeUnit::Microsecond => {
                    hash_primitive_array::<TimestampMicrosecondType, H>(array, state)
                }
                TimeUnit::Nanosecond => {
                    hash_primitive_array::<TimestampNanosecondType, H>(array, state)
                }
            }
            if let Some(tz_value) = tz {
                tz_value.hash(state);
            }
        }
        DataType::Utf8 => hash_string_array::<i32, H>(array, state),
        DataType::LargeUtf8 => hash_string_array::<i64, H>(array, state),
        DataType::Utf8View => hash_string_view_array::<H>(array, state),
        DataType::Binary => hash_binary_array::<i32, H>(array, state),
        DataType::LargeBinary => hash_binary_array::<i64, H>(array, state),
        DataType::BinaryView => hash_binary_view_array::<H>(array, state),
        DataType::Decimal128(a, b) => {
            a.hash(state);
            b.hash(state);
            hash_primitive_array::<Decimal128Type, H>(array, state);
        }
        DataType::Decimal256(a, b) => {
            a.hash(state);
            b.hash(state);
            hash_primitive_array::<Decimal256Type, H>(array, state);
        }
        _ => {
            // Fallback that requires cloning the array data
            let array_data = array.to_data();
            hash_array_data(&array_data, state);
        }
    }
}

fn hash_null_array<H: Hasher>(array: &ArrayRef, state: &mut H) {
    let array = array.as_any().downcast_ref::<NullArray>().unwrap();
    if let Some(nulls) = array.nulls() {
        nulls.buffer().as_slice().hash(state);
    }
}

fn hash_primitive_array<T: ArrowPrimitiveType, H: Hasher>(array: &ArrayRef, state: &mut H) {
    let array = array.as_any().downcast_ref::<PrimitiveArray<T>>().unwrap();
    array.values().to_byte_slice().hash(state);
}

fn hash_string_array<S: OffsetSizeTrait, H: Hasher>(array: &ArrayRef, state: &mut H) {
    let array = array
        .as_any()
        .downcast_ref::<GenericStringArray<S>>()
        .unwrap();
    array.value_offsets().to_byte_slice().hash(state);
    array.value_data().to_byte_slice().hash(state);
}

fn hash_string_view_array<H: Hasher>(array: &ArrayRef, state: &mut H) {
    let array = array.as_any().downcast_ref::<StringViewArray>().unwrap();

    // Hash view buffer - use as_slice() to convert to hashable slice
    array.views().to_byte_slice().hash(state);

    // Hash data buffers
    for buffer in array.data_buffers() {
        buffer.to_byte_slice().hash(state);
    }
}

fn hash_binary_array<S: OffsetSizeTrait, H: Hasher>(array: &ArrayRef, state: &mut H) {
    let array = array
        .as_any()
        .downcast_ref::<GenericBinaryArray<S>>()
        .unwrap();
    array.value_offsets().to_byte_slice().hash(state);
    array.value_data().to_byte_slice().hash(state);
}

fn hash_binary_view_array<H: Hasher>(array: &ArrayRef, state: &mut H) {
    let array = array.as_any().downcast_ref::<BinaryViewArray>().unwrap();

    // Hash view buffer
    array.views().to_byte_slice().hash(state);

    // Hash data buffers
    for buffer in array.data_buffers() {
        buffer.to_byte_slice().hash(state);
    }
}

fn hash_array_data<H: Hasher>(array_data: &ArrayData, state: &mut H) {
    for buffer in array_data.buffers() {
        buffer.to_byte_slice().hash(state);
    }

    // For nested types (list, struct), recursively hash child arrays
    let child_data = array_data.child_data();
    for child in child_data {
        hash_array_data(child, state);
    }
}
