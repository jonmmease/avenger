use std::cell::LazyCell;
use std::fmt::{self, Debug, Display, Formatter};
use std::ops::{Deref, Range};
use std::sync::Arc;

use ecow::{EcoString, EcoVec, eco_format, eco_vec};
use crate::typst_utils::debug;

use crate::typst_syntax::{
    DiagSpan, FileId, RangeMapper, Span, SpanKind, SpanNumber, Spanned, SubRange,
    SyntaxKind, SyntaxMode,
};

/// A node in the untyped syntax tree.
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct SyntaxNode {
    /// The underlying node data, potentially with wrapped warning messages.
    data: Node,
    /// The node's span, at the top-level to guarantee efficient access.
    span: Span,
    // We would love to move the `SyntaxKind` up here as well, but keeping it in
    // `Node` saves 8 bytes :/
}

/// The data for nodes in the tree, plus their [`SyntaxKind`]. May actually be a
/// warning message wrapping a child [`Node`].
///
/// Contains the [`SyntaxKind`] at the top-level for efficient access. This
/// requires being careful when mutating the kind, as warnings store this type
/// as their child, which duplicates the kind. Deduplicating the syntax kinds
/// would require a whole other enum type, and makes mutable access too painful.
///
/// The only other invariant for syntax kinds is that error nodes always contain
/// [`SyntaxKind::Error`], but leaf and inner nodes never do. The syntax kind of
/// a warning depends on what it wraps.
///
/// The simplest way to get the underlying data by descending into the children
/// of warnings is via a loop and match like below. The [`SyntaxNode::node_ref`]
/// helper does this for the by-reference case, but mutation is usually more
/// involved, so only gets the [`SyntaxNode::inner_and_span_mut`] helper.
/// ```ignore
/// let mut data = &mut node.data;
/// let value = loop {
///     match data {
///         Leaf(_, _) | Inner(_, _) | Error(_, _) => break "value",
///         Warning(warn, _) => data = &mut Arc::make_mut(warn).child,
///     }
/// };
/// ```
#[derive(Clone, Eq, PartialEq, Hash)]
enum Node {
    Leaf(EcoString, SyntaxKind),
    Inner(Arc<InnerNode>, SyntaxKind),
    Error(Arc<ErrorNode>, SyntaxKind),
    Warning(Arc<WarningWrapper>, SyntaxKind),
}

