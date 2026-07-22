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
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, AnalysisService,
    CompletionKind as AvengerCompletionKind, CompletionOptions as AnalysisCompletionOptions,
    CompletionTextFormat, DocumentSnapshot, PositionRequest, SourceRevision, SyntaxAnalysis,
    WorkspaceAnalysis, WorkspaceSnapshot, analyze_syntax,
};
use avenger_lang_compiler::{CompileFailure, Compiler};
use avenger_lang_core::{
    Diagnostic as AvengerDiagnostic, DiagnosticSeverity as AvengerDiagnosticSeverity, ProjectRoot,
    SourceFile, SourceMap, SourceOrigin, project::normalize_path,
};
use documents::{DocumentStore, OpenDocument};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use position::{PositionEncoding, PositionIndex};
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
};
use tower_lsp_server::{Client, LanguageServer, LspService, Server, jsonrpc, ls_types::*};

const SERVER_NAME: &str = "avenger-lsp";
const SEMANTIC_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Clone)]
struct Backend {
    client: Client,
    inner: Arc<BackendInner>,
}

struct BackendInner {
    state: RwLock<ServerState>,
    semantic_tasks: Mutex<HashMap<PathBuf, SemanticTask>>,
    publish_lock: Mutex<()>,
    watchers: StdMutex<HashMap<PathBuf, RecommendedWatcher>>,
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
}

#[derive(Clone)]
struct SemanticInput {
    workspace: Workspace,
    snapshot: WorkspaceSnapshot,
    versions: BTreeMap<Uri, i32>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            inner: Arc::new(BackendInner {
                state: RwLock::new(ServerState {
                    position_encoding: PositionEncoding::Utf16,
                    position_encoding_kind: PositionEncodingKind::UTF16,
                    hierarchical_symbols: true,
                    completion_snippets: false,
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
            }),
        }
    }

    async fn report_error(&self, context: &str, error: impl std::fmt::Display) {
        let message = format!("{context}: {error}");
        tracing::warn!("{message}");
        eprintln!("{SERVER_NAME}: {message}");
        self.client.log_message(MessageType::WARNING, message).await;
    }

