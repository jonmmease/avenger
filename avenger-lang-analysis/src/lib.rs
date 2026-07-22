//! Editor-neutral language intelligence contracts for Avenger.
//!
//! This crate deliberately uses byte offsets and Avenger source identities.
//! Protocol adapters such as `avenger-lsp` are responsible for translating
//! these contracts to editor-specific positions and wire types.

use std::{collections::BTreeMap, sync::Arc};

use avenger_lang_core::{
    Diagnostic, LineIndex, SourceFile, SourceId, SourceOrigin, SourceSpan,
    syntax::{
        TolerantParsedFile, TolerantSyntaxNode, TolerantSyntaxNodeId, TolerantSyntaxNodeKind,
    },
};

#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    pub origin: SourceOrigin,
    pub revision: SourceRevision,
    pub text: Arc<str>,
    pub line_index: LineIndex,
}

impl DocumentSnapshot {
    pub fn new(origin: SourceOrigin, revision: SourceRevision, text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        Self {
            origin,
            revision,
            line_index: LineIndex::new(text.clone()),
            text,
        }
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxContextKind {
    Root,
    Declaration,
    Property,
    Query,
    Expression,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxContext {
    pub kind: SyntaxContextKind,
    pub span: SourceSpan,
    pub declaration_keyword: Option<String>,
    pub property_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SyntaxAnalysis {
    pub revision: SourceRevision,
    pub parsed: TolerantParsedFile,
    pub diagnostics: Vec<Diagnostic>,
    pub symbols: Vec<DocumentSymbol>,
}

impl SyntaxAnalysis {
    pub fn context_at(&self, byte_offset: usize) -> SyntaxContext {
        let node = self.parsed.enclosing_node(byte_offset);
        match node.map(|node| &node.kind) {
            Some(TolerantSyntaxNodeKind::Declaration { keyword, .. }) => SyntaxContext {
                kind: SyntaxContextKind::Declaration,
                span: node.unwrap().span,
                declaration_keyword: Some(keyword.clone()),
                property_name: None,
            },
            Some(TolerantSyntaxNodeKind::Property { name }) => SyntaxContext {
                kind: SyntaxContextKind::Property,
                span: node.unwrap().span,
                declaration_keyword: None,
                property_name: Some(name.clone()),
            },
            Some(TolerantSyntaxNodeKind::SqlIsland { context }) => SyntaxContext {
                kind: if matches!(
                    context,
                    avenger_lang_core::syntax::SqlIslandContext::QueryProperty
                ) {
                    SyntaxContextKind::Query
                } else {
                    SyntaxContextKind::Expression
                },
                span: node.unwrap().span,
                declaration_keyword: None,
                property_name: None,
            },
            Some(TolerantSyntaxNodeKind::Error | TolerantSyntaxNodeKind::MissingToken { .. }) => {
                SyntaxContext {
                    kind: SyntaxContextKind::Error,
                    span: node.unwrap().span,
                    declaration_keyword: None,
                    property_name: None,
                }
            }
            _ => SyntaxContext {
                kind: SyntaxContextKind::Root,
                span: self.parsed.nodes[0].span,
                declaration_keyword: None,
                property_name: None,
            },
        }
    }
}

pub fn analyze_syntax(snapshot: &DocumentSnapshot) -> SyntaxAnalysis {
    let source = SourceFile::new(
        SourceId::new(0),
        snapshot.origin.clone(),
        snapshot.text.clone(),
    );
    let parsed = avenger_lang_core::syntax::parse_file_tolerant(&source);
    let diagnostics = parsed.diagnostics.clone();
    let symbols = declaration_symbols(&parsed.nodes, None);
    SyntaxAnalysis {
        revision: snapshot.revision.clone(),
        parsed,
        diagnostics,
        symbols,
    }
}

fn declaration_symbols(
    nodes: &[TolerantSyntaxNode],
    parent: Option<TolerantSyntaxNodeId>,
) -> Vec<DocumentSymbol> {
    nodes
        .iter()
        .filter_map(|node| {
            let TolerantSyntaxNodeKind::Declaration { keyword, name } = &node.kind else {
                return None;
            };
            if normalized_parent(nodes, node.parent) != parent {
                return None;
            }
            Some(DocumentSymbol {
                name: name.clone().unwrap_or_else(|| keyword.clone()),
                detail: Some(keyword.clone()),
                kind: symbol_kind(keyword),
                span: node.span,
                selection_span: node.span,
                children: declaration_symbols(nodes, Some(node.id)),
            })
        })
        .collect()
}

fn normalized_parent(
    nodes: &[TolerantSyntaxNode],
    mut parent: Option<TolerantSyntaxNodeId>,
) -> Option<TolerantSyntaxNodeId> {
    while let Some(id) = parent {
        let node = &nodes[id.get() as usize];
        if matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. }) {
            return Some(id);
        }
        parent = node.parent;
    }
    None
}

fn symbol_kind(keyword: &str) -> SymbolKind {
    match keyword.to_ascii_lowercase().as_str() {
        "chart" => SymbolKind::Chart,
        "define" => SymbolKind::Definition,
        "catalog" => SymbolKind::Catalog,
        "schema" => SymbolKind::Schema,
        "table" => SymbolKind::Table,
        "group" => SymbolKind::Group,
        "mark" => SymbolKind::Mark,
        "transform" => SymbolKind::Transform,
        "param" => SymbolKind::Param,
        "store" => SymbolKind::Store,
        "selection" => SymbolKind::Selection,
        "tool" => SymbolKind::Tool,
        "widget" => SymbolKind::Widget,
        "event" => SymbolKind::Event,
        _ => SymbolKind::Definition,
    }
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
