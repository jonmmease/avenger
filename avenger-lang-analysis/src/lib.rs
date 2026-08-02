//! Editor-neutral language intelligence contracts for Avenger.
//!
//! This crate deliberately uses byte offsets and Avenger source identities.
//! Protocol adapters such as `avenger-lsp` are responsible for translating
//! these contracts to editor-specific positions and wire types.

mod editing;
mod intelligence;
mod sql_intelligence;

pub use editing::RenameError;
pub use intelligence::{
    AnalysisQueryError, CompletionOptions, DocumentSemanticIndex, IndexedBinding, IndexedReference,
    IndexedSymbol, IndexedValueKind, WorkspaceSemanticIndex,
};
pub use sql_intelligence::{
    SqlCompletionDebug, SqlCompletionMetrics, SqlExpectedRole, SqlRepairStrategy,
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
    CompileFailure, Compiler, DatasetStageId, DatasetStageKind, ModuleAnalysis,
};
use avenger_lang_core::{
    ContentVersion, Diagnostic, ImportCapabilities, LineIndex, LoadedSource, ModuleRoot,
    SourceFile, SourceId, SourceLoader, SourceLoaderError, SourceOrigin, SourceSpan,
    module_graph::normalize_path,
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
    pub roots: Vec<ModuleRoot>,
    pub open_documents: BTreeMap<SourceOrigin, DocumentSnapshot>,
    pub known_disk_sources: Vec<SourceOrigin>,
    pub native_registry_profile: String,
}

