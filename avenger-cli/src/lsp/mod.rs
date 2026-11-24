//! `avenger lsp`: the Language Server Protocol server of avenger-sql scripts (`.av.sql`),
//! modules (`.av.sqlm`), and the `{avenger-sql}` cells of Quarto documents (`.qmd`), over
//! avenger-sql-language.
//!
//! The server keeps every project that holds an open file live: the language analysis of every
//! script and module of the project, with the text of an open file in place of the text on disk,
//! on the main loop, and the loader of its data scripts on a thread of its own, whose loads come
//! back to the main loop. An edit analyzes the file and the files that it reaches, and the server
//! publishes the diagnostics of those files alone. A saved data script or module, and a changed
//! data file, load again. A file outside every project, or without a role in its project, is
//! analyzed alone.
//!
//! Each cell of a document of a project is a chart script of the project's analysis, and the
//! server answers for positions inside cells and nothing else, so other servers of a document
//! answer for the rest of it.

pub(crate) mod convert;
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

use self::convert::{Encoding, LineIndex, Place};
use self::loader::{Load, Loaded, LoaderThread};
use avenger_sql_project::qmd::{is_document, Qmd};

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
            Encoding::Utf32 => PositionEncodingKind::UTF32,
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
        // Indentation as the user types: Return, and a line's first `)`, `]`, or `|>`.
        document_on_type_formatting_provider: Some(lsp_types::DocumentOnTypeFormattingOptions {
            first_trigger_character: avenger_sql_language::ON_TYPE_TRIGGERS[0].to_string(),
            more_trigger_character: Some(
                (avenger_sql_language::ON_TYPE_TRIGGERS[1..].iter())
                    .map(char::to_string)
                    .collect(),
            ),
        }),
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
    /// The analysis, with the client's texts of the open files and the texts on disk of the
    /// others.
    analysis: Analysis,
    /// The watch of the project's root, when the client watches no files for the server.
    watch: Option<Watch>,
}

/// Where a file of the client is to the server.
enum Located {
    /// A script or a module of the project at `root`, at `path` relative to the root.
    Project { root: PathBuf, path: String },
    /// A document of the project at `root`, at `path` relative to the root.
    Document { root: PathBuf, path: String },
    /// A script or a module that is analyzed alone.
    Loose,
    /// A document outside every project, whose cells are analyzed alone.
    LooseDocument,
}

/// A document outside every project, and the script of its cells, analyzed alone.
struct LooseDocument {
    qmd: Qmd,
    script: Document,
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
    /// The open Quarto documents outside every project, by path.
    loose_documents: BTreeMap<PathBuf, LooseDocument>,
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
            loose_documents: BTreeMap::new(),
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

    /// Where the file at `path` is: a script, a module, or a document of its project, whose
    /// project the server opens, or a file that is analyzed alone.
    fn locate(&mut self, path: &Path) -> Located {
        let loose = match is_document(&path.to_string_lossy()) {
            true => Located::LooseDocument,
            false => Located::Loose,
        };
        let Some(root) = find_root(path) else {
            return loose;
        };
        let Some(relative) = relative(&root, path) else {
            return loose;
        };
        if !self.open_project(&root) {
            return loose;
        }
        let state = &self.projects[&root];
        match state.files.manifest().role(&relative) {
            Ok(Some(ManifestRole::DataScript | ManifestRole::ChartScript))
            | Ok(Some(ManifestRole::Module(_))) => Located::Project {
                root,
                path: relative,
            },
            Ok(Some(ManifestRole::Document)) => Located::Document {
                root,
                path: relative,
            },
            _ => loose,
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
        let mut texts = Vec::new();
        for (path, file) in &self.open {
            let Some(path) = relative(root, path) else {
                continue;
            };
            match analysis.qmd(&path) {
                Some(_) => drop(analysis.set_document(&path, Some(file.text.clone()))),
                None => texts.push((path, file.text.clone())),
            }
        }
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
            Located::Document { root, path } => {
                let reached = self.set_document(&root, &path, Some(text));
                self.publish(&root, &reached);
            }
            Located::LooseDocument => {
                let qmd = Qmd::parse(text);
                let script = Document::new_cells(
                    qmd.script.text.as_str(),
                    qmd.script.cell_starts(),
                    Arc::new(MemoryCatalog::new()),
                    Arc::new(Library::new()),
                    Options::default(),
                );
                let document = LooseDocument { qmd, script };
                self.loose_documents.insert(path.to_path_buf(), document);
                self.publish_loose(path);
            }
        }
    }

