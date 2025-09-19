//! Theme context for hierarchical element styling

use std::sync::Arc;

/// Context for theme queries, providing information about the element being styled
#[derive(Debug, Clone)]
pub struct ThemeContext {
    /// Element type (e.g., "axis", "legend", "mark", "title")
    pub element_type: String,

    /// Optional subtype for the element:
    /// - For marks: "symbol", "line", "rect", "text", etc.
    /// - For axes: "x", "y", "r", "theta", etc.
    /// - For legends: "symbol", "line", "colorbar", etc.
    pub subtype: Option<String>,

    /// Optional classes/tags for the element
    pub classes: Vec<String>,

    /// Optional unique identifier
    pub id: Option<String>,

    /// Parent context for hierarchical CSS selectors
    pub parent: Option<Arc<ThemeContext>>,
}

impl ThemeContext {
    /// Create a new context for a specific element
    pub fn new(element_type: impl Into<String>) -> Self {
        Self {
            element_type: element_type.into(),
            subtype: None,
            classes: Vec::new(),
            id: None,
            parent: None,
        }
    }

    /// Create a child context
    pub fn child(&self, element_type: impl Into<String>) -> Self {
        Self {
            element_type: element_type.into(),
            subtype: None,
            classes: Vec::new(),
            id: None,
            parent: Some(Arc::new(self.clone())),
        }
    }

    /// Set the subtype for the element
    pub fn with_subtype(mut self, subtype: impl Into<String>) -> Self {
        self.subtype = Some(subtype.into());
        self
    }

    /// Deprecated: Use with_subtype() instead
    pub fn with_mark(mut self, mark: impl Into<String>) -> Self {
        self.subtype = Some(mark.into());
        self
    }

    /// Add a channel to the context (adds as a class)
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.classes.push(channel.into());
        self
    }

    /// Add a class to the context
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.classes.push(class.into());
        self
    }

    /// Set the ID
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// Helper trait for building contexts fluently
pub trait ContextBuilder {
    fn axis_context(subtype: &str) -> ThemeContext {
        ThemeContext::new("axis").with_subtype(subtype)
    }

    fn legend_context(subtype: &str) -> ThemeContext {
        ThemeContext::new("legend").with_subtype(subtype)
    }

    fn mark_context(mark_type: &str) -> ThemeContext {
        ThemeContext::new("mark").with_subtype(mark_type)
    }

    fn title_context() -> ThemeContext {
        ThemeContext::new("title")
    }

    fn subtitle_context() -> ThemeContext {
        ThemeContext::new("subtitle")
    }
}

// Blanket implementation
impl ContextBuilder for ThemeContext {}
