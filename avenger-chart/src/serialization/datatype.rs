//! Serializable wrapper for DataType

use datafusion::{arrow::datatypes::DataType, common::ScalarValue};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::scalar::SerializableScalar;

/// Wrapper for DataType that implements Serialize/Deserialize
/// using a null ScalarValue of the appropriate type
#[derive(Debug, Clone, PartialEq)]
pub struct SerializableDataType(pub DataType);

impl SerializableDataType {
    /// Create from a DataType
    pub fn new(data_type: DataType) -> Self {
        Self(data_type)
    }

    /// Get the inner DataType
    pub fn into_inner(self) -> DataType {
        self.0
    }

    /// Get a reference to the inner DataType
    pub fn as_data_type(&self) -> &DataType {
        &self.0
    }
}

impl From<DataType> for SerializableDataType {
    fn from(data_type: DataType) -> Self {
        Self::new(data_type)
    }
}

impl From<SerializableDataType> for DataType {
    fn from(serializable: SerializableDataType) -> Self {
        serializable.into_inner()
    }
}

impl Serialize for SerializableDataType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Create a null ScalarValue of this type
        let null_scalar = ScalarValue::try_from(&self.0).map_err(|e| {
            serde::ser::Error::custom(format!("Failed to create null scalar: {}", e))
        })?;

        // Use SerializableScalar to serialize it
        SerializableScalar(null_scalar).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SerializableDataType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize as SerializableScalar
        let scalar = SerializableScalar::deserialize(deserializer)?;

        // Extract the DataType
        let data_type = scalar.0.data_type();
        Ok(Self(data_type))
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::datatypes::{Field, Fields};

    use super::*;

    #[test]
    fn test_serializable_datatype_simple() {
        let dt = DataType::Int32;
        let serializable = SerializableDataType::from(dt.clone());

        let json = serde_json::to_string(&serializable).unwrap();
        let deserialized: SerializableDataType = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.0, dt);
    }

    #[test]
    fn test_serializable_datatype_list() {
        let dt = DataType::new_list(DataType::Float64, true);
        let serializable = SerializableDataType::from(dt.clone());

        let json = serde_json::to_string(&serializable).unwrap();
        let deserialized: SerializableDataType = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.0, dt);
    }

    #[test]
    fn test_serializable_datatype_struct() {
        let fields = Fields::from(vec![
            Field::new("a", DataType::Int32, false),
            Field::new("b", DataType::Utf8, true),
        ]);
        let dt = DataType::Struct(fields);
        let serializable = SerializableDataType::from(dt.clone());

        let json = serde_json::to_string(&serializable).unwrap();
        let deserialized: SerializableDataType = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.0, dt);
    }
}
