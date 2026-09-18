use crate::{Error, Result};
use datafusion::{arrow::datatypes::DataType, common::ScalarValue};
use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

macro_rules! name_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Arc<str>);
        impl $name {
            /// Create a nonempty caller-assigned name in the compiler's namespace.
            pub fn new(name: impl Into<String>) -> Result<Self> {
                let name = name.into();
                if name.trim().is_empty() {
                    return Err(Error::InvalidDefinition(
                        "identity names must not be empty".into(),
                    ));
                }
                Ok(Self(name.into()))
            }
            /// Return the original name without normalization.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
name_type!(
    SelectionId,
    "One shared named selection in a compiler-resolved namespace."
);
name_type!(
    ProducerId,
    "An independently updated selection declaration."
);
name_type!(
    ViewId,
    "A semantic view shared by marks that use the same self-filter exclusion."
);
name_type!(ScopeId, "A partitioning level in an ordered facet address.");
name_type!(
    ProjectionId,
    "A projection local to one producer definition."
);

/// A checked composite key at one level of a nested facet address.
#[derive(Clone, Debug)]
pub struct FacetKey {
    scope: ScopeId,
    values: Vec<ScalarValue>,
}
impl PartialEq for FacetKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for FacetKey {}
impl Hash for FacetKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.scope.hash(state);
        self.values.len().hash(state);
        for value in &self.values {
            value.data_type().hash(state);
            value.hash(state);
        }
    }
}
impl FacetKey {
    /// Preserve key order and exact types using dataflow's supported key types.
    pub fn new(scope: ScopeId, values: Vec<ScalarValue>) -> Result<Self> {
        if values.is_empty() {
            return Err(Error::InvalidDefinition(
                "facet keys must have a component".into(),
            ));
        }
        for value in &values {
            if !matches!(
                value.data_type(),
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
                return Err(Error::InvalidDefinition(format!(
                    "unsupported facet key type {}",
                    value.data_type()
                )));
            }
        }
        Ok(Self { scope, values })
    }
    /// Return this partitioning level's identity.
    pub fn scope(&self) -> &ScopeId {
        &self.scope
    }
    /// Return the composite key in partition-expression order.
    pub fn values(&self) -> &[ScalarValue] {
        &self.values
    }
}
impl PartialOrd for FacetKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for FacetKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.scope.cmp(&other.scope).then_with(|| {
            self.values
                .iter()
                .zip(&other.values)
                .map(|(a, b)| crate::values::scalar_cmp(a, b))
                .find(|order| !order.is_eq())
                .unwrap_or_else(|| self.values.len().cmp(&other.values.len()))
        })
    }
}

/// The full semantic view instance, including every parent facet key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewAddress {
    pub view: ViewId,
    pub scope: Vec<FacetKey>,
}
impl ViewAddress {
    /// Address a view outside a facet scope.
    pub fn root(view: ViewId) -> Self {
        Self {
            view,
            scope: vec![],
        }
    }
}

/// A producer's selection, declaration, and originating view instance.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProducerAddress {
    pub selection: SelectionId,
    pub producer: ProducerId,
    pub origin: ViewAddress,
}
