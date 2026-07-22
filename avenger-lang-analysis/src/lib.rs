//! Editor-neutral language intelligence contracts for Avenger.
//!
//! This crate deliberately uses byte offsets and Avenger source identities.
//! Protocol adapters such as `avenger-lsp` are responsible for translating
//! these contracts to editor-specific positions and wire types.

mod intelligence;

pub use intelligence::{
    AnalysisQueryError, CompletionOptions, DocumentSemanticIndex, IndexedBinding, IndexedReference,
    IndexedSymbol, IndexedValueKind, WorkspaceSemanticIndex,
};

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use avenger_lang_compiler::{
    CompileFailure, Compiler, DatasetStageId, DatasetStageKind, ProjectAnalysis,
};
use avenger_lang_core::{
    ContentVersion, Diagnostic, ImportCapabilities, LineIndex, LoadedSource, ProjectRoot,
    SourceFile, SourceId, SourceLoader, SourceLoaderError, SourceOrigin, SourceSpan,
    project::normalize_path,
    syntax::{
        TolerantParsedFile, TolerantSyntaxNode, TolerantSyntaxNodeId, TolerantSyntaxNodeKind,
    },
};
use futures::{
    future::{Either, select},
    pin_mut,
    task::AtomicWaker,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    pub origin: SourceOrigin,
    pub revision: SourceRevision,
    pub text: Arc<str>,
    pub line_index: LineIndex,
}

#[derive(Clone)]
pub struct WorkspaceSnapshot {
    pub generation: AnalysisGeneration,
    pub project_root: std::path::PathBuf,
    pub roots: Vec<ProjectRoot>,
    pub open_documents: BTreeMap<SourceOrigin, DocumentSnapshot>,
    pub known_disk_sources: Vec<SourceOrigin>,
    pub native_registry_profile: String,
}