/// Data attached to a node, accessed by reference via [`SyntaxNode::node_ref`].
enum NodeRef<'a> {
    Leaf(&'a EcoString),
    Inner(&'a Arc<InnerNode>),
    Error(&'a Arc<ErrorNode>),
}

impl SyntaxNode {
    /// Access the underlying node data by reference, descending past warnings.
    fn node_ref(&self) -> NodeRef<'_> {
        let mut data = &self.data;
        loop {
            match data {
                Node::Leaf(text, _) => break NodeRef::Leaf(text),
                Node::Inner(inner, _) => break NodeRef::Inner(inner),
                Node::Error(err, _) => break NodeRef::Error(err),
                Node::Warning(warn, _) => data = &warn.child,
            }
        }
    }

    /// Access an inner node and the node's overall span mutably, descending
    /// past warnings. If this only returned the `&mut InnerNode`, the caller
    /// wouldn't be able to also get a mutable reference to the span since the
    /// inner node would borrow mutably from `self`.
    fn inner_and_span_mut(&mut self) -> Option<(&mut InnerNode, &mut Span)> {
        let mut data = &mut self.data;
        loop {
            match data {
                Node::Leaf(_, _) | Node::Error(_, _) => break None,
                Node::Inner(inner, _) => {
                    break Some((Arc::make_mut(inner), &mut self.span));
                }
                Node::Warning(warn, _) => data = &mut Arc::make_mut(warn).child,
            }
        }
    }

    /// Access the hints for an error or warning mutably.
    fn hints_mut(&mut self) -> Option<&mut EcoVec<(EcoString, Option<SubRange>)>> {
        match &mut self.data {
            Node::Leaf(_, _) | Node::Inner(_, _) => None,
            Node::Error(err, _) => Some(&mut Arc::make_mut(err).hints),
            Node::Warning(warn, _) => Some(&mut Arc::make_mut(warn).hints),
        }
    }
}

impl SyntaxNode {
    /// Create a new leaf node.
    #[track_caller]
    pub fn leaf(kind: SyntaxKind, text: impl Into<EcoString>) -> Self {
        debug_assert!(!kind.is_error());
        Self {
            data: Node::Leaf(text.into(), kind),
            span: Span::detached(),
        }
    }

    /// Create a new inner node with children.
    #[track_caller]
    pub fn inner(kind: SyntaxKind, children: Vec<SyntaxNode>) -> Self {
        debug_assert!(!kind.is_error());
        Self {
            data: Node::Inner(Arc::new(InnerNode::new(children)), kind),
            span: Span::detached(),
        }
    }

    /// Create a new error node with a user-presentable message for the given
    /// text. Note that the message is the first argument, and the text causing
    /// the error is the second argument.
    pub fn error(message: impl Into<EcoString>, text: impl Into<EcoString>) -> Self {
        Self {
            data: Node::Error(
                Arc::new(ErrorNode::new(message.into(), text.into())),
                SyntaxKind::Error,
            ),
            span: Span::detached(),
        }
    }

    /// Add a warning message to an existing node.
    pub fn warn(&mut self, message: impl Into<EcoString>) {
        let kind = self.kind();
        let child = std::mem::replace(&mut self.data, Node::Leaf(EcoString::new(), kind));
        let warn = Arc::new(WarningWrapper::new(child, None, message.into()));
        self.data = Node::Warning(warn, kind);
    }

    /// Add a warning around this node at a particular sub-range of the node's
    /// text. Panics if the range is empty or exceeds the length of the wrapped
    /// text.
    #[track_caller]
    pub fn warn_at(
        &mut self,
        Range { start, end }: Range<usize>,
        message: impl Into<EcoString>,
    ) {
        assert!(end <= self.len()); // This isn't checked by `SubRange::new`.
        let sub_range = SubRange::new(start, end).expect("a valid sub-range");
        let kind = self.kind();
        let child = std::mem::replace(&mut self.data, Node::Leaf(EcoString::new(), kind));
        let warn = Arc::new(WarningWrapper::new(child, Some(sub_range), message.into()));
        self.data = Node::Warning(warn, kind);
    }

    /// Add a user-presentable hint to an existing error or warning. Panics if
    /// this is not an error or warning.
    #[track_caller]
    pub fn hint(&mut self, hint: impl Into<EcoString>) {
        let hints = self.hints_mut().expect("expected an error or warning");
        hints.push((hint.into(), None));
    }

    /// Add a user-presentable hint to an existing error or warning at a
    /// sub-range of the text. Panics if the range is empty or exceeds the
    /// length of the wrapped text. Panics if this is not an error or warning
    /// node.
    #[track_caller]
    pub fn hint_at(
        &mut self,
        Range { start, end }: Range<usize>,
        hint: impl Into<EcoString>,
    ) {
        assert!(end <= self.len()); // This isn't checked by `SubRange::new`.
        let sub_range = SubRange::new(start, end).expect("a valid sub-range");
        let hints = self.hints_mut().expect("expected an error or warning");
        hints.push((hint.into(), Some(sub_range)));
    }

    /// Add multiple hints while building an error or warning. Panics if this is
    /// not an error or warning.
    #[track_caller]
    pub fn with_hints(mut self, new_hints: impl IntoIterator<Item = EcoString>) -> Self {
        let hints = self.hints_mut().expect("expected an error or warning");
        let iter = new_hints.into_iter().map(|h| (h, None));
        hints.extend(iter);
        self
    }

    /// Create a dummy node of the given kind.
    ///
    /// Panics if `kind` is [`SyntaxKind::Error`].
    #[track_caller]
    pub const fn placeholder(kind: SyntaxKind) -> Self {
        if kind.is_error() {
            panic!("cannot create error placeholder");
        }
        Self {
            data: Node::Leaf(EcoString::new(), kind),
            span: Span::detached(),
        }
    }

    /// The type of the node.
    pub fn kind(&self) -> SyntaxKind {
        match self.data {
            Node::Leaf(_, kind)
            | Node::Inner(_, kind)
            | Node::Error(_, kind)
            | Node::Warning(_, kind) => kind,
        }
    }

    /// Return `true` if the length is 0.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The byte length of the node in the source text.
    pub fn len(&self) -> usize {
        match self.node_ref() {
            NodeRef::Leaf(text) => text.len(),
            NodeRef::Inner(inner) => inner.len,
            NodeRef::Error(err) => err.text.len(),
        }
    }

    /// The span of the node.
    pub fn span(&self) -> Span {
        self.span
    }

    /// The text of the node if it is a leaf or error node.
    ///
    /// Returns the empty string if this is an inner node.
    pub fn leaf_text(&self) -> &EcoString {
        static EMPTY: EcoString = EcoString::new();
        match self.node_ref() {
            NodeRef::Leaf(text) => text,
            NodeRef::Inner(_) => &EMPTY,
            NodeRef::Error(err) => &err.text,
        }
    }

    /// Clone the full text from the node. If this is an inner node, it will
    /// traverse the tree to build the text which may be expensive.
    pub fn full_text(&self) -> EcoString {
        match &self.data {
            Node::Leaf(leaf, _) => leaf.clone(),
            Node::Error(err, _) => err.text.clone(),
            Node::Inner(_, _) | Node::Warning(_, _) => {
                let mut buffer = EcoString::with_capacity(self.len());
                self.traverse(|node| {
                    match node.node_ref() {
                        NodeRef::Leaf(text) => buffer.push_str(text),
                        NodeRef::Inner(_) => {}
                        NodeRef::Error(err) => buffer.push_str(&err.text),
                    }
                    node.children()
                });
                buffer
            }
        }
    }

    /// The node's children.
    pub fn children(&self) -> std::slice::Iter<'_, SyntaxNode> {
        match self.node_ref() {
            NodeRef::Leaf(_) | NodeRef::Error(_) => [].iter(),
            NodeRef::Inner(inner) => inner.children.iter(),
        }
    }

    /// Whether the node has diagnostic errors and/or warnings in it or its
    /// children. [`Diagnosis`] has public fields, so you can write
    /// `node.diagnosis().errors` to determine if a node is erroneous.
    ///
    /// This can be used to determine whether [`Self::errors_and_warnings`] will
    /// return an empty vector without traversing the tree if it will not.
    pub fn diagnosis(&self) -> Diagnosis {
        let diagnosis = match self.node_ref() {
            NodeRef::Leaf(_) => Diagnosis::default(),
            NodeRef::Inner(inner) => inner.diagnosis,
            NodeRef::Error(_) => Diagnosis { errors: true, warnings: false },
        };
        match &self.data {
            Node::Warning(_, _) => Diagnosis { warnings: true, errors: diagnosis.errors },
            _ => diagnosis,
        }
    }

    /// The error and warning diagnostics for this node and its descendants.
    pub fn errors_and_warnings(&self) -> (Vec<SyntaxDiagnostic>, Vec<SyntaxDiagnostic>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        self.traverse(|node| {
            let mut data = &node.data;
            loop {
                match data {
                    Node::Inner(inner, _) if inner.diagnosis.either() => {
                        break inner.children.iter();
                    }
                    Node::Leaf(_, _) | Node::Inner(_, _) => break [].iter(),
                    Node::Error(err, _) => {
                        errors.push(err.diagnostic(node.span));
                        break [].iter();
                    }
                    Node::Warning(warn, _) => {
                        warnings.push(warn.diagnostic(node.span));
                        data = &warn.child;
                    }
                }
            }
        });
        (errors, warnings)
    }

    /// Set a synthetic span for the node and all its descendants.
    pub fn synthesize(&mut self, span: Span) {
        // Sub-ranges are removed since the overall range is not accurate.
        self.synthesize_with(0, &|_, _| span, &|_, sub_range| *sub_range = None);
    }

    /// Set a raw range span for each node.
    ///
    /// The range is determined by mapping the node's ranges through the given
    /// `mapper`.
    ///
    /// Returns an error with the mapper's length if it was shorter than the
    /// length of the source text.
    pub fn synthesize_mapped(
        &mut self,
        id: FileId,
        mapper: &RangeMapper,
    ) -> Result<(), EcoString> {
        if self.len() > mapper.total_len() {
            // TODO: Should we error if not exactly equal?
            return Err(eco_format!(
                "text length ({}) is greater than mapper length ({})",
                self.len(),
                mapper.total_len(),
            ));
        }
        self.synthesize_with(
            0,
            &|offset, len| Span::from_range(id, mapper.map(offset..offset + len)),
            &|offset, sub_range| {
                if let Some(sr) = sub_range {
                    *sr = mapper.map_sub_range(offset, *sr);
                }
            },
        );
        Ok(())
    }

    /// Set a custom span for each node given its offset and length, and update
    /// any sub-ranges based on their offset.
    ///
    /// Should be called with `offset = 0` on the root node.
    fn synthesize_with(
        &mut self,
        mut offset: usize,
        map_span: &impl Fn(usize, usize) -> Span,
        update_sub_range: &impl Fn(usize, &mut Option<SubRange>),
    ) {
        let mut data = &mut self.data;
        loop {
            match data {
                Node::Leaf(leaf, _) => {
                    self.span = map_span(offset, leaf.len());
                    break;
                }
                Node::Inner(inner, _) => {
                    let inner = Arc::make_mut(inner);
                    self.span = map_span(offset, inner.len);
                    inner.upper = self.span.number();
                    for child in &mut inner.children {
                        child.synthesize_with(offset, map_span, update_sub_range);
                        offset += child.len();
                    }
                    break;
                }
                Node::Error(err, _) => {
                    let err = Arc::make_mut(err);
                    for (_hint, sub_range) in err.hints.make_mut() {
                        update_sub_range(offset, sub_range);
                    }
                    self.span = map_span(offset, err.text.len());
                    break;
                }
                Node::Warning(warn, _) => {
                    let warn = Arc::make_mut(warn);
                    update_sub_range(offset, &mut warn.sub_range);
                    for (_hint, sub_range) in warn.hints.make_mut() {
                        update_sub_range(offset, sub_range);
                    }
                    data = &mut warn.child;
                }
            }
        }
    }
}

