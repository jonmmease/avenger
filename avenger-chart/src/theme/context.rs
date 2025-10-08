//! Theme context for hierarchical element styling

use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use std::collections::HashMap;
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

    /// Custom attributes for CSS attribute selectors
    /// Values are stored as strings and matched against CSS selectors like [attr=value]
    pub attributes: HashMap<String, String>,

    /// Parent context for hierarchical CSS selectors
    pub parent: Option<Arc<ThemeContext>>,

    /// Parameter values for CSS variable resolution
    /// These params can override CSS variables defined in the theme
    pub params: IndexMap<String, ScalarValue>,
}

impl ThemeContext {
    /// Create a new context for a specific element with runtime parameters
    ///
    /// Runtime parameters are required to ensure CSS variables and media queries
    /// can be properly evaluated throughout the theme system.
    pub fn new(element_type: impl Into<String>, params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            element_type: element_type.into(),
            subtype: None,
            classes: Vec::new(),
            id: None,
            attributes: HashMap::new(),
            parent: None,
            params,
        }
    }

    /// Create a child context (inherits params from parent)
    pub fn child(&self, element_type: impl Into<String>) -> Self {
        Self {
            element_type: element_type.into(),
            subtype: None,
            classes: Vec::new(),
            id: None,
            attributes: HashMap::new(),
            parent: Some(Arc::new(self.clone())),
            params: self.params.clone(), // Inherit params from parent
        }
    }

    /// Set the subtype for the element
    pub fn with_subtype(mut self, subtype: impl Into<String>) -> Self {
        self.subtype = Some(subtype.into());
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

    /// Add a custom attribute to the context
    ///
    /// Attributes can be used in CSS selectors like `[attr=value]` or `[attr="value"]`.
    /// Values can be numbers or strings - they will be stored and matched as strings.
    ///
    /// # Examples
    ///
    /// ```
    /// use avenger_chart::theme::ThemeContext;
    /// use indexmap::IndexMap;
    ///
    /// let params = IndexMap::new();
    /// let ctx = ThemeContext::new("mark", params)
    ///     .with_subtype("symbol")
    ///     .with_attribute("cardinality", "5");
    /// ```
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}
