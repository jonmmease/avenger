use std::{fmt, sync::Arc};

macro_rules! identity {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Arc<str>);

        impl $name {
            /// Borrow the caller-assigned value.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.into())
            }
        }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value.into())
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

identity!(PanelId, "Stable caller-assigned identity of one plot area.");
identity!(
    GroupId,
    "Stable caller-assigned identity of a logical container."
);
identity!(
    GuideKey,
    "Identity of one guide family within a planning call."
);
identity!(
    EquivalenceKey,
    "Caller-issued evidence that guide contributions have equivalent content."
);

/// A panel or logical group, with distinct identity namespaces.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeId {
    /// One plot area.
    Panel(PanelId),
    /// A logical container.
    Group(GroupId),
}

impl From<PanelId> for NodeId {
    fn from(id: PanelId) -> Self {
        Self::Panel(id)
    }
}
impl From<GroupId> for NodeId {
    fn from(id: GroupId) -> Self {
        Self::Group(id)
    }
}
impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panel(id) => write!(f, "panel {id}"),
            Self::Group(id) => write!(f, "group {id}"),
        }
    }
}
