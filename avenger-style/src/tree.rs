//! The style tree: the nodes of a chart with their elements, attributes, classes, ids, and inline
//! styles, which selectors match.

use std::fmt;

use selectors::{
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint},
    bloom::BloomFilter,
    matching::{ElementSelectorFlags, MatchingContext},
    Element as SelectorElement, OpaqueElement,
};

use crate::{
    selector::{Name, PseudoClass, PseudoElement, Selectors},
    values::Value,
    vocab::Element,
    Property,
};

/// The id of a node of a style tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The value of an attribute: one value, or the two forms of the `type` of a mark and of the
/// `marks` of a scale, either of which a selector matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributeValue {
    One(String),
    Forms([String; 2]),
}

/// A node of a style tree.
#[derive(Clone, Debug)]
pub struct Node {
    pub element: Element,
    pub id: Option<String>,
    pub classes: Vec<String>,
    attributes: Vec<(&'static str, AttributeValue)>,
    /// The inline styles of the node: the style fields of the chart, which rank above the normal
    /// declarations of every stylesheet.
    pub inline: Vec<(Property, Value)>,
    parent: Option<NodeId>,
    /// The node's position among its parent's children.
    position: u32,
    children: Vec<NodeId>,
}

impl Node {
    /// The value of the attribute `name`.
    pub fn attribute(&self, name: &str) -> Option<&AttributeValue> {
        (self.attributes.iter())
            .find(|(attribute, _)| *attribute == name)
            .map(|(_, value)| value)
    }

    /// The first form of the attribute `name`: its only value, or the kind of a mark's `type`.
    pub fn attribute_str(&self, name: &str) -> Option<&str> {
        match self.attribute(name)? {
            AttributeValue::One(value) => Some(value),
            AttributeValue::Forms([kind, _]) => Some(kind),
        }
    }
}

/// A tree of style nodes, in an order where each node follows its parent and its earlier
/// siblings, which the evaluator builds from a chart.
#[derive(Clone, Debug)]
pub struct StyleTree {
    nodes: Vec<Node>,
}

impl StyleTree {
    /// A tree of one root node.
    pub fn new(root: Element) -> Self {
        StyleTree {
            nodes: vec![Node {
                element: root,
                id: None,
                classes: Vec::new(),
                attributes: Vec::new(),
                inline: Vec::new(),
                parent: None,
                position: 0,
                children: Vec::new(),
            }],
        }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Adds a child of `element` after the other children of `parent`.
    pub fn add(&mut self, parent: NodeId, element: Element) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        let position = self.nodes[parent.index()].children.len() as u32;
        self.nodes[parent.index()].children.push(id);
        self.nodes.push(Node {
            element,
            id: None,
            classes: Vec::new(),
            attributes: Vec::new(),
            inline: Vec::new(),
            parent: Some(parent),
            position,
            children: Vec::new(),
        });
        id
    }

    pub fn set_id(&mut self, node: NodeId, id: impl Into<String>) {
        self.nodes[node.index()].id = Some(id.into());
    }

    pub fn add_class(&mut self, node: NodeId, class: impl Into<String>) {
        self.nodes[node.index()].classes.push(class.into());
    }

    /// Sets the attribute `name`.
    pub fn set_attribute(&mut self, node: NodeId, name: &'static str, value: impl Into<String>) {
        self.set(node, name, AttributeValue::One(value.into()));
    }

    /// Sets the two forms of the attribute `name`: the kind of a mark and its function name, or
    /// the lists of both for the marks of a scale.
    pub fn set_attribute_forms(
        &mut self,
        node: NodeId,
        name: &'static str,
        kind: impl Into<String>,
        function: impl Into<String>,
    ) {
        self.set(
            node,
            name,
            AttributeValue::Forms([kind.into(), function.into()]),
        );
    }

    fn set(&mut self, node: NodeId, name: &'static str, value: AttributeValue) {
        let attributes = &mut self.nodes[node.index()].attributes;
        match attributes
            .iter_mut()
            .find(|(attribute, _)| *attribute == name)
        {
            Some((_, old)) => *old = value,
            None => attributes.push((name, value)),
        }
    }