    /// Sets the text of the document at `path` of the project at `root` to `text`, or removes
    /// the document without one, and returns the files that the change reaches, the document
    /// with them.
    fn set_document(&mut self, root: &Path, path: &str, text: Option<String>) -> BTreeSet<String> {
        let state = self.projects.get_mut(root).expect("an open project");
        state.analysis.set_document(path, text)
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
        let loose = self.loose.remove(&path).is_some();
        if loose || self.loose_documents.remove(&path).is_some() {
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
                        // A document, new, changed, or removed, registers nothing, so the
                        // project need not open again.
                        if let Ok(Some(ManifestRole::Document)) = role {
                            let text = std::fs::read_to_string(path).ok();
                            let reached = self.set_document(&root, &relative, text);
                            self.publish(&root, &reached);
                            continue;
                        }
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
        let state = self.projects.get(root);
        for path in client_files(&old) {
            let held = state.is_some_and(|state| {
                state.analysis.qmd(&path).is_some()
                    || state.analysis.project().document(&path).is_some()
            });
            if held {
                continue;
            }
            if let Ok(uri) = Url::from_file_path(root.join(&path)) {
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
            reached = client_files(&self.projects[&root]);
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

    /// Publishes the diagnostics of the files of the client that hold the files at `paths` of
    /// the analysis of the project at `root`: a document with those of its cells' script and of
    /// its cells' options. A file that the project no longer holds has none.
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
            let mut diagnostics = Vec::new();
            if let Some(document) = state.analysis.qmd(path) {
                let place = Place::new(&document.text, Some(document), self.encoding);
                diagnostics.extend(option_diagnostics(&place, document));
                diagnostics
                    .extend((project.diagnostics(path).iter()).map(|d| lsp_diagnostic(&place, d)));
            } else if let Some(document) = project.document(path) {
                let place = Place::new(document.text(), None, self.encoding);
                diagnostics
                    .extend((project.diagnostics(path).iter()).map(|d| lsp_diagnostic(&place, d)));
            }
            self.publish_diagnostics(uri, diagnostics, version);
        }
    }

    /// Publishes the diagnostics of the file at `path` that is analyzed alone.
    fn publish_loose(&self, path: &Path) {
        let Ok(uri) = Url::from_file_path(path) else {
            return;
        };
        let mut diagnostics = Vec::new();
        if let Some(document) = self.loose.get(path) {
            let place = Place::new(document.text(), None, self.encoding);
            diagnostics.extend(
                document
                    .diagnostics()
                    .iter()
                    .map(|d| lsp_diagnostic(&place, d)),
            );
        }
        if let Some(document) = self.loose_documents.get(path) {
            let place = Place::new(&document.qmd.text, Some(&document.qmd), self.encoding);
            diagnostics.extend(option_diagnostics(&place, &document.qmd));
            diagnostics
                .extend((document.script.diagnostics().iter()).map(|d| lsp_diagnostic(&place, d)));
        }
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

/// The paths of the files of the client that the project of `state` holds: its scripts and
/// modules, and its documents.
fn client_files(state: &ProjectState) -> BTreeSet<String> {
    (state.analysis.project().files().map(str::to_string)).collect()
}

/// The diagnostic of the protocol of `diagnostic`, at its place `place`, with its help after its
/// message.
/// The diagnostics of the errors of the options of the cells of `document`, whose place in the
/// client's file is `place`.
fn option_diagnostics(place: &Place, document: &Qmd) -> Vec<lsp_types::Diagnostic> {
    let mut diagnostics = Vec::new();
    for cell in &document.cells {
        for diagnostic in cell.option_diagnostics() {
            let range = diagnostic.range;
            let at = |offset: avenger_sql_language::Offset| {
                place.document_position(cell.to_document(usize::from(offset)))
            };
            diagnostics.push(lsp_types::Diagnostic {
                range: lsp_types::Range::new(at(range.start()), at(range.end())),
                severity: Some(convert::severity(diagnostic.severity)),
                source: Some(convert::source(diagnostic.source).to_string()),
                message: diagnostic.message,
                ..Default::default()
            });
        }
    }
    diagnostics
}

fn lsp_diagnostic(place: &Place, diagnostic: &Diagnostic) -> lsp_types::Diagnostic {
    let message = match &diagnostic.help {
        Some(help) => format!("{}\n\nhelp: {help}", diagnostic.message),
        None => diagnostic.message.clone(),
    };
    lsp_types::Diagnostic {
        range: place.range(diagnostic.range),
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
