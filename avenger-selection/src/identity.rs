use crate::{Error, Result};
use std::{fmt, sync::Arc};

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
    "An opaque view-instance ID. Share it across layers, and use distinct IDs for distinct facets."
);
name_type!(
    ProjectionId,
    "A projection local to one producer definition."
);

/// Private composite identity for routing and contribution attribution.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ProducerAddress {
    pub(crate) selection: SelectionId,
    pub(crate) producer: ProducerId,
    pub(crate) origin: ViewId,
}
