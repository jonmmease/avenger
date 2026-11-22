//! `avenger lsp`: the Language Server Protocol server of avenger-sql scripts (`.av.sql`) and
//! modules (`.av.sqlm`), over avenger-sql-language.
//!
//! The server keeps every project that holds an open file live: the language analysis of every
//! script and module of the project, with the text of an open file in place of the text on disk,
//! on the main loop, and the loader of its data scripts on a thread of its own, whose loads come
//! back to the main loop. An edit analyzes the file and the files that it reaches, and the server
//! publishes the diagnostics of those files alone. A saved data script or module, and a changed
//! data file, load again. A file outside every project, or without a role in its project, is
//! analyzed alone.

mod convert;
mod loader;
mod requests;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_language::{Diagnostic, Document, FileRole, Role};
use avenger_sql_project::{
    find_root,
    watch::{watched, Burst, Watch, Watched},
    Analysis, ProjectFiles, Role as ManifestRole, MANIFEST, MODULE_SUFFIX,
};
use crossbeam_channel::{never, select, Receiver, Sender};
use lsp_server::{Connection, Message, Notification, Request};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument,
        DidSaveTextDocument, Notification as _, PublishDiagnostics,
    },
    request::{RegisterCapability, Request as _},
    CodeActionKind, CodeActionProviderCapability, CompletionOptions, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    FileSystemWatcher, GlobPattern, InitializeParams, InitializeResult, OneOf,
    PositionEncodingKind, PublishDiagnosticsParams, Registration, RegistrationParams,
    RenameOptions, SaveOptions, SelectionRangeProviderCapability, SemanticTokensFullOptions,
    SemanticTokensLegend, SemanticTokensOptions, SemanticTokensServerCapabilities,
    ServerCapabilities, ServerInfo, SignatureHelpOptions, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, Url,
    WorkDoneProgressOptions,
};

use self::convert::{Encoding, LineIndex};
use self::loader::{Load, Loaded, LoaderThread};

/// Serves the protocol on standard input and standard output until the client exits.
pub fn serve_stdio() -> Result<(), String> {
    let (connection, threads) = Connection::stdio();
    serve(connection)?;
    threads.join().map_err(|error| error.to_string())
}