#[derive(Clone, Debug)]
pub struct RootAnalysis {
    pub root: ProjectRoot,
    pub result: Result<ProjectAnalysis, CompileFailure>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceAnalysis {
    pub generation: AnalysisGeneration,
    pub project_root: std::path::PathBuf,
    pub known_sources: Vec<SourceOrigin>,
    pub syntax: BTreeMap<SourceOrigin, SyntaxAnalysis>,
    pub semantic_roots: BTreeMap<String, RootAnalysis>,
    pub dataset_contexts: BTreeMap<SourceOrigin, Vec<DatasetContext>>,
    pub registry: avenger_chart_schema::NativeSchemaSnapshot,
    pub semantic_index: WorkspaceSemanticIndex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetContext {
    pub root_uri: String,
    pub stage: DatasetStageId,
    pub stage_kind: DatasetStageKind,
    pub span: SourceSpan,
}

impl WorkspaceAnalysis {
    /// Build an immediate tolerant, syntax-only analysis while slower project
    /// semantics are still debouncing. The same query APIs and revision guards
    /// apply; semantic candidates simply become available in a later snapshot.
    pub fn syntax_only(
        generation: AnalysisGeneration,
        project_root: std::path::PathBuf,
        known_sources: Vec<SourceOrigin>,
        syntax: BTreeMap<SourceOrigin, SyntaxAnalysis>,
        registry: avenger_chart_schema::NativeSchemaSnapshot,
    ) -> Self {
        let semantic_roots = BTreeMap::new();
        let semantic_index = WorkspaceSemanticIndex::build(&syntax, &semantic_roots);
        Self {
            generation,
            project_root,
            known_sources,
            syntax,
            semantic_roots,
            dataset_contexts: BTreeMap::new(),
            registry,
            semantic_index,
        }
    }

    /// Overlay the latest tolerant document analyses on the last published
    /// semantic snapshot. This is the explicit last-good fallback used during
    /// ordinary incomplete edits.
    pub fn with_syntax(
        &self,
        generation: AnalysisGeneration,
        syntax: BTreeMap<SourceOrigin, SyntaxAnalysis>,
    ) -> Self {
        let semantic_index = WorkspaceSemanticIndex::build(&syntax, &self.semantic_roots);
        Self {
            generation,
            project_root: self.project_root.clone(),
            known_sources: self.known_sources.clone(),
            syntax,
            semantic_roots: self.semantic_roots.clone(),
            dataset_contexts: self.dataset_contexts.clone(),
            registry: self.registry.clone(),
            semantic_index,
        }
    }

    /// Retain successful semantic indexes from the preceding snapshot for
    /// roots that are currently malformed, while keeping the current failures
    /// available for diagnostics.
    pub fn with_last_good_semantics(&self, previous: &Self) -> Self {
        let mut index_roots = self.semantic_roots.clone();
        for (root, prior) in &previous.semantic_roots {
            let current_failed = index_roots
                .get(root)
                .is_some_and(|current| current.result.is_err());
            if current_failed && prior.result.is_ok() {
                index_roots.insert(root.clone(), prior.clone());
            }
        }
        let semantic_index = WorkspaceSemanticIndex::build(&self.syntax, &index_roots);
        let mut output = self.clone();
        output.semantic_index = semantic_index;
        output
    }

    pub fn dataset_context_at(
        &self,
        origin: &SourceOrigin,
        byte_offset: usize,
    ) -> Option<&DatasetContext> {
        self.dataset_contexts.get(origin)?.iter().find(|context| {
            context.span.range.start <= byte_offset && byte_offset <= context.span.range.end
        })
    }

    pub fn complete(
        &self,
        request: &PositionRequest,
        options: CompletionOptions,
        cancellation: &AnalysisCancellation,
    ) -> Result<CompletionResult, AnalysisQueryError> {
        intelligence::QueryContext::new(
            self.generation,
            &self.project_root,
            &self.known_sources,
            &self.registry,
            &self.syntax,
            &self.semantic_index,
        )
        .complete(request, options, cancellation)
    }

    pub fn hover(
        &self,
        request: &PositionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<Option<HoverResult>, AnalysisQueryError> {
        intelligence::QueryContext::new(
            self.generation,
            &self.project_root,
            &self.known_sources,
            &self.registry,
            &self.syntax,
            &self.semantic_index,
        )
        .hover(request, cancellation)
    }

    pub fn definition(
        &self,
        request: &PositionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<NavigationResult, AnalysisQueryError> {
        intelligence::QueryContext::new(
            self.generation,
            &self.project_root,
            &self.known_sources,
            &self.registry,
            &self.syntax,
            &self.semantic_index,
        )
        .definition(request, cancellation)
    }

    pub fn references(
        &self,
        request: &PositionRequest,
        include_declaration: bool,
        cancellation: &AnalysisCancellation,
    ) -> Result<NavigationResult, AnalysisQueryError> {
        intelligence::QueryContext::new(
            self.generation,
            &self.project_root,
            &self.known_sources,
            &self.registry,
            &self.syntax,
            &self.semantic_index,
        )
        .references(request, include_declaration, cancellation)
    }
}

#[derive(Clone)]
pub struct AnalysisService {
    compiler: Compiler,
}

impl AnalysisService {
    pub fn new(compiler: Compiler) -> Self {
        Self { compiler }
    }

    pub fn compiler(&self) -> &Compiler {
        &self.compiler
    }

    pub async fn analyze_workspace(
        &self,
        snapshot: WorkspaceSnapshot,
        cancellation: &AnalysisCancellation,
    ) -> Result<WorkspaceAnalysis, AnalysisCancelled> {
        cancellation.check()?;
        let syntax = snapshot
            .open_documents
            .iter()
            .map(|(origin, document)| (origin.clone(), analyze_syntax(document)))
            .collect();
        cancellation.check()?;
        let registry = self.compiler.language_host().authoring_schema().clone();

        let loader = snapshot.source_loader(self.compiler.options().source_loader.clone());
        let compiler = self.compiler.fork_with_source_loader(Arc::new(loader));
        let data_roots = snapshot
            .roots
            .iter()
            .filter(|root| root.role == avenger_lang_core::ProjectDependencyRole::DataConfiguration)
            .cloned()
            .collect::<Vec<_>>();
        let chart_roots = snapshot
            .roots
            .iter()
            .filter(|root| root.role == avenger_lang_core::ProjectDependencyRole::RootChart)
            .cloned()
            .collect::<Vec<_>>();
        let mut semantic_roots = BTreeMap::new();
        let mut dataset_contexts = BTreeMap::<SourceOrigin, Vec<DatasetContext>>::new();
        for root in chart_roots {
            cancellation.check()?;
            let mut closure_roots = vec![root.clone()];
            closure_roots.extend(data_roots.iter().cloned());
            let analysis = compiler.analyze_project_roots(closure_roots, snapshot.generation.get());
            pin_mut!(analysis);
            let cancelled = cancellation.cancelled();
            pin_mut!(cancelled);
            let result = match select(analysis, cancelled).await {
                Either::Left((result, _)) => result,
                Either::Right(((), _)) => return Err(AnalysisCancelled),
            };
            let root_uri = root.origin.canonical_uri();
            if let Ok(project) = &result {
                for (stage, dataset) in project.datasets.iter() {
                    if let Some(source) = project.sources.get(dataset.provenance.stage_span.source)
                    {
                        dataset_contexts
                            .entry(source.origin.clone())
                            .or_default()
                            .push(DatasetContext {
                                root_uri: root_uri.clone(),
                                stage: stage.clone(),
                                stage_kind: dataset.provenance.stage_kind.clone(),
                                span: dataset.provenance.stage_span,
                            });
                    }
                }
            }
            semantic_roots.insert(root_uri, RootAnalysis { root, result });
        }
        for contexts in dataset_contexts.values_mut() {
            contexts.sort_by_key(|context| {
                (
                    context.span.range.len(),
                    context.span.range.start,
                    context.stage.clone(),
                )
            });
        }
        cancellation.check()?;
        let semantic_index = WorkspaceSemanticIndex::build(&syntax, &semantic_roots);
        Ok(WorkspaceAnalysis {
            generation: snapshot.generation,
            project_root: snapshot.project_root,
            known_sources: snapshot.known_disk_sources,
            syntax,
            semantic_roots,
            dataset_contexts,
            registry,
            semantic_index,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct AnalysisCancellation {
    inner: Arc<AnalysisCancellationInner>,
}

#[derive(Debug, Default)]
struct AnalysisCancellationInner {
    cancelled: AtomicBool,
    waker: AtomicWaker,
}

impl AnalysisCancellation {
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.waker.wake();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), AnalysisCancelled> {
        if self.is_cancelled() {
            Err(AnalysisCancelled)
        } else {
            Ok(())
        }
    }

    async fn cancelled(&self) {
        futures::future::poll_fn(|context| {
            if self.is_cancelled() {
                return std::task::Poll::Ready(());
            }
            self.inner.waker.register(context.waker());
            if self.is_cancelled() {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
        .await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("analysis cancelled")]
pub struct AnalysisCancelled;

impl WorkspaceSnapshot {
    pub fn source_loader(&self, fallback: Arc<dyn SourceLoader>) -> SnapshotSourceLoader {
        SnapshotSourceLoader::new(self.open_documents.clone(), fallback)
    }
}

/// Immutable open-buffer overlay for one analysis generation.
#[derive(Clone)]
pub struct SnapshotSourceLoader {
    overlay: Arc<BTreeMap<SourceOrigin, DocumentSnapshot>>,
    fallback: Arc<dyn SourceLoader>,
}

impl SnapshotSourceLoader {
    pub fn new(
        overlay: BTreeMap<SourceOrigin, DocumentSnapshot>,
        fallback: Arc<dyn SourceLoader>,
    ) -> Self {
        Self {
            overlay: Arc::new(overlay),
            fallback,
        }
    }

    pub fn overlay(&self) -> &BTreeMap<SourceOrigin, DocumentSnapshot> {
        &self.overlay
    }
}

#[async_trait]
impl SourceLoader for SnapshotSourceLoader {
    async fn load(
        &self,
        origin: &SourceOrigin,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        if let Some(document) = self.overlay.get(origin) {
            if !origin_allowed(origin, capabilities) {
                return Err(SourceLoaderError::CapabilityDenied(origin.clone()));
            }
            return Ok(LoadedSource::new(
                document.origin.clone(),
                document.text.clone(),
                ContentVersion::new(document.revision.as_str()),
            ));
        }
        self.fallback.load(origin, capabilities).await
    }
}

fn origin_allowed(origin: &SourceOrigin, capabilities: &ImportCapabilities) -> bool {
    match origin {
        SourceOrigin::Memory(_) => capabilities.allow_memory,
        SourceOrigin::File(path) => {
            capabilities.allow_filesystem
                && normalize_path(path).starts_with(normalize_path(&capabilities.project_root))
        }
        SourceOrigin::Std(_) => capabilities.allow_std,
        SourceOrigin::Http(_) => capabilities.allow_http,
    }
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

    pub fn from_text(text: &str) -> Self {
        let digest = Sha256::digest(text.as_bytes());
        Self::new(format!("sha256:{digest:x}"))
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

    use super::SourceRevision;

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
        let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
            .expect("read analysis source");
        let session_context = ["Session", "Context"].concat();
        assert!(
            !source.contains(&session_context),
            "workspace analysis must not retain a mutable DataFusion session"
        );
    }

    #[test]
    fn source_revisions_are_content_identities() {
        assert_eq!(
            SourceRevision::from_text("same"),
            SourceRevision::from_text("same")
        );
        assert_ne!(
            SourceRevision::from_text("before"),
            SourceRevision::from_text("after")
        );
    }
}