#[derive(Clone, Debug)]
pub struct RootAnalysis {
    pub root: ModuleRoot,
    pub result: Result<ModuleAnalysis, CompileFailure>,
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
            &self.semantic_roots,
            &self.dataset_contexts,
        )
        .complete(request, options, cancellation)
    }

    pub fn format_document(
        &self,
        request: &DocumentRequest,
        line_ending: LineEnding,
        cancellation: &AnalysisCancellation,
    ) -> Result<Option<FormattingResult>, AnalysisQueryError> {
        editing::format_document(self, request, line_ending, cancellation)
    }

    pub fn semantic_tokens(
        &self,
        request: &DocumentRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<SemanticTokensResult, AnalysisQueryError> {
        editing::semantic_tokens(self, request, cancellation)
    }

    pub fn prepare_rename(
        &self,
        request: &PositionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<Option<PrepareRenameResult>, AnalysisQueryError> {
        editing::prepare_rename(self, request, cancellation)
    }

    pub fn rename(
        &self,
        request: &PositionRequest,
        new_name: &str,
        cancellation: &AnalysisCancellation,
    ) -> Result<WorkspaceEdit, RenameError> {
        editing::rename(self, request, new_name, cancellation)
    }

    pub fn code_actions(
        &self,
        request: &CodeActionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<Vec<CodeAction>, AnalysisQueryError> {
        editing::code_actions(self, request, cancellation)
    }

    pub fn pin_import_target(
        &self,
        request: &CodeActionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<Option<PinImportTarget>, AnalysisQueryError> {
        editing::pin_import_target(self, request, cancellation)
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
            &self.semantic_roots,
            &self.dataset_contexts,
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
            &self.semantic_roots,
            &self.dataset_contexts,
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
            &self.semantic_roots,
            &self.dataset_contexts,
        )
        .references(request, include_declaration, cancellation)
    }

    pub fn chart_runnables(
        &self,
        request: &DocumentRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<ChartRunnablesResult, AnalysisQueryError> {
        cancellation
            .check()
            .map_err(|_| AnalysisQueryError::Cancelled)?;
        let syntax = self
            .syntax
            .get(&request.source)
            .ok_or(AnalysisQueryError::UnknownSource)?;
        if syntax.revision != request.source_revision {
            return Err(AnalysisQueryError::StaleRevision);
        }
        let chart_items = syntax
            .parsed
            .module_syntax
            .items
            .iter()
            .filter(|item| {
                item.keyword
                    .as_ref()
                    .is_some_and(|keyword| keyword.text.eq_ignore_ascii_case("chart"))
            })
            .collect::<Vec<_>>();
        let singleton = chart_items.len() == 1;
        let runnables = chart_items
            .into_iter()
            .filter_map(|item| {
                let selector = item
                    .chart_name
                    .as_ref()
                    .or(item.name.as_ref())
                    .map(|name| name.text.clone());
                if selector.is_none() && !singleton {
                    return None;
                }
                let selection_span = item.chart_name.as_ref().or(item.name.as_ref()).map_or_else(
                    || {
                        item.keyword
                            .as_ref()
                            .map_or(item.declaration_span, |keyword| keyword.span)
                    },
                    |name| name.span,
                );
                Some(ChartRunnable {
                    label: selector.as_ref().map_or_else(
                        || "Run chart".to_owned(),
                        |name| format!("Run chart {name}"),
                    ),
                    selector,
                    span: item.declaration_span,
                    selection_span,
                })
            })
            .collect();
        Ok(ChartRunnablesResult {
            runnables,
            generation: self.generation,
            source_revision: request.source_revision.clone(),
        })
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

    /// Fetch and hash one explicitly requested remote definition import.
    ///
    /// Ordinary project analysis retains its configured network capability;
    /// only this user-selected operation enables HTTP for the one origin.
    pub async fn pin_http_import(
        &self,
        url: &str,
        cancellation: &AnalysisCancellation,
    ) -> Result<String, PinImportError> {
        cancellation
            .check()
            .map_err(|_| PinImportError::Cancelled)?;
        let origin = SourceOrigin::Http(url.to_owned());
        let mut capabilities = self.compiler.options().import_capabilities.clone();
        capabilities.allow_http = true;
        let load = self
            .compiler
            .options()
            .source_loader
            .load(&origin, &capabilities);
        pin_mut!(load);
        let cancelled = cancellation.cancelled();
        pin_mut!(cancelled);
        let loaded = match select(load, cancelled).await {
            Either::Left((result, _)) => {
                result.map_err(|error| PinImportError::Load(error.to_string()))?
            }
            Either::Right(((), _)) => return Err(PinImportError::Cancelled),
        };
        let source = SourceFile::new(SourceId::new(0), loaded.origin, loaded.text.to_string());
        let parsed = avenger_lang_core::syntax::parse_file(&source)
            .map_err(|_| PinImportError::NotDefinition)?;
        if !parsed
            .ast
            .items
            .iter()
            .any(|item| item.declaration.keyword.as_str() == "define")
        {
            return Err(PinImportError::NotDefinition);
        }
        let digest = Sha256::digest(loaded.text.as_bytes());
        Ok(format!("{digest:x}"))
    }

    pub async fn analyze_workspace(
        &self,
        snapshot: WorkspaceSnapshot,
        cancellation: &AnalysisCancellation,
    ) -> Result<WorkspaceAnalysis, AnalysisCancelled> {
        cancellation.check()?;
        if snapshot.native_registry_profile
            != self
                .compiler
                .language_host()
                .registry()
                .profile_id()
                .as_str()
        {
            // A profile change invalidates every schema-derived answer. Treat
            // the snapshot like cancelled work so it can never publish.
            return Err(AnalysisCancelled);
        }
        let mut syntax = snapshot
            .open_documents
            .iter()
            .map(|(origin, document)| (origin.clone(), analyze_syntax(document)))
            .collect::<BTreeMap<_, _>>();
        cancellation.check()?;
        let registry = self.compiler.language_host().authoring_schema().clone();

        let loader = snapshot.source_loader(self.compiler.options().source_loader.clone());
        let capabilities = self.compiler.options().import_capabilities.clone();
        for origin in &snapshot.known_disk_sources {
            if syntax.contains_key(origin) {
                continue;
            }
            cancellation.check()?;
            let load = loader.load(origin, &capabilities);
            pin_mut!(load);
            let cancelled = cancellation.cancelled();
            pin_mut!(cancelled);
            let loaded = match select(load, cancelled).await {
                Either::Left((Ok(loaded), _)) => loaded,
                Either::Left((Err(_), _)) => continue,
                Either::Right(((), _)) => return Err(AnalysisCancelled),
            };
            let document = DocumentSnapshot::new(
                loaded.origin.clone(),
                SourceRevision::from_text(&loaded.text),
                loaded.text,
            );
            syntax.insert(document.origin.clone(), analyze_syntax(&document));
        }
        cancellation.check()?;
        let compiler = self
            .compiler
            .fork_with_source_loader(Arc::new(loader.clone()));
        let data_roots = snapshot
            .roots
            .iter()
            .filter(|root| root.role == avenger_lang_core::ModuleDependencyRole::AmbientDataRoot)
            .cloned()
            .collect::<Vec<_>>();
        let chart_roots = snapshot
            .roots
            .iter()
            .filter(|root| root.role == avenger_lang_core::ModuleDependencyRole::RequestedModule)
            .cloned()
            .collect::<Vec<_>>();
        let mut semantic_roots = BTreeMap::new();
        let mut dataset_contexts = BTreeMap::<SourceOrigin, Vec<DatasetContext>>::new();
        for root in chart_roots {
            cancellation.check()?;
            let mut closure_roots = vec![root.clone()];
            closure_roots.extend(data_roots.iter().cloned());
            let analysis = compiler.analyze_module_roots(closure_roots, snapshot.generation.get());
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentRequest {
    pub source: SourceOrigin,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
    Cr,
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
pub struct ChartRunnable {
    pub label: String,
    pub selector: Option<String>,
    pub span: SourceSpan,
    pub selection_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartRunnablesResult {
    pub runnables: Vec<ChartRunnable>,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
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
                kind: match context {
                    avenger_lang_core::syntax::SqlIslandContext::QueryProperty => {
                        SyntaxContextKind::Query
                    }
                    avenger_lang_core::syntax::SqlIslandContext::ProjectionProperty
                    | avenger_lang_core::syntax::SqlIslandContext::PropertyExpression
                    | avenger_lang_core::syntax::SqlIslandContext::TerminatedExpression
                    | avenger_lang_core::syntax::SqlIslandContext::ArrayExpression
                    | avenger_lang_core::syntax::SqlIslandContext::AliasedExpression => {
                        SyntaxContextKind::Expression
                    }
                },
                span: node.unwrap().span,
                declaration_keyword: None,
                property_name: None,
            },
            Some(TolerantSyntaxNodeKind::ChannelMode { .. }) => SyntaxContext {
                kind: SyntaxContextKind::Expression,
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
    let symbols = declaration_symbols(&parsed, None);
    SyntaxAnalysis {
        revision: snapshot.revision.clone(),
        parsed,
        diagnostics,
        symbols,
    }
}

fn declaration_symbols(
    parsed: &TolerantParsedFile,
    parent: Option<TolerantSyntaxNodeId>,
) -> Vec<DocumentSymbol> {
    parsed
        .nodes
        .iter()
        .filter_map(|node| {
            let TolerantSyntaxNodeKind::Declaration { keyword, name } = &node.kind else {
                return None;
            };
            if normalized_parent(&parsed.nodes, node.parent) != parent {
                return None;
            }
            let selection_span = symbol_selection_span(parsed, node, name.as_deref(), keyword);
            Some(DocumentSymbol {
                name: name.clone().unwrap_or_else(|| keyword.clone()),
                detail: Some(keyword.clone()),
                kind: symbol_kind(keyword),
                span: node.span,
                selection_span,
                children: declaration_symbols(parsed, Some(node.id)),
            })
        })
        .collect()
}

fn symbol_selection_span(
    parsed: &TolerantParsedFile,
    node: &TolerantSyntaxNode,
    name: Option<&str>,
    keyword: &str,
) -> SourceSpan {
    let desired = name.unwrap_or(keyword);
    parsed
        .tokens
        .tokens()
        .iter()
        .take_while(|token| token.span().range.start < node.span.range.end)
        .filter(|token| node.span.range.start <= token.span().range.start)
        .take_while(|token| {
            !matches!(
                token.token(),
                Some(sqlparser::tokenizer::Token::LBrace | sqlparser::tokenizer::Token::SemiColon)
            )
        })
        .filter_map(|token| match token.token() {
            Some(sqlparser::tokenizer::Token::Word(word)) if word.value == desired => {
                Some(token.span())
            }
            _ => None,
        })
        .last()
        .unwrap_or(node.span)
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
        "mark" => SymbolKind::Mark,
        "transform" => SymbolKind::Transform,
        "param" => SymbolKind::Param,
        "store" => SymbolKind::Store,
        "selection" => SymbolKind::Selection,
        "tool" => SymbolKind::Tool,
        "widget" => SymbolKind::Widget,
        "on" | "event" => SymbolKind::Event,
        "view" => SymbolKind::View,
        "field" => SymbolKind::Field,
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
    Keyword,
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
pub struct SemanticTokensResult {
    pub tokens: Vec<SemanticToken>,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceTextEdit {
    pub span: SourceSpan,
    pub new_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattingResult {
    pub edit: SourceTextEdit,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrepareRenameResult {
    pub span: SourceSpan,
    pub placeholder: String,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedSourceEdits {
    pub source_revision: SourceRevision,
    pub edits: Vec<SourceTextEdit>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceEdit {
    pub sources: BTreeMap<SourceOrigin, VersionedSourceEdits>,
    /// New authored files to create before applying `sources` edits.
    pub create_files: BTreeMap<SourceOrigin, String>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeActionRequest {
    pub source: SourceOrigin,
    pub range: SourceSpan,
    pub source_revision: SourceRevision,
    pub diagnostic_codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinImportTarget {
    pub url: String,
    pub insertion_span: SourceSpan,
    pub generation: AnalysisGeneration,
    pub source_revision: SourceRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PinImportError {
    #[error("the pin-import request was cancelled")]
    Cancelled,
    #[error("could not fetch import: {0}")]
    Load(String),
    #[error("the fetched source is not a valid Avenger definition")]
    NotDefinition,
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