/// Serves the protocol on `connection`, from the client's `initialize` to its `exit`.
pub fn serve(connection: Connection) -> Result<(), String> {
    let error = |error: &dyn std::fmt::Display| error.to_string();
    let (id, params) = connection.initialize_start().map_err(|e| error(&e))?;
    let init: InitializeParams = serde_json::from_value(params).map_err(|e| error(&e))?;
    let capabilities = &init.capabilities;
    let utf8 = (capabilities.general.as_ref())
        .and_then(|general| general.position_encodings.as_ref())
        .is_some_and(|encodings| encodings.contains(&PositionEncodingKind::UTF8));
    let encoding = match utf8 {
        true => Encoding::Utf8,
        false => Encoding::Utf16,
    };
    let client_watches = (capabilities.workspace.as_ref())
        .and_then(|workspace| workspace.did_change_watched_files.as_ref())
        .and_then(|watched| watched.dynamic_registration)
        .unwrap_or(false);
    let insert_replace = (capabilities.text_document.as_ref())
        .and_then(|text| text.completion.as_ref())
        .and_then(|completion| completion.completion_item.as_ref())
        .and_then(|item| item.insert_replace_support)
        .unwrap_or(false);
    let result = InitializeResult {
        capabilities: server_capabilities(encoding),
        server_info: Some(ServerInfo {
            name: "avenger".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };
    let result = serde_json::to_value(result).map_err(|e| error(&e))?;
    connection
        .initialize_finish(id, result)
        .map_err(|e| error(&e))?;
    let mut server = Server::new(connection.sender.clone(), encoding, insert_replace);
    server.client_watches = client_watches;
    if client_watches {
        server.register_watcher();
    }
    server.run(&connection)
}

/// The capabilities of the server, with the position encoding of `encoding`.
fn server_capabilities(encoding: Encoding) -> ServerCapabilities {
    let mut types = convert::TOKEN_TYPES.to_vec();
    types.push(convert::NAMESPACE);
    ServerCapabilities {
        position_encoding: Some(match encoding {
            Encoding::Utf8 => PositionEncodingKind::UTF8,
            Encoding::Utf16 => PositionEncodingKind::UTF16,
        }),
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(false),
                })),
                ..Default::default()
            },
        )),
        hover_provider: Some(true.into()),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some([".", "'", "/", "@", "("].map(String::from).to_vec()),
            ..Default::default()
        }),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(["(", ","].map(String::from).to_vec()),
            retrigger_characters: None,
            work_done_progress_options: WorkDoneProgressOptions::default(),
        }),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),
        document_symbol_provider: Some(OneOf::Left(true)),
        folding_range_provider: Some(true.into()),
        selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: types,
                    token_modifiers: convert::TOKEN_MODIFIERS.to_vec(),
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                range: None,
                work_done_progress_options: WorkDoneProgressOptions::default(),
            },
        )),
        document_formatting_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Options(
            lsp_types::CodeActionOptions {
                code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

/// A file that the client has open: its text, which the client edits, and its version.
struct OpenFile {
    text: String,
    version: i32,
}

/// A project that holds an open file, which the server keeps live.
struct ProjectState {
    /// The id of the project's loads, new each time the project opens.
    id: u64,
    /// Whether the first load came back. Until it does, the project's tables of files have no
    /// schemas, so the server publishes none of its diagnostics.
    loaded: bool,
    files: ProjectFiles,
    analysis: Analysis,
    /// The watch of the project's root, when the client watches no files for the server.
    watch: Option<Watch>,
}

/// Where a file of the client is to the server.
enum Located {
    /// A script or a module of the project at `root`, at `path` relative to the root.
    Project { root: PathBuf, path: String },
    /// A script or a module that is analyzed alone.
    Loose,
}

struct Server {
    sender: Sender<Message>,
    encoding: Encoding,
    /// Whether the client takes a completion's insert and replace ranges.
    insert_replace: bool,
    /// Whether the client watches the files for the server.
    client_watches: bool,
    /// The runtime of the reads of projects on the main loop.
    runtime: tokio::runtime::Runtime,
    open: HashMap<PathBuf, OpenFile>,
    projects: BTreeMap<PathBuf, ProjectState>,
    /// The documents of the open files that are analyzed alone, by path.
    loose: BTreeMap<PathBuf, Document>,
    loader: LoaderThread,
    loaded: Receiver<Loaded>,
    wake: Sender<()>,
    wakes: Receiver<()>,
    next_request: i32,
    /// The id of the next project that opens.
    next_project: u64,
}

impl Server {
    fn new(sender: Sender<Message>, encoding: Encoding, insert_replace: bool) -> Server {
        let (results, loaded) = crossbeam_channel::unbounded();
        let (wake, wakes) = crossbeam_channel::unbounded();
        Server {
            sender,
            encoding,
            insert_replace,
            client_watches: false,
            runtime: tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime of the main loop"),
            open: HashMap::new(),
            projects: BTreeMap::new(),
            loose: BTreeMap::new(),
            loader: LoaderThread::start(results),
            loaded,
            wake,
            wakes,
            next_request: 0,
            next_project: 0,
        }
    }

    /// The main loop: the client's messages, the loader's results, and the watches' changes,
    /// until the client shuts the server down.
    fn run(&mut self, connection: &Connection) -> Result<(), String> {
        loop {
            let wait = self.poll_watches();
            let timeout = match wait {
                Some(wait) => crossbeam_channel::after(wait),
                None => never(),
            };
            select! {
                recv(connection.receiver) -> message => {
                    let Ok(message) = message else {
                        return Ok(());
                    };
                    match message {
                        Message::Request(request) => {
                            let shutdown = connection.handle_shutdown(&request);
                            if shutdown.map_err(|error| error.to_string())? {
                                return Ok(());
                            }
                            self.request(request);
                        }
                        Message::Notification(notification) => self.notification(notification),
                        Message::Response(_) => {}
                    }
                }
                recv(self.loaded) -> loaded => {
                    if let Ok(loaded) = loaded {
                        self.apply(loaded);
                    }
                }
                recv(self.wakes) -> _ => {}
                recv(timeout) -> _ => {}
            }
        }
    }

    fn send_notification<N: lsp_types::notification::Notification>(&self, params: N::Params) {
        let notification = Notification::new(N::METHOD.to_string(), params);
        let _ = self.sender.send(Message::Notification(notification));
    }

    /// Asks the client to watch every file of its workspace for the server.
    fn register_watcher(&mut self) {
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*".to_string()),
                kind: None,
            }],
        };
        let params = RegistrationParams {
            registrations: vec![Registration {
                id: "avenger-files".to_string(),
                method: DidChangeWatchedFiles::METHOD.to_string(),
                register_options: serde_json::to_value(options).ok(),
            }],
        };
        self.next_request += 1;
        let request = Request::new(
            format!("avenger-{}", self.next_request).into(),
            RegisterCapability::METHOD.to_string(),
            params,
        );
        let _ = self.sender.send(Message::Request(request));
    }

    fn notification(&mut self, notification: Notification) {
        let method = notification.method.as_str();
        let params = notification.params;
        match method {
            DidOpenTextDocument::METHOD => {
                if let Ok(params) = serde_json::from_value::<DidOpenTextDocumentParams>(params) {
                    let document = params.text_document;
                    self.open_file(&document.uri, document.text, document.version);
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(params) = serde_json::from_value::<DidChangeTextDocumentParams>(params) {
                    self.change_file(params);
                }
            }
            DidSaveTextDocument::METHOD => {
                if let Ok(params) = serde_json::from_value::<DidSaveTextDocumentParams>(params) {
                    self.save_file(&params.text_document.uri);
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(params) = serde_json::from_value::<DidCloseTextDocumentParams>(params) {
                    self.close_file(&params.text_document.uri);
                }
            }
            DidChangeWatchedFiles::METHOD => {
                if let Ok(params) = serde_json::from_value::<DidChangeWatchedFilesParams>(params) {
                    let paths = (params.changes.iter())
                        .filter_map(|change| file_path(&change.uri))
                        .collect();
                    self.files_changed(paths);
                }
            }
            _ => {}
        }
    }

    /// Where the file at `path` is: a script or a module of its project, whose project the
    /// server opens, or a file that is analyzed alone.
    fn locate(&mut self, path: &Path) -> Located {
        let Some(root) = find_root(path) else {
            return Located::Loose;
        };
        let Some(relative) = relative(&root, path) else {
            return Located::Loose;
        };
        if !self.open_project(&root) {
            return Located::Loose;
        }
        let state = &self.projects[&root];
        match state.files.manifest().role(&relative) {
            Ok(Some(ManifestRole::DataScript | ManifestRole::ChartScript))
            | Ok(Some(ManifestRole::Module(_))) => Located::Project {
                root,
                path: relative,
            },
            _ => Located::Loose,
        }
    }

    /// Opens the project at `root`, unless it is open, and starts its load, and whether it is
    /// open. A project that does not open, such as one whose manifest has an error, shows its
    /// error, and its files are analyzed alone.
    fn open_project(&mut self, root: &Path) -> bool {
        if self.projects.contains_key(root) {
            return true;
        }
        let opened = self.runtime.block_on(async {
            let files = ProjectFiles::find(root).await?;
            let analysis = Analysis::new(&files).await?;
            Ok::<_, avenger_sql_project::ProjectError>((files, analysis))
        });
        let (files, mut analysis) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                self.show_error(format!("{}: {error}", root.display()));
                return false;
            }
        };
        // The open files of the project analyze with the client's texts.
        let texts: Vec<(String, String)> = (self.open.iter())
            .filter_map(|(path, file)| Some((relative(root, path)?, file.text.clone())))
            .collect();
        analysis.project_mut().set_texts(texts);
        let watch = match self.client_watches {
            true => None,
            false => {
                let wake = self.wake.clone();
                Watch::new(root, move || {
                    let _ = wake.send(());
                })
                .ok()
            }
        };
        let id = self.next_project;
        self.next_project += 1;
        self.loader.send(Load::Project {
            project: id,
            files: Box::new(files.clone()),
        });
        self.projects.insert(
            root.to_path_buf(),
            ProjectState {
                id,
                loaded: false,
                files,
                analysis,
                watch,
            },
        );
        true
    }

    fn open_file(&mut self, uri: &Url, text: String, version: i32) {
        let Some(path) = file_path(uri) else {
            return;
        };
        self.open.insert(
            path.clone(),
            OpenFile {
                text: text.clone(),
                version,
            },
        );
        self.analyze(&path, text);
    }

    /// Analyzes the file at `path` with `text`, and publishes the diagnostics of the files that
    /// the change reaches.
    fn analyze(&mut self, path: &Path, text: String) {
        match self.locate(path) {
            Located::Project { root, path } => {
                let state = self.projects.get_mut(&root).expect("an open project");
                let project = state.analysis.project_mut();
                let current = project.document(&path).map(|document| document.text());
                let reached = match current == Some(text.as_str()) {
                    true => BTreeSet::from([path]),
                    false => project.set_texts([(path, text)]),
                };
                self.publish(&root, &reached);
            }
            Located::Loose => {
                let role = match path.to_string_lossy().ends_with(MODULE_SUFFIX) {
                    true => Role::Module,
                    false => Role::Script,
                };
                let document = Document::new(
                    &text,
                    role,
                    Arc::new(MemoryCatalog::new()),
                    Arc::new(Library::new()),
                    Options::default(),
                );
                self.loose.insert(path.to_path_buf(), document);
                self.publish_loose(path);
            }
        }
    }

    fn change_file(&mut self, params: DidChangeTextDocumentParams) {
        let Some(path) = file_path(&params.text_document.uri) else {
            return;
        };
        let Some(file) = self.open.get_mut(&path) else {
            return;
        };
        for change in params.content_changes {
            match change.range {
                Some(range) => {
                    let range = LineIndex::new(&file.text, self.encoding).text_range(range);
                    let range = usize::from(range.start())..usize::from(range.end());
                    file.text.replace_range(range, &change.text);
                }
                None => file.text = change.text,
            }
        }
        file.version = params.text_document.version;
        let text = file.text.clone();
        self.analyze(&path, text);
    }

    /// Loads a saved data script or module again, with the text that the client saved.
    fn save_file(&mut self, uri: &Url) {
        let Some(path) = file_path(uri) else {
            return;
        };
        let Some(text) = self.open.get(&path).map(|file| file.text.clone()) else {
            return;
        };
        if let Located::Project { root, path } = self.locate(&path) {
            let state = &self.projects[&root];
            let role = state.analysis.project().role(&path).cloned();
            if matches!(role, Some(FileRole::Data | FileRole::Module { .. })) {
                let project = state.id;
                self.loader.send(Load::Script {
                    project,
                    path,
                    text,
                });
            }
        }
    }

    /// Takes the text on disk of a file that the client closes, and clears the diagnostics of a
    /// file that is analyzed alone.
    fn close_file(&mut self, uri: &Url) {
        let Some(path) = file_path(uri) else {
            return;
        };
        self.open.remove(&path);
        if self.loose.remove(&path).is_some() {
            self.publish_diagnostics(uri.clone(), Vec::new(), None);
            return;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            self.analyze(&path, text);
        }
    }

    /// Follows the files at `paths` that changed on disk: a data file loads the data scripts
    /// that read it again, a script or a module that the client has not open analyzes its text
    /// on disk, and a changed manifest opens its project again.
    fn files_changed(&mut self, paths: Vec<PathBuf>) {
        let roots: Vec<PathBuf> = self.projects.keys().cloned().collect();
        for root in roots {
            let changed: Vec<&PathBuf> = (paths.iter()).filter(|p| p.starts_with(&root)).collect();
            if changed.is_empty() {
                continue;
            }
            if changed.iter().any(|path| **path == root.join(MANIFEST)) {
                self.reopen(&root);
                continue;
            }
            let mut data = Vec::new();
            let mut listed = false;
            for path in changed {
                if self.open.contains_key(path.as_path()) {
                    continue;
                }
                let state = self.projects.get_mut(&root).expect("an open project");
                match watched(&root, &state.files, path) {
                    Some(Watched::Data(relative)) => data.push(relative),
                    Some(Watched::Project(relative)) => {
                        let role = state.files.manifest().role(&relative);
                        let script = matches!(
                            role,
                            Ok(Some(
                                ManifestRole::DataScript
                                    | ManifestRole::ChartScript
                                    | ManifestRole::Module(_)
                            ))
                        );
                        if !script {
                            continue;
                        }
                        let id = state.id;
                        let project = state.analysis.project_mut();
                        let known = project.document(&relative).is_some();
                        match (std::fs::read_to_string(path), known) {
                            (Ok(text), true) => {
                                let reached = project.set_texts([(relative.clone(), text.clone())]);
                                let role = project.role(&relative).cloned();
                                self.publish(&root, &reached);
                                if matches!(role, Some(FileRole::Data | FileRole::Module { .. })) {
                                    self.loader.send(Load::Script {
                                        project: id,
                                        path: relative,
                                        text,
                                    });
                                }
                            }
                            // A new script or module, and a removed one, change the files of the
                            // project, which opens again.
                            _ => listed = true,
                        }
                    }
                    None => {}
                }
            }
            if listed {
                self.reopen(&root);
            } else if !data.is_empty() {
                let project = self.projects[&root].id;
                self.loader.send(Load::Files {
                    project,
                    paths: data,
                });
            }
        }
    }

    /// Opens the project at `root` again, with its files as they are on disk now.
    fn reopen(&mut self, root: &Path) {
        let Some(old) = self.projects.remove(root) else {
            self.open_project(root);
            return;
        };
        self.loader.send(Load::Close { project: old.id });
        self.open_project(root);
        // The diagnostics of files that the project no longer holds. The others stay until the
        // first load of the project replaces them.
        let project = self
            .projects
            .get(root)
            .map(|state| state.analysis.project());
        for path in old.analysis.project().files() {
            if project.is_some_and(|project| project.document(path).is_some()) {
                continue;
            }
            if let Ok(uri) = Url::from_file_path(root.join(path)) {
                self.publish_diagnostics(uri, Vec::new(), None);
            }
        }
    }

    /// The paths of the bursts of the watches that ended, which the server follows, and the
    /// time until the next burst would end.
    fn poll_watches(&mut self) -> Option<Duration> {
        let mut ended = Vec::new();
        let mut wait: Option<Duration> = None;
        for state in self.projects.values_mut() {
            let Some(watch) = &mut state.watch else {
                continue;
            };
            match watch.changed() {
                Burst::Ended(paths) => ended.extend(paths),
                Burst::Waiting(time) => wait = Some(wait.map_or(time, |wait| wait.min(time))),
                Burst::Quiet => {}
            }
        }
        if !ended.is_empty() {
            let paths = ended.iter().filter_map(|path| canonical(path)).collect();
            self.files_changed(paths);
        }
        wait
    }

    /// Takes a load of the loader: the tables of its files and the loader's errors, and
    /// publishes the diagnostics of the files that they reach.
    /// Applies a load of a project that is open, and publishes the diagnostics of the files that
    /// it reaches, or of all the project's files after its first load.
    fn apply(&mut self, loaded: Loaded) {
        let Some((root, state)) =
            (self.projects.iter_mut()).find(|(_, state)| state.id == loaded.project)
        else {
            return;
        };
        let root = root.clone();
        let first = !state.loaded;
        state.loaded = true;
        let mut reached = match loaded.state {
            Ok(load) => state.analysis.apply(&load),
            Err(error) => {
                self.show_error(format!("{}: {error}", root.display()));
                BTreeSet::new()
            }
        };
        if first {
            let project = self.projects[&root].analysis.project();
            reached = project.files().map(str::to_string).collect();
        }
        self.publish(&root, &reached);
    }

    fn show_error(&self, message: String) {
        self.send_notification::<lsp_types::notification::ShowMessage>(
            lsp_types::ShowMessageParams {
                typ: lsp_types::MessageType::ERROR,
                message,
            },
        );
    }

    /// Publishes the diagnostics of the files at `paths` of the project at `root`, and none for
    /// a path that the project no longer holds.
    fn publish(&self, root: &Path, paths: &BTreeSet<String>) {
        let Some(state) = self.projects.get(root).filter(|state| state.loaded) else {
            return;
        };
        let project = state.analysis.project();
        for path in paths {
            let file = root.join(path);
            let Ok(uri) = Url::from_file_path(&file) else {
                continue;
            };
            let version = self.open.get(&file).map(|file| file.version);
            let diagnostics = match project.document(path) {
                Some(document) => {
                    let index = LineIndex::new(document.text(), self.encoding);
                    (project.diagnostics(path).iter())
                        .map(|diagnostic| lsp_diagnostic(&index, diagnostic))
                        .collect()
                }
                None => Vec::new(),
            };
            self.publish_diagnostics(uri, diagnostics, version);
        }
    }

    fn publish_loose(&self, path: &Path) {
        let (Some(document), Ok(uri)) = (self.loose.get(path), Url::from_file_path(path)) else {
            return;
        };
        let index = LineIndex::new(document.text(), self.encoding);
        let diagnostics = (document.diagnostics().iter())
            .map(|diagnostic| lsp_diagnostic(&index, diagnostic))
            .collect();
        let version = self.open.get(path).map(|file| file.version);
        self.publish_diagnostics(uri, diagnostics, version);
    }

    fn publish_diagnostics(
        &self,
        uri: Url,
        diagnostics: Vec<lsp_types::Diagnostic>,
        version: Option<i32>,
    ) {
        self.send_notification::<PublishDiagnostics>(PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        });
    }
}