    /// Adds an inline style of `property`, which replaces an earlier one of the same property.
    pub fn set_inline(&mut self, node: NodeId, property: Property, value: Value) {
        let inline = &mut self.nodes[node.index()].inline;
        inline.retain(|(existing, _)| *existing != property);
        inline.push((property, value));
    }

    pub fn node(&self, node: NodeId) -> &Node {
        &self.nodes[node.index()]
    }

    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.nodes[node.index()].parent
    }

    pub fn children(&self, node: NodeId) -> &[NodeId] {
        &self.nodes[node.index()].children
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The nodes in tree order, each after its parent.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> {
        (0..self.nodes.len() as u32).map(NodeId)
    }

    pub(crate) fn element(&self, node: NodeId) -> NodeRef<'_> {
        NodeRef { tree: self, node }
    }
}

/// A node of a tree, which selectors match.
#[derive(Clone, Copy)]
pub(crate) struct NodeRef<'a> {
    tree: &'a StyleTree,
    node: NodeId,
}

impl fmt::Debug for NodeRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.data().element, self.node.0)
    }
}

impl NodeRef<'_> {
    fn data(&self) -> &Node {
        self.tree.node(self.node)
    }

    fn with(&self, node: NodeId) -> Self {
        NodeRef {
            tree: self.tree,
            node,
        }
    }

    fn sibling(&self, offset: i64) -> Option<Self> {
        let parent = self.data().parent?;
        let position = i64::from(self.data().position) + offset;
        let siblings = self.tree.children(parent);
        usize::try_from(position)
            .ok()
            .and_then(|position| siblings.get(position))
            .map(|node| self.with(*node))
    }
}

impl SelectorElement for NodeRef<'_> {
    type Impl = Selectors;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self.data())
    }

    fn parent_element(&self) -> Option<Self> {
        self.data().parent.map(|parent| self.with(parent))
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
        self.sibling(-1)
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.sibling(1)
    }

    fn first_element_child(&self) -> Option<Self> {
        self.data().children.first().map(|child| self.with(*child))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        false
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.data().element.name() == local_name
    }

    fn has_namespace(&self, ns: &str) -> bool {
        ns.is_empty()
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.data().element == other.data().element
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&Name>,
        local_name: &Name,
        operation: &AttrSelectorOperation<&Name>,
    ) -> bool {
        match self.data().attribute(&local_name.0) {
            None => false,
            Some(AttributeValue::One(value)) => operation.eval_str(value),
            Some(AttributeValue::Forms(forms)) => forms.iter().any(|form| operation.eval_str(form)),
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo_class: &PseudoClass,
        _context: &mut MatchingContext<Selectors>,
    ) -> bool {
        match pseudo_class {
            // No node has keyboard focus until the accessibility tree gives focus to one.
            PseudoClass::FocusVisible => false,
        }
    }

    fn match_pseudo_element(
        &self,
        pseudo_element: &PseudoElement,
        _context: &mut MatchingContext<Selectors>,
    ) -> bool {
        match *pseudo_element {}
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        false
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &Name, case_sensitivity: CaseSensitivity) -> bool {
        (self.data().id.as_ref())
            .is_some_and(|own| case_sensitivity.eq(own.as_bytes(), id.0.as_bytes()))
    }

    fn has_class(&self, name: &Name, case_sensitivity: CaseSensitivity) -> bool {
        (self.data().classes.iter())
            .any(|class| case_sensitivity.eq(class.as_bytes(), name.0.as_bytes()))
    }

    fn has_custom_state(&self, _name: &Name) -> bool {
        false
    }

    fn imported_part(&self, _name: &Name) -> Option<Name> {
        None
    }

    fn is_part(&self, _name: &Name) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.data().children.is_empty()
    }

    fn is_root(&self) -> bool {
        self.data().parent.is_none()
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}
