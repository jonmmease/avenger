//! Serializable wrapper for IndexMap<String, ScalarValue>

use super::SerializableScalar;
use datafusion_common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Serializable wrapper for IndexMap<String, ScalarValue>
/// Stores as Vec<(String, SerializableScalar)> for serialization
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SerializableScalarMap(pub Vec<(String, SerializableScalar)>);

impl From<IndexMap<String, ScalarValue>> for SerializableScalarMap {
    fn from(map: IndexMap<String, ScalarValue>) -> Self {
        let vec = map
            .into_iter()
            .map(|(k, v)| (k, SerializableScalar::new(v)))
            .collect();
        Self(vec)
    }
}

impl From<SerializableScalarMap> for IndexMap<String, ScalarValue> {
    fn from(wrapper: SerializableScalarMap) -> Self {
        wrapper
            .0
            .into_iter()
            .map(|(k, v)| (k, v.into_inner()))
            .collect()
    }
}

/// Nested version for IndexMap<String, IndexMap<String, ScalarValue>>
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SerializableNestedScalarMap(pub Vec<(String, SerializableScalarMap)>);

impl From<IndexMap<String, IndexMap<String, ScalarValue>>> for SerializableNestedScalarMap {
    fn from(map: IndexMap<String, IndexMap<String, ScalarValue>>) -> Self {
        let vec = map
            .into_iter()
            .map(|(k, inner)| (k, SerializableScalarMap::from(inner)))
            .collect();
        Self(vec)
    }
}

impl From<SerializableNestedScalarMap> for IndexMap<String, IndexMap<String, ScalarValue>> {
    fn from(wrapper: SerializableNestedScalarMap) -> Self {
        wrapper
            .0
            .into_iter()
            .map(|(k, v)| (k, IndexMap::from(v)))
            .collect()
    }
}