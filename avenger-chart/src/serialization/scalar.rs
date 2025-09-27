//! Serializable wrapper for ScalarValue

use datafusion_common::ScalarValue;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Wrapper for ScalarValue that implements Serialize/Deserialize
/// using datafusion-proto-common's protobuf support
#[derive(Debug, Clone, PartialEq)]
pub struct SerializableScalar(pub ScalarValue);

impl SerializableScalar {
    /// Create from a ScalarValue
    pub fn new(scalar: ScalarValue) -> Self {
        Self(scalar)
    }

    /// Get the inner ScalarValue
    pub fn into_inner(self) -> ScalarValue {
        self.0
    }

    /// Get a reference to the inner ScalarValue
    pub fn as_scalar(&self) -> &ScalarValue {
        &self.0
    }
}

impl From<ScalarValue> for SerializableScalar {
    fn from(scalar: ScalarValue) -> Self {
        Self::new(scalar)
    }
}

impl From<SerializableScalar> for ScalarValue {
    fn from(serializable: SerializableScalar) -> Self {
        serializable.into_inner()
    }
}

impl Serialize for SerializableScalar {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use prost::Message;

        // Convert ScalarValue to protobuf
        let proto_scalar = datafusion_proto_common::protobuf_common::ScalarValue::try_from(&self.0)
            .map_err(|e| serde::ser::Error::custom(format!("Failed to convert to protobuf: {}", e)))?;

        // Serialize to bytes
        let mut buf = Vec::new();
        proto_scalar.encode(&mut buf)
            .map_err(|e| serde::ser::Error::custom(format!("Failed to encode protobuf: {}", e)))?;

        // Encode as base64 for JSON compatibility
        let base64_str = BASE64.encode(&buf);
        serializer.serialize_str(&base64_str)
    }
}

impl<'de> Deserialize<'de> for SerializableScalar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use prost::Message;

        // Deserialize as string
        let base64_str = String::deserialize(deserializer)?;

        // Decode base64
        let buf = BASE64.decode(&base64_str)
            .map_err(|e| serde::de::Error::custom(format!("Failed to decode base64: {}", e)))?;

        // Decode protobuf
        let proto_scalar = datafusion_proto_common::protobuf_common::ScalarValue::decode(&buf[..])
            .map_err(|e| serde::de::Error::custom(format!("Failed to decode protobuf: {}", e)))?;

        // Convert back to ScalarValue
        let scalar = ScalarValue::try_from(&proto_scalar)
            .map_err(|e| serde::de::Error::custom(format!("Failed to convert from protobuf: {}", e)))?;

        Ok(Self(scalar))
    }
}


