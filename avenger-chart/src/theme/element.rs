//! Minimal element type for CSS selector matching that converts from ThemeContext

use super::selector_impl::{ChartPseudoClass, ChartPseudoElement, ChartSelectors, ChartString};
use crate::theme::ThemeContext;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::matching::ElementSelectorFlags;
use selectors::{Element, OpaqueElement};
use std::collections::HashMap;
use std::sync::Arc;

/// Element for CSS selector matching (created from ThemeContext)
#[derive(Debug, Clone)]
pub struct CssElement {
    pub element_type: ChartString,

    /// The "type" attribute for [type="..."] selectors
    pub type_attr: Option<ChartString>,

    pub id: Option<ChartString>,
    pub classes: Vec<ChartString>,

    /// Custom attributes for CSS attribute selectors like [attr=value]
    pub attributes: HashMap<String, String>,

    /// Parent context stored as ThemeContext rather than CssElement for efficiency.
    ///
    /// This allows multiple CssElements to share the same Arc<ThemeContext> parent
    /// rather than each maintaining separate CssElement parent chains. The parent
    /// is converted to CssElement lazily in parent_element() only when selector
    /// matching needs to traverse up the tree.
    pub parent: Option<Arc<ThemeContext>>,
}

impl From<&ThemeContext> for CssElement {
    fn from(context: &ThemeContext) -> Self {
        let classes = context
            .classes
            .iter()
            .map(|c| ChartString::from(c.as_str()))
            .collect();

        Self {
            element_type: ChartString::from(context.element_type.as_str()),
            type_attr: context
                .subtype
                .as_ref()
                .map(|s| ChartString::from(s.as_str())),
            id: context.id.as_ref().map(|s| ChartString::from(s.as_str())),
            classes,
            attributes: context.attributes.clone(),
            parent: context.parent.clone(),
        }
    }
}

impl Element for CssElement {
    type Impl = ChartSelectors;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self)
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn parent_element(&self) -> Option<Self> {
        self.parent.as_ref().map(|p| CssElement::from(p.as_ref()))
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        // No sibling support - positional pseudo-classes are not supported
        None
    }

    fn next_sibling_element(&self) -> Option<Self> {
        // No sibling support - positional pseudo-classes are not supported
        None
    }

    fn first_element_child(&self) -> Option<Self> {
        None
    }

    fn is_html_element_in_html_document(&self) -> bool {
        false
    }

    fn has_local_name(&self, name: &str) -> bool {
        self.element_type.0 == name
    }

    fn has_namespace(&self, _ns: &str) -> bool {
        false
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.element_type == other.element_type
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&ChartString>,
        local_name: &ChartString,
        operation: &AttrSelectorOperation<&ChartString>,
    ) -> bool {
        match local_name.0.as_str() {
            "type" => {
                // Special handling for type attribute (maps to subtype field)
                if let Some(ref type_attr) = self.type_attr {
                    operation.eval_str(&type_attr.0)
                } else {
                    false
                }
            }
            attr_name => {
                // General attribute matching for custom attributes
                if let Some(attr_value) = self.attributes.get(attr_name) {
                    operation.eval_str(attr_value)
                } else {
                    false
                }
            }
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &ChartPseudoClass,
        _context: &mut selectors::context::MatchingContext<ChartSelectors>,
    ) -> bool {
        match *pc {}
    }

    fn match_pseudo_element(
        &self,
        pe: &ChartPseudoElement,
        _context: &mut selectors::context::MatchingContext<ChartSelectors>,
    ) -> bool {
        match *pe {}
    }

    fn is_link(&self) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        false
    }

    fn is_root(&self) -> bool {
        false
    }

    fn is_part(&self, _name: &ChartString) -> bool {
        false
    }

    fn imported_part(&self, _name: &ChartString) -> Option<ChartString> {
        None
    }

    fn has_id(&self, id: &ChartString, case_sensitivity: CaseSensitivity) -> bool {
        self.id.as_ref().map_or(false, |self_id| {
            case_sensitivity.eq(self_id.0.as_bytes(), id.0.as_bytes())
        })
    }

    fn has_class(&self, class: &ChartString, case_sensitivity: CaseSensitivity) -> bool {
        self.classes
            .iter()
            .any(|c| case_sensitivity.eq(c.0.as_bytes(), class.0.as_bytes()))
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {
        // No-op
    }

    fn has_custom_state(&self, _name: &ChartString) -> bool {
        false
    }

    fn add_element_unique_hashes(&self, _filter: &mut selectors::bloom::BloomFilter) -> bool {
        false
    }
}
