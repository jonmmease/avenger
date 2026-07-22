//! Editor-neutral language intelligence contracts for Avenger.
//!
//! This crate deliberately uses byte offsets and Avenger source identities.
//! Protocol adapters such as `avenger-lsp` are responsible for translating
//! these contracts to editor-specific positions and wire types.

use std::{collections::BTreeMap, sync::Arc};

use avenger_lang_core::{SourceOrigin, SourceSpan};

/// Monotonic identity for one immutable workspace-analysis generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnalysisGeneration(u64);

impl AnalysisGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Immutable content identity for one source revision.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRevision(Arc<str>);

impl SourceRevision {
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Common source-position input for editor intelligence requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionRequest {
    pub source: SourceOrigin,
    pub byte_offset: usize,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompletionKind {
    Keyword,
    Declaration,
    Property,
    EnumValue,
    Variable,
    Field,
    Function,
    Type,
    Module,
    Catalog,
    Schema,
    Table,
    Snippet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionTextFormat {
    PlainText,
    Snippet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompletionOrigin {
    Syntax,
    AuthoringSchema,
    LexicalScope,
    QueryScope,
    DatasetSchema,
    Catalog,
    FunctionRegistry,
    Recovery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub replacement: SourceSpan,
    pub insert_text: String,
    pub insert_text_format: CompletionTextFormat,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub filter_text: Option<String>,
    pub sort_key: String,
    pub origin: CompletionOrigin,
    pub deprecated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionResult {
    pub items: Vec<CompletionItem>,
    pub is_incomplete: bool,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverResult {
    pub span: SourceSpan,
    pub markdown: String,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SymbolKind {
    Chart,
    Definition,
    Catalog,
    Schema,
    Table,
    Group,
    Mark,
    Transform,
    Param,
    Store,
    Selection,
    Tool,
    Widget,
    View,
    Event,
    Field,
    Property,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: SymbolKind,
    pub span: SourceSpan,
    pub selection_span: SourceSpan,
    pub children: Vec<DocumentSymbol>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationTarget {
    pub origin: SourceOrigin,
    pub span: SourceSpan,
    pub selection_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationResult {
    pub targets: Vec<NavigationTarget>,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticTokenKind {
    Binding,
    Parameter,
    Variable,
    Property,
    Field,
    Function,
    Type,
    Namespace,
    UnresolvedReference,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SemanticTokenModifiers {
    pub declaration: bool,
    pub readonly: bool,
    pub deprecated: bool,
    pub default_library: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticToken {
    pub span: SourceSpan,
    pub kind: SemanticTokenKind,
    pub modifiers: SemanticTokenModifiers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceTextEdit {
    pub span: SourceSpan,
    pub new_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedSourceEdits {
    pub source_revision: SourceRevision,
    pub edits: Vec<SourceTextEdit>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceEdit {
    pub sources: BTreeMap<SourceOrigin, VersionedSourceEdits>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CodeActionKind {
    QuickFix,
    RefactorInline,
    RefactorExtract,
    Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeAction {
    pub title: String,
    pub kind: CodeActionKind,
    pub diagnostic_codes: Vec<String>,
    pub preferred: bool,
    pub edit: WorkspaceEdit,
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn editor_neutral_crate_has_no_lsp_or_zed_dependency() {
        let manifest = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read analysis manifest");
        for forbidden in ["tower-lsp", "ls-types", "zed_extension_api"] {
            assert!(
                !manifest.contains(forbidden),
                "editor-neutral analysis crate must not depend on {forbidden}"
            );
        }
    }
}
