//! Native Language Server Protocol transport for Avenger.
//!
//! Language intelligence belongs in `avenger-lang-analysis`; this crate owns
//! protocol lifecycle, capability negotiation, document synchronization, and
//! editor wire conversion. Standard output is always reserved for JSON-RPC.

mod documents;
mod position;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, AnalysisService,
    CodeActionKind as AnalysisCodeActionKind, CodeActionRequest,
    CompletionInvocation as AnalysisCompletionInvocation,
    CompletionOptions as AnalysisCompletionOptions,
    CompletionSemanticKind as AnalysisCompletionSemanticKind, CompletionTextFormat,
    DocumentRequest, DocumentSnapshot, LineEnding, PositionRequest, RenameError,
    SemanticTokenKind as AvengerSemanticTokenKind,
    SemanticTokenModifiers as AvengerSemanticTokenModifiers, SourceRevision, SourceTextEdit,
    SyntaxAnalysis, VersionedSourceEdits, WorkspaceAnalysis,
    WorkspaceEdit as AnalysisWorkspaceEdit, WorkspaceSnapshot, analyze_syntax,
};
use avenger_lang_compiler::{CompileFailure, Compiler};
use avenger_lang_core::{
    Diagnostic as AvengerDiagnostic, DiagnosticSeverity as AvengerDiagnosticSeverity, ModuleRoot,
    SourceFile, SourceMap, SourceOrigin, module_graph::normalize_path,
};
use documents::{DocumentStore, OpenDocument};
use notify::{
    Event as NotifyEvent, EventKind as NotifyEventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use position::{PositionEncoding, PositionIndex};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore},
    task::JoinHandle,
};
use tower_lsp_server::{Client, LanguageServer, LspService, Server, jsonrpc, ls_types::*};

const SERVER_NAME: &str = "avenger-lsp";
const PIN_IMPORT_ACTION_KIND: &str = "source.pinImport";
const WATCH_CHART_COMMAND: &str = "avenger.watchChart";
static INVALID_COMPLETION_RANGES: AtomicU64 = AtomicU64::new(0);
static CANCELLED_COMPLETION_WORKERS: AtomicU64 = AtomicU64::new(0);

struct CancelAnalysisOnDrop {
    cancellation: AnalysisCancellation,
    armed: bool,
}

