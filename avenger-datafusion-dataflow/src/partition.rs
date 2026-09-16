use std::{fmt, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, SchemaRef},
    common::ScalarValue,
};

use crate::{Error, Result};

/// An ordered typed key local to one partitioning level, independent of a graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PartitionKey(pub(crate) Arc<[ScalarValue]>);

impl PartitionKey {
    /// Return key components in partition-expression order.
    pub fn values(&self) -> &[ScalarValue] {
        &self.0
    }

    pub(crate) fn size(&self) -> usize {
        std::mem::size_of::<Self>() + self.0.iter().map(ScalarValue::size).sum::<usize>()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ScopeIdentity {
    pub index: usize,
    pub name: Arc<str>,
}

/// A child definition shared by every instance discovered from its source.
#[derive(Clone, Debug)]
pub struct ScopeHandle {
    pub(crate) graph: u64,
    pub(crate) index: usize,
    pub(crate) parent: usize,
    pub(crate) path: Arc<[ScopeIdentity]>,
    pub(crate) key_schema: SchemaRef,
}

impl ScopeHandle {
    /// Return the definition's diagnostic name.
    pub fn name(&self) -> &str {
        &self.path.last().expect("child scope path").name
    }

    /// Return the ordered fields of the partition key.
    pub fn key_schema(&self) -> &SchemaRef {
        &self.key_schema
    }

    /// Validate a local key, with exact types and no implicit casts.
    pub fn key(&self, values: impl IntoIterator<Item = ScalarValue>) -> Result<PartitionKey> {
        let values: Vec<_> = values.into_iter().collect();
        if values.len() != self.key_schema.fields().len() {
            return Err(Error::InvalidKey(format!(
                "{} expects {} components, received {}",
                self.name(),
                self.key_schema.fields().len(),
                values.len()
            )));
        }
        for (value, field) in values.iter().zip(self.key_schema.fields()) {
            validate_key_type(field.data_type())?;
            if value.data_type() != *field.data_type() {
                return Err(Error::InvalidKey(format!(
                    "{} expects {}, received {}",
                    field.name(),
                    field.data_type(),
                    value.data_type()
                )));
            }
        }
        Ok(PartitionKey(values.into()))
    }

    /// Address a top-level instance without discovering or creating it.
    pub fn instance(&self, values: impl IntoIterator<Item = ScalarValue>) -> Result<ScopeInstance> {
        if self.parent != 0 {
            return Err(Error::InvalidScopeAddress(
                "nested instances require parent.child()".into(),
            ));
        }
        Ok(ScopeInstance {
            graph: self.graph,
            path: vec![(self.path[0].clone(), self.key(values)?)].into(),
        })
    }
}

/// A complete definition-and-key address. It retains no input or result tables.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopeInstance {
    pub(crate) graph: u64,
    pub(crate) path: Arc<[(ScopeIdentity, PartitionKey)]>,
}

impl ScopeInstance {
    /// Address an immediate child without querying data.
    pub fn child(
        &self,
        scope: &ScopeHandle,
        values: impl IntoIterator<Item = ScalarValue>,
    ) -> Result<Self> {
        if self.graph != scope.graph {
            return Err(Error::ForeignHandle);
        }
        if self.scope() != scope.parent
            || self.path.len() + 1 != scope.path.len()
            || self
                .path
                .iter()
                .zip(scope.path.iter())
                .any(|((identity, _), expected)| identity != expected)
        {
            return Err(Error::InvalidScopeAddress(
                "scope is not an immediate child of this instance".into(),
            ));
        }
        let mut path = self.path.to_vec();
        path.push((
            scope.path.last().expect("child scope path").clone(),
            scope.key(values)?,
        ));
        Ok(Self {
            graph: self.graph,
            path: path.into(),
        })
    }

    pub(crate) fn scope(&self) -> usize {
        self.path.last().expect("nonempty instance address").0.index
    }

    pub(crate) fn size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .path
                .iter()
                .map(|(identity, key)| {
                    std::mem::size_of::<ScopeIdentity>() + identity.name.len() + key.size()
                })
                .sum::<usize>()
    }
}

impl fmt::Display for ScopeInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (identity, key)) in self.path.iter().enumerate() {
            if index != 0 {
                write!(f, "/")?;
            }
            write!(f, "{}{:?}", identity.name, key.values())?;
        }
        Ok(())
    }
}

pub(crate) fn validate_key_type(data_type: &DataType) -> Result<()> {
    if matches!(
        data_type,
        DataType::Null
            | DataType::Boolean
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::Date32
            | DataType::Date64
            | DataType::Timestamp(_, _)
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _)
    ) {
        Ok(())
    } else {
        Err(Error::UnsupportedKeyType(data_type.to_string()))
    }
}