impl SyntaxNode {
    /// Convert the child to another kind.
    ///
    /// Panics if trying to convert to or from an error.
    #[track_caller]
    pub(super) fn convert_to_kind(&mut self, new_kind: SyntaxKind) {
        if new_kind.is_error() {
            panic!("cannot convert to an error, use `convert_to_error` instead");
        } else if self.kind().is_error() {
            // `.kind()` checks both errors and warnings that wrap errors.
            panic!("cannot convert an error to a different kind");
        }
        // Must assign through warnings as well, since they duplicate the kind.
        let mut data = &mut self.data;
        loop {
            match data {
                Node::Leaf(_, kind) | Node::Inner(_, kind) => {
                    *kind = new_kind;
                    break;
                }
                Node::Error(_, _) => unreachable!(),
                Node::Warning(warn, kind) => {
                    *kind = new_kind;
                    data = &mut Arc::make_mut(warn).child;
                }
            }
        }
    }

    /// Convert the child to an error, if it isn't already one.
    pub(super) fn convert_to_error(&mut self, message: impl Into<EcoString>) {
        if !self.kind().is_error() {
            let text = std::mem::take(self).full_text();
            *self = SyntaxNode::error(message.into(), text);
        }
    }

    /// Convert the child to an error stating that the given thing was
    /// expected, but the current kind was found.
    pub(super) fn expected(&mut self, expected: &str) {
        let kind = self.kind();
        self.convert_to_error(eco_format!("expected {expected}, found {}", kind.name()));
        if kind.is_keyword() && matches!(expected, "identifier" | "pattern") {
            self.hint(eco_format!(
                "keyword `{text}` is not allowed as an identifier; try `{text}_` instead",
                text = self.leaf_text(),
            ));
        }
    }

