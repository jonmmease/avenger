//! The source file that a label's spans point into.

use std::ops::Range;
use std::sync::LazyLock;

use typst_syntax::{
    FileId, RangeMapper, RootedPath, Span, SyntaxKind, SyntaxNode, VirtualPath,
    VirtualRoot,
};

/// The file id of every label. Upstream interns file ids into a global table, which a span's
/// debug output looks up; a label is its own file, so one id, interned once, serves them all.
pub(crate) fn label_file() -> FileId {
    static ID: LazyLock<FileId> = LazyLock::new(|| {
        let path = VirtualPath::new("/label.typ").expect("the label's path is valid");
        RootedPath::new(VirtualRoot::Project, path).intern()
    });
    *ID
}

/// A span covering a byte range of a label's source. Upstream makes range spans only by
/// mapping syntax nodes onto the source, as `parse_label` does, so this maps a leaf as long as
/// the range.
pub(crate) fn label_span(range: Range<usize>) -> Span {
    let mut leaf = SyntaxNode::leaf(SyntaxKind::Text, " ".repeat(range.len()));
    let mapper =
        RangeMapper::new(std::iter::once(range)).expect("one range is a valid mapping");
    leaf.synthesize_mapped(label_file(), &mapper)
        .expect("the range is as long as the leaf");
    leaf.span()
}
