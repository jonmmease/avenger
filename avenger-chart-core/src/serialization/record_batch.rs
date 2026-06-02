//! Serializable wrapper for Arrow record batches.

use std::io::Cursor;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion::arrow::{
    ipc::{reader::StreamReader, writer::StreamWriter},
    record_batch::RecordBatch,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serializable wrapper for a single Arrow `RecordBatch`.
#[derive(Debug, Clone, PartialEq)]
pub struct SerializableRecordBatch(pub RecordBatch);

impl SerializableRecordBatch {
    pub fn new(batch: RecordBatch) -> Self {
        Self(batch)
    }

    pub fn into_inner(self) -> RecordBatch {
        self.0
    }
}

impl From<RecordBatch> for SerializableRecordBatch {
    fn from(batch: RecordBatch) -> Self {
        Self::new(batch)
    }
}

impl From<SerializableRecordBatch> for RecordBatch {
    fn from(wrapper: SerializableRecordBatch) -> Self {
        wrapper.into_inner()
    }
}

impl Serialize for SerializableRecordBatch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut buf = Vec::new();
        let mut writer = StreamWriter::try_new(&mut buf, &self.0.schema())
            .map_err(|err| serde::ser::Error::custom(format!("serialize RecordBatch: {err}")))?;
        writer
            .write(&self.0)
            .map_err(|err| serde::ser::Error::custom(format!("serialize RecordBatch: {err}")))?;
        writer
            .finish()
            .map_err(|err| serde::ser::Error::custom(format!("serialize RecordBatch: {err}")))?;
        drop(writer);

        if serializer.is_human_readable() {
            BASE64.encode(&buf).serialize(serializer)
        } else {
            buf.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for SerializableRecordBatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let buf = if deserializer.is_human_readable() {
            let base64_str = String::deserialize(deserializer)?;
            BASE64
                .decode(base64_str)
                .map_err(|err| serde::de::Error::custom(format!("decode base64: {err}")))?
        } else {
            Vec::<u8>::deserialize(deserializer)?
        };

        let cursor = Cursor::new(buf);
        let mut reader = StreamReader::try_new(cursor, None)
            .map_err(|err| serde::de::Error::custom(format!("read RecordBatch: {err}")))?;
        let batch = reader
            .next()
            .transpose()
            .map_err(|err| serde::de::Error::custom(format!("read RecordBatch: {err}")))?
            .ok_or_else(|| serde::de::Error::custom("RecordBatch stream contained no batches"))?;
        Ok(Self(batch))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::{
        array::{Int32Array, StringArray},
        datatypes::{DataType, Field, Schema},
    };

    use super::*;

    #[test]
    fn record_batch_roundtrips_through_json() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("a"), None])),
            ],
        )
        .unwrap();
        let json = serde_json::to_string(&SerializableRecordBatch::from(batch.clone())).unwrap();
        let restored: SerializableRecordBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.0.schema(), batch.schema());
        assert_eq!(restored.0.num_rows(), batch.num_rows());
    }
}