    /// Convert the child to an error stating it was unexpected.
    pub(super) fn unexpected(&mut self) {
        self.convert_to_error(eco_format!("unexpected {}", self.kind().name()));
    }

    /// Traverse the tree in-order, calling `f` on each node and recursing on
    /// the returned nodes. Note that `f` can prune the traversal at any point
    /// by yielding `[].iter()` instead of the actual children slice of an inner
    /// node.
    fn traverse(&self, mut f: impl FnMut(&Self) -> std::slice::Iter<'_, Self>) {
        fn recursive_step(
            node: &SyntaxNode,
            f: &mut impl FnMut(&SyntaxNode) -> std::slice::Iter<'_, SyntaxNode>,
        ) {
            for child in f(node) {
                recursive_step(child, f);
            }
        }
        // We pass in `&mut impl FnMut` so our caller doesn't have to.
        recursive_step(self, &mut f);
    }

    /// The number of descendants, including the node itself.
    pub(super) fn descendants(&self) -> usize {
        match self.node_ref() {
            NodeRef::Leaf(_) | NodeRef::Error(_) => 1,
            NodeRef::Inner(inner) => inner.descendants,
        }
    }

    /// The node's children, mutably.
    pub(super) fn children_mut(&mut self) -> &mut [SyntaxNode] {
        if let Some((inner, _)) = self.inner_and_span_mut() {
            &mut inner.children
        } else {
            &mut []
        }
    }
}