impl CancelAnalysisOnDrop {
    fn new(cancellation: AnalysisCancellation) -> Self {
        Self {
            cancellation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelAnalysisOnDrop {
    fn drop(&mut self) {
        if self.armed {
            CANCELLED_COMPLETION_WORKERS.fetch_add(1, Ordering::Relaxed);
            self.cancellation.cancel();
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PinImportResolveData {
    operation: String,
    uri: Uri,
    version: i32,
    range_start: usize,
    range_end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LspServerConfig {
    pub semantic_debounce: Duration,
    pub max_document_bytes: usize,
    pub max_diagnostics_per_document: usize,
    pub max_workspaces: usize,
    pub max_semantic_tokens_per_document: usize,
    pub max_concurrent_requests: usize,
    pub max_analysis_cache_entries: usize,
    pub max_dataset_cache_entries: usize,
}

impl Default for LspServerConfig {
    fn default() -> Self {
        Self {
            semantic_debounce: Duration::from_millis(120),
            max_document_bytes: 8 * 1024 * 1024,
            max_diagnostics_per_document: 200,
            max_workspaces: 32,
            max_semantic_tokens_per_document: 100_000,
            max_concurrent_requests: 16,
            max_analysis_cache_entries: 64,
            max_dataset_cache_entries: 512,
        }
    }
}

impl LspServerConfig {
    fn normalized(self) -> Self {
        Self {
            semantic_debounce: self.semantic_debounce,
            max_document_bytes: self.max_document_bytes.max(1),
            max_diagnostics_per_document: self.max_diagnostics_per_document.max(1),
            max_workspaces: self.max_workspaces.max(1),
            max_semantic_tokens_per_document: self.max_semantic_tokens_per_document.max(1),
            max_concurrent_requests: self.max_concurrent_requests.max(1),
            max_analysis_cache_entries: self.max_analysis_cache_entries.max(1),
            max_dataset_cache_entries: self.max_dataset_cache_entries.max(1),
        }
    }
}

#[derive(Clone)]
struct Backend {
    client: Client,
    inner: Arc<BackendInner>,
}

struct BackendInner {
    config: LspServerConfig,
    state: RwLock<ServerState>,
    semantic_tasks: Mutex<HashMap<PathBuf, SemanticTask>>,
    publish_lock: Mutex<()>,
    watchers: StdMutex<HashMap<PathBuf, RecommendedWatcher>>,
    request_limit: Arc<Semaphore>,
}

struct SemanticTask {
    cancellation: AnalysisCancellation,
    handle: JoinHandle<()>,
}

struct ServerState {
    position_encoding: PositionEncoding,
    position_encoding_kind: PositionEncodingKind,
    hierarchical_symbols: bool,
    completion_snippets: bool,
    document_changes: bool,
    resource_create: bool,
    code_action_literals: bool,
    code_action_resolve_edit: bool,
    code_action_preferred: bool,
    documents: DocumentStore,
    syntax: HashMap<Uri, SyntaxAnalysis>,
    workspaces: BTreeMap<PathBuf, Workspace>,
    workspace_generations: BTreeMap<PathBuf, u64>,
    published_semantic: BTreeMap<PathBuf, BTreeSet<Uri>>,
    semantic_analysis: BTreeMap<PathBuf, Arc<WorkspaceAnalysis>>,
    initialized: bool,
    shutting_down: bool,
}

#[derive(Clone)]
struct Workspace {
    service: AnalysisService,
    ambient_modules: BTreeSet<SourceOrigin>,
    disk_sources: Arc<StdMutex<BTreeSet<SourceOrigin>>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AvengerSettings {
    #[serde(default)]
    ambient_modules: Vec<String>,
}

#[derive(Clone)]
struct SemanticInput {
    workspace: Workspace,
    snapshot: WorkspaceSnapshot,
    versions: BTreeMap<Uri, i32>,
}

impl Backend {
    #[cfg(test)]
    fn new(client: Client) -> Self {
        Self::with_config(client, LspServerConfig::default())
    }

    fn with_config(client: Client, config: LspServerConfig) -> Self {
        let config = config.normalized();
        let request_limit = Arc::new(Semaphore::new(config.max_concurrent_requests));
        Self {
            client,
            inner: Arc::new(BackendInner {
                config,
                state: RwLock::new(ServerState {
                    position_encoding: PositionEncoding::Utf16,
                    position_encoding_kind: PositionEncodingKind::UTF16,
                    hierarchical_symbols: true,
                    completion_snippets: false,
                    document_changes: false,
                    resource_create: false,
                    code_action_literals: false,
                    code_action_resolve_edit: false,
                    code_action_preferred: false,
                    documents: DocumentStore::default(),
                    syntax: HashMap::new(),
                    workspaces: BTreeMap::new(),
                    workspace_generations: BTreeMap::new(),
                    published_semantic: BTreeMap::new(),
                    semantic_analysis: BTreeMap::new(),
                    initialized: false,
                    shutting_down: false,
                }),
                semantic_tasks: Mutex::new(HashMap::new()),
                publish_lock: Mutex::new(()),
                watchers: StdMutex::new(HashMap::new()),
                request_limit,
            }),
        }
    }

    async fn request_permit(&self) -> jsonrpc::Result<OwnedSemaphorePermit> {
        Arc::clone(&self.inner.request_limit)
            .acquire_owned()
            .await
            .map_err(|_| jsonrpc::Error::internal_error())
    }

    async fn report_error(&self, context: &str, error: impl std::fmt::Display) {
        let message = format!("{context}: {error}");
        tracing::warn!("{message}");
        eprintln!("{SERVER_NAME}: {message}");
        self.client.log_message(MessageType::WARNING, message).await;
    }

    async fn ensure_workspace_for_uri(&self, uri: &Uri) -> Option<PathBuf> {
        let path = normalize_existing_path(uri.to_file_path()?.into_owned());
        {
            let state = self.inner.state.read().await;
            if let Some(root) = owning_workspace(&state.workspaces, &path) {
                return Some(root);
            }
            if state.workspaces.len() >= self.inner.config.max_workspaces {
                drop(state);
                self.report_error(
                    "could not create workspace",
                    format!(
                        "configured workspace limit {} reached",
                        self.inner.config.max_workspaces
                    ),
                )
                .await;
                return None;
            }
        }
        let root = path.parent()?.to_path_buf();
        match Workspace::new(root.clone()) {
            Ok(workspace) => {
                self.inner
                    .state
                    .write()
                    .await
                    .workspaces
                    .insert(root.clone(), workspace);
                if self.inner.state.read().await.initialized {
                    self.start_watcher(root.clone()).await;
                }
                Some(root)
            }
            Err(error) => {
                self.report_error("could not create workspace", error).await;
                None
            }
        }
    }

    async fn publish_syntax(&self, document: OpenDocument) {
        let origin = origin_for_uri(&document.uri);
        let snapshot = DocumentSnapshot::new(
            origin,
            SourceRevision::from_text(&document.text),
            document.text.clone(),
        );
        let analysis = analyze_syntax(&snapshot);
        let diagnostics = analysis
            .diagnostics
            .iter()
            .filter_map(|diagnostic| {
                lsp_diagnostic_for_single_source(diagnostic, &document.uri, &document.positions)
            })
            .take(self.inner.config.max_diagnostics_per_document)
            .collect();
        self.inner
            .state
            .write()
            .await
            .syntax
            .insert(document.uri.clone(), analysis);

        let _publication = self.inner.publish_lock.lock().await;
        let current = {
            let state = self.inner.state.read().await;
            state
                .documents
                .get(&document.uri)
                .is_some_and(|current| current.version == document.version)
        };
        if current {
            self.client
                .publish_diagnostics(document.uri, diagnostics, Some(document.version))
                .await;
        }
    }

    async fn schedule_semantic(&self, workspace_root: PathBuf) {
        let input = {
            let mut state = self.inner.state.write().await;
            if state.shutting_down {
                return;
            }
            let generation = state
                .workspace_generations
                .entry(workspace_root.clone())
                .or_default();
            *generation = generation.saturating_add(1);
            semantic_input(&state, &workspace_root)
        };
        let Some(input) = input else {
            return;
        };
        let generation = input.snapshot.generation;
        let cancellation = AnalysisCancellation::default();
        let mut tasks = self.inner.semantic_tasks.lock().await;
        if let Some(previous) = tasks.remove(&workspace_root) {
            previous.cancellation.cancel();
            previous.handle.abort();
        }
        let backend = self.clone();
        let task_cancellation = cancellation.clone();
        let task_root = workspace_root.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(backend.inner.config.semantic_debounce).await;
            let result = analyze_on_worker(
                input.workspace.service.clone(),
                input.snapshot,
                task_cancellation,
            )
            .await;
            if let Ok(analysis) = result {
                input.workspace.service.compiler().trim_editor_caches(
                    backend.inner.config.max_analysis_cache_entries,
                    backend.inner.config.max_dataset_cache_entries,
                );
                backend
                    .publish_semantic(task_root, generation, input.versions, analysis)
                    .await;
            }
        });
        tasks.insert(
            workspace_root,
            SemanticTask {
                cancellation,
                handle,
            },
        );
    }

    async fn publish_semantic(
        &self,
        workspace_root: PathBuf,
        generation: AnalysisGeneration,
        versions: BTreeMap<Uri, i32>,
        analysis: WorkspaceAnalysis,
    ) {
        let analysis = {
            let state = self.inner.state.read().await;
            state
                .semantic_analysis
                .get(&workspace_root)
                .map_or(analysis.clone(), |previous| {
                    analysis.with_last_good_semantics(previous)
                })
        };
        let mut by_uri = semantic_diagnostics(&analysis, self.position_encoding().await);
        for diagnostics in by_uri.values_mut() {
            diagnostics.truncate(self.inner.config.max_diagnostics_per_document);
        }
        // Successful roots still need an explicit empty semantic batch, while
        // open files retain their tolerant diagnostics in the merged publish.
        for uri in versions.keys() {
            by_uri.entry(uri.clone()).or_default();
        }
        for (origin, syntax) in &analysis.syntax {
            let Some(uri) = uri_for_origin(origin) else {
                continue;
            };
            let Some(document) = self.document(&uri).await else {
                continue;
            };
            let diagnostics = syntax
                .diagnostics
                .iter()
                .filter_map(|diagnostic| {
                    lsp_diagnostic_for_single_source(diagnostic, &uri, &document.positions)
                })
                .collect::<Vec<_>>();
            by_uri.entry(uri).or_default().extend(diagnostics);
        }
        for diagnostics in by_uri.values_mut() {
            diagnostics.sort_by_key(|diagnostic| {
                (
                    diagnostic.range.start,
                    diagnostic.range.end,
                    format!("{:?}", diagnostic.code),
                    diagnostic.message.clone(),
                )
            });
            diagnostics.dedup();
            diagnostics.truncate(self.inner.config.max_diagnostics_per_document);
        }

        let _publication = self.inner.publish_lock.lock().await;
        let (current, previous) = {
            let mut state = self.inner.state.write().await;
            let current = !state.shutting_down
                && state
                    .workspace_generations
                    .get(&workspace_root)
                    .is_some_and(|current| *current == generation.get());
            if !current {
                (false, BTreeSet::new())
            } else {
                state
                    .semantic_analysis
                    .insert(workspace_root.clone(), Arc::new(analysis.clone()));
                let previous = state
                    .published_semantic
                    .insert(workspace_root, by_uri.keys().cloned().collect())
                    .unwrap_or_default();
                (true, previous)
            }
        };
        if !current {
            return;
        }
        let current_uris: BTreeSet<_> = by_uri.keys().cloned().collect();
        for uri in previous.difference(&current_uris) {
            self.client
                .publish_diagnostics(uri.clone(), vec![], None)
                .await;
        }
        for (uri, diagnostics) in by_uri {
            let version = versions.get(&uri).copied();
            // The publication lock guarantees a newer edit will publish after
            // this batch. Recheck versions so stale generations never win.
            if let Some(expected) = version {
                let still_current = self
                    .inner
                    .state
                    .read()
                    .await
                    .documents
                    .get(&uri)
                    .is_some_and(|document| document.version == expected);
                if !still_current {
                    continue;
                }
            }
            self.client
                .publish_diagnostics(uri, diagnostics, version)
                .await;
        }
    }

    async fn document(&self, uri: &Uri) -> Option<OpenDocument> {
        self.inner.state.read().await.documents.get(uri).cloned()
    }

    async fn position_encoding(&self) -> PositionEncoding {
        self.inner.state.read().await.position_encoding
    }

    async fn query_snapshot(
        &self,
        uri: &Uri,
        position: Position,
    ) -> Option<(
        OpenDocument,
        Arc<WorkspaceAnalysis>,
        PositionRequest,
        bool,
        AnalysisGeneration,
    )> {
        let (document, root, workspace, generation, syntax, latest, snippets) = {
            let state = self.inner.state.read().await;
            let document = state.documents.get(uri)?.clone();
            let path = uri.to_file_path()?.into_owned();
            let root = owning_workspace(&state.workspaces, &path)?;
            let workspace = state.workspaces.get(&root)?.clone();
            let generation = AnalysisGeneration::new(
                state
                    .workspace_generations
                    .get(&root)
                    .copied()
                    .unwrap_or_default(),
            );
            let syntax = state
                .syntax
                .iter()
                .filter_map(|(uri, syntax)| {
                    let origin = origin_for_uri(uri);
                    origin_is_in_workspace(&origin, &root).then(|| (origin, syntax.clone()))
                })
                .collect::<BTreeMap<_, _>>();
            (
                document,
                root.clone(),
                workspace,
                generation,
                syntax,
                state.semantic_analysis.get(&root).cloned(),
                state.completion_snippets,
            )
        };
        let byte_offset = document.positions.offset(position).ok()?;
        let origin = origin_for_uri(uri);
        let revision = SourceRevision::from_text(&document.text);
        let analysis = if let Some(latest) = latest {
            let is_current = latest
                .syntax
                .get(&origin)
                .is_some_and(|current| current.revision == revision);
            if is_current {
                latest
            } else {
                Arc::new(latest.with_syntax(generation, syntax))
            }
        } else {
            let known_sources = workspace.disk_sources().into_iter().collect();
            Arc::new(WorkspaceAnalysis::syntax_only(
                generation,
                root,
                known_sources,
                syntax,
                workspace
                    .service
                    .compiler()
                    .language_host()
                    .authoring_schema()
                    .clone(),
            ))
        };
        Some((
            document,
            analysis,
            PositionRequest {
                source: origin,
                byte_offset,
                source_revision: revision,
            },
            snippets,
            generation,
        ))
    }

    async fn completion_snapshot_is_current(
        &self,
        uri: &Uri,
        document_version: i32,
        source_revision: &SourceRevision,
        workspace_generation: AnalysisGeneration,
    ) -> bool {
        let state = self.inner.state.read().await;
        let Some(document) = state.documents.get(uri) else {
            return false;
        };
        if document.version != document_version
            || SourceRevision::from_text(&document.text) != *source_revision
        {
            return false;
        }
        let Some(path) = uri.to_file_path().map(|path| path.into_owned()) else {
            return false;
        };
        let Some(root) = owning_workspace(&state.workspaces, &path) else {
            return false;
        };
        state
            .workspace_generations
            .get(&root)
            .is_some_and(|current| *current == workspace_generation.get())
    }

    async fn workspace_edit(
        &self,
        edit: AnalysisWorkspaceEdit,
    ) -> jsonrpc::Result<tower_lsp_server::ls_types::WorkspaceEdit> {
        let encoding = self.position_encoding().await;
        let state = self.inner.state.read().await;
        if !edit.create_files.is_empty() && !state.resource_create {
            return Err(jsonrpc::Error::invalid_params(
                "client does not support create-file workspace edits",
            ));
        }
        let mut documents = Vec::new();
        for (origin, source_edits) in edit.sources {
            let canonical_uri = uri_for_origin(&origin).ok_or_else(|| {
                jsonrpc::Error::invalid_params("rename target has no editable file URI")
            })?;
            let open_document = state
                .documents
                .values()
                .find(|document| origin_for_uri(&document.uri) == origin);
            let (uri, positions, version) = if let Some(document) = open_document {
                if SourceRevision::from_text(&document.text) != source_edits.source_revision {
                    return Err(jsonrpc::Error::invalid_params(
                        "an affected open document changed before the edit was returned",
                    ));
                }
                (
                    document.uri.clone(),
                    document.positions.clone(),
                    Some(document.version),
                )
            } else {
                let SourceOrigin::File(path) = &origin else {
                    return Err(jsonrpc::Error::invalid_params(
                        "closed non-file sources cannot be edited",
                    ));
                };
                let text = std::fs::read_to_string(path).map_err(|error| {
                    jsonrpc::Error::invalid_params(format!(
                        "could not read affected source {}: {error}",
                        path.display()
                    ))
                })?;
                if SourceRevision::from_text(&text) != source_edits.source_revision {
                    return Err(jsonrpc::Error::invalid_params(
                        "an affected disk document changed before the edit was returned",
                    ));
                }
                (
                    canonical_uri,
                    PositionIndex::new(Arc::<str>::from(text), encoding),
                    None,
                )
            };
            let edits = source_edits
                .edits
                .into_iter()
                .map(|edit| {
                    positions
                        .lsp_range(edit.span.range.as_range())
                        .map(|range| OneOf::Left(TextEdit::new(range, edit.new_text)))
                        .map_err(|error| {
                            jsonrpc::Error::invalid_params(format!(
                                "affected edit range is invalid: {error}"
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            documents.push(TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier { uri, version },
                edits,
            });
        }
        let document_changes = if edit.create_files.is_empty() {
            DocumentChanges::Edits(documents)
        } else {
            let mut operations = Vec::new();
            for (origin, contents) in edit.create_files {
                let SourceOrigin::File(path) = &origin else {
                    return Err(jsonrpc::Error::invalid_params(
                        "only local files can be created by a workspace edit",
                    ));
                };
                if path.exists() {
                    return Err(jsonrpc::Error::invalid_params(format!(
                        "refusing to overwrite existing file {}",
                        path.display()
                    )));
                }
                let uri = uri_for_origin(&origin).ok_or_else(|| {
                    jsonrpc::Error::invalid_params("new file has no editable URI")
                })?;
                operations.push(DocumentChangeOperation::Op(ResourceOp::Create(
                    CreateFile {
                        uri: uri.clone(),
                        options: Some(CreateFileOptions {
                            overwrite: Some(false),
                            ignore_if_exists: Some(false),
                        }),
                        annotation_id: None,
                    },
                )));
                operations.push(DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier { uri, version: None },
                    edits: vec![OneOf::Left(TextEdit::new(
                        Range::new(Position::new(0, 0), Position::new(0, 0)),
                        contents,
                    ))],
                }));
            }
            operations.extend(documents.into_iter().map(DocumentChangeOperation::Edit));
            DocumentChanges::Operations(operations)
        };
        Ok(tower_lsp_server::ls_types::WorkspaceEdit {
            changes: None,
            document_changes: Some(document_changes),
            change_annotations: None,
        })
    }

    async fn start_watcher(&self, root: PathBuf) {
        if self
            .inner
            .watchers
            .lock()
            .expect("watcher lock poisoned")
            .contains_key(&root)
        {
            return;
        }
        let backend = self.clone();
        let watched_root = root.clone();
        let runtime = tokio::runtime::Handle::current();
        let watcher = RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else {
                    return;
                };
                let backend = backend.clone();
                let root = watched_root.clone();
                runtime.spawn(async move { backend.handle_watcher_event(root, event).await });
            },
            notify::Config::default(),
        );
        match watcher {
            Ok(mut watcher) => {
                if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
                    self.report_error("could not watch workspace", error).await;
                    return;
                }
                self.inner
                    .watchers
                    .lock()
                    .expect("watcher lock poisoned")
                    .insert(root, watcher);
            }
            Err(error) => {
                self.report_error("could not create file watcher", error)
                    .await
            }
        }
    }

    async fn handle_watcher_event(&self, root: PathBuf, event: NotifyEvent) {
        let update = watcher_source_update(&root, &event);
        if matches!(update, WatcherSourceUpdate::None) {
            return;
        }
        let workspace = {
            let state = self.inner.state.read().await;
            if state.shutting_down {
                return;
            }
            state.workspaces.get(&root).cloned()
        };
        let Some(workspace) = workspace else {
            return;
        };
        match update {
            WatcherSourceUpdate::None => return,
            WatcherSourceUpdate::Paths(paths) => workspace.update_disk_sources(paths),
            WatcherSourceUpdate::Rescan => {
                let scan_root = root.clone();
                let Ok(sources) =
                    tokio::task::spawn_blocking(move || scan_avenger_files(&scan_root)).await
                else {
                    return;
                };
                workspace.replace_disk_sources(sources);
            }
        }
        self.schedule_semantic(root).await;
    }

    async fn stop_workspace(&self, root: &Path) {
        if let Some(task) = self.inner.semantic_tasks.lock().await.remove(root) {
            task.cancellation.cancel();
            task.handle.abort();
        }
        let watcher = self
            .inner
            .watchers
            .lock()
            .expect("watcher lock poisoned")
            .remove(root);
        if let Some(watcher) = watcher {
            drop_watcher_off_request_path(vec![watcher]);
        }
    }
}

fn drop_watcher_off_request_path(watchers: Vec<RecommendedWatcher>) {
    // On macOS, dropping an FSEvents-backed recursive watcher can synchronously
    // wait for the native event thread. Zed gives `shutdown` five seconds, so
    // detach that destructor from the JSON-RPC request while removing the
    // watcher from server state immediately.
    std::mem::drop(tokio::task::spawn_blocking(move || drop(watchers)));
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        let encoding_kind = negotiate_position_encoding(&params.capabilities);
        let hierarchical_symbols = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|text| text.document_symbol.as_ref())
            .and_then(|symbols| symbols.hierarchical_document_symbol_support)
            .unwrap_or(false);
        let completion_snippets = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|text| text.completion.as_ref())
            .and_then(|completion| completion.completion_item.as_ref())
            .and_then(|item| item.snippet_support)
            .unwrap_or(false);
        let document_changes = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.document_changes)
            .unwrap_or(false);
        let resource_create = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.resource_operations.as_ref())
            .is_some_and(|operations| operations.contains(&ResourceOperationKind::Create));
        let code_action = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|text| text.code_action.as_ref());
        let code_action_literals = code_action
            .and_then(|action| action.code_action_literal_support.as_ref())
            .is_some();
        let code_action_preferred = code_action
            .and_then(|action| action.is_preferred_support)
            .unwrap_or(false);
        let code_action_resolve_edit = code_action.is_some_and(|action| {
            action.data_support.unwrap_or(false)
                && action.resolve_support.as_ref().is_some_and(|support| {
                    support.properties.iter().any(|property| property == "edit")
                })
        });
        let mut roots = Vec::new();
        if let Some(folders) = params.workspace_folders.as_ref() {
            roots.extend(
                folders
                    .iter()
                    .filter_map(|folder| folder.uri.to_file_path().map(|path| path.into_owned())),
            );
        }
        #[allow(deprecated)]
        if roots.is_empty() {
            roots.extend(
                params
                    .root_uri
                    .as_ref()
                    .and_then(Uri::to_file_path)
                    .map(|path| path.into_owned()),
            );
        }
        #[allow(deprecated)]
        if roots.is_empty()
            && let Some(root_path) = params.root_path.as_ref()
        {
            roots.push(PathBuf::from(root_path));
        }

        let settings = params
            .initialization_options
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default();
        let mut workspaces = BTreeMap::new();
        for root in roots.into_iter().take(self.inner.config.max_workspaces) {
            let root = normalize_existing_path(root);
            match Workspace::with_settings(root.clone(), &settings) {
                Ok(workspace) => {
                    workspaces.insert(root, workspace);
                }
                Err(error) => {
                    eprintln!("{SERVER_NAME}: could not initialize workspace: {error}");
                }
            }
        }
        {
            let mut state = self.inner.state.write().await;
            state.position_encoding = PositionEncoding::from_lsp(&encoding_kind);
            state.position_encoding_kind = encoding_kind.clone();
            state.hierarchical_symbols = hierarchical_symbols;
            state.completion_snippets = completion_snippets;
            state.document_changes = document_changes;
            state.resource_create = resource_create;
            state.code_action_literals = code_action_literals;
            state.code_action_preferred = code_action_preferred;
            state.code_action_resolve_edit = code_action_resolve_edit;
            state.workspaces = workspaces;
            state.workspace_generations = state
                .workspaces
                .keys()
                .cloned()
                .map(|root| (root, 0))
                .collect();
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(encoding_kind),
                text_document_sync: Some(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                        ..TextDocumentSyncOptions::default()
                    }
                    .into(),
                ),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                completion_provider: Some(tower_lsp_server::ls_types::CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        "$".to_owned(),
                        ".".to_owned(),
                        "@".to_owned(),
                        ":".to_owned(),
                        "\"".to_owned(),
                        "'".to_owned(),
                    ]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                            legend: semantic_tokens_legend(),
                            range: Some(false),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                code_action_provider: (document_changes && code_action_literals).then(|| {
                    CodeActionProviderCapability::Options(CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR_INLINE,
                            CodeActionKind::REFACTOR_EXTRACT,
                            CodeActionKind::new(PIN_IMPORT_ACTION_KIND),
                        ]),
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        resolve_provider: Some(code_action_resolve_edit),
                    })
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    ..WorkspaceServerCapabilities::default()
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: SERVER_NAME.to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            ..InitializeResult::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let roots = {
            let mut state = self.inner.state.write().await;
            state.initialized = true;
            state.workspaces.keys().cloned().collect::<Vec<_>>()
        };
        for root in roots {
            self.start_watcher(root).await;
        }
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        self.inner.state.write().await.shutting_down = true;
        let tasks = {
            let mut tasks = self.inner.semantic_tasks.lock().await;
            tasks.drain().map(|(_, task)| task).collect::<Vec<_>>()
        };
        for task in tasks {
            task.cancellation.cancel();
            task.handle.abort();
        }
        let watchers: Vec<_> = {
            let mut watchers = self.inner.watchers.lock().expect("watcher lock poisoned");
            watchers.drain().map(|(_, watcher)| watcher).collect()
        };
        if !watchers.is_empty() {
            drop_watcher_off_request_path(watchers);
        }
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let encoding = self.position_encoding().await;
        let opened = self.inner.state.write().await.documents.open(
            params.text_document,
            encoding,
            self.inner.config.max_document_bytes,
        );
        match opened {
            Ok(document) => {
                let workspace = self.ensure_workspace_for_uri(&document.uri).await;
                self.publish_syntax(document).await;
                if let Some(workspace) = workspace {
                    self.schedule_semantic(workspace).await;
                }
            }
            Err(error) => self.report_error("didOpen rejected", error).await,
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let encoding = self.position_encoding().await;
        let changed = self.inner.state.write().await.documents.change(
            &uri,
            params.text_document.version,
            &params.content_changes,
            encoding,
            self.inner.config.max_document_bytes,
        );
        match changed {
            Ok(document) => {
                let workspace = self.ensure_workspace_for_uri(&uri).await;
                self.publish_syntax(document).await;
                if let Some(workspace) = workspace {
                    self.schedule_semantic(workspace).await;
                }
            }
            Err(error) => self.report_error("didChange rejected", error).await,
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        let encoding = self.position_encoding().await;
        let saved = self.inner.state.write().await.documents.save(
            &uri,
            params.text,
            encoding,
            self.inner.config.max_document_bytes,
        );
        match saved {
            Ok(document) => {
                self.publish_syntax(document).await;
                if let Some(workspace) = self.ensure_workspace_for_uri(&uri).await {
                    self.schedule_semantic(workspace).await;
                }
            }
            Err(error) => self.report_error("didSave rejected", error).await,
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let closed = self.inner.state.write().await.documents.close(&uri);
        match closed {
            Ok(_) => {
                self.inner.state.write().await.syntax.remove(&uri);
                self.client
                    .publish_diagnostics(uri.clone(), vec![], None)
                    .await;
                if let Some(workspace) = self.ensure_workspace_for_uri(&uri).await {
                    self.schedule_semantic(workspace).await;
                }
            }
            Err(error) => self.report_error("didClose rejected", error).await,
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let settings =
            serde_json::from_value::<AvengerSettings>(params.settings).unwrap_or_default();
        let roots = {
            let mut state = self.inner.state.write().await;
            for (root, workspace) in &mut state.workspaces {
                workspace.ambient_modules = configured_ambient_modules(root, &settings);
            }
            state.workspaces.keys().cloned().collect::<Vec<_>>()
        };
        for root in roots {
            self.schedule_semantic(root).await;
        }
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> jsonrpc::Result<Option<DocumentSymbolResponse>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document.uri;
        let state = self.inner.state.read().await;
        let Some(document) = state.documents.get(&uri) else {
            return Ok(None);
        };
        let Some(analysis) = state.syntax.get(&uri) else {
            return Ok(None);
        };
        if state.hierarchical_symbols {
            let symbols = analysis
                .symbols
                .iter()
                .filter_map(|symbol| nested_symbol(symbol, &document.positions))
                .collect();
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        } else {
            let mut symbols = Vec::new();
            flatten_symbols(
                &analysis.symbols,
                None,
                &uri,
                &document.positions,
                &mut symbols,
            );
            Ok(Some(DocumentSymbolResponse::Flat(symbols)))
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> jsonrpc::Result<Option<WorkspaceSymbolResponse>> {
        let _permit = self.request_permit().await?;
        let state = self.inner.state.read().await;
        let query = params.query.to_ascii_lowercase();
        let mut symbols = BTreeMap::new();
        for analysis in state.semantic_analysis.values() {
            for (origin, document) in &analysis.semantic_index.documents {
                let Some(uri) = uri_for_origin(origin) else {
                    continue;
                };
                let Some(text) = source_text_for_origin(analysis, origin) else {
                    continue;
                };
                let positions = PositionIndex::new(text, state.position_encoding);
                for symbol in document
                    .symbols
                    .iter()
                    .filter(|symbol| symbol.parent.is_none() && symbol.keyword != "import")
                    .filter(|symbol| {
                        query.is_empty()
                            || symbol.name.to_ascii_lowercase().contains(&query)
                            || symbol.keyword.to_ascii_lowercase().contains(&query)
                            || origin.canonical_uri().to_ascii_lowercase().contains(&query)
                    })
                {
                    let Ok(range) = positions.lsp_range(symbol.selection_span.range.as_range())
                    else {
                        continue;
                    };
                    symbols
                        .entry((
                            origin.clone(),
                            symbol.selection_span.range.start,
                            symbol.name.clone(),
                        ))
                        .or_insert_with(|| {
                            #[allow(deprecated)]
                            SymbolInformation {
                                name: symbol.name.clone(),
                                kind: symbol_kind(symbol.kind),
                                tags: None,
                                deprecated: None,
                                location: Location::new(uri.clone(), range),
                                container_name: Some(format!(
                                    "{} · {}",
                                    origin.canonical_uri(),
                                    symbol.keyword
                                )),
                            }
                        });
                }
            }
        }
        Ok(Some(WorkspaceSymbolResponse::Flat(
            symbols.into_values().collect(),
        )))
    }

    async fn code_lens(&self, params: CodeLensParams) -> jsonrpc::Result<Option<Vec<CodeLens>>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document.uri;
        let Some((document, analysis, request, _, _)) =
            self.query_snapshot(&uri, Position::new(0, 0)).await
        else {
            return Ok(None);
        };
        let Ok(result) = analysis.chart_runnables(
            &DocumentRequest {
                source: request.source,
                source_revision: request.source_revision,
            },
            &AnalysisCancellation::default(),
        ) else {
            return Ok(None);
        };
        let lenses = result
            .runnables
            .into_iter()
            .filter_map(|runnable| {
                let range = document
                    .positions
                    .lsp_range(runnable.selection_span.range.as_range())
                    .ok()?;
                Some(CodeLens {
                    range,
                    command: Some(Command::new(
                        runnable.label,
                        WATCH_CHART_COMMAND.to_owned(),
                        Some(vec![serde_json::json!({
                            "moduleUri": uri.as_str(),
                            "chart": runnable.selector,
                        })]),
                    )),
                    data: None,
                })
            })
            .collect();
        Ok(Some(lenses))
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> jsonrpc::Result<Option<Vec<TextEdit>>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document.uri;
        let Some((document, analysis, request, _, _)) =
            self.query_snapshot(&uri, Position::new(0, 0)).await
        else {
            return Ok(None);
        };
        let line_ending = detected_line_ending(&document.text);
        let result = analysis.format_document(
            &DocumentRequest {
                source: request.source,
                source_revision: request.source_revision,
            },
            line_ending,
            &AnalysisCancellation::default(),
        );
        let Ok(Some(result)) = result else {
            return Ok(None);
        };
        let Some(range) = document
            .positions
            .lsp_range(result.edit.span.range.as_range())
            .ok()
        else {
            return Ok(None);
        };
        Ok(Some(vec![TextEdit::new(range, result.edit.new_text)]))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> jsonrpc::Result<Option<tower_lsp_server::ls_types::SemanticTokensResult>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document.uri;
        let Some((document, analysis, request, _, _)) =
            self.query_snapshot(&uri, Position::new(0, 0)).await
        else {
            return Ok(None);
        };
        let result = analysis.semantic_tokens(
            &DocumentRequest {
                source: request.source,
                source_revision: request.source_revision,
            },
            &AnalysisCancellation::default(),
        );
        let Ok(result) = result else {
            return Ok(None);
        };
        let token_count = result
            .tokens
            .len()
            .min(self.inner.config.max_semantic_tokens_per_document);
        let data = encode_semantic_tokens(&result.tokens[..token_count], &document.positions);
        Ok(Some(
            tower_lsp_server::ls_types::SemanticTokensResult::Tokens(SemanticTokens {
                result_id: Some(format!(
                    "{}:{}",
                    result.generation.get(),
                    result.source_revision.as_str()
                )),
                data,
            }),
        ))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> jsonrpc::Result<Option<PrepareRenameResponse>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document.uri;
        let Some((document, analysis, request, _, _)) =
            self.query_snapshot(&uri, params.position).await
        else {
            return Ok(None);
        };
        let Ok(Some(result)) = analysis.prepare_rename(&request, &AnalysisCancellation::default())
        else {
            return Ok(None);
        };
        let Some(range) = document
            .positions
            .lsp_range(result.span.range.as_range())
            .ok()
        else {
            return Ok(None);
        };
        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range,
            placeholder: result.placeholder,
        }))
    }

    async fn rename(
        &self,
        params: RenameParams,
    ) -> jsonrpc::Result<Option<tower_lsp_server::ls_types::WorkspaceEdit>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document_position.text_document.uri;
        let Some((_, analysis, request, _, _)) = self
            .query_snapshot(&uri, params.text_document_position.position)
            .await
        else {
            return Ok(None);
        };
        if !self.inner.state.read().await.document_changes {
            return Err(jsonrpc::Error::invalid_params(
                "client does not support versioned documentChanges",
            ));
        }
        let edit = analysis
            .rename(&request, &params.new_name, &AnalysisCancellation::default())
            .map_err(rename_error)?;
        self.workspace_edit(edit).await.map(Some)
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> jsonrpc::Result<Option<CodeActionResponse>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document.uri;
        let Some((document, analysis, position_request, _, _)) =
            self.query_snapshot(&uri, params.range.start).await
        else {
            return Ok(None);
        };
        let state = self.inner.state.read().await;
        let supported = state.document_changes && state.code_action_literals;
        let preferred_support = state.code_action_preferred;
        let resource_create = state.resource_create;
        let resolve_edit = state.code_action_resolve_edit;
        drop(state);
        if !supported {
            return Ok(None);
        }
        let Ok(byte_range) = document.positions.byte_range(params.range) else {
            return Ok(None);
        };
        let Some(source_id) = analysis
            .syntax
            .get(&position_request.source)
            .map(|syntax| syntax.parsed.tokens.source())
        else {
            return Ok(None);
        };
        let diagnostic_codes = params
            .context
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic.code.as_ref()? {
                NumberOrString::String(code) => Some(code.clone()),
                NumberOrString::Number(code) => Some(code.to_string()),
            })
            .collect::<Vec<_>>();
        let analysis_request = CodeActionRequest {
            source: position_request.source,
            range: avenger_lang_core::SourceSpan {
                source: source_id,
                range: avenger_lang_core::ByteSpan {
                    start: byte_range.start,
                    end: byte_range.end,
                },
            },
            source_revision: position_request.source_revision,
            diagnostic_codes,
        };
        let actions = analysis.code_actions(&analysis_request, &AnalysisCancellation::default());
        let Ok(actions) = actions else {
            return Ok(None);
        };
        let mut response = Vec::new();
        for action in actions {
            if action.kind == AnalysisCodeActionKind::RefactorExtract && !resource_create {
                continue;
            }
            let kind = match action.kind {
                AnalysisCodeActionKind::QuickFix => CodeActionKind::QUICKFIX,
                AnalysisCodeActionKind::RefactorInline => CodeActionKind::REFACTOR_INLINE,
                AnalysisCodeActionKind::RefactorExtract => CodeActionKind::REFACTOR_EXTRACT,
                AnalysisCodeActionKind::Source => CodeActionKind::SOURCE,
            };
            if !code_action_kind_allowed(&kind, params.context.only.as_deref()) {
                continue;
            }
            let matching_diagnostics = params
                .context
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code.as_ref().is_some_and(|code| {
                        let code = match code {
                            NumberOrString::String(code) => code.clone(),
                            NumberOrString::Number(code) => code.to_string(),
                        };
                        action.diagnostic_codes.contains(&code)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            let edit = self.workspace_edit(action.edit).await?;
            response.push(CodeActionOrCommand::CodeAction(
                tower_lsp_server::ls_types::CodeAction {
                    title: action.title,
                    kind: Some(kind),
                    diagnostics: (!matching_diagnostics.is_empty()).then_some(matching_diagnostics),
                    edit: Some(edit),
                    command: None,
                    is_preferred: preferred_support.then_some(action.preferred),
                    disabled: None,
                    data: None,
                },
            ));
        }
        let pin_kind = CodeActionKind::new(PIN_IMPORT_ACTION_KIND);
        if resolve_edit
            && code_action_kind_allowed(&pin_kind, params.context.only.as_deref())
            && let Ok(Some(_target)) =
                analysis.pin_import_target(&analysis_request, &AnalysisCancellation::default())
        {
            let data = serde_json::to_value(PinImportResolveData {
                operation: PIN_IMPORT_ACTION_KIND.to_owned(),
                uri,
                version: document.version,
                range_start: byte_range.start,
                range_end: byte_range.end,
            })
            .map_err(|error| {
                jsonrpc::Error::invalid_params(format!(
                    "could not encode pin-import resolve data: {error}"
                ))
            })?;
            response.push(CodeActionOrCommand::CodeAction(
                tower_lsp_server::ls_types::CodeAction {
                    title: "Fetch and pin remote import".to_owned(),
                    kind: Some(pin_kind),
                    diagnostics: None,
                    edit: None,
                    command: None,
                    is_preferred: preferred_support.then_some(true),
                    disabled: None,
                    data: Some(data),
                },
            ));
        }
        Ok((!response.is_empty()).then_some(response))
    }

    async fn code_action_resolve(
        &self,
        mut action: tower_lsp_server::ls_types::CodeAction,
    ) -> jsonrpc::Result<tower_lsp_server::ls_types::CodeAction> {
        let _permit = self.request_permit().await?;
        let data: PinImportResolveData =
            serde_json::from_value(action.data.clone().ok_or_else(|| {
                jsonrpc::Error::invalid_params("code action has no resolve data")
            })?)
            .map_err(|error| {
                jsonrpc::Error::invalid_params(format!("invalid resolve data: {error}"))
            })?;
        if data.operation != PIN_IMPORT_ACTION_KIND {
            return Err(jsonrpc::Error::invalid_params(
                "unsupported code action resolve operation",
            ));
        }
        let document = self.document(&data.uri).await.ok_or_else(|| {
            jsonrpc::Error::invalid_params("pin-import document is no longer open")
        })?;
        if document.version != data.version {
            return Err(jsonrpc::Error::invalid_params(
                "pin-import document changed before the action was resolved",
            ));
        }
        let position = document
            .positions
            .position(data.range_start)
            .map_err(|error| jsonrpc::Error::invalid_params(error.to_string()))?;
        let Some((document, analysis, position_request, _, _)) =
            self.query_snapshot(&data.uri, position).await
        else {
            return Err(jsonrpc::Error::invalid_params(
                "pin-import analysis is no longer available",
            ));
        };
        let source_id = analysis
            .syntax
            .get(&position_request.source)
            .map(|syntax| syntax.parsed.tokens.source())
            .ok_or_else(|| jsonrpc::Error::invalid_params("pin-import source is not indexed"))?;
        let request = CodeActionRequest {
            source: position_request.source.clone(),
            range: avenger_lang_core::SourceSpan {
                source: source_id,
                range: avenger_lang_core::ByteSpan {
                    start: data.range_start,
                    end: data.range_end,
                },
            },
            source_revision: position_request.source_revision.clone(),
            diagnostic_codes: Vec::new(),
        };
        let target = analysis
            .pin_import_target(&request, &AnalysisCancellation::default())
            .map_err(|error| jsonrpc::Error::invalid_params(error.to_string()))?
            .ok_or_else(|| {
                jsonrpc::Error::invalid_params("remote import is already pinned or changed")
            })?;
        let path = data.uri.to_file_path().ok_or_else(|| {
            jsonrpc::Error::invalid_params("pin-import source is not a local file")
        })?;
        let workspace = {
            let state = self.inner.state.read().await;
            let root = owning_workspace(&state.workspaces, &path).ok_or_else(|| {
                jsonrpc::Error::invalid_params("pin-import workspace is unavailable")
            })?;
            state.workspaces.get(&root).cloned().ok_or_else(|| {
                jsonrpc::Error::invalid_params("pin-import workspace is unavailable")
            })?
        };
        let hash = workspace
            .service
            .pin_http_import(&target.url, &AnalysisCancellation::default())
            .await
            .map_err(|error| jsonrpc::Error::invalid_params(error.to_string()))?;
        let current = self.document(&data.uri).await.ok_or_else(|| {
            jsonrpc::Error::invalid_params("pin-import document closed during fetch")
        })?;
        if current.version != document.version
            || SourceRevision::from_text(&current.text) != target.source_revision
        {
            return Err(jsonrpc::Error::invalid_params(
                "pin-import document changed during fetch",
            ));
        }
        action.edit = Some(
            self.workspace_edit(AnalysisWorkspaceEdit {
                sources: BTreeMap::from([(
                    position_request.source,
                    VersionedSourceEdits {
                        source_revision: target.source_revision,
                        edits: vec![SourceTextEdit {
                            span: target.insertion_span,
                            new_text: format!(" sha256 '{hash}'"),
                        }],
                    },
                )]),
                create_files: BTreeMap::new(),
            })
            .await?,
        );
        action.data = None;
        Ok(action)
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> jsonrpc::Result<Option<CompletionResponse>> {
        let _permit = self.request_permit().await?;
        let invocation = params
            .context
            .as_ref()
            .map(|context| {
                if context.trigger_kind == CompletionTriggerKind::TRIGGER_CHARACTER {
                    AnalysisCompletionInvocation::TriggerCharacter(
                        context.trigger_character.clone().unwrap_or_default(),
                    )
                } else if context.trigger_kind
                    == CompletionTriggerKind::TRIGGER_FOR_INCOMPLETE_COMPLETIONS
                {
                    AnalysisCompletionInvocation::TriggerForIncompleteCompletions
                } else {
                    AnalysisCompletionInvocation::Invoked
                }
            })
            .unwrap_or_default();
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some((document, analysis, request, snippets, generation)) =
            self.query_snapshot(&uri, position).await
        else {
            return Ok(None);
        };
        let request_revision = request.source_revision.clone();
        let cancellation = AnalysisCancellation::default();
        let mut cancel_on_drop = CancelAnalysisOnDrop::new(cancellation.clone());
        let worker_cancellation = cancellation.clone();
        let worker = tokio::task::spawn_blocking(move || {
            analysis.complete(
                &request,
                AnalysisCompletionOptions {
                    snippets,
                    invocation,
                },
                &worker_cancellation,
            )
        })
        .await;
        cancel_on_drop.disarm();
        let result = worker.ok().and_then(Result::ok);
        drop(cancel_on_drop);
        let Some(result) = result else {
            return Ok(None);
        };
        if !self
            .completion_snapshot_is_current(&uri, document.version, &request_revision, generation)
            .await
        {
            return Ok(None);
        }
        let mut items = Vec::with_capacity(result.items.len());
        for item in result.items {
            match completion_item(item, &document.positions) {
                Ok(item) => items.push(item),
                Err(error) => {
                    debug_assert!(
                        false,
                        "analysis produced an invalid completion range for {}: {error}",
                        uri.as_str()
                    );
                    tracing::warn!(
                        source = %uri.as_str(),
                        %error,
                        "dropping completion item with an invalid authored range"
                    );
                }
            }
        }
        Ok(Some(CompletionResponse::List(CompletionList {
            is_incomplete: result.is_incomplete,
            items,
        })))
    }

    async fn hover(&self, params: HoverParams) -> jsonrpc::Result<Option<Hover>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((document, analysis, request, _, _)) = self.query_snapshot(&uri, position).await
        else {
            return Ok(None);
        };
        let cancellation = AnalysisCancellation::default();
        let Ok(Some(result)) = analysis.hover(&request, &cancellation) else {
            return Ok(None);
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: result.markdown,
            }),
            range: document
                .positions
                .lsp_range(result.span.range.as_range())
                .ok(),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((_, analysis, request, _, _)) = self.query_snapshot(&uri, position).await else {
            return Ok(None);
        };
        let cancellation = AnalysisCancellation::default();
        let Ok(result) = analysis.definition(&request, &cancellation) else {
            return Ok(None);
        };
        let locations =
            navigation_locations(&analysis, result.targets, self.position_encoding().await);
        Ok((!locations.is_empty()).then_some(GotoDefinitionResponse::Array(locations)))
    }