    async fn ensure_workspace_for_uri(&self, uri: &Uri) -> Option<PathBuf> {
        let path = uri.to_file_path()?.into_owned();
        {
            let state = self.inner.state.read().await;
            if let Some(root) = owning_workspace(&state.workspaces, &path) {
                return Some(root);
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
            tokio::time::sleep(SEMANTIC_DEBOUNCE).await;
            let result = analyze_on_worker(
                input.workspace.service.clone(),
                input.snapshot,
                task_cancellation,
            )
            .await;
            if let Ok(analysis) = result {
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
    ) -> Option<(OpenDocument, Arc<WorkspaceAnalysis>, PositionRequest, bool)> {
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
            let known_sources = scan_avenger_files(&root).into_iter().collect();
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
        ))
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
                if event.is_ok() {
                    let backend = backend.clone();
                    let root = watched_root.clone();
                    runtime.spawn(async move { backend.schedule_semantic(root).await });
                }
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

    async fn stop_workspace(&self, root: &Path) {
        if let Some(task) = self.inner.semantic_tasks.lock().await.remove(root) {
            task.cancellation.cancel();
            task.handle.abort();
        }
        self.inner
            .watchers
            .lock()
            .expect("watcher lock poisoned")
            .remove(root);
    }
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

        let mut workspaces = BTreeMap::new();
        for root in roots {
            let root = normalize_existing_path(root);
            match Workspace::new(root.clone()) {
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
                completion_provider: Some(tower_lsp_server::ls_types::CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        "$".to_owned(),
                        ".".to_owned(),
                        "@".to_owned(),
                        ":".to_owned(),
                        "'".to_owned(),
                        "\"".to_owned(),
                    ]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
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
        let roots = self
            .inner
            .state
            .read()
            .await
            .workspaces
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for root in roots {
            self.stop_workspace(&root).await;
        }
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let encoding = self.position_encoding().await;
        let opened = self
            .inner
            .state
            .write()
            .await
            .documents
            .open(params.text_document, encoding);
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
        let saved = self
            .inner
            .state
            .write()
            .await
            .documents
            .save(&uri, params.text, encoding);
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

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> jsonrpc::Result<Option<DocumentSymbolResponse>> {
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

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> jsonrpc::Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some((document, analysis, request, snippets)) =
            self.query_snapshot(&uri, position).await
        else {
            return Ok(None);
        };
        let cancellation = AnalysisCancellation::default();
        let result = analysis.complete(
            &request,
            AnalysisCompletionOptions { snippets },
            &cancellation,
        );
        let Ok(result) = result else {
            return Ok(None);
        };
        let items = result
            .items
            .into_iter()
            .filter_map(|item| completion_item(item, &document.positions))
            .collect();
        Ok(Some(CompletionResponse::List(CompletionList {
            is_incomplete: result.is_incomplete,
            items,
        })))
    }

    async fn hover(&self, params: HoverParams) -> jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((document, analysis, request, _)) = self.query_snapshot(&uri, position).await
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
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((_, analysis, request, _)) = self.query_snapshot(&uri, position).await else {
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
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some((_, analysis, request, _)) = self.query_snapshot(&uri, position).await else {
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
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((document, analysis, request, _)) = self.query_snapshot(&uri, position).await
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
        let roots = {
            let state = self.inner.state.read().await;
            params
                .changes
                .iter()
                .filter_map(|change| change.uri.to_file_path())
                .filter_map(|path| owning_workspace(&state.workspaces, &path))
                .collect::<BTreeSet<_>>()
        };
        for root in roots {
            self.schedule_semantic(root).await;
        }
    }
}

impl Workspace {
    fn new(path: PathBuf) -> Result<Self, avenger_lang_compiler::CompilerBuildError> {
        let compiler = Compiler::builder().project_root(&path).build()?;
        Ok(Self {
            service: AnalysisService::new(compiler),
        })
    }
}

/// Run the native Avenger language server over standard input and output.
///
/// Standard output is reserved exclusively for LSP framing. Callers must send
/// human-readable logs to standard error or through LSP client notifications.
pub async fn run_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
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
    normalize_path(&path)
}

fn owning_workspace(workspaces: &BTreeMap<PathBuf, Workspace>, path: &Path) -> Option<PathBuf> {
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
    let mut origins = scan_avenger_files(workspace_root);
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
    let roots = origins
        .iter()
        .filter_map(|origin| match origin {
            SourceOrigin::File(path) if is_data_path(path) => {
                Some(ProjectRoot::data(origin.clone()))
            }
            SourceOrigin::File(path) if is_chart_path(path) => {
                Some(ProjectRoot::chart(origin.clone()))
            }
            _ => None,
        })
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

fn scan_avenger_files(root: &Path) -> BTreeSet<SourceOrigin> {
    fn visit(path: &Path, depth: usize, output: &mut BTreeSet<SourceOrigin>) {
        if depth > 64 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let hidden = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.') || name == "target");
                if !hidden {
                    visit(&path, depth + 1, output);
                }
            } else if path.to_string_lossy().ends_with(".avenger") {
                output.insert(SourceOrigin::File(normalize_existing_path(path)));
            }
        }
    }
    let mut output = BTreeSet::new();
    visit(root, 0, &mut output);
    output
}

fn origin_is_in_workspace(origin: &SourceOrigin, workspace_root: &Path) -> bool {
    matches!(origin, SourceOrigin::File(path) if path.starts_with(workspace_root))
}

fn is_data_path(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".data.avenger")
}

fn is_definition_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.ends_with(".mark.avenger")
        || path.ends_with(".tool.avenger")
        || path.ends_with(".transform.avenger")
}

fn is_chart_path(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".avenger") && !is_data_path(path) && !is_definition_path(path)
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
        A::Chart | A::Group | A::Mark | A::Widget | A::View => SymbolKind::OBJECT,
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
) -> Option<tower_lsp_server::ls_types::CompletionItem> {
    let range = positions
        .lsp_range(item.replacement.range.as_range())
        .ok()?;
    Some(tower_lsp_server::ls_types::CompletionItem {
        label: item.label,
        kind: Some(match item.kind {
            AvengerCompletionKind::Keyword => CompletionItemKind::KEYWORD,
            AvengerCompletionKind::Declaration => CompletionItemKind::CLASS,
            AvengerCompletionKind::Property => CompletionItemKind::PROPERTY,
            AvengerCompletionKind::EnumValue => CompletionItemKind::ENUM_MEMBER,
            AvengerCompletionKind::Variable => CompletionItemKind::VARIABLE,
            AvengerCompletionKind::Field => CompletionItemKind::FIELD,
            AvengerCompletionKind::Function => CompletionItemKind::FUNCTION,
            AvengerCompletionKind::Type => CompletionItemKind::TYPE_PARAMETER,
            AvengerCompletionKind::Module => CompletionItemKind::MODULE,
            AvengerCompletionKind::Catalog | AvengerCompletionKind::Schema => {
                CompletionItemKind::MODULE
            }
            AvengerCompletionKind::Table => CompletionItemKind::STRUCT,
            AvengerCompletionKind::Snippet => CompletionItemKind::SNIPPET,
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
    use std::{fs, time::Duration};

    use futures::StreamExt;
    use serde_json::json;
    use tempfile::tempdir;
    use tower::{Service, ServiceExt};
    use tower_lsp_server::{ClientSocket, LspService, jsonrpc::Request, ls_types::*};

    use super::Backend;

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
    async fn transcript_completion_hover_definition_references_and_highlights() {
        let project = tempdir().unwrap();
        let chart = project.path().join("chart.avenger");
        let text = "avenger 1; chart cartesian as chart { param as width { type: float64; default: 640.0; } mark symbol as points { size: $wid; } }";
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
                    "position": { "line": 0, "character": completion_offset }
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