impl Debug for SyntaxNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.data.fmt(f)
    }
}

impl Debug for Node {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Node::Leaf(text, kind) => write!(f, "{kind:?}: {text:?}"),
            Node::Inner(inner, kind) => inner.debug_fmt(f, *kind),
            Node::Error(err, _) => err.fmt(f),
            Node::Warning(warn, _) => warn.fmt(f),
        }
    }
}

impl Default for SyntaxNode {
    fn default() -> Self {
        Self::leaf(SyntaxKind::End, EcoString::new())
    }
}

/// An inner node in the untyped syntax tree.
#[derive(Clone, Eq, PartialEq, Hash)]
struct InnerNode {
    /// The byte length of the node in the source.
    len: usize,
    /// The number of nodes in the whole subtree, including this node.
    descendants: usize,
    /// Whether this node or any of its children contain an error/warning
    /// diagnostic.
    diagnosis: Diagnosis,
    /// The upper bound of this node's numbering range.
    upper: u64,
    /// This node's children, losslessly make up this node.
    children: Vec<SyntaxNode>,
}

impl InnerNode {
    /// Create a new inner node with the given children.
    fn new(children: Vec<SyntaxNode>) -> Self {
        let mut len = 0;
        let mut descendants = 1;
        let mut diagnosis = Diagnosis::default();

        for child in &children {
            len += child.len();
            descendants += child.descendants();
            diagnosis = diagnosis.or(child.diagnosis());
        }

        Self { len, descendants, diagnosis, upper: 0, children }
    }

    /// Format the inner node with its `SyntaxKind` for debugging.
    fn debug_fmt(&self, f: &mut Formatter, kind: SyntaxKind) -> fmt::Result {
        write!(f, "{kind:?}: {}", self.len)?;
        if !self.children.is_empty() {
            f.write_str(" ")?;
            f.debug_list().entries(&self.children).finish()?;
        }
        Ok(())
    }
}

/// Whether a node has diagnostic errors and/or warnings in it or its children.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
pub struct Diagnosis {
    pub errors: bool,
    pub warnings: bool,
}

impl Diagnosis {
    /// Whether there were errors or warnings.
    pub fn either(self) -> bool {
        self.errors | self.warnings
    }

    /// Whether there were both errors and warnings.
    pub fn both(self) -> bool {
        self.errors & self.warnings
    }

    /// Apply the `OR` of both fields separately.
    pub fn or(mut self, other: Self) -> Self {
        self.errors |= other.errors;
        self.warnings |= other.warnings;
        self
    }

    /// Whether any node in the given slice has errors or warnings.
    fn any(slice: &[SyntaxNode]) -> Self {
        slice
            .iter()
            .map(SyntaxNode::diagnosis)
            .fold(Self::default(), Self::or)
    }
}