    async fn references(&self, params: ReferenceParams) -> jsonrpc::Result<Option<Vec<Location>>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some((_, analysis, request, _, _)) = self.query_snapshot(&uri, position).await else {
            return Ok(None);
        };
        let cancellation = AnalysisCancellation::default();
        let Ok(result) =
            analysis.references(&request, params.context.include_declaration, &cancellation)
        else {
            return Ok(None);
        };
        Ok(Some(navigation_locations(
            &analysis,
            result.targets,
            self.position_encoding().await,
        )))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> jsonrpc::Result<Option<Vec<DocumentHighlight>>> {
        let _permit = self.request_permit().await?;
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((document, analysis, request, _, _)) = self.query_snapshot(&uri, position).await
        else {
            return Ok(None);
        };
        let cancellation = AnalysisCancellation::default();
        let Ok(result) = analysis.references(&request, true, &cancellation) else {
            return Ok(None);
        };
        let highlights = result
            .targets
            .into_iter()
            .filter(|target| target.origin == request.source)
            .filter_map(|target| {
                Some(DocumentHighlight {
                    range: document
                        .positions
                        .lsp_range(target.selection_span.range.as_range())
                        .ok()?,
                    kind: Some(DocumentHighlightKind::READ),
                })
            })
            .collect();
        Ok(Some(highlights))
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        for removed in params.event.removed {
            if let Some(path) = removed.uri.to_file_path() {
                let path = normalize_existing_path(path.into_owned());
                self.inner.state.write().await.workspaces.remove(&path);
                self.inner
                    .state
                    .write()
                    .await
                    .workspace_generations
                    .remove(&path);
                self.stop_workspace(&path).await;
            }
        }
        for added in params.event.added {
            let Some(path) = added.uri.to_file_path() else {
                continue;
            };
            let path = normalize_existing_path(path.into_owned());
            match Workspace::new(path.clone()) {
                Ok(workspace) => {
                    self.inner
                        .state
                        .write()
                        .await
                        .workspaces
                        .insert(path.clone(), workspace);
                    self.inner
                        .state
                        .write()
                        .await
                        .workspace_generations
                        .insert(path.clone(), 0);
                    self.start_watcher(path).await;
                }
                Err(error) => self.report_error("could not add workspace", error).await,
            }
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let updates = {
            let state = self.inner.state.read().await;
            let mut updates = BTreeMap::<PathBuf, Vec<PathBuf>>::new();
            for change in &params.changes {
                let Some(path) = change.uri.to_file_path() else {
                    continue;
                };
                let path = path.into_owned();
                if !is_avenger_source_path(&path) {
                    continue;
                }
                let Some(root) = owning_workspace(&state.workspaces, &path) else {
                    continue;
                };
                updates.entry(root).or_default().push(path);
            }
            updates
        };
        for (root, paths) in updates {
            let workspace = {
                let state = self.inner.state.read().await;
                state.workspaces.get(&root).cloned()
            };
            if let Some(workspace) = workspace {
                workspace.update_disk_sources(paths);
            }
            self.schedule_semantic(root).await;
        }
    }
}

impl Workspace {
    fn new(path: PathBuf) -> Result<Self, avenger_lang_compiler::CompilerBuildError> {
        Self::with_settings(path, &AvengerSettings::default())
    }

