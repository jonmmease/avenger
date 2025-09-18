//! Minimal element type for CSS selector matching that converts from ThemeContext

use super::selector_impl::{ChartPseudoClass, ChartPseudoElement, ChartSelectors, ChartString};
use crate::theme::ThemeContext;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::matching::ElementSelectorFlags;
use selectors::{Element, OpaqueElement};

/// Element for CSS selector matching (created from ThemeContext)
#[derive(Debug, Clone)]
pub struct CssElement {
    pub element_type: ChartString,
    pub type_attr: Option<ChartString>, // The "type" attribute for [type="..."] selectors
    pub id: Option<ChartString>,
    pub classes: Vec<ChartString>,
    pub is_first_child: bool,
    pub is_last_child: bool,
    pub child_index: usize,
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
            is_first_child: context.is_first_child,
            is_last_child: context.is_last_child,
            child_index: context.child_index,
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
        None
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
        if self.is_first_child || self.child_index == 0 {
            None
        } else {
            // Return a dummy sibling with proper index
            // The previous sibling should have child_index - 1
            Some(CssElement {
                element_type: self.element_type.clone(),
                type_attr: self.type_attr.clone(),
                id: None,
                classes: Vec::new(),
                // If this element is at index 1, prev sibling is at index 0 (first child)
                is_first_child: self.child_index == 1,
                is_last_child: false,
                child_index: self.child_index - 1,
            })
        }
    }

    fn next_sibling_element(&self) -> Option<Self> {
        if self.is_last_child {
            None
        } else {
            // Return a dummy sibling with proper index
            // The next sibling should have child_index + 1
            Some(CssElement {
                element_type: self.element_type.clone(),
                type_attr: self.type_attr.clone(),
                id: None,
                classes: Vec::new(),
                is_first_child: false,
                // We don't know if the next sibling is the last, so we say false
                // This could cause issues, but we can't know without more context
                is_last_child: false,
                child_index: self.child_index + 1,
            })
        }
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
                    // Use the selectors crate's built-in evaluation!
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
            ChartPseudoClass::FirstChild => self.is_first_child,
            ChartPseudoClass::LastChild => self.is_last_child,
            ChartPseudoClass::NthChild(n) => {
                // nth-child(1) means first child (index 0)
                // Ensure n is positive to avoid underflow
                if *n > 0 {
                    self.child_index == (*n as usize).saturating_sub(1)
                } else {
                    false
                }
            }
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