/// A syntactical error or warning. This is mainly used by converting it to a
/// `SourceDiagnostic` during evaluation.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct SyntaxDiagnostic {
    /// `true` if the diagnostic is an error, `false` if it's a warning.
    pub is_error: bool,
    /// The span targeted by the diagnostic.
    pub span: DiagSpan,
    /// The main diagnostic message.
    pub message: EcoString,
    /// Additional hints to the user indicating how this issue could be avoided
    /// or worked around.
    pub hints: EcoVec<Spanned<EcoString, DiagSpan>>,
}

/// An error node in the untyped syntax tree.
#[derive(Clone, Eq, PartialEq, Hash)]
struct ErrorNode {
    /// The source text of the node.
    text: EcoString,
    /// The error message.
    message: EcoString,
    /// Additional hints to the user indicating how this error could be avoided
    /// or worked around.
    hints: EcoVec<(EcoString, Option<SubRange>)>,
}

impl ErrorNode {
    /// Create a new error node.
    fn new(message: EcoString, text: EcoString) -> Self {
        Self { text, message, hints: eco_vec![] }
    }

    /// Produce the syntax diagnostic for an error.
    fn diagnostic(&self, span: Span) -> SyntaxDiagnostic {
        SyntaxDiagnostic {
            is_error: true,
            span: span.into(),
            message: self.message.clone(),
            hints: build_diagnostic_hints(span, &self.hints),
        }
    }
}

impl Debug for ErrorNode {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        if self.text.is_empty() && self.hints.is_empty() {
            write!(f, "Error: {:?}", self.message)
        } else {
            let mut out = f.debug_struct("Error:");
            out.field("text", &self.text);
            out.field("message", &self.message);
            for (hint, sub_range) in &self.hints {
                let field = if let Some(sub_range) = sub_range {
                    let selected = &self.text[sub_range.to_relative()];
                    &format!("hint @({selected:?})")
                } else {
                    "hint"
                };
                out.field(field, hint);
            }
            out.finish()
        }
    }
}

/// A warning message wrapped around a node in the tree.
///
/// Warnings transparently wrap another node and do not have spans or text of
/// their own. This means their child cannot be directly found or mutated, only
/// affected _through_ the warning, usually via the [`SyntaxNode::node_ref`] and
/// [`SyntaxNode::inner_and_span_mut`] methods.
#[derive(Clone, Eq, PartialEq, Hash)]
struct WarningWrapper {
    /// The wrapped node data.
    child: Node,
    /// A relative sub-range for targeting text not grouped by an existing span.
    ///
    /// Warnings may need to target a range of text that isn't actually grouped
    /// by the syntax tree, this sub-range can select that text.
    sub_range: Option<SubRange>,
    /// The warning message.
    message: EcoString,
    /// Additional hints to the user indicating how this warning could be
    /// avoided or worked around.
    hints: EcoVec<(EcoString, Option<SubRange>)>,
}

impl WarningWrapper {
    /// Wrap an existing syntax node in a warning node.
    fn new(child: Node, sub_range: Option<SubRange>, message: EcoString) -> Self {
        Self { child, sub_range, message, hints: eco_vec![] }
    }

    /// Produce the syntax diagnostic for a warning.
    fn diagnostic(&self, span: Span) -> SyntaxDiagnostic {
        SyntaxDiagnostic {
            is_error: false,
            span: DiagSpan::from_span(span, self.sub_range),
            message: self.message.clone(),
            hints: build_diagnostic_hints(span, &self.hints),
        }
    }
}

impl Debug for WarningWrapper {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let full_text = LazyCell::new(|| {
            let data = self.child.clone();
            let temp_node = SyntaxNode { data, span: Span::detached() };
            temp_node.full_text()
        });
        let debug_field = |field, message, sub_range: Option<SubRange>| {
            // Inner closure has `move`, so need to explicitly capture by ref.
            let full_text = &full_text;
            debug(move |f| {
                if let Some(sr) = sub_range {
                    let selected = &full_text[sr.to_relative()];
                    write!(f, "{field} @({selected:?}): {message:?}")
                } else {
                    write!(f, "{field}: {message:?}")
                }
            })
        };