    fn with_settings(
        path: PathBuf,
        settings: &AvengerSettings,
    ) -> Result<Self, avenger_lang_compiler::CompilerBuildError> {
        let compiler = Compiler::builder().project_root(&path).build()?;
        let disk_sources = scan_avenger_files(&path);
        Ok(Self {
            service: AnalysisService::new(compiler),
            ambient_modules: configured_ambient_modules(&path, settings),
            disk_sources: Arc::new(StdMutex::new(disk_sources)),
        })
    }

    fn disk_sources(&self) -> BTreeSet<SourceOrigin> {
        self.disk_sources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn update_disk_sources(&self, paths: Vec<PathBuf>) {
        let mut sources = self
            .disk_sources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for path in paths {
            let origin = SourceOrigin::File(normalize_existing_path(path.clone()));
            if path.is_file() {
                sources.insert(origin);
            } else {
                sources.remove(&origin);
            }
        }
    }

    fn replace_disk_sources(&self, replacement: BTreeSet<SourceOrigin>) {
        *self
            .disk_sources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = replacement;
    }
}

/// Run the native Avenger language server over standard input and output.
///
/// Standard output is reserved exclusively for LSP framing. Callers must send
/// human-readable logs to standard error or through LSP client notifications.
pub async fn run_stdio() {
    run_stdio_with_config(LspServerConfig::default()).await;
}

/// Run the native Avenger language server with explicit resource limits.
///
/// Standard output is reserved exclusively for LSP framing. Callers must send
/// human-readable logs to standard error or through LSP client notifications.
pub async fn run_stdio_with_config(config: LspServerConfig) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(move |client| Backend::with_config(client, config));
    Server::new(stdin, stdout, socket).serve(service).await;
}

fn code_action_kind_allowed(kind: &CodeActionKind, only: Option<&[CodeActionKind]>) -> bool {
    only.is_none_or(|requested| {
        requested.iter().any(|requested| {
            requested.as_str().is_empty()
                || kind == requested
                || kind
                    .as_str()
                    .strip_prefix(requested.as_str())
                    .is_some_and(|suffix| suffix.starts_with('.'))
        })
    })
}

fn negotiate_position_encoding(capabilities: &ClientCapabilities) -> PositionEncodingKind {
    capabilities
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_ref())
        .filter(|encodings| {
            encodings
                .iter()
                .any(|encoding| encoding == &PositionEncodingKind::UTF8)
        })
        .map_or(PositionEncodingKind::UTF16, |_| PositionEncodingKind::UTF8)
}

fn normalize_existing_path(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or_else(|_| normalize_path(&path))
}

fn owning_workspace(workspaces: &BTreeMap<PathBuf, Workspace>, path: &Path) -> Option<PathBuf> {
    let path = normalize_existing_path(path.to_path_buf());
    workspaces
        .keys()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

fn origin_for_uri(uri: &Uri) -> SourceOrigin {
    uri.to_file_path()
        .map(|path| SourceOrigin::File(normalize_existing_path(path.into_owned())))
        .unwrap_or_else(|| SourceOrigin::Memory(uri.as_str().to_owned()))
}

fn uri_for_origin(origin: &SourceOrigin) -> Option<Uri> {
    Uri::from_str(&origin.canonical_uri()).ok()
}

fn semantic_input(state: &ServerState, workspace_root: &Path) -> Option<SemanticInput> {
    let workspace = state.workspaces.get(workspace_root)?.clone();
    let mut origins = workspace.disk_sources();
    let mut open_documents = BTreeMap::new();
    let mut versions = BTreeMap::new();
    for document in state.documents.values() {
        let origin = origin_for_uri(&document.uri);
        if !origin_is_in_workspace(&origin, workspace_root) {
            continue;
        }
        origins.insert(origin.clone());
        versions.insert(document.uri.clone(), document.version);
        open_documents.insert(
            origin.clone(),
            DocumentSnapshot::new(
                origin,
                SourceRevision::from_text(&document.text),
                document.text.clone(),
            ),
        );
    }
    origins.extend(workspace.ambient_modules.iter().cloned());
    let roots = origins
        .iter()
        .filter(|origin| !workspace.ambient_modules.contains(*origin))
        .cloned()
        .map(ModuleRoot::requested)
        .chain(
            workspace
                .ambient_modules
                .iter()
                .cloned()
                .map(ModuleRoot::ambient_data),
        )
        .collect();
    let generation = AnalysisGeneration::new(
        state
            .workspace_generations
            .get(workspace_root)
            .copied()
            .unwrap_or_default(),
    );
    let native_registry_profile = workspace
        .service
        .compiler()
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    Some(SemanticInput {
        workspace,
        snapshot: WorkspaceSnapshot {
            generation,
            project_root: workspace_root.to_path_buf(),
            roots,
            open_documents,
            known_disk_sources: origins.into_iter().collect(),
            native_registry_profile,
        },
        versions,
    })
}

fn configured_ambient_modules(
    workspace_root: &Path,
    settings: &AvengerSettings,
) -> BTreeSet<SourceOrigin> {
    settings
        .ambient_modules
        .iter()
        .filter_map(|configured| {
            if configured.starts_with("file:") {
                return Uri::from_str(configured)
                    .ok()?
                    .to_file_path()
                    .map(|path| SourceOrigin::File(normalize_existing_path(path.into_owned())));
            }
            let path = Path::new(configured);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace_root.join(path)
            };
            Some(SourceOrigin::File(normalize_existing_path(path)))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WatcherSourceUpdate {
    None,
    Paths(Vec<PathBuf>),
    Rescan,
}

fn watcher_source_update(root: &Path, event: &NotifyEvent) -> WatcherSourceUpdate {
    if event.need_rescan() {
        return WatcherSourceUpdate::Rescan;
    }
    let paths = event
        .paths
        .iter()
        .filter(|path| !workspace_path_is_ignored(root, path))
        .cloned()
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return WatcherSourceUpdate::None;
    }
    let directory_change = matches!(
        event.kind,
        NotifyEventKind::Create(CreateKind::Folder)
            | NotifyEventKind::Remove(RemoveKind::Folder)
            | NotifyEventKind::Modify(ModifyKind::Name(_))
    ) && paths.iter().any(|path| !is_avenger_source_path(path));
    if directory_change {
        return WatcherSourceUpdate::Rescan;
    }
    let paths = paths
        .into_iter()
        .filter(|path| is_avenger_source_path(path))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        WatcherSourceUpdate::None
    } else {
        WatcherSourceUpdate::Paths(paths)
    }
}

fn is_avenger_source_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("avenger"))
}

fn workspace_path_is_ignored(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    relative.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name.starts_with('.') || matches!(name.as_ref(), "target" | "node_modules")
    })
}

fn scan_avenger_files(root: &Path) -> BTreeSet<SourceOrigin> {
    fn visit(root: &Path, path: &Path, depth: usize, output: &mut BTreeSet<SourceOrigin>) {
        if depth > 64 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if !workspace_path_is_ignored(root, &path) {
                    visit(root, &path, depth + 1, output);
                }
            } else if file_type.is_file() && is_avenger_source_path(&path) {
                output.insert(SourceOrigin::File(normalize_existing_path(path)));
            }
        }
    }
    let mut output = BTreeSet::new();
    visit(root, root, 0, &mut output);
    output
}

fn detected_line_ending(text: &str) -> LineEnding {
    if text.contains("\r\n") {
        LineEnding::Crlf
    } else if text.contains('\r') {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::KEYWORD,
            SemanticTokenType::PARAMETER,
            SemanticTokenType::VARIABLE,
            SemanticTokenType::PROPERTY,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::TYPE,
            SemanticTokenType::NAMESPACE,
        ],
        token_modifiers: vec![
            SemanticTokenModifier::DECLARATION,
            SemanticTokenModifier::READONLY,
            SemanticTokenModifier::DEPRECATED,
            SemanticTokenModifier::DEFAULT_LIBRARY,
        ],
    }
}

fn encode_semantic_tokens(
    tokens: &[avenger_lang_analysis::SemanticToken],
    positions: &PositionIndex,
) -> Vec<SemanticToken> {
    let mut output = Vec::new();
    let mut previous_line = 0;
    let mut previous_start = 0;
    for token in tokens {
        let Ok(range) = positions.lsp_range(token.span.range.as_range()) else {
            continue;
        };
        if range.start.line != range.end.line || range.start.character == range.end.character {
            continue;
        }
        let delta_line = range.start.line.saturating_sub(previous_line);
        let delta_start = if delta_line == 0 {
            range.start.character.saturating_sub(previous_start)
        } else {
            range.start.character
        };
        output.push(SemanticToken {
            delta_line,
            delta_start,
            length: range.end.character - range.start.character,
            token_type: semantic_token_type(token.kind),
            token_modifiers_bitset: semantic_token_modifiers(token.modifiers),
        });
        previous_line = range.start.line;
        previous_start = range.start.character;
    }
    output
}

fn semantic_token_type(kind: AvengerSemanticTokenKind) -> u32 {
    match kind {
        AvengerSemanticTokenKind::Keyword => 0,
        AvengerSemanticTokenKind::Parameter => 1,
        AvengerSemanticTokenKind::Binding
        | AvengerSemanticTokenKind::Variable
        | AvengerSemanticTokenKind::UnresolvedReference => 2,
        AvengerSemanticTokenKind::Property | AvengerSemanticTokenKind::Field => 3,
        AvengerSemanticTokenKind::Function => 4,
        AvengerSemanticTokenKind::Type => 5,
        AvengerSemanticTokenKind::Namespace => 6,
    }
}

fn semantic_token_modifiers(modifiers: AvengerSemanticTokenModifiers) -> u32 {
    u32::from(modifiers.declaration)
        | (u32::from(modifiers.readonly) << 1)
        | (u32::from(modifiers.deprecated) << 2)
        | (u32::from(modifiers.default_library) << 3)
}

fn rename_error(error: RenameError) -> jsonrpc::Error {
    jsonrpc::Error::invalid_params(error.to_string())
}

fn origin_is_in_workspace(origin: &SourceOrigin, workspace_root: &Path) -> bool {
    matches!(origin, SourceOrigin::File(path) if path.starts_with(workspace_root))
}

fn semantic_diagnostics(
    analysis: &WorkspaceAnalysis,
    encoding: PositionEncoding,
) -> BTreeMap<Uri, Vec<Diagnostic>> {
    let mut output = BTreeMap::new();
    for root in analysis.semantic_roots.values() {
        let Err(failure) = &root.result else {
            continue;
        };
        append_compile_failure(&mut output, failure, encoding);
    }
    output
}

async fn analyze_on_worker(
    service: AnalysisService,
    snapshot: WorkspaceSnapshot,
    cancellation: AnalysisCancellation,
) -> Result<WorkspaceAnalysis, avenger_lang_analysis::AnalysisCancelled> {
    // DataFusion planning currently contains non-Send futures. Keep that
    // implementation detail off the protocol runtime by owning each semantic
    // generation on a current-thread executor hosted by a blocking worker.
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build semantic analysis runtime")
            .block_on(service.analyze_workspace(snapshot, &cancellation))
    })
    .await
    .unwrap_or(Err(avenger_lang_analysis::AnalysisCancelled))
}

fn append_compile_failure(
    output: &mut BTreeMap<Uri, Vec<Diagnostic>>,
    failure: &CompileFailure,
    encoding: PositionEncoding,
) {
    for diagnostic in &failure.diagnostics {
        let Some(primary_source) = failure.sources.get(diagnostic.primary.span.source) else {
            continue;
        };
        let Some(uri) = uri_for_origin(&primary_source.origin) else {
            continue;
        };
        if let Some(converted) =
            lsp_diagnostic(diagnostic, primary_source, &failure.sources, encoding)
        {
            output.entry(uri).or_default().push(converted);
        }
    }
}

fn lsp_diagnostic_for_single_source(
    diagnostic: &AvengerDiagnostic,
    uri: &Uri,
    positions: &PositionIndex,
) -> Option<Diagnostic> {
    let range = positions
        .lsp_range(diagnostic.primary.span.range.as_range())
        .ok()?;
    let related = diagnostic
        .secondary
        .iter()
        .map(|label| (label.span.range.as_range(), label.message.clone()))
        .chain(
            diagnostic
                .trace
                .iter()
                .map(|frame| (frame.span.range.as_range(), frame.message.clone())),
        )
        .filter_map(|(range, message)| {
            Some(DiagnosticRelatedInformation {
                location: Location::new(uri.clone(), positions.lsp_range(range).ok()?),
                message,
            })
        })
        .collect::<Vec<_>>();
    Some(make_diagnostic(diagnostic, range, related))
}

fn lsp_diagnostic(
    diagnostic: &AvengerDiagnostic,
    primary_source: &SourceFile,
    sources: &SourceMap,
    encoding: PositionEncoding,
) -> Option<Diagnostic> {
    let primary_index = PositionIndex::new(Arc::from(primary_source.text()), encoding);
    let range = primary_index
        .lsp_range(diagnostic.primary.span.range.as_range())
        .ok()?;
    let related = diagnostic
        .secondary
        .iter()
        .map(|label| (label.span, label.message.clone()))
        .chain(
            diagnostic
                .trace
                .iter()
                .map(|frame| (frame.span, frame.message.clone())),
        )
        .filter_map(|(span, message)| {
            let source = sources.get(span.source)?;
            let uri = uri_for_origin(&source.origin)?;
            let positions = PositionIndex::new(Arc::from(source.text()), encoding);
            Some(DiagnosticRelatedInformation {
                location: Location::new(uri, positions.lsp_range(span.range.as_range()).ok()?),
                message,
            })
        })
        .collect::<Vec<_>>();
    Some(make_diagnostic(diagnostic, range, related))
}

fn make_diagnostic(
    diagnostic: &AvengerDiagnostic,
    range: Range,
    related: Vec<DiagnosticRelatedInformation>,
) -> Diagnostic {
    let mut message = diagnostic.message.clone();
    if !diagnostic.primary.message.is_empty() && diagnostic.primary.message != diagnostic.message {
        message.push_str("\n\n");
        message.push_str(&diagnostic.primary.message);
    }
    for note in &diagnostic.notes {
        message.push_str("\n\nNote: ");
        message.push_str(note);
    }
    Diagnostic::new(
        range,
        Some(match diagnostic.severity {
            AvengerDiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
            AvengerDiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
            AvengerDiagnosticSeverity::Information => DiagnosticSeverity::INFORMATION,
            AvengerDiagnosticSeverity::Hint => DiagnosticSeverity::HINT,
        }),
        Some(NumberOrString::String(diagnostic.code.as_str().to_owned())),
        Some(SERVER_NAME.to_owned()),
        message,
        (!related.is_empty()).then_some(related),
        None,
    )
}