/// The diagnostic of the protocol of `diagnostic`, whose text `index` indexes, with its help
/// after its message.
fn lsp_diagnostic(index: &LineIndex, diagnostic: &Diagnostic) -> lsp_types::Diagnostic {
    let message = match &diagnostic.help {
        Some(help) => format!("{}\n\nhelp: {help}", diagnostic.message),
        None => diagnostic.message.clone(),
    };
    lsp_types::Diagnostic {
        range: index.range(diagnostic.range),
        severity: Some(convert::severity(diagnostic.severity)),
        source: Some(convert::source(diagnostic.source).to_string()),
        message,
        ..Default::default()
    }
}

/// The path on disk of the file of `uri`: canonical, or for a file that does not exist, the
/// canonical path of its directory with its name.
fn file_path(uri: &Url) -> Option<PathBuf> {
    canonical(&uri.to_file_path().ok()?)
}

fn canonical(path: &Path) -> Option<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(path) => Some(path),
        Err(_) => Some(
            std::fs::canonicalize(path.parent()?)
                .ok()?
                .join(path.file_name()?),
        ),
    }
}

/// The path `path` relative to `root`, with its names separated by `/`.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let names: Option<Vec<&str>> = (path.strip_prefix(root).ok()?.components())
        .map(|component| component.as_os_str().to_str())
        .collect();
    Some(names?.join("/"))
}