        write!(f, "Warning: ")?;
        // Use `debug_set` instead of `debug_struct` so we don't have to add a
        // field name when outputting the child.
        let mut out = f.debug_set();
        out.entry(&debug_field("message", &self.message, self.sub_range));
        for (hint, sub_range) in &self.hints {
            out.entry(&debug_field("hint", hint, *sub_range));
        }
        out.entry(&self.child);
        out.finish()
    }
}

/// Map a vector of hints with optional sub-ranges to one with optional
/// diagnostic spans derived from a parent span.
fn build_diagnostic_hints(
    parent_span: Span,
    hints: &EcoVec<(EcoString, Option<SubRange>)>,
) -> EcoVec<Spanned<EcoString, DiagSpan>> {
    hints
        .iter()
        .map(|(message, sub_range)| {
            let msg = message.clone();
            match *sub_range {
                Some(sr) => Spanned::new(msg, DiagSpan::from_span(parent_span, Some(sr))),
                None => Spanned::detached(msg),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test the debug output of a `SyntaxNode`.
    #[test]
    fn test_debug() {
        // A standard syntax tree:
        assert_eq!(
            format!("{:#?}", crate::typst_syntax::parse("= Head <label>")),
            "\
Markup: 14 [
    Heading: 6 [
        HeadingMarker: \"=\",
        Space: \" \",
        Markup: 4 [
            Text: \"Head\",
        ],
    ],
    Space: \" \",
    Label: \"<label>\",
]"
        );
        // A basic syntax error:
        assert_eq!(
            format!("{:#?}", crate::typst_syntax::parse("#")),
            "\
Markup: 1 [
    Hash: \"#\",
    Error: \"expected expression\",
]"
        );
        // A syntax error with multiple hints:
        assert_eq!(
            format!("{:#?}", crate::typst_syntax::parse("##")),
            "\
Markup: 2 [
    Hash: \"#\",
    Error: {
        text: \"#\",
        message: \"the character `#` is not valid in code\",
        hint: \"the preceding hash is causing this to parse in code mode\",
        hint: \"try escaping the preceding hash: `\\\\#`\",
    },
]"
        );
        // A warning with a hint:
        assert_eq!(
            format!("{:#?}", crate::typst_syntax::parse("**")),
            "\
Markup: 2 [
    Warning: {
        message: \"no text within stars\",
        hint: \"using multiple consecutive stars (e.g. **) has no additional effect\",
        Strong: 2 [
            Star: \"*\",
            Markup: 0,
            Star: \"*\",
        ],
    },
]"
        );
    }

    #[test]
    fn test_debug_sub_range() {
        // An example warning for text at a sub-range:
        let mut root = crate::typst_syntax::parse("= =head");
        let heading_body = &mut root.children_mut()[0];
        heading_body.warn_at(0..3, "equal space equal!");
        heading_body.hint("try equal equal space?");
        assert_eq!(
            format!("{root:#?}"),
            "\
Markup: 7 [
    Warning: {
        message @(\"= =\"): \"equal space equal!\",
        hint: \"try equal equal space?\",
        Heading: 7 [
            HeadingMarker: \"=\",
            Space: \" \",
            Markup: 5 [
                Text: \"=head\",
            ],
        ],
    },
]"
        );

        // An example for hints at sub-ranges:
        let mut root = crate::typst_syntax::parse("<unclosed");
        let node = &mut root.children_mut()[0];
        // Hint on the "unclosed label" error:
        node.hint_at(0..1, "greater");
        node.hint_at(3..8, "open!");
        // Adding a warning with hints around the error:
        node.warn_at(3..9, "opened?");
        node.hint_at(0..9, "full text"); // no special treatment
        assert_eq!(
            format!("{root:#?}"),
            "\
Markup: 9 [
    Warning: {
        message @(\"closed\"): \"opened?\",
        hint @(\"<unclosed\"): \"full text\",
        Error: {
            text: \"<unclosed\",
            message: \"unclosed label\",
            hint @(\"<\"): \"greater\",
            hint @(\"close\"): \"open!\",
        },
    },
]"
        );
    }
}