fn nested_symbol(
    symbol: &avenger_lang_analysis::DocumentSymbol,
    positions: &PositionIndex,
) -> Option<tower_lsp_server::ls_types::DocumentSymbol> {
    #[allow(deprecated)]
    Some(tower_lsp_server::ls_types::DocumentSymbol {
        name: symbol.name.clone(),
        detail: symbol.detail.clone(),
        kind: symbol_kind(symbol.kind),
        tags: None,
        deprecated: None,
        range: positions.lsp_range(symbol.span.range.as_range()).ok()?,
        selection_range: positions
            .lsp_range(symbol.selection_span.range.as_range())
            .ok()?,
        children: Some(
            symbol
                .children
                .iter()
                .filter_map(|child| nested_symbol(child, positions))
                .collect(),
        ),
    })
}

fn flatten_symbols(
    symbols: &[avenger_lang_analysis::DocumentSymbol],
    parent: Option<&str>,
    uri: &Uri,
    positions: &PositionIndex,
    output: &mut Vec<SymbolInformation>,
) {
    for symbol in symbols {
        let Ok(range) = positions.lsp_range(symbol.selection_span.range.as_range()) else {
            continue;
        };
        #[allow(deprecated)]
        output.push(SymbolInformation {
            name: symbol.name.clone(),
            kind: symbol_kind(symbol.kind),
            tags: None,
            deprecated: None,
            location: Location::new(uri.clone(), range),
            container_name: parent.map(str::to_owned),
        });
        flatten_symbols(&symbol.children, Some(&symbol.name), uri, positions, output);
    }
}

fn symbol_kind(kind: avenger_lang_analysis::SymbolKind) -> SymbolKind {
    use avenger_lang_analysis::SymbolKind as A;
    match kind {
        A::Chart | A::Mark | A::Widget | A::View => SymbolKind::OBJECT,
        A::Definition | A::Transform | A::Tool => SymbolKind::FUNCTION,
        A::Catalog | A::Schema => SymbolKind::NAMESPACE,
        A::Table | A::Store => SymbolKind::ARRAY,
        A::Param | A::Selection => SymbolKind::VARIABLE,
        A::Event => SymbolKind::EVENT,
        A::Field | A::Property => SymbolKind::FIELD,
    }
}

fn completion_item(
    item: avenger_lang_analysis::CompletionItem,
    positions: &PositionIndex,
) -> Result<tower_lsp_server::ls_types::CompletionItem, String> {
    let range = positions
        .lsp_range(item.replacement.range.as_range())
        .map_err(|error| {
            INVALID_COMPLETION_RANGES.fetch_add(1, Ordering::Relaxed);
            error.to_string()
        })?;
    Ok(tower_lsp_server::ls_types::CompletionItem {
        label: item.label,
        kind: Some(match item.semantic_kind {
            AnalysisCompletionSemanticKind::DslKeyword
            | AnalysisCompletionSemanticKind::SqlKeyword
            | AnalysisCompletionSemanticKind::SqlOperator => CompletionItemKind::KEYWORD,
            AnalysisCompletionSemanticKind::Declaration => CompletionItemKind::CLASS,
            AnalysisCompletionSemanticKind::NativeKind => CompletionItemKind::CONSTRUCTOR,
            AnalysisCompletionSemanticKind::Property
            | AnalysisCompletionSemanticKind::ContextualMember => CompletionItemKind::PROPERTY,
            AnalysisCompletionSemanticKind::EnumValue => CompletionItemKind::ENUM_MEMBER,
            AnalysisCompletionSemanticKind::ScalarParam
            | AnalysisCompletionSemanticKind::SelectionParam => CompletionItemKind::VARIABLE,
            AnalysisCompletionSemanticKind::StoreParam => CompletionItemKind::STRUCT,
            AnalysisCompletionSemanticKind::DataColumn
            | AnalysisCompletionSemanticKind::StructField => CompletionItemKind::FIELD,
            AnalysisCompletionSemanticKind::ContextualRoot => CompletionItemKind::VARIABLE,
            AnalysisCompletionSemanticKind::WindowName => CompletionItemKind::VARIABLE,
            AnalysisCompletionSemanticKind::ScalarFunction
            | AnalysisCompletionSemanticKind::AggregateFunction
            | AnalysisCompletionSemanticKind::WindowFunction
            | AnalysisCompletionSemanticKind::TableFunction => CompletionItemKind::FUNCTION,
            AnalysisCompletionSemanticKind::SqlType => CompletionItemKind::TYPE_PARAMETER,
            AnalysisCompletionSemanticKind::Relation => CompletionItemKind::STRUCT,
            AnalysisCompletionSemanticKind::Catalog | AnalysisCompletionSemanticKind::Schema => {
                CompletionItemKind::MODULE
            }
        }),
        detail: item.detail,
        documentation: item.documentation.map(|value| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            })
        }),
        deprecated: item.deprecated.then_some(true),
        sort_text: Some(item.sort_key),
        filter_text: item.filter_text,
        insert_text_format: Some(match item.insert_text_format {
            CompletionTextFormat::PlainText => InsertTextFormat::PLAIN_TEXT,
            CompletionTextFormat::Snippet => InsertTextFormat::SNIPPET,
        }),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: item.insert_text,
        })),
        ..Default::default()
    })
}

fn navigation_locations(
    analysis: &WorkspaceAnalysis,
    targets: Vec<avenger_lang_analysis::NavigationTarget>,
    encoding: PositionEncoding,
) -> Vec<Location> {
    targets
        .into_iter()
        .filter_map(|target| {
            let uri = uri_for_origin(&target.origin)?;
            let text = source_text_for_origin(analysis, &target.origin)?;
            let positions = PositionIndex::new(text, encoding);
            let range = positions
                .lsp_range(target.selection_span.range.as_range())
                .ok()?;
            Some(Location::new(uri, range))
        })
        .collect()
}

