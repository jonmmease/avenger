macro_rules! identity {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);
        impl $name {
            /// Return the caller-supplied identifier.
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
                Self(value)
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
identity!(WidgetId, "Stable control identity within a widget runtime.");
identity!(ChoiceItemId, "Stable item identity within a choice group.");

/// A control or one item within a grouped control.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WidgetTarget {
    pub widget: WidgetId,
    pub item: Option<ChoiceItemId>,
}
impl WidgetTarget {
    /// Target one standalone control.
    pub fn new(widget: impl Into<WidgetId>) -> Self {
        Self {
            widget: widget.into(),
            item: None,
        }
    }
    /// Target one item of a choice group.
    pub fn item(widget: impl Into<WidgetId>, item: impl Into<ChoiceItemId>) -> Self {
        Self {
            widget: widget.into(),
            item: Some(item.into()),
        }
    }
}
