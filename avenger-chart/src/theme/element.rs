//! Minimal element type for CSS selector matching that converts from ThemeContext

use super::selector_impl::{ChartPseudoClass, ChartPseudoElement, ChartSelectors, ChartString};
use crate::theme::ThemeContext;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::matching::ElementSelectorFlags;
use selectors::{Element, OpaqueElement};
use std::sync::Arc;

/// Element for CSS selector matching (created from ThemeContext)
#[derive(Debug, Clone)]
pub struct CssElement {
    pub element_type: ChartString,
    // The "type" attribute for [type="..."] selectors
    pub type_attr: Option<ChartString>,
    pub id: Option<ChartString>,
    pub classes: Vec<ChartString>,
    pub parent: Option<Arc<ThemeContext>>,
}

impl From<&ThemeContext> for CssElement {
    fn from(context: &ThemeContext) -> Self {
        let mut classes = Vec::new();

        // Add regular classes only (NOT subtype)
        for class in &context.classes {
            classes.push(ChartString::from(class.as_str()));
        }

        Self {
            element_type: ChartString::from(context.element_type.as_str()),
            type_attr: context
                .subtype
                .as_ref()
                .map(|s| ChartString::from(s.as_str())),
            id: context.id.as_ref().map(|s| ChartString::from(s.as_str())),
            classes,
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
        // No sibling support - we removed pseudo-classes like :first-child
        None
    }

    fn next_sibling_element(&self) -> Option<Self> {
        // No sibling support - we removed pseudo-classes like :last-child
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
                if let Some(ref type_attr) = self.type_attr {
                    operation.eval_str(&type_attr.0)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &ChartPseudoClass,
        _context: &mut selectors::context::MatchingContext<ChartSelectors>,
    ) -> bool {
        match pc {
            ChartPseudoClass::Hover | ChartPseudoClass::Active => false,
        }
    }

    fn match_pseudo_element(
        &self,
        _pe: &ChartPseudoElement,
        _context: &mut selectors::context::MatchingContext<ChartSelectors>,
    ) -> bool {
        match *_pe {}
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