fn source_text_for_origin(analysis: &WorkspaceAnalysis, origin: &SourceOrigin) -> Option<Arc<str>> {
    if let Some(syntax) = analysis.syntax.get(origin) {
        return Some(Arc::from(syntax.parsed.tokens.text()));
    }
    for root in analysis.semantic_roots.values() {
        let Ok(project) = &root.result else {
            continue;
        };
        if let Some(source) = project
            .sources
            .iter()
            .map(|(_, source)| source)
            .find(|source| &source.origin == origin)
        {
            return Some(Arc::from(source.text()));
        }
    }
    match origin {
        SourceOrigin::File(path) => std::fs::read_to_string(path).ok().map(Arc::from),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        sync::{Arc, Mutex as StdMutex, atomic::Ordering},
        time::Duration,
    };

    use avenger_lang_analysis::{
        AnalysisCancellation, AnalysisGeneration, AnalysisService,
        CompletionItem as AnalysisCompletionItem, CompletionKind as AnalysisCompletionKind,
        CompletionOrigin as AnalysisCompletionOrigin,
        CompletionQualification as AnalysisCompletionQualification, CompletionSemanticKind,
        CompletionTextFormat as AnalysisCompletionTextFormat,
        CompletionValidity as AnalysisCompletionValidity,
    };
    use avenger_lang_compiler::Compiler;
    use avenger_lang_core::{ByteSpan, SourceId, SourceOrigin, SourceSpan};
    use futures::StreamExt;
    use serde_json::json;
    use tempfile::tempdir;
    use tower::{Service, ServiceExt};
    use tower_lsp_server::{ClientSocket, LspService, jsonrpc::Request, ls_types::*};

    use super::{
        Backend, CANCELLED_COMPLETION_WORKERS, CancelAnalysisOnDrop, INVALID_COMPLETION_RANGES,
        LspServerConfig, WatcherSourceUpdate, Workspace, completion_item, normalize_existing_path,
        position::{PositionEncoding, PositionIndex},
        scan_avenger_files, watcher_source_update,
    };

    #[test]
    fn watcher_updates_only_avenger_sources_and_ignores_build_artifacts() {
        use notify::{
            Event, EventKind,
            event::{CreateKind, DataChange, ModifyKind},
        };

        let root = std::path::Path::new("/workspace");
        let rust = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
            .add_path(root.join("src/lib.rs"));
        assert_eq!(
            watcher_source_update(root, &rust),
            WatcherSourceUpdate::None
        );

        let build_output = Event::new(EventKind::Create(CreateKind::File))
            .add_path(root.join("target/generated.avenger"));
        assert_eq!(
            watcher_source_update(root, &build_output),
            WatcherSourceUpdate::None
        );

        let chart = root.join("charts/main.avenger");
        let source = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
            .add_path(chart.clone());
        assert_eq!(
            watcher_source_update(root, &source),
            WatcherSourceUpdate::Paths(vec![chart])
        );

        let directory =
            Event::new(EventKind::Create(CreateKind::Folder)).add_path(root.join("charts/new"));
        assert_eq!(
            watcher_source_update(root, &directory),
            WatcherSourceUpdate::Rescan
        );
    }

    #[test]
    fn workspace_source_inventory_is_cached_and_incrementally_updated() {
        let directory = tempdir().unwrap();
        let chart = directory.path().join("chart.avenger");
        fs::write(&chart, "avenger 1; chart cartesian {}").unwrap();
        fs::create_dir(directory.path().join("target")).unwrap();
        fs::write(
            directory.path().join("target/generated.avenger"),
            "avenger 1; chart cartesian {}",
        )
        .unwrap();

        let scanned = scan_avenger_files(directory.path());
        assert_eq!(scanned.len(), 1);
        assert!(scanned.contains(&SourceOrigin::File(normalize_existing_path(chart.clone()))));

        let workspace = Workspace::new(directory.path().to_path_buf()).unwrap();
        assert_eq!(workspace.disk_sources().len(), 1);
        let second = directory.path().join("second.avenger");
        fs::write(&second, "avenger 1; chart polar {}").unwrap();
        workspace.update_disk_sources(vec![second.clone()]);
        assert!(
            workspace
                .disk_sources()
                .contains(&SourceOrigin::File(normalize_existing_path(second.clone())))
        );
        fs::remove_file(&second).unwrap();
        workspace.update_disk_sources(vec![second.clone()]);
        assert!(
            !workspace
                .disk_sources()
                .contains(&SourceOrigin::File(normalize_existing_path(second)))
        );
    }

    #[test]
    fn dropping_a_completion_request_cancels_its_worker_token() {
        let cancellation = AnalysisCancellation::default();
        {
            let _guard = CancelAnalysisOnDrop::new(cancellation.clone());
            assert!(!cancellation.is_cancelled());
        }
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn cancelling_a_request_future_cancels_the_completion_worker() {
        let cancellation = AnalysisCancellation::default();
        let worker_cancellation = cancellation.clone();
        let before = CANCELLED_COMPLETION_WORKERS.load(Ordering::Relaxed);
        let task = tokio::spawn(async move {
            let _guard = CancelAnalysisOnDrop::new(worker_cancellation);
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        assert!(cancellation.is_cancelled());
        assert_eq!(
            CANCELLED_COMPLETION_WORKERS.load(Ordering::Relaxed),
            before + 1
        );
    }

    #[test]
    fn completed_completion_request_does_not_report_cancellation() {
        let cancellation = AnalysisCancellation::default();
        let before = CANCELLED_COMPLETION_WORKERS.load(Ordering::Relaxed);
        {
            let mut guard = CancelAnalysisOnDrop::new(cancellation.clone());
            guard.disarm();
        }
        assert!(!cancellation.is_cancelled());
        assert_eq!(CANCELLED_COMPLETION_WORKERS.load(Ordering::Relaxed), before);
    }

    #[test]
    fn completion_wire_edits_preserve_quoted_unicode_and_count_invalid_ranges() {
        fn column(
            source: SourceId,
            range: std::ops::Range<usize>,
            label: &str,
            insert: &str,
        ) -> AnalysisCompletionItem {
            AnalysisCompletionItem {
                label: label.to_owned(),
                replacement: SourceSpan {
                    source,
                    range: ByteSpan {
                        start: range.start,
                        end: range.end,
                    },
                },
                insert_text: insert.to_owned(),
                insert_text_format: AnalysisCompletionTextFormat::PlainText,
                kind: AnalysisCompletionKind::Field,
                semantic_kind: CompletionSemanticKind::DataColumn,
                semantic_identity: format!("column:{label}"),
                match_text: label.to_owned(),
                qualification: AnalysisCompletionQualification::Unqualified,
                data_type: Some("Utf8".to_owned()),
                nullable: Some(true),
                source_stage: Some("test".to_owned()),
                expected_type_compatible: None,
                confidence: 100,
                semantic_proximity: 100,
                usage_prevalence: 0,
                validity: AnalysisCompletionValidity::Strict,
                detail: Some("Utf8 · nullable".to_owned()),
                documentation: None,
                filter_text: Some(insert.to_owned()),
                sort_key: "00".to_owned(),
                origin: AnalysisCompletionOrigin::DatasetSchema,
                deprecated: false,
            }
        }

        let text: Arc<str> = Arc::from("😀\r\nSELECT w.\"café\", w.\"a\"\"b\"");
        let positions = PositionIndex::new(Arc::clone(&text), PositionEncoding::Utf16);
        let source = SourceId::new(7);
        for (authored, label, insert) in [
            ("\"café\"", "café", "\"café\""),
            ("\"a\"\"b\"", "a\"b", "\"a\"\"b\""),
        ] {
            let start = text.find(authored).unwrap();
            let item = completion_item(
                column(source, start..start + authored.len(), label, insert),
                &positions,
            )
            .unwrap();
            assert_eq!(item.filter_text.as_deref(), Some(insert));
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!("expected text edit")
            };
            assert_eq!(edit.new_text, insert);
            assert_eq!(edit.range.start.line, 1);
            assert_eq!(
                edit.range.start.character,
                text[text.find("SELECT").unwrap()..start]
                    .encode_utf16()
                    .count() as u32
            );
        }

        let before = INVALID_COMPLETION_RANGES.load(Ordering::Relaxed);
        let invalid = column(source, text.len()..text.len() + 1, "bad", "\"bad\"");
        assert!(completion_item(invalid, &positions).is_err());
        assert_eq!(
            INVALID_COMPLETION_RANGES.load(Ordering::Relaxed),
            before + 1
        );
    }

    #[tokio::test]
    async fn initializes_and_shuts_down_in_memory() {
        let (mut service, _socket) = LspService::new(Backend::new);
        let initialize = Request::build("initialize")
            .id(1)
            .params(json!({
                "capabilities": {
                    "general": { "positionEncodings": ["utf-8", "utf-16"] },
                    "textDocument": {
                        "documentSymbol": { "hierarchicalDocumentSymbolSupport": true }
                    }
                }
            }))
            .finish();
        let response = service
            .ready()
            .await
            .expect("initialize service ready")
            .call(initialize)
            .await
            .expect("initialize service call")
            .expect("initialize response");
        let result: InitializeResult = serde_json::from_value(
            serde_json::to_value(response.result().expect("initialize result"))
                .expect("serialize initialize result"),
        )
        .expect("deserialize initialize result");
        assert_eq!(
            result.capabilities.position_encoding,
            Some(PositionEncodingKind::UTF8)
        );
        assert!(result.capabilities.text_document_sync.is_some());
        assert!(result.capabilities.document_formatting_provider.is_some());
        assert!(result.capabilities.semantic_tokens_provider.is_some());
        assert!(result.capabilities.rename_provider.is_some());
        assert!(result.capabilities.workspace_symbol_provider.is_some());
        assert!(result.capabilities.code_lens_provider.is_some());
        let completion = result
            .capabilities
            .completion_provider
            .as_ref()
            .expect("completion capabilities");
        assert_eq!(completion.resolve_provider, Some(false));
        let expected_triggers = ["$", ".", "@", ":", "\"", "'"].map(str::to_owned);
        assert_eq!(
            completion.trigger_characters.as_deref(),
            Some(expected_triggers.as_slice())
        );
        assert_eq!(
            result.server_info.as_ref().map(|info| info.name.as_str()),
            Some("avenger-lsp")
        );

        let shutdown = Request::build("shutdown").id(2).finish();
        let response = service
            .ready()
            .await
            .expect("shutdown service ready")
            .call(shutdown)
            .await
            .expect("shutdown service call")
            .expect("shutdown response");
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn chart_code_lenses_and_workspace_symbols_use_module_item_identity() {
        let project = tempdir().unwrap();
        let module = project.path().join("dashboard.avenger");
        let text = r#"avenger 1;

export define mark badge {
  mark symbol {}
}

chart cartesian as overview {
  mark badge {}
}

chart cartesian as detail {
  mark badge {}
}
"#;
        fs::write(&module, text).unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let module_uri = Uri::from_file_path(&module).unwrap();
        let (mut service, mut socket) = LspService::new(Backend::new);
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {},
                    "workspaceFolders": [{ "uri": root_uri, "name": "module" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": module_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": text
                    }
                }))
                .finish(),
        )
        .await;
        let _syntax = next_notification(&mut socket, "textDocument/publishDiagnostics").await;

        let response = call(
            &mut service,
            Request::build("textDocument/codeLens")
                .id(2)
                .params(json!({ "textDocument": { "uri": module_uri } }))
                .finish(),
        )
        .await
        .expect("code lens response");
        let lenses: Vec<CodeLens> =
            serde_json::from_value(serde_json::to_value(response.result().unwrap()).unwrap())
                .unwrap();
        assert_eq!(lenses.len(), 2);
        let selectors = lenses
            .iter()
            .map(|lens| {
                let command = lens.command.as_ref().unwrap();
                assert_eq!(command.command, "avenger.watchChart");
                let payload = command.arguments.as_ref().unwrap()[0].as_object().unwrap();
                assert_eq!(payload["moduleUri"].as_str(), Some(module_uri.as_str()));
                payload["chart"].as_str().unwrap().to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(selectors, ["overview", "detail"]);

        let _semantic = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let response = call(
            &mut service,
            Request::build("workspace/symbol")
                .id(3)
                .params(json!({ "query": "badge" }))
                .finish(),
        )
        .await
        .expect("workspace symbol response");
        let symbols: WorkspaceSymbolResponse =
            serde_json::from_value(serde_json::to_value(response.result().unwrap()).unwrap())
                .unwrap();
        let WorkspaceSymbolResponse::Flat(symbols) = symbols else {
            panic!("expected flat workspace symbols");
        };
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "badge");
        assert!(
            symbols[0]
                .container_name
                .as_deref()
                .is_some_and(|context| context.contains("define"))
        );
    }

    #[tokio::test]
    async fn exported_rename_preserves_each_importers_local_spelling() {
        let project = tempdir().unwrap();
        let library = project.path().join("library.avenger");
        let direct = project.path().join("direct.avenger");
        let aliased = project.path().join("aliased.avenger");
        let qualified = project.path().join("qualified.avenger");
        let library_text = "avenger 1; export define mark badge { mark symbol {} }";
        fs::write(&library, library_text).unwrap();
        fs::write(
            &direct,
            "avenger 1; import { badge } from 'library.avenger'; chart cartesian { mark badge {} }",
        )
        .unwrap();
        fs::write(
            &aliased,
            "avenger 1; import { badge as b } from 'library.avenger'; chart cartesian { mark b {} }",
        )
        .unwrap();
        fs::write(
            &qualified,
            "avenger 1; import * as defs from 'library.avenger'; chart cartesian { mark defs.badge {} }",
        )
        .unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let library_uri = Uri::from_file_path(&library).unwrap();
        let captured = Arc::new(StdMutex::new(None::<Backend>));
        let captured_factory = Arc::clone(&captured);
        let (mut service, mut socket) = LspService::new(move |client| {
            let backend = Backend::new(client);
            *captured_factory.lock().expect("capture backend") = Some(backend.clone());
            backend
        });
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "workspace": {
                            "workspaceEdit": { "documentChanges": true }
                        }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "rename" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": library_uri,
                        "languageId": "avenger",
                        "version": 7,
                        "text": library_text
                    }
                }))
                .finish(),
        )
        .await;
        let _syntax = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let backend = captured
            .lock()
            .expect("captured backend")
            .clone()
            .expect("backend");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !backend
                    .inner
                    .state
                    .read()
                    .await
                    .semantic_analysis
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("semantic analysis timeout");

        let response = call(
            &mut service,
            Request::build("textDocument/rename")
                .id(2)
                .params(json!({
                    "textDocument": { "uri": library_uri },
                    "position": {
                        "line": 0,
                        "character": library_text.find("badge").unwrap() + 1
                    },
                    "newName": "status_badge"
                }))
                .finish(),
        )
        .await
        .expect("rename response");
        let edit: WorkspaceEdit =
            serde_json::from_value(serde_json::to_value(response.result().unwrap()).unwrap())
                .unwrap();
        let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
            panic!("expected versioned document edits");
        };
        assert_eq!(documents.len(), 4);
        let edits = documents
            .iter()
            .map(|document| {
                (
                    document.text_document.uri.clone(),
                    document
                        .edits
                        .iter()
                        .map(|edit| match edit {
                            OneOf::Left(edit) => edit.new_text.clone(),
                            OneOf::Right(edit) => edit.text_edit.new_text.clone(),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let canonical_uri =
            |path: &std::path::Path| Uri::from_file_path(fs::canonicalize(path).unwrap()).unwrap();
        assert!(
            edits[&canonical_uri(&direct)]
                .iter()
                .any(|text| text == "status_badge as badge")
        );
        assert!(
            edits[&canonical_uri(&aliased)]
                .iter()
                .any(|text| text == "status_badge")
        );
        assert!(
            edits[&canonical_uri(&qualified)]
                .iter()
                .any(|text| text == "status_badge")
        );
    }

    #[tokio::test]
    async fn defaults_to_utf16_when_client_omits_position_encodings() {
        let (mut service, _socket) = LspService::new(Backend::new);
        let initialize = Request::build("initialize")
            .id(1)
            .params(json!({ "capabilities": {} }))
            .finish();
        let response = service
            .ready()
            .await
            .expect("initialize service ready")
            .call(initialize)
            .await
            .expect("initialize service call")
            .expect("initialize response");
        let result: InitializeResult = serde_json::from_value(
            serde_json::to_value(response.result().expect("initialize result"))
                .expect("serialize initialize result"),
        )
        .expect("deserialize initialize result");
        assert_eq!(
            result.capabilities.position_encoding,
            Some(PositionEncodingKind::UTF16)
        );
    }

    #[tokio::test]
    async fn utf16_transcript_preserves_complex_type_and_unicode_name_spans() {
        let project = tempdir().unwrap();
        let chart = project.path().join("unicode.avenger");
        let text = "avenger 1; chart cartesian as chart { title: '😀'; param CAST(NULL AS DOUBLE) as café; store as rows { field struct(field(float64, 'x')) données; } mark symbol { size: encoded $café; } }";
        fs::write(&chart, text).unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let (mut service, mut socket) = LspService::new(Backend::new);

        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {},
                    "workspaceFolders": [{ "uri": root_uri, "name": "unicode" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": text
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;

        let reference_start = text.rfind("$café").unwrap();
        let reference_cursor = reference_start + "$ca".len();
        let character = text[..reference_cursor].encode_utf16().count() as u32;
        let hover = call(
            &mut service,
            Request::build("textDocument/hover")
                .id(2)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": character }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let hover: Hover =
            serde_json::from_value(serde_json::to_value(hover.result().unwrap()).unwrap()).unwrap();
        let range = hover.range.expect("hover range");
        assert_eq!(
            range.start.character,
            text[..reference_start].encode_utf16().count() as u32
        );
        assert_eq!(
            range.end.character,
            text[..reference_start + "$café".len()]
                .encode_utf16()
                .count() as u32
        );

        let symbols = call(
            &mut service,
            Request::build("textDocument/documentSymbol")
                .id(3)
                .params(json!({ "textDocument": { "uri": chart_uri } }))
                .finish(),
        )
        .await
        .unwrap();
        let symbols = serde_json::to_string(symbols.result().unwrap()).unwrap();
        assert!(symbols.contains("café"), "{symbols}");
        assert!(symbols.contains("données"), "{symbols}");

        let semantic = call(
            &mut service,
            Request::build("textDocument/semanticTokens/full")
                .id(4)
                .params(json!({ "textDocument": { "uri": chart_uri } }))
                .finish(),
        )
        .await
        .unwrap();
        let semantic: SemanticTokensResult =
            serde_json::from_value(serde_json::to_value(semantic.result().unwrap()).unwrap())
                .unwrap();
        assert!(matches!(
            semantic,
            SemanticTokensResult::Tokens(tokens) if !tokens.data.is_empty()
        ));
    }

    #[test]
    fn file_uri_normalization_round_trips_special_paths_and_symlinks() {
        let directory = tempdir().unwrap();
        for name in [
            "space chart.avenger",
            "unicode-é中.avenger",
            "hash#question?.avenger",
        ] {
            let path = directory.path().join(name);
            fs::write(&path, "avenger 1; chart cartesian as chart {}").unwrap();
            let canonical = fs::canonicalize(&path).unwrap();
            let uri = Uri::from_file_path(&path).unwrap();
            let origin = super::origin_for_uri(&uri);
            assert_eq!(origin, SourceOrigin::File(canonical.clone()));
            let round_trip = super::uri_for_origin(&origin).unwrap();
            assert_eq!(round_trip.to_file_path().unwrap().into_owned(), canonical);
        }

        #[cfg(unix)]
        {
            let target = directory.path().join("target.avenger");
            let alias = directory.path().join("alias.avenger");
            fs::write(&target, "avenger 1; chart cartesian as chart {}").unwrap();
            std::os::unix::fs::symlink(&target, &alias).unwrap();
            assert_eq!(
                super::origin_for_uri(&Uri::from_file_path(&alias).unwrap()),
                SourceOrigin::File(fs::canonicalize(target).unwrap())
            );
        }

        let missing = directory.path().join("nested/../missing.avenger");
        assert_eq!(
            super::normalize_existing_path(missing),
            directory.path().join("missing.avenger")
        );
    }

    #[tokio::test]
    async fn configured_document_diagnostic_token_and_request_bounds_are_enforced() {
        let project = tempdir().unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let config = LspServerConfig {
            semantic_debounce: Duration::from_secs(60),
            max_document_bytes: 128,
            max_diagnostics_per_document: 1,
            max_semantic_tokens_per_document: 1,
            max_concurrent_requests: 2,
            ..LspServerConfig::default()
        };
        let captured = Arc::new(StdMutex::new(None::<Backend>));
        let captured_factory = Arc::clone(&captured);
        let (mut service, mut socket) = LspService::new(move |client| {
            let backend = Backend::with_config(client, config);
            *captured_factory.lock().unwrap() = Some(backend.clone());
            backend
        });
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {},
                    "workspaceFolders": [{ "uri": root_uri, "name": "bounded" }]
                }))
                .finish(),
        )
        .await;
        let backend = captured.lock().unwrap().clone().unwrap();
        assert_eq!(backend.inner.request_limit.available_permits(), 2);
        let uri = Uri::from_file_path(project.path().join("bounded-chart.avenger")).unwrap();
        let text = "avenger 1; chart cartesian as chart { mark symbol as points { siez: 1; color: $missing; }";
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": text
                    }
                }))
                .finish(),
        )
        .await;
        let publish = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let diagnostics: PublishDiagnosticsParams =
            serde_json::from_value(publish.params().cloned().unwrap()).unwrap();
        assert_eq!(diagnostics.diagnostics.len(), 1);

        let semantic = call(
            &mut service,
            Request::build("textDocument/semanticTokens/full")
                .id(2)
                .params(json!({ "textDocument": { "uri": uri } }))
                .finish(),
        )
        .await
        .unwrap();
        let semantic: SemanticTokensResult =
            serde_json::from_value(serde_json::to_value(semantic.result().unwrap()).unwrap())
                .unwrap();
        assert!(matches!(
            semantic,
            SemanticTokensResult::Tokens(tokens) if tokens.data.len() <= 1
        ));

        let oversized_uri =
            Uri::from_file_path(project.path().join("oversized-chart.avenger")).unwrap();
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": oversized_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": "x".repeat(129)
                    }
                }))
                .finish(),
        )
        .await;
        assert!(backend.document(&oversized_uri).await.is_none());

        let shutdown = call(&mut service, Request::build("shutdown").id(3).finish())
            .await
            .unwrap();
        assert!(shutdown.is_ok());
        assert!(backend.inner.semantic_tasks.lock().await.is_empty());
        assert!(backend.inner.watchers.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rapid_edits_cancel_prior_generation_and_shutdown_releases_workspace_resources() {
        let project = tempdir().unwrap();
        let root = fs::canonicalize(project.path()).unwrap();
        let chart = root.join("chart.avenger");
        let second_chart = root.join("second.avenger");
        let initial = "avenger 1; chart cartesian as chart {}";
        fs::write(&chart, initial).unwrap();
        fs::write(&second_chart, initial).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let second_chart_uri = Uri::from_file_path(&second_chart).unwrap();
        let root_uri = Uri::from_file_path(&root).unwrap();
        let config = LspServerConfig {
            semantic_debounce: Duration::from_secs(60),
            ..LspServerConfig::default()
        };
        let captured = Arc::new(StdMutex::new(None::<Backend>));
        let captured_factory = Arc::clone(&captured);
        let (mut service, mut socket) = LspService::new(move |client| {
            let backend = Backend::with_config(client, config);
            *captured_factory.lock().unwrap() = Some(backend.clone());
            backend
        });
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {},
                    "workspaceFolders": [{ "uri": root_uri, "name": "rapid" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": initial
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let backend = captured.lock().unwrap().clone().unwrap();
        let first_cancellation = backend
            .inner
            .semantic_tasks
            .lock()
            .await
            .get(&root)
            .unwrap()
            .cancellation
            .clone();
        let (initial_document, _, initial_request, _, initial_generation) = backend
            .query_snapshot(&chart_uri, Position::new(0, 0))
            .await
            .unwrap();
        assert!(
            backend
                .completion_snapshot_is_current(
                    &chart_uri,
                    initial_document.version,
                    &initial_request.source_revision,
                    initial_generation,
                )
                .await
        );

        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": second_chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": initial
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        assert!(
            !backend
                .completion_snapshot_is_current(
                    &chart_uri,
                    initial_document.version,
                    &initial_request.source_revision,
                    initial_generation,
                )
                .await,
            "a sibling edit must stale the captured workspace generation"
        );
        let (pre_edit_document, _, pre_edit_request, _, pre_edit_generation) = backend
            .query_snapshot(&chart_uri, Position::new(0, 0))
            .await
            .unwrap();
        assert!(
            backend
                .completion_snapshot_is_current(
                    &chart_uri,
                    pre_edit_document.version,
                    &pre_edit_request.source_revision,
                    pre_edit_generation,
                )
                .await
        );

        let mut replacement = String::new();
        for version in 2..=12 {
            replacement = format!(
                "avenger 1; chart cartesian as chart {{ mark symbol as points {{ size: encoded {version}; }} }}"
            );
            call(
                &mut service,
                Request::build("textDocument/didChange")
                    .params(json!({
                        "textDocument": { "uri": chart_uri, "version": version },
                        "contentChanges": [{ "text": replacement }]
                    }))
                    .finish(),
            )
            .await;
            let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
            call(
                &mut service,
                Request::build("textDocument/didSave")
                    .params(json!({
                        "textDocument": { "uri": chart_uri },
                        "text": replacement
                    }))
                    .finish(),
            )
            .await;
            let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        }
        call(
            &mut service,
            Request::build("textDocument/didChange")
                .params(json!({
                    "textDocument": { "uri": second_chart_uri, "version": 2 },
                    "contentChanges": [{ "text": replacement }]
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        assert!(first_cancellation.is_cancelled());
        assert_eq!(backend.inner.semantic_tasks.lock().await.len(), 1);
        assert_eq!(backend.inner.watchers.lock().unwrap().len(), 1);
        assert_eq!(backend.document(&chart_uri).await.unwrap().version, 12);
        assert_eq!(
            backend.document(&second_chart_uri).await.unwrap().version,
            2
        );
        assert!(
            !backend
                .completion_snapshot_is_current(
                    &chart_uri,
                    pre_edit_document.version,
                    &pre_edit_request.source_revision,
                    pre_edit_generation,
                )
                .await,
            "an edited document must stale its captured completion snapshot"
        );
        let (_, stale_analysis, _, _, _) = backend
            .query_snapshot(&chart_uri, Position::new(0, 0))
            .await
            .unwrap();
        backend
            .publish_semantic(
                root.clone(),
                AnalysisGeneration::new(1),
                BTreeMap::new(),
                (*stale_analysis).clone(),
            )
            .await;
        assert!(
            backend
                .inner
                .state
                .read()
                .await
                .semantic_analysis
                .is_empty()
        );

        call(
            &mut service,
            Request::build("workspace/didChangeWorkspaceFolders")
                .params(json!({
                    "event": {
                        "added": [],
                        "removed": [{ "uri": root_uri, "name": "rapid" }]
                    }
                }))
                .finish(),
        )
        .await;
        assert!(backend.inner.state.read().await.workspaces.is_empty());
        assert!(backend.inner.semantic_tasks.lock().await.is_empty());
        assert!(backend.inner.watchers.lock().unwrap().is_empty());

        call(
            &mut service,
            Request::build("workspace/didChangeWorkspaceFolders")
                .params(json!({
                    "event": {
                        "added": [{ "uri": root_uri, "name": "rapid" }],
                        "removed": []
                    }
                }))
                .finish(),
        )
        .await;
        assert_eq!(backend.inner.state.read().await.workspaces.len(), 1);
        assert_eq!(backend.inner.watchers.lock().unwrap().len(), 1);

        let shutdown = tokio::time::timeout(
            Duration::from_secs(1),
            call(&mut service, Request::build("shutdown").id(2).finish()),
        )
        .await
        .expect("shutdown must respond before editor timeout")
        .unwrap();
        assert!(shutdown.is_ok());
        assert!(backend.inner.semantic_tasks.lock().await.is_empty());
        assert!(backend.inner.watchers.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transcript_open_diagnostics_symbols_change_close_and_shutdown() {
        let project = tempdir().unwrap();
        let chart = project.path().join("chart.avenger");
        fs::write(&chart, "avenger 1; chart cartesian as chart {").unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let (mut service, mut socket) = LspService::new(Backend::new);

        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "textDocument": {
                            "documentSymbol": {
                                "hierarchicalDocumentSymbolSupport": true
                            }
                        }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": "avenger 1; chart cartesian as chart {"
                    }
                }))
                .finish(),
        )
        .await;

        let publish = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let params: PublishDiagnosticsParams =
            serde_json::from_value(publish.params().cloned().unwrap()).unwrap();
        assert_eq!(params.uri, chart_uri);
        assert_eq!(params.version, Some(1));
        assert!(!params.diagnostics.is_empty());
        assert!(
            params
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.source.as_deref() == Some("avenger-lsp"))
        );

        let symbols = call(
            &mut service,
            Request::build("textDocument/documentSymbol")
                .id(2)
                .params(json!({ "textDocument": { "uri": chart_uri } }))
                .finish(),
        )
        .await
        .expect("document symbols response");
        let symbols: DocumentSymbolResponse =
            serde_json::from_value(serde_json::to_value(symbols.result().unwrap()).unwrap())
                .unwrap();
        assert!(
            matches!(symbols, DocumentSymbolResponse::Nested(symbols) if symbols[0].name == "chart")
        );

        call(
            &mut service,
            Request::build("textDocument/didChange")
                .params(json!({
                    "textDocument": { "uri": chart_uri, "version": 2 },
                    "contentChanges": [{
                        "range": {
                            "start": { "line": 0, "character": 37 },
                            "end": { "line": 0, "character": 37 }
                        },
                        "text": " }"
                    }]
                }))
                .finish(),
        )
        .await;
        let publish = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let params: PublishDiagnosticsParams =
            serde_json::from_value(publish.params().cloned().unwrap()).unwrap();
        assert_eq!(params.version, Some(2));

        call(
            &mut service,
            Request::build("textDocument/didClose")
                .params(json!({ "textDocument": { "uri": chart_uri } }))
                .finish(),
        )
        .await;
        let publish = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let params: PublishDiagnosticsParams =
            serde_json::from_value(publish.params().cloned().unwrap()).unwrap();
        assert_eq!(params.uri, chart_uri);
        assert!(params.diagnostics.is_empty());

        let shutdown = call(&mut service, Request::build("shutdown").id(3).finish())
            .await
            .expect("shutdown response");
        assert!(shutdown.is_ok());
        call(&mut service, Request::build("exit").finish()).await;
    }

    #[tokio::test]
    async fn request_before_initialize_is_rejected() {
        let (mut service, _socket) = LspService::new(Backend::new);
        let response = call(
            &mut service,
            Request::build("textDocument/documentSymbol")
                .id(1)
                .params(json!({
                    "textDocument": { "uri": "file:///tmp/chart.avenger" }
                }))
                .finish(),
        )
        .await
        .expect("error response");
        assert!(response.is_error());
    }

    #[tokio::test]
    async fn transcript_format_semantic_tokens_prepare_and_versioned_rename() {
        let project = tempdir().unwrap();
        let chart = project.path().join("chart.avenger");
        let text = "avenger 1; chart cartesian as chart { param 640.0 as width; mark symbol as points { siez: 12.0; size: encoded $width; } }";
        fs::write(&chart, text).unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let (mut service, _socket) = LspService::new(Backend::new);
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "workspace": {
                            "workspaceEdit": { "documentChanges": true }
                        },
                        "textDocument": {
                            "codeAction": {
                                "codeActionLiteralSupport": {
                                    "codeActionKind": { "valueSet": ["quickfix"] }
                                },
                                "isPreferredSupport": true
                            }
                        }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 7,
                        "text": text
                    }
                }))
                .finish(),
        )
        .await;

        let formatting = call(
            &mut service,
            Request::build("textDocument/formatting")
                .id(2)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "options": { "tabSize": 2, "insertSpaces": true }
                }))
                .finish(),
        )
        .await
        .unwrap();
        assert!(!formatting.is_error(), "{formatting:?}");
        assert!(formatting.result().is_some(), "{formatting:?}");
        let formatting: Vec<TextEdit> =
            serde_json::from_value(serde_json::to_value(formatting.result().unwrap()).unwrap())
                .unwrap();
        assert_eq!(formatting.len(), 1);
        assert!(formatting[0].new_text.contains("param 640.0 as width"));

        let semantic = call(
            &mut service,
            Request::build("textDocument/semanticTokens/full")
                .id(3)
                .params(json!({ "textDocument": { "uri": chart_uri } }))
                .finish(),
        )
        .await
        .unwrap();
        let semantic: SemanticTokensResult =
            serde_json::from_value(serde_json::to_value(semantic.result().unwrap()).unwrap())
                .unwrap();
        assert!(
            matches!(semantic, SemanticTokensResult::Tokens(tokens) if !tokens.data.is_empty())
        );

        let reference_character = text.rfind("width").unwrap() as u32 + 1;
        let prepared = call(
            &mut service,
            Request::build("textDocument/prepareRename")
                .id(4)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": reference_character }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let prepared: PrepareRenameResponse =
            serde_json::from_value(serde_json::to_value(prepared.result().unwrap()).unwrap())
                .unwrap();
        assert!(matches!(
            prepared,
            PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }
                if placeholder == "width"
        ));

        let renamed = call(
            &mut service,
            Request::build("textDocument/rename")
                .id(5)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": reference_character },
                    "newName": "canvas_width"
                }))
                .finish(),
        )
        .await
        .unwrap();
        let renamed: WorkspaceEdit =
            serde_json::from_value(serde_json::to_value(renamed.result().unwrap()).unwrap())
                .unwrap();
        let Some(DocumentChanges::Edits(documents)) = renamed.document_changes else {
            panic!("rename did not return versioned document changes");
        };
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].text_document.version, Some(7));
        assert_eq!(documents[0].edits.len(), 2);

        let typo_start = text.find("siez").unwrap() as u32;
        let actions = call(
            &mut service,
            Request::build("textDocument/codeAction")
                .id(6)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "range": {
                        "start": { "line": 0, "character": typo_start },
                        "end": { "line": 0, "character": typo_start + 4 }
                    },
                    "context": { "diagnostics": [], "only": ["quickfix"] }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let actions: CodeActionResponse =
            serde_json::from_value(serde_json::to_value(actions.result().unwrap()).unwrap())
                .unwrap();
        assert!(actions.iter().any(|action| {
            matches!(
                action,
                CodeActionOrCommand::CodeAction(action)
                    if action.title == "Replace `siez` with `size`"
                        && action.is_preferred == Some(true)
                        && action.edit.as_ref().is_some_and(|edit| {
                            matches!(edit.document_changes, Some(DocumentChanges::Edits(ref docs)) if docs[0].text_document.version == Some(7))
                        })
            )
        }));
    }

    #[tokio::test]
    async fn transcript_extract_and_lazy_pin_import_use_capability_safe_workspace_edits() {
        let project = tempdir().unwrap();
        let root = fs::canonicalize(project.path()).unwrap();
        let chart = root.join("chart.avenger");
        let remote_url = "https://example.test/badge.avenger";
        let text = format!(
            "avenger 1; import {{ badge }} from '{remote_url}'; chart cartesian as chart {{ mark group as cluster {{ mark symbol {{}} }} }}"
        );
        fs::write(&chart, &text).unwrap();
        let root_uri = Uri::from_file_path(&root).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let remote_origin = SourceOrigin::Http(remote_url.to_owned());
        let remote_definition = "avenger 1; export define mark badge { mark symbol {} }";
        let loader = avenger_lang_core::InMemorySourceLoader::default().with_source(
            avenger_lang_core::LoadedSource::new(
                remote_origin,
                remote_definition,
                avenger_lang_core::ContentVersion::new("remote-fixture"),
            ),
        );
        let compiler = Compiler::builder()
            .project_root(&root)
            .source_loader(Arc::new(loader))
            .build()
            .unwrap();
        let captured = Arc::new(StdMutex::new(None::<Backend>));
        let captured_factory = Arc::clone(&captured);
        let (mut service, mut socket) = LspService::new(move |client| {
            let backend = Backend::new(client);
            *captured_factory.lock().unwrap() = Some(backend.clone());
            backend
        });
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "workspace": {
                            "workspaceEdit": {
                                "documentChanges": true,
                                "resourceOperations": ["create"]
                            }
                        },
                        "textDocument": {
                            "codeAction": {
                                "codeActionLiteralSupport": {
                                    "codeActionKind": {
                                        "valueSet": ["quickfix", "refactor", "source"]
                                    }
                                },
                                "dataSupport": true,
                                "resolveSupport": { "properties": ["edit"] }
                            }
                        }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }]
                }))
                .finish(),
        )
        .await;
        let backend = captured.lock().unwrap().clone().unwrap();
        backend.inner.state.write().await.workspaces.insert(
            root.clone(),
            Workspace {
                service: AnalysisService::new(compiler),
                ambient_modules: Default::default(),
                disk_sources: Arc::new(StdMutex::new(scan_avenger_files(&root))),
            },
        );
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": text.clone()
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;

        let url_start = text.find(remote_url).unwrap();
        let offered = call(
            &mut service,
            Request::build("textDocument/codeAction")
                .id(2)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "range": {
                        "start": { "line": 0, "character": url_start },
                        "end": { "line": 0, "character": url_start + remote_url.len() }
                    },
                    "context": { "diagnostics": [], "only": ["source.pinImport"] }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let offered: CodeActionResponse =
            serde_json::from_value(serde_json::to_value(offered.result().unwrap()).unwrap())
                .unwrap();
        let CodeActionOrCommand::CodeAction(pin) = offered.into_iter().next().unwrap() else {
            panic!("expected literal pin action")
        };
        assert!(pin.edit.is_none());
        assert!(pin.data.is_some());
        let stale_pin = pin.clone();
        let resolved = call(
            &mut service,
            Request::build("codeAction/resolve")
                .id(3)
                .params(serde_json::to_value(pin).unwrap())
                .finish(),
        )
        .await
        .unwrap();
        let resolved: tower_lsp_server::ls_types::CodeAction =
            serde_json::from_value(serde_json::to_value(resolved.result().unwrap()).unwrap())
                .unwrap();
        let Some(DocumentChanges::Edits(pin_edits)) = resolved.edit.unwrap().document_changes
        else {
            panic!("pin did not return versioned document edits")
        };
        assert_eq!(pin_edits[0].text_document.version, Some(1));
        assert!(matches!(
            &pin_edits[0].edits[0],
            OneOf::Left(edit) if edit.new_text.starts_with(" sha256 '") && edit.new_text.len() == 74
        ));
        call(
            &mut service,
            Request::build("textDocument/didChange")
                .params(json!({
                    "textDocument": { "uri": chart_uri, "version": 2 },
                    "contentChanges": [{ "text": text.clone() }]
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let stale = call(
            &mut service,
            Request::build("codeAction/resolve")
                .id(31)
                .params(serde_json::to_value(stale_pin).unwrap())
                .finish(),
        )
        .await
        .unwrap();
        assert!(stale.is_error());

        let group_start = text.find("mark group as cluster").unwrap();
        let extracted = call(
            &mut service,
            Request::build("textDocument/codeAction")
                .id(4)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "range": {
                        "start": { "line": 0, "character": group_start },
                        "end": { "line": 0, "character": group_start + "mark group as cluster".len() }
                    },
                    "context": { "diagnostics": [], "only": ["refactor.extract"] }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let extracted: CodeActionResponse =
            serde_json::from_value(serde_json::to_value(extracted.result().unwrap()).unwrap())
                .unwrap();
        let CodeActionOrCommand::CodeAction(extracted) = extracted.into_iter().next().unwrap()
        else {
            panic!("expected literal extract action")
        };
        let Some(DocumentChanges::Operations(operations)) =
            extracted.edit.unwrap().document_changes
        else {
            panic!("extract did not return create-file operations")
        };
        assert!(matches!(
            operations.first(),
            Some(DocumentChangeOperation::Op(ResourceOp::Create(_)))
        ));
        assert_eq!(
            operations
                .iter()
                .filter(|operation| matches!(operation, DocumentChangeOperation::Edit(_)))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn transcript_inline_definition_returns_a_versioned_compiler_expansion() {
        let project = tempdir().unwrap();
        let chart = project.path().join("chart.avenger");
        let definition = project.path().join("badge.avenger");
        let text = "avenger 1; import { badge } from 'badge.avenger'; chart cartesian as chart { mark badge as imported {} }";
        fs::write(
            &definition,
            "avenger 1; export define mark badge { mark symbol as body {} }",
        )
        .unwrap();
        fs::write(&chart, text).unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let (mut service, mut socket) = LspService::new(Backend::new);
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "workspace": {
                            "workspaceEdit": { "documentChanges": true }
                        },
                        "textDocument": {
                            "codeAction": {
                                "codeActionLiteralSupport": {
                                    "codeActionKind": { "valueSet": ["refactor.inline"] }
                                }
                            }
                        }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 9,
                        "text": text
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;

        let start = text.find("mark badge").unwrap();
        let response = call(
            &mut service,
            Request::build("textDocument/codeAction")
                .id(2)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "range": {
                        "start": { "line": 0, "character": start },
                        "end": { "line": 0, "character": start + "mark badge".len() }
                    },
                    "context": { "diagnostics": [], "only": ["refactor.inline"] }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let response: CodeActionResponse =
            serde_json::from_value(serde_json::to_value(response.result().unwrap()).unwrap())
                .unwrap();
        let CodeActionOrCommand::CodeAction(action) = response.into_iter().next().unwrap() else {
            panic!("expected inline action")
        };
        let Some(DocumentChanges::Edits(edits)) = action.edit.unwrap().document_changes else {
            panic!("inline action did not return versioned edits")
        };
        assert_eq!(edits[0].text_document.version, Some(9));
        assert!(matches!(
            &edits[0].edits[0],
            OneOf::Left(edit)
                if edit.new_text.starts_with("mark group as imported")
                    && edit.new_text.contains("component_kind: badge")
        ));
    }

    #[tokio::test]
    async fn transcript_completion_hover_definition_references_and_highlights() {
        let project = tempdir().unwrap();
        let chart = project.path().join("chart.avenger");
        let text = "avenger 1; chart cartesian as chart { param 640.0 as width; mark symbol as points { size: encoded $wid; } }";
        fs::write(&chart, text).unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let (mut service, mut socket) = LspService::new(Backend::new);
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "textDocument": {
                            "completion": {
                                "completionItem": { "snippetSupport": true }
                            }
                        }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": text
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;

        let completion_offset = text.find("$wid").unwrap() + "$wid".len();
        let completion = call(
            &mut service,
            Request::build("textDocument/completion")
                .id(2)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": completion_offset },
                    "context": {
                        "triggerKind": 2,
                        "triggerCharacter": "$"
                    }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let completion_value = serde_json::to_value(completion.result().unwrap()).unwrap();
        let completion: CompletionResponse = serde_json::from_value(completion_value).unwrap();
        let CompletionResponse::List(completion) = completion else {
            panic!("expected completion list")
        };
        let width = completion
            .items
            .iter()
            .find(|item| item.label == "$width")
            .expect("width completion");
        assert!(matches!(width.text_edit, Some(CompletionTextEdit::Edit(_))));
        assert_eq!(width.filter_text.as_deref(), Some("$width"));

        let reference_offset = text.find("$wid").unwrap() + 2;
        let hover = call(
            &mut service,
            Request::build("textDocument/hover")
                .id(3)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": reference_offset }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let hover: Hover =
            serde_json::from_value(serde_json::to_value(hover.result().unwrap()).unwrap()).unwrap();
        assert!(matches!(hover.contents, HoverContents::Markup(_)));

        // Complete the reference so lexical identity resolution is exact.
        call(
            &mut service,
            Request::build("textDocument/didChange")
                .params(json!({
                    "textDocument": { "uri": chart_uri, "version": 2 },
                    "contentChanges": [{
                        "range": {
                            "start": { "line": 0, "character": completion_offset - 4 },
                            "end": { "line": 0, "character": completion_offset }
                        },
                        "text": "$width"
                    }]
                }))
                .finish(),
        )
        .await;
        let updated_reference = text.find("$wid").unwrap() + 2;
        let definition = call(
            &mut service,
            Request::build("textDocument/definition")
                .id(4)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": updated_reference }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let definition: GotoDefinitionResponse =
            serde_json::from_value(serde_json::to_value(definition.result().unwrap()).unwrap())
                .unwrap();
        assert!(
            matches!(definition, GotoDefinitionResponse::Array(ref values) if values.len() == 1)
        );

        let references = call(
            &mut service,
            Request::build("textDocument/references")
                .id(5)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": updated_reference },
                    "context": { "includeDeclaration": true }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let references: Vec<Location> =
            serde_json::from_value(serde_json::to_value(references.result().unwrap()).unwrap())
                .unwrap();
        assert_eq!(references.len(), 2);

        let highlights = call(
            &mut service,
            Request::build("textDocument/documentHighlight")
                .id(6)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": { "line": 0, "character": updated_reference }
                }))
                .finish(),
        )
        .await
        .unwrap();
        let highlights: Vec<DocumentHighlight> =
            serde_json::from_value(serde_json::to_value(highlights.result().unwrap()).unwrap())
                .unwrap();
        assert_eq!(highlights.len(), 2);
    }

    #[tokio::test]
    async fn transcript_sql_completion_uses_last_good_datafusion_schemas() {
        fn position(text: &str, offset: usize) -> serde_json::Value {
            let prefix = &text[..offset];
            let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
            let character = prefix
                .rsplit_once('\n')
                .map_or(prefix.len(), |(_, line)| line.len());
            json!({ "line": line, "character": character })
        }

        let project = tempdir().unwrap();
        let data = project.path().join("data.avenger");
        let chart = project.path().join("chart.avenger");
        let valid = r#"avenger 1;
export schema tables as vega {
  table inline as movies {
    values: [{ title: 'A'; rating: 8.5; }];
  }
  table sql as popular {
    sql: SELECT "title", "rating" FROM vega.movies;
  }
}
"#;
        let chart_text = r#"avenger 1;
chart cartesian as chart {
  data: { table: vega.movies; }
  mark symbol { x: encoded "title"; y: encoded "rating"; }
}
"#;
        fs::write(&data, valid).unwrap();
        fs::write(&chart, chart_text).unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let data_uri = Uri::from_file_path(&data).unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None::<Backend>));
        let captured_factory = std::sync::Arc::clone(&captured);
        let (mut service, mut socket) = LspService::new(move |client| {
            let backend = Backend::new(client);
            *captured_factory.lock().expect("capture backend") = Some(backend.clone());
            backend
        });
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "general": { "positionEncodings": ["utf-8"] },
                        "textDocument": { "completion": { "completionItem": {} } }
                    },
                    "initializationOptions": {
                        "ambientModules": ["data.avenger"]
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": data_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": valid
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        // The first publication is tolerant syntax; the second publishes the
        // immutable DataFusion-backed project snapshot used as last-good
        // semantics while the query below is incomplete.
        let semantic = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let semantic: PublishDiagnosticsParams =
            serde_json::from_value(semantic.params().cloned().unwrap()).unwrap();
        assert!(
            semantic.diagnostics.is_empty(),
            "valid semantic fixture failed: {:?}",
            semantic.diagnostics
        );
        let backend = captured
            .lock()
            .expect("captured backend")
            .clone()
            .expect("backend");
        let state = backend.inner.state.read().await;
        let analysis = state
            .semantic_analysis
            .values()
            .next()
            .expect("published semantic analysis");
        assert!(
            analysis.semantic_roots.values().any(|root| {
                root.result
                    .as_ref()
                    .is_ok_and(|project| !project.datasets.is_empty())
            }),
            "semantic roots: {:?}",
            analysis.semantic_roots
        );
        drop(state);

        for (version, edited) in [
            (
                2,
                valid.replace(
                    "SELECT \"title\", \"rating\" FROM vega.movies",
                    "FROM vega.movies AS m SELECT m.\"",
                ),
            ),
            (
                3,
                valid.replace(
                    "SELECT \"title\", \"rating\" FROM vega.movies",
                    "SELECT m.\" FROM vega.movies AS m",
                ),
            ),
        ] {
            call(
                &mut service,
                Request::build("textDocument/didChange")
                    .params(json!({
                        "textDocument": { "uri": data_uri, "version": version },
                        "contentChanges": [{ "text": edited }]
                    }))
                    .finish(),
            )
            .await;
            let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
            let cursor = edited.find("m.\"").unwrap() + 3;
            let completion = call(
                &mut service,
                Request::build("textDocument/completion")
                    .id(version + 10)
                    .params(json!({
                        "textDocument": { "uri": data_uri },
                        "position": position(&edited, cursor)
                    }))
                    .finish(),
            )
            .await
            .unwrap();
            let completion: CompletionResponse =
                serde_json::from_value(serde_json::to_value(completion.result().unwrap()).unwrap())
                    .unwrap();
            let CompletionResponse::List(completion) = completion else {
                panic!("expected SQL completion list")
            };
            for expected in ["title", "rating"] {
                let item = completion
                    .items
                    .iter()
                    .find(|item| item.label == expected)
                    .unwrap_or_else(|| panic!("missing {expected}: {:?}", completion.items));
                let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                    panic!("expected exact text edit for {expected}")
                };
                assert_eq!(edit.range.start.line, edit.range.end.line);
                assert_eq!(edit.range.start.character + 1, edit.range.end.character);
                assert_eq!(edit.new_text, format!("\"{expected}\""));
                assert_eq!(
                    item.filter_text.as_deref(),
                    Some(format!("\"{expected}\"").as_str())
                );
                assert!(item.detail.is_some());
            }
        }
    }

    #[tokio::test]
    async fn transcript_completion_is_silent_in_lexically_and_structurally_invalid_domains() {
        fn position(text: &str, offset: usize) -> serde_json::Value {
            let prefix = &text[..offset];
            let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
            let character = prefix
                .rsplit_once('\n')
                .map_or(prefix.len(), |(_, line)| line.len());
            json!({ "line": line, "character": character })
        }

        let project = tempdir().unwrap();
        let chart = project.path().join("completion-silence.avenger");
        let text = r#"avenger 1;
chart cartesian as chart {
  param 1 as invented_name;
  mark symbol as points {
    x: encoded 'string text';
    y: encoded rat;
    size: encoded 1 /* comment text */;
  }
}
"#;
        fs::write(&chart, text).unwrap();
        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let (mut service, mut socket) = LspService::new(Backend::new);
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "general": { "positionEncodings": ["utf-8"] },
                        "textDocument": { "completion": { "completionItem": {} } }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "silence" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": text
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;

        for (ordinal, (name, offset)) in [
            (
                "SQL string",
                text.find("string text").unwrap() + "string".len(),
            ),
            (
                "SQL comment",
                text.find("comment text").unwrap() + "comment".len(),
            ),
            ("bare column-like word", text.find("rat;").unwrap() + 3),
            (
                "invented binder name",
                text.find("invented_name").unwrap() + "invented".len(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let response = call(
                &mut service,
                Request::build("textDocument/completion")
                    .id(10 + ordinal as i64)
                    .params(json!({
                        "textDocument": { "uri": chart_uri },
                        "position": position(text, offset),
                        "context": { "triggerKind": 3 }
                    }))
                    .finish(),
            )
            .await
            .unwrap_or_else(|| panic!("missing completion response for {name}"));
            let completion: CompletionResponse =
                serde_json::from_value(serde_json::to_value(response.result().unwrap()).unwrap())
                    .unwrap();
            let CompletionResponse::List(completion) = completion else {
                panic!("expected completion list for {name}")
            };
            assert!(
                completion.items.is_empty(),
                "{name}: {:#?}",
                completion.items
            );
        }

        let shutdown = call(&mut service, Request::build("shutdown").id(99).finish())
            .await
            .unwrap();
        assert!(shutdown.is_ok());
    }

    #[tokio::test]
    async fn transcript_transform_sql_completion_uses_explicit_data_imports() {
        fn position(text: &str, offset: usize) -> serde_json::Value {
            let prefix = &text[..offset];
            let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
            let character = prefix
                .rsplit_once('\n')
                .map_or(prefix.len(), |(_, line)| line.len());
            json!({ "line": line, "character": character })
        }

        let project = tempdir().unwrap();
        let data = project.path().join("data.avenger");
        let definition = project.path().join("dot.avenger");
        let transform_definition = project.path().join("transforms.avenger");
        let chart = project.path().join("chart.avenger");
        let data_text = r#"avenger 1;
export schema tables as vega {
  table inline as movies {
    values: [{ title: 'A'; rating: 8.5; }];
  }
}
"#;
        let chart_text = r#"avenger 1;
import { vega } from 'data.avenger';
import { dot } from 'dot.avenger';
import { pass } from 'transforms.avenger';
chart cartesian as chart {
  data: { table: vega.movies; }
  transform pass {}
  transform sql as rows {
    query:
      FROM vega.movies AS m
      SELECT m."title", m."rating";
  }
  mark dot as points {}
}
"#;
        fs::write(&data, data_text).unwrap();
        fs::write(
            &definition,
            "avenger 1; export define mark dot { mark symbol {} }",
        )
        .unwrap();
        fs::write(
            &transform_definition,
            "avenger 1; export define transform pass { transform filter { predicate: true; } }",
        )
        .unwrap();
        fs::write(&chart, chart_text).unwrap();

        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let definition_uri = Uri::from_file_path(fs::canonicalize(&definition).unwrap()).unwrap();
        let transform_definition_uri =
            Uri::from_file_path(fs::canonicalize(&transform_definition).unwrap()).unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None::<Backend>));
        let captured_factory = std::sync::Arc::clone(&captured);
        let (mut service, _socket) = LspService::new(move |client| {
            let backend = Backend::new(client);
            *captured_factory.lock().expect("capture backend") = Some(backend.clone());
            backend
        });
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": {
                        "general": { "positionEncodings": ["utf-8"] },
                        "textDocument": { "completion": { "completionItem": {} } }
                    },
                    "workspaceFolders": [{ "uri": root_uri, "name": "fixture" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": chart_text
                    }
                }))
                .finish(),
        )
        .await;

        let backend = captured
            .lock()
            .expect("captured backend")
            .clone()
            .expect("backend");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let ready = backend
                    .inner
                    .state
                    .read()
                    .await
                    .semantic_analysis
                    .values()
                    .any(|analysis| {
                        analysis.semantic_roots.values().any(|root| {
                            root.result
                                .as_ref()
                                .is_ok_and(|project| !project.datasets.is_empty())
                        })
                    });
                if ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("semantic analysis timeout");
        let cursor = chart_text.find("m.\"title\"").unwrap() + 3;
        let completion = call(
            &mut service,
            Request::build("textDocument/completion")
                .id(2)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": position(chart_text, cursor)
                }))
                .finish(),
        )
        .await
        .unwrap();
        let completion: CompletionResponse =
            serde_json::from_value(serde_json::to_value(completion.result().unwrap()).unwrap())
                .unwrap();
        let CompletionResponse::List(completion) = completion else {
            panic!("expected SQL completion list")
        };
        for expected in ["title", "rating"] {
            let item = completion
                .items
                .iter()
                .find(|item| item.label == expected)
                .unwrap_or_else(|| panic!("missing {expected}: {:?}", completion.items));
            assert!(item.detail.is_some());
        }

        let definition_cursor = chart_text.find("mark dot").unwrap() + "mark d".len();
        let definition_response = call(
            &mut service,
            Request::build("textDocument/definition")
                .id(3)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": position(chart_text, definition_cursor)
                }))
                .finish(),
        )
        .await
        .unwrap();
        let definition_response: GotoDefinitionResponse = serde_json::from_value(
            serde_json::to_value(definition_response.result().unwrap()).unwrap(),
        )
        .unwrap();
        let GotoDefinitionResponse::Array(locations) = definition_response else {
            panic!("expected definition locations")
        };
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].uri, definition_uri);

        let transform_cursor = chart_text.find("transform pass").unwrap() + "transform p".len();
        let transform_definition_response = call(
            &mut service,
            Request::build("textDocument/definition")
                .id(4)
                .params(json!({
                    "textDocument": { "uri": chart_uri },
                    "position": position(chart_text, transform_cursor)
                }))
                .finish(),
        )
        .await
        .unwrap();
        let transform_definition_response: GotoDefinitionResponse = serde_json::from_value(
            serde_json::to_value(transform_definition_response.result().unwrap()).unwrap(),
        )
        .unwrap();
        let GotoDefinitionResponse::Array(transform_locations) = transform_definition_response
        else {
            panic!("expected transform definition locations")
        };
        assert_eq!(transform_locations.len(), 1);
        assert_eq!(transform_locations[0].uri, transform_definition_uri);
    }

    #[tokio::test]
    async fn imported_transform_sql_errors_stay_on_the_failing_stage() {
        let project = tempdir().unwrap();
        let transforms = project.path().join("transforms.avenger");
        let chart = project.path().join("chart.avenger");
        fs::write(
            &transforms,
            "avenger 1; export define transform pass { transform filter { predicate: true; } }",
        )
        .unwrap();
        let valid = r#"avenger 1;
import { pass } from 'transforms.avenger';
schema tables as local {
  table inline as points { values: [{ x: 1; }]; }
}
chart cartesian as chart {
  data: { table: local.points; }
  transform pass {}
  mark symbol { x: encoded "x"; }
}
"#;
        let invalid = valid.replace(
            "  transform pass {}",
            "  transform sql as filtered {\n    query: SELECT * FROM input WHERE \"value\" > 10;\n  }\n  transform pass {}",
        );
        fs::write(&chart, valid).unwrap();

        let root_uri = Uri::from_file_path(project.path()).unwrap();
        let chart_uri = Uri::from_file_path(&chart).unwrap();
        let (mut service, mut socket) = LspService::new(Backend::new);
        call(
            &mut service,
            Request::build("initialize")
                .id(1)
                .params(json!({
                    "capabilities": { "general": { "positionEncodings": ["utf-8"] } },
                    "workspaceFolders": [{ "uri": root_uri, "name": "diagnostics" }]
                }))
                .finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("initialized").params(json!({})).finish(),
        )
        .await;
        call(
            &mut service,
            Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": chart_uri,
                        "languageId": "avenger",
                        "version": 1,
                        "text": valid
                    }
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;

        call(
            &mut service,
            Request::build("textDocument/didChange")
                .params(json!({
                    "textDocument": { "uri": chart_uri, "version": 2 },
                    "contentChanges": [{ "text": invalid }]
                }))
                .finish(),
        )
        .await;
        let _ = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let semantic = next_notification(&mut socket, "textDocument/publishDiagnostics").await;
        let semantic: PublishDiagnosticsParams =
            serde_json::from_value(semantic.params().cloned().unwrap()).unwrap();
        let diagnostic = semantic
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("No field named value"))
            .expect("missing DataFusion field diagnostic");
        let sql_offset = invalid.find("transform sql").unwrap();
        let expected_line = invalid[..sql_offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u32;
        assert_eq!(diagnostic.range.start.line, expected_line);
    }

    async fn call(
        service: &mut LspService<Backend>,
        request: Request,
    ) -> Option<tower_lsp_server::jsonrpc::Response> {
        service
            .ready()
            .await
            .expect("service ready")
            .call(request)
            .await
            .expect("service call")
    }

    async fn next_notification(socket: &mut ClientSocket, method: &str) -> Request {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let request = socket.next().await.expect("client notification");
                if request.method() == method {
                    return request;
                }
            }
        })
        .await
        .expect("notification timeout")
    }
}
