//! `avenger lsp` in process, over `lsp-server`'s connection in memory, on copies of
//! avenger-sql-project's fixture project: the capabilities, diagnostics that an edit publishes in
//! another file, the requests, positions in UTF-16, loose scripts, a saved data script, a data
//! file that changes on disk, under the server's watch and the client's, and the program over its
//! standard input and output, and the cells of Quarto documents. The ignored test `lsp_latency`
//! times edits in a generated project.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidChangeWatchedFiles, DidOpenTextDocument, DidSaveTextDocument,
        Exit, Initialized, Notification as _, PublishDiagnostics,
    },
    request::{
        CodeActionRequest, Completion, DocumentSymbolRequest, FoldingRangeRequest, Formatting,
        GotoDefinition, HoverRequest, Initialize, OnTypeFormatting, PrepareRenameRequest,
        References, RegisterCapability, Rename, Request as _, SelectionRangeRequest,
        SemanticTokensFullRequest, Shutdown, SignatureHelpRequest,
    },
    ClientCapabilities, GeneralClientCapabilities, InitializeParams, InitializeResult, Position,
    PositionEncodingKind, PublishDiagnosticsParams, Url,
};
use serde_json::{json, Value};

const SUMMARIES: &str = "data/summaries.av.sql";
const ORIGINS: &str = "charts/origins.av.sql";

/// A copy of the fixture project in a temporary directory, by its canonical root.
fn project() -> (tempfile::TempDir, PathBuf) {
    fn walk(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            let target = to.join(path.file_name().unwrap());
            match path.is_dir() {
                true => walk(&path, &target),
                false => drop(std::fs::copy(&path, &target).unwrap()),
            }
        }
    }
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-sql-project/tests/projects/vega");
    let directory = tempfile::tempdir().unwrap();
    walk(&fixture, directory.path());
    let root = directory.path().canonicalize().unwrap();
    (directory, root)
}

/// A client of a server that runs in a thread.
struct Client {
    connection: Connection,
    thread: Option<std::thread::JoinHandle<Result<(), String>>>,
    next: i32,
    /// The latest diagnostics of each file that the server published.
    diagnostics: std::collections::HashMap<Url, Vec<lsp_types::Diagnostic>>,
    /// The methods of the requests of the server, which the client answers with no result.
    requests: Vec<String>,
    result: InitializeResult,
}

impl Client {
    /// A server, initialized with `capabilities`.
    fn start(capabilities: ClientCapabilities) -> Client {
        let (server, connection) = Connection::memory();
        let thread = std::thread::spawn(move || avenger_cli::lsp::serve(server));
        let mut client = Client {
            connection,
            thread: Some(thread),
            next: 0,
            diagnostics: Default::default(),
            requests: Vec::new(),
            result: InitializeResult::default(),
        };
        let params = InitializeParams {
            capabilities,
            ..Default::default()
        };
        let result = client.request::<Initialize>(serde_json::to_value(params).unwrap());
        client.result = serde_json::from_value(result).unwrap();
        client.notify::<Initialized>(json!({}));
        client
    }

    fn notify<N: lsp_types::notification::Notification>(&self, params: Value) {
        let notification = Notification::new(N::METHOD.to_string(), params);
        let _ = (self.connection.sender).send(Message::Notification(notification));
    }

    /// The result of the request, after the messages before it, which the client takes.
    fn request<R: lsp_types::request::Request>(&mut self, params: Value) -> Value {
        self.next += 1;
        let id = RequestId::from(self.next);
        let request = Request::new(id.clone(), R::METHOD.to_string(), params);
        self.connection
            .sender
            .send(Message::Request(request))
            .unwrap();
        loop {
            match self.receive(Duration::from_secs(30)) {
                Some(Message::Response(response)) if response.id == id => {
                    return match response.error {
                        Some(error) => json!({"error": error.message}),
                        None => response.result.unwrap_or(Value::Null),
                    }
                }
                Some(_) => {}
                None => panic!("no response to {}", R::METHOD),
            }
        }
    }

    /// The next message, which updates the diagnostics, or none within `timeout`. The client
    /// answers a request of the server.
    fn receive(&mut self, timeout: Duration) -> Option<Message> {
        let message = self.connection.receiver.recv_timeout(timeout).ok()?;
        match &message {
            Message::Notification(notification)
                if notification.method == PublishDiagnostics::METHOD =>
            {
                let params: PublishDiagnosticsParams =
                    serde_json::from_value(notification.params.clone()).unwrap();
                self.diagnostics.insert(params.uri, params.diagnostics);
            }
            Message::Request(request) => {
                self.requests.push(request.method.clone());
                let response = Response::new_ok(request.id.clone(), Value::Null);
                let _ = self.connection.sender.send(Message::Response(response));
            }
            _ => {}
        }
        Some(message)
    }

    /// Takes messages until `done` holds of the diagnostics, or fails after a while.
    fn until(&mut self, what: &str, done: impl Fn(&Client) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(start.elapsed() < Duration::from_secs(30), "no {what}");
            self.receive(Duration::from_millis(100));
        }
    }

    fn messages(&self, uri: &Url) -> Vec<String> {
        (self.diagnostics.get(uri).into_iter().flatten())
            .map(|diagnostic| diagnostic.message.clone())
            .collect()
    }

    fn open(&self, uri: &Url, text: &str) {
        self.notify::<DidOpenTextDocument>(json!({
            "textDocument": {"uri": uri, "languageId": "avenger-sql", "version": 1, "text": text}
        }));
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // After a failure, the server may not answer, and the test fails as it is.
        if std::thread::panicking() {
            return;
        }
        self.request::<Shutdown>(Value::Null);
        self.notify::<Exit>(Value::Null);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap().unwrap();
        }
    }
}

fn uri(root: &Path, path: &str) -> Url {
    Url::from_file_path(root.join(path)).unwrap()
}

fn position(line: u32, character: u32) -> Value {
    serde_json::to_value(Position::new(line, character)).unwrap()
}

#[test]
fn the_server_takes_utf_8_positions_when_the_client_offers_them() {
    let client = Client::start(ClientCapabilities::default());
    let capabilities = &client.result.capabilities;
    assert_eq!(
        capabilities.position_encoding,
        Some(PositionEncodingKind::UTF16)
    );
    assert!(capabilities.semantic_tokens_provider.is_some());
    assert!(capabilities.rename_provider.is_some());
    let on_type = capabilities
        .document_on_type_formatting_provider
        .as_ref()
        .unwrap();
    assert_eq!(on_type.first_trigger_character, "\n");
    assert_eq!(
        on_type.more_trigger_character.as_deref(),
        Some(&[")", "]", ">", " "].map(String::from)[..])
    );
    drop(client);
    let utf8 = ClientCapabilities {
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(vec![
                PositionEncodingKind::UTF8,
                PositionEncodingKind::UTF16,
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let client = Client::start(utf8);
    let encoding = client.result.capabilities.position_encoding.clone();
    assert_eq!(encoding, Some(PositionEncodingKind::UTF8));
}

#[test]
fn an_edit_publishes_the_diagnostics_of_the_files_that_it_reaches() {
    let (_directory, root) = project();
    let mut client = Client::start(ClientCapabilities::default());
    let (summaries, origins) = (uri(&root, SUMMARIES), uri(&root, ORIGINS));
    let text = std::fs::read_to_string(root.join(SUMMARIES)).unwrap();
    client.open(&summaries, &text);
    // The project opens, and after its first load, publishes every file's diagnostics, which
    // are none: no error of a table that the load has not read yet.
    client.until("the chart's diagnostics", |c| {
        c.diagnostics.contains_key(&origins)
    });
    assert_eq!(client.messages(&origins), Vec::<String>::new());

    // An edit that renames the column that the chart reads publishes the chart's error.
    let line = text
        .lines()
        .position(|line| line.contains("AS cars"))
        .unwrap() as u32;
    let character = text
        .lines()
        .nth(line as usize)
        .unwrap()
        .find("cars")
        .unwrap() as u32;
    client.notify::<DidChangeTextDocument>(json!({
        "textDocument": {"uri": summaries, "version": 2},
        "contentChanges": [{
            "range": {"start": position(line, character), "end": position(line, character + 4)},
            "text": "n"
        }]
    }));
    client.until("the chart's error", |c| {
        c.messages(&origins)
            .iter()
            .any(|m| m.contains("Unrecognized name: cars"))
    });
}

#[test]
fn the_requests_answer_across_the_files_of_a_project() {
    let (_directory, root) = project();
    let mut client = Client::start(ClientCapabilities::default());
    let origins = uri(&root, ORIGINS);
    let text = std::fs::read_to_string(root.join(ORIGINS)).unwrap();
    client.open(&origins, &text);
    client.until("the load", |c| {
        c.diagnostics.contains_key(&origins) && c.messages(&origins).is_empty()
    });
    let line = text
        .lines()
        .position(|line| line.starts_with("FROM heavy_by_origin"))
        .unwrap() as u32;
    let at = json!({"textDocument": {"uri": origins}, "position": position(line, 7)});

    // The definition of the table is the statement of the data script that defines it.
    let definition = client.request::<GotoDefinition>(at.clone());
    assert_eq!(
        definition["uri"],
        json!(uri(&root, SUMMARIES)),
        "{definition}"
    );
    let hover = client.request::<HoverRequest>(at.clone());
    let shown = hover["contents"]["value"].as_str().unwrap_or_default();
    assert!(shown.contains("heavy_by_origin"), "{hover}");
    let mut references = at.clone();
    references["context"] = json!({"includeDeclaration": true});
    let references = client.request::<References>(references);
    let files: Vec<&str> = (references.as_array().unwrap().iter())
        .filter_map(|location| location["uri"].as_str())
        .collect();
    assert!(files.len() >= 2, "{references}");

    // A rename edits the data script and the chart.
    let mut rename = at.clone();
    rename["newName"] = json!("heavy_origins");
    let edit = client.request::<Rename>(rename);
    let changes = edit["changes"].as_object().unwrap();
    assert!(
        changes.contains_key(uri(&root, SUMMARIES).as_str()),
        "{edit}"
    );
    assert!(changes.contains_key(origins.as_str()), "{edit}");

    // Completion after `FROM ` offers the project's tables.
    let mut completion = at.clone();
    completion["position"] = position(line, 5);
    let completions = client.request::<Completion>(completion);
    let labels: Vec<&str> = (completions["items"].as_array().unwrap().iter())
        .filter_map(|item| item["label"].as_str())
        .collect();
    assert!(labels.contains(&"wet_days"), "{labels:?}");

    let document = json!({"textDocument": {"uri": origins}});
    let symbols = client.request::<DocumentSymbolRequest>(document.clone());
    assert!(!symbols.as_array().unwrap().is_empty(), "{symbols}");
    let tokens = client.request::<SemanticTokensFullRequest>(document.clone());
    assert!(
        tokens["data"].as_array().unwrap().len() >= 5 * 5,
        "{tokens}"
    );
    let mut formatting = document;
    formatting["options"] = json!({"tabSize": 2, "insertSpaces": true});
    assert!(client.request::<Formatting>(formatting).is_array());

    let prepared = client.request::<PrepareRenameRequest>(at.clone());
    assert_eq!(prepared["start"], position(line, 5), "{prepared}");
    let mut selection = json!({"textDocument": {"uri": origins}});
    selection["positions"] = json!([position(line, 7)]);
    let ranges = client.request::<SelectionRangeRequest>(selection);
    assert!(ranges[0]["parent"].is_object(), "{ranges}");
    let summaries = json!({"textDocument": {"uri": uri(&root, SUMMARIES)}});
    let folds = client.request::<FoldingRangeRequest>(summaries);
    assert!(folds.as_array().unwrap().len() >= 3, "{folds}");

    // Signature help in the arguments of a mark.
    let (line, call) = (text.lines().enumerate())
        .find_map(|(n, line)| Some((n as u32, line.find("mark_bar(")? as u32)))
        .unwrap();
    let mut signature = json!({"textDocument": {"uri": origins}});
    signature["position"] = position(line, call + "mark_bar(".len() as u32);
    let help = client.request::<SignatureHelpRequest>(signature);
    let label = help["signatures"][0]["label"].as_str().unwrap_or_default();
    assert!(label.starts_with("mark_bar("), "{help}");
}

/// Positions in UTF-16, after a character of two units, and two edits of one change, the second
/// at a position in the text after the first.
#[test]
fn edits_take_positions_in_utf_16_code_units() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("emoji.av.sql");
    let text = "/* \u{1F600} */ FROM sales\n";
    std::fs::write(&path, text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let emoji = Url::from_file_path(&path).unwrap();
    client.open(&emoji, text);
    client.until("diagnostics", |c| c.diagnostics.contains_key(&emoji));
    // `sales` is at unit 14: `/* ` is 3 units, the emoji 2, and ` */ FROM ` 9.
    let range =
        |start: u32, end: u32| json!({"start": position(0, start), "end": position(0, end)});
    client.notify::<DidChangeTextDocument>(json!({
        "textDocument": {"uri": emoji, "version": 2},
        "contentChanges": [
            {"range": range(14, 19), "text": "cars"},
            {"range": range(14, 18), "text": "nada"}
        ]
    }));
    client.until("the error", |c| {
        c.messages(&emoji)
            .iter()
            .any(|m| m.contains("Table not found: nada"))
    });
    let diagnostic = &client.diagnostics[&emoji][0];
    assert_eq!(
        diagnostic.range.start,
        Position::new(0, 14),
        "{diagnostic:?}"
    );
    assert_eq!(diagnostic.range.end, Position::new(0, 18), "{diagnostic:?}");
}

#[test]
fn a_loose_script_is_analyzed_alone() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("alone.av.sql");
    std::fs::write(&path, "FROM sales |> SELECT x\n").unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let alone = Url::from_file_path(&path).unwrap();
    client.open(&alone, "FROM sales |> SELECT x\n");
    client.until("the loose script's errors", |c| {
        !c.messages(&alone).is_empty()
    });
    assert!(client.messages(&alone)[0].contains("Table not found: sales"));

    // The quick fix of a diagnostic is a code action.
    let fix = Url::from_file_path(directory.path().canonicalize().unwrap().join("fix.av.sql"));
    let fix = fix.unwrap();
    client.open(&fix, "FROM penguins |> SELECT 5abc\n");
    client.until("the fix's error", |c| !c.messages(&fix).is_empty());
    let actions = client.request::<CodeActionRequest>(json!({
        "textDocument": {"uri": fix},
        "range": {"start": position(0, 26), "end": position(0, 26)},
        "context": {"diagnostics": []}
    }));
    let edit = &actions[0]["edit"]["changes"][fix.as_str()][0];
    assert_eq!(edit["newText"], "`5abc`", "{actions}");
}

#[test]
fn a_changed_data_file_loads_its_scripts_again() {
    let (_directory, root) = project();
    let mut client = Client::start(ClientCapabilities::default());
    let origins = uri(&root, ORIGINS);
    let text = std::fs::read_to_string(root.join(ORIGINS)).unwrap();
    client.open(&origins, &text);
    // After the first load, the regions have their column `region`.
    client.until("the load", |c| {
        c.diagnostics.contains_key(&origins) && c.messages(&origins).is_empty()
    });

    // Without the column, the chart that reads it has an error after the load.
    std::fs::write(
        root.join("data/files/regions.csv"),
        "Origin,area\nUSA,Americas\n",
    )
    .unwrap();
    client.until("the chart's error after the load", |c| {
        c.messages(&origins)
            .iter()
            .any(|m| m.contains("Unrecognized name: region"))
    });
}

#[test]
fn a_saved_data_script_loads_again() {
    let (_directory, root) = project();
    std::fs::write(
        root.join("data/files/areas.csv"),
        "Origin,area\nUSA,Americas\n",
    )
    .unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let (regions, origins) = (uri(&root, "data/regions.av.sql"), uri(&root, ORIGINS));
    let text = std::fs::read_to_string(root.join("data/regions.av.sql")).unwrap();
    client.open(&regions, &text);
    client.until("the load", |c| c.diagnostics.contains_key(&origins));
    assert_eq!(client.messages(&origins), Vec::<String>::new());

    // The edit reads other files, which the project has not read until the save.
    let edited = text.replace("regions.csv", "areas.csv");
    client.notify::<DidChangeTextDocument>(json!({
        "textDocument": {"uri": regions, "version": 2},
        "contentChanges": [{"text": edited}]
    }));
    client.until("the unread table", |c| {
        c.messages(&origins)
            .iter()
            .any(|m| m.contains("has not read them"))
    });
    std::fs::write(root.join("data/regions.av.sql"), &edited).unwrap();
    client.notify::<DidSaveTextDocument>(json!({"textDocument": {"uri": regions}}));
    client.until("the chart's error after the load", |c| {
        c.messages(&origins)
            .iter()
            .any(|m| m.contains("Unrecognized name: region"))
    });
}

#[test]
fn the_client_watches_the_files_when_it_offers_to() {
    let (_directory, root) = project();
    let capabilities: ClientCapabilities = serde_json::from_value(json!({
        "workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}
    }))
    .unwrap();
    let mut client = Client::start(capabilities);
    let origins = uri(&root, ORIGINS);
    let text = std::fs::read_to_string(root.join(ORIGINS)).unwrap();
    client.open(&origins, &text);
    client.until("the load", |c| c.diagnostics.contains_key(&origins));
    assert_eq!(client.requests, [RegisterCapability::METHOD]);

    let file = root.join("data/files/regions.csv");
    std::fs::write(&file, "Origin,area\nUSA,Americas\n").unwrap();
    client.notify::<DidChangeWatchedFiles>(json!({
        "changes": [{"uri": Url::from_file_path(&file).unwrap(), "type": 2}]
    }));
    client.until("the chart's error after the load", |c| {
        c.messages(&origins)
            .iter()
            .any(|m| m.contains("Unrecognized name: region"))
    });
}

/// The program `avenger lsp`, over its standard input and standard output.
#[test]
fn the_program_serves_standard_input_and_output() {
    use std::io::{BufReader, Write as _};
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_avenger"))
        .arg("lsp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let params = serde_json::to_value(InitializeParams::default()).unwrap();
    let messages = [
        Message::Request(Request::new(1.into(), Initialize::METHOD.into(), params)),
        Message::Notification(Notification::new(Initialized::METHOD.into(), json!({}))),
        Message::Request(Request::new(2.into(), Shutdown::METHOD.into(), Value::Null)),
        Message::Notification(Notification::new(Exit::METHOD.into(), Value::Null)),
    ];
    for message in messages {
        message.write(&mut stdin).unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ids = Vec::new();
    while let Some(message) = Message::read(&mut stdout).unwrap() {
        match message {
            Message::Response(response) => {
                assert!(response.error.is_none(), "{response:?}");
                ids.push(response.id);
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(ids, [RequestId::from(1), RequestId::from(2)]);
    assert!(child.wait().unwrap().success());
}

/// A document of the fixture project: a cell, prose, and a cell in an indented fence with an
/// error.
const REPORT: &str = "# Report\n\n```{avenger-sql}\nFROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n```\n\n1. Weather\n\n    ```{avenger-sql}\n    FROM wet_days |>   SELECT nope\n    ```\n";

#[test]
fn the_cells_of_a_document_answer_at_their_places_in_it() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    std::fs::write(root.join("reports/overview.qmd"), REPORT).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let report = uri(&root, "reports/overview.qmd");
    client.open(&report, REPORT);
    client.until("the document's diagnostics", |c| {
        c.diagnostics.contains_key(&report)
    });
    let diagnostics = &client.diagnostics[&report];
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("Unrecognized name: nope"));
    assert_eq!(diagnostics[0].range.start, Position::new(10, 30));

    // Requests inside a cell answer, and in the prose, nothing.
    let at = |line, character| json!({"textDocument": {"uri": report}, "position": position(line, character)});
    let definition = client.request::<GotoDefinition>(at(3, 7));
    assert_eq!(
        definition["uri"],
        json!(uri(&root, SUMMARIES)),
        "{definition}"
    );
    let hover = client.request::<HoverRequest>(at(3, 7));
    assert_eq!(hover["range"]["start"], position(3, 5), "{hover}");
    assert_eq!(client.request::<HoverRequest>(at(0, 3)), Value::Null);
    assert_eq!(client.request::<HoverRequest>(at(5, 1)), Value::Null);
    let hover = client.request::<HoverRequest>(at(10, 10));
    assert_eq!(hover["range"]["start"], position(10, 9), "{hover}");

    // The tokens are in the cells' lines.
    let document = json!({"textDocument": {"uri": report}});
    let tokens = client.request::<SemanticTokensFullRequest>(document.clone());
    assert_eq!(tokens["data"][0], 3, "{tokens}");
    // Formatting replaces the lines of the indented cell, with its indent.
    let mut formatting = document;
    formatting["options"] = json!({"tabSize": 2, "insertSpaces": true});
    let edits = client.request::<Formatting>(formatting);
    assert_eq!(edits.as_array().unwrap().len(), 1, "{edits}");
    assert_eq!(edits[0]["range"]["start"], position(10, 0), "{edits}");
    assert_eq!(edits[0]["range"]["end"], position(11, 0), "{edits}");
    assert_eq!(
        edits[0]["newText"],
        "    FROM wet_days\n    |> SELECT nope\n"
    );
}

#[test]
fn a_rename_reaches_the_cells_of_a_closed_document() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    let text = "```{avenger-sql}\nFROM heavy_by_origin |> SELECT cars\n```\n\n  ```{avenger-sql}\n  FROM heavy_by_origin |> SELECT Origin\n  ```\n";
    std::fs::write(root.join("reports/overview.qmd"), text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let (summaries, report) = (uri(&root, SUMMARIES), uri(&root, "reports/overview.qmd"));
    let source = std::fs::read_to_string(root.join(SUMMARIES)).unwrap();
    client.open(&summaries, &source);
    client.until("the document's diagnostics", |c| {
        c.diagnostics.contains_key(&report)
    });
    assert_eq!(client.messages(&report), Vec::<String>::new());

    let line = source
        .lines()
        .position(|line| line.contains("TABLE heavy_by_origin"))
        .unwrap();
    let character = source
        .lines()
        .nth(line)
        .unwrap()
        .find("heavy_by_origin")
        .unwrap();
    let edit = client.request::<Rename>(json!({
        "textDocument": {"uri": summaries},
        "position": position(line as u32, character as u32),
        "newName": "heavy_origins"
    }));
    let edits = edit["changes"][report.as_str()].as_array().unwrap();
    let starts: Vec<&Value> = edits.iter().map(|edit| &edit["range"]["start"]).collect();
    assert_eq!(starts, [&position(1, 5), &position(5, 7)], "{edit}");
    assert!(
        edit["changes"][uri(&root, ORIGINS).as_str()].is_array(),
        "{edit}"
    );

    // A change of the document on disk publishes its diagnostics.
    let broken = text.replace("SELECT cars", "SELECT carz");
    std::fs::write(root.join("reports/overview.qmd"), broken).unwrap();
    client.until("the document's error", |c| {
        c.messages(&report)
            .iter()
            .any(|m| m.contains("Unrecognized name: carz"))
    });
}

#[test]
fn an_edit_in_an_indented_cell_takes_utf_16_positions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("alone.qmd");
    let text =
        "Prose \u{1F600}.\n\n    ```{avenger-sql}\n    /* \u{1F600} */ FROM sales\n    ```\n";
    std::fs::write(&path, text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let alone = Url::from_file_path(&path).unwrap();
    client.open(&alone, text);
    client.until("diagnostics", |c| c.diagnostics.contains_key(&alone));
    // `sales` is at unit 18 of its line: the indent is 4 units, and the cell's 14 before it.
    let range =
        |start: u32, end: u32| json!({"start": position(3, start), "end": position(3, end)});
    client.notify::<DidChangeTextDocument>(json!({
        "textDocument": {"uri": alone, "version": 2},
        "contentChanges": [{"range": range(18, 23), "text": "nada"}]
    }));
    client.until("the error", |c| {
        c.messages(&alone)
            .iter()
            .any(|m| m.contains("Table not found: nada"))
    });
    let diagnostic = &client.diagnostics[&alone][0];
    assert_eq!(
        diagnostic.range.start,
        Position::new(3, 18),
        "{diagnostic:?}"
    );
    assert_eq!(diagnostic.range.end, Position::new(3, 22), "{diagnostic:?}");
}

/// The time of edits of a data script in a project of many data scripts and charts: from each
/// edit to the publication of the diagnostics of the edited script.
#[test]
#[ignore]
fn lsp_latency() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::write(root.join("avenger.toml"), "[project]\nname = \"bench\"\n").unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::create_dir_all(root.join("charts")).unwrap();
    let (scripts, charts) = (100, 200);
    for script in 0..scripts {
        let from = match script {
            0 => "(SELECT 1 AS x, 2 AS y)".to_string(),
            n => format!("t{}", n - 1),
        };
        let text = format!("CREATE VIEW t{script} AS (FROM {from} |> SELECT x, y);\n");
        std::fs::write(root.join(format!("data/s{script}.av.sql")), text).unwrap();
    }
    for chart in 0..charts {
        let text = format!(
            "FROM t{} |> AGGREGATE mark_symbol(x => x, y => y) AS chart\n",
            chart % scripts
        );
        std::fs::write(root.join(format!("charts/c{chart}.av.sql")), text).unwrap();
    }
    let mut client = Client::start(ClientCapabilities::default());
    let edited = uri(&root, "data/s50.av.sql");
    let text = std::fs::read_to_string(root.join("data/s50.av.sql")).unwrap();
    let start = Instant::now();
    client.open(&edited, &text);
    client.until("diagnostics", |c| c.diagnostics.contains_key(&edited));
    println!("open: {:?}", start.elapsed());
    let mut times = Vec::new();
    for version in 2..12 {
        client.diagnostics.remove(&edited);
        let start = Instant::now();
        let text = format!("{text}-- edit {version}\n");
        client.notify::<DidChangeTextDocument>(json!({
            "textDocument": {"uri": edited, "version": version},
            "contentChanges": [{"text": text}]
        }));
        client.until("diagnostics", |c| c.diagnostics.contains_key(&edited));
        times.push(start.elapsed());
    }
    times.sort();
    println!(
        "{scripts} data scripts, {charts} charts: edit median {:?}, max {:?}",
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

/// The semantic tokens of the client file `uri`, as absolute lines, columns, lengths, and types,
/// after a check that each starts after the one before ends.
fn tokens(client: &mut Client, uri: &Url) -> Vec<(u32, u32, u32, u32)> {
    let tokens = client.request::<SemanticTokensFullRequest>(json!({"textDocument": {"uri": uri}}));
    let data: Vec<u32> = (tokens["data"].as_array().unwrap().iter())
        .map(|value| value.as_u64().unwrap() as u32)
        .collect();
    let (mut line, mut start, mut end) = (0, 0, 0);
    let mut absolute = Vec::new();
    for token in data.chunks(5) {
        line += token[0];
        start = match token[0] {
            0 => {
                assert!(
                    start + token[1] >= end,
                    "an overlap at {line}:{}",
                    start + token[1]
                );
                start + token[1]
            }
            _ => token[1],
        };
        end = start + token[2];
        absolute.push((line, start, token[2], token[3]));
    }
    absolute
}

/// The index of the token type `kind` in the legend of `client`'s server.
fn token_kind(client: &Client, kind: &str) -> u32 {
    let capabilities = serde_json::to_value(&client.result.capabilities).unwrap();
    let legend = &capabilities["semanticTokensProvider"]["legend"]["tokenTypes"];
    (legend.as_array().unwrap().iter())
        .position(|name| name == kind)
        .unwrap() as u32
}

/// Whether a token of `tokens` on `line` starts or ends inside the columns `start..end`, or
/// covers them.
fn touches(tokens: &[(u32, u32, u32, u32)], line: u32, (start, end): (u32, u32)) -> bool {
    (tokens.iter())
        .filter(|token| token.0 == line)
        .any(|&(_, column, length, _)| column < end && column + length > start)
}

#[test]
fn the_typst_markup_of_a_title_takes_no_tokens_and_keeps_its_errors() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    let text = "1. Weights\n\n    ```{avenger-sql}\n    FROM heavy_by_origin\n    |> AGGREGATE mark_bar(x => Origin, y => cars, orient => 'vertical', title => '\u{e9} #emph[Cars] $x')\n    ```\n\n```{avenger-sql}\nFROM heavy_by_origin\n|> AGGREGATE mark_bar(x => Origin, y => cars, title => title('*Plain*', syntax => 'plain'))\n```\n";
    std::fs::write(root.join("reports/overview.qmd"), text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let report = uri(&root, "reports/overview.qmd");
    client.open(&report, text);
    client.until("the document's diagnostics", |c| {
        c.diagnostics.contains_key(&report)
    });
    // The unclosed `$` is an error of Typst, on its line of the document.
    let diagnostics = &client.diagnostics[&report];
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].source.as_deref(), Some("typst"));
    assert_eq!(diagnostics[0].range.start.line, 4);

    // No token starts, ends, or lies inside the title's string, its quotes included, at UTF-16
    // columns, where `é` is one unit.
    let tokens = tokens(&mut client, &report);
    let units = |text: &str| text.encode_utf16().count() as u32;
    let line = text.lines().nth(4).unwrap();
    let title = line.find("'\u{e9}").unwrap();
    let string = (units(&line[..title]), units(line.trim_end_matches(')')));
    assert!(!touches(&tokens, 4, string), "{tokens:?}");
    // A string of the same call that isn't markup keeps its token.
    let vertical = line.find("'vertical'").unwrap();
    let (column, length) = (units(&line[..vertical]), units("'vertical'"));
    let member = token_kind(&client, "enumMember");
    assert!(tokens.contains(&(4, column, length, member)), "{tokens:?}");
    // A title of plain syntax is no markup, and its string takes one `string` token.
    let line = text.lines().nth(9).unwrap();
    let plain = line.find("'*Plain*'").unwrap();
    let (column, length) = (units(&line[..plain]), units("'*Plain*'"));
    let string = token_kind(&client, "string");
    assert!(tokens.contains(&(9, column, length, string)), "{tokens:?}");
}

#[test]
fn the_values_of_typed_literals_take_no_tokens() {
    let (_directory, root) = project();
    let text = "FROM heavy_by_origin\n|> EXTEND DATE '2024-01-03' AS d, TIMESTAMP '2024-01-03 12:30:00+05:30' AS t, NUMERIC '1.25' AS n, JSON '{}' AS j, 'plain' AS p\n";
    std::fs::write(root.join("charts/typed.av.sql"), text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let chart = uri(&root, "charts/typed.av.sql");
    client.open(&chart, text);
    client.until("the chart's diagnostics", |c| {
        c.diagnostics.contains_key(&chart)
    });
    let tokens = tokens(&mut client, &chart);
    let line = text.lines().nth(1).unwrap();
    let span = |literal: &str| {
        let at = line.find(literal).unwrap() as u32;
        (at, at + literal.len() as u32)
    };
    for value in [
        "'2024-01-03'",
        "'2024-01-03 12:30:00+05:30'",
        "'1.25'",
        "'{}'",
    ] {
        assert!(!touches(&tokens, 1, span(value)), "{value}: {tokens:?}");
    }
    // The keywords of the literals keep theirs, and a plain string keeps its `string` token.
    let keyword = token_kind(&client, "keyword");
    let (date, _) = span("DATE");
    assert!(tokens.contains(&(1, date, 4, keyword)), "{tokens:?}");
    let (plain, end) = span("'plain'");
    let string = token_kind(&client, "string");
    assert!(
        tokens.contains(&(1, plain, end - plain, string)),
        "{tokens:?}"
    );
}

#[test]
fn format_patterns_take_no_tokens() {
    let (_directory, root) = project();
    let text = "FROM wet_days\n|> AGGREGATE mark_bar(x => x(date, axis => axis(labels => labels(format => \"%'.2f\"))), y => precipitation) AS chart, ANY_VALUE(FORMAT_DATE('%Y-%m', date)) AS month, ANY_VALUE(CONCAT('plain', weather)) AS w\n";
    std::fs::write(root.join("charts/formats.av.sql"), text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let chart = uri(&root, "charts/formats.av.sql");
    client.open(&chart, text);
    client.until("the chart's diagnostics", |c| {
        c.diagnostics.contains_key(&chart)
    });
    let tokens = tokens(&mut client, &chart);
    let line = text.lines().nth(1).unwrap();
    let span = |literal: &str| {
        let at = line.find(literal).unwrap() as u32;
        (at, at + literal.len() as u32)
    };
    for format in ["\"%'.2f\"", "'%Y-%m'"] {
        assert!(!touches(&tokens, 1, span(format)), "{format}: {tokens:?}");
    }
    let (plain, end) = span("'plain'");
    let string = token_kind(&client, "string");
    assert!(
        tokens.contains(&(1, plain, end - plain, string)),
        "{tokens:?}"
    );
}

#[test]
fn regex_patterns_take_no_tokens() {
    let (_directory, root) = project();
    let text = "FROM wet_days\n|> EXTEND REGEXP_REPLACE(weather, r'(\\w)+', 'x') AS r, REGEXP_CONTAINS(weather, '^s') AS s\n";
    std::fs::write(root.join("charts/regexes.av.sql"), text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let chart = uri(&root, "charts/regexes.av.sql");
    client.open(&chart, text);
    client.until("the chart's diagnostics", |c| {
        c.diagnostics.contains_key(&chart)
    });
    let tokens = tokens(&mut client, &chart);
    let line = text.lines().nth(1).unwrap();
    let span = |literal: &str| {
        let at = line.find(literal).unwrap() as u32;
        (at, at + literal.len() as u32)
    };
    for pattern in ["r'(\\w)+'", "'^s'"] {
        assert!(!touches(&tokens, 1, span(pattern)), "{pattern}: {tokens:?}");
    }
    // The replacement is a string of its own, with its token.
    let (replacement, end) = span("'x'");
    let string = token_kind(&client, "string");
    assert!(
        tokens.contains(&(1, replacement, end - replacement, string)),
        "{tokens:?}"
    );
}

#[test]
fn a_module_s_member_answers_by_its_qualified_name() {
    let (_directory, root) = project();
    let chart = "IMPORT MODULE weather;\nFROM vega_datasets.seattle_weather\n|> WHERE weather.warm(temp_max)\n";
    std::fs::write(root.join("charts/warm.av.sql"), chart).unwrap();
    let typing = "IMPORT MODULE weather;\nFROM vega_datasets.seattle_weather\n|> WHERE weather.\n";
    std::fs::write(root.join("charts/typing.av.sql"), typing).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let warm = uri(&root, "charts/warm.av.sql");
    client.open(&warm, chart);
    client.until("the load", |c| {
        c.diagnostics.contains_key(&warm) && c.messages(&warm).is_empty()
    });
    let module = uri(&root, "modules/weather.av.sqlm");
    let member = "|> WHERE weather.".len() as u32;
    let at = json!({"textDocument": {"uri": warm}, "position": position(2, member + 1)});

    let definition = client.request::<GotoDefinition>(at.clone());
    assert_eq!(definition["uri"], json!(module), "{definition}");
    let hover = client.request::<HoverRequest>(at.clone());
    let shown = hover["contents"]["value"].as_str().unwrap_or_default();
    assert!(shown.contains("warm(t FLOAT64)"), "{hover}");
    let mut signature = json!({"textDocument": {"uri": warm}});
    signature["position"] = position(2, member + "warm(".len() as u32);
    let help = client.request::<SignatureHelpRequest>(signature);
    let label = help["signatures"][0]["label"].as_str().unwrap_or_default();
    assert_eq!(label, "weather.warm(t FLOAT64)", "{help}");

    let mut references = at.clone();
    references["context"] = json!({"includeDeclaration": true});
    let references = client.request::<References>(references);
    let files: Vec<&str> = (references.as_array().unwrap().iter())
        .filter_map(|location| location["uri"].as_str())
        .collect();
    assert!(
        files.contains(&module.as_str()) && files.contains(&warm.as_str()),
        "{references}"
    );
    let mut rename = at.clone();
    rename["newName"] = json!("mild");
    let edit = client.request::<Rename>(rename);
    let changes = edit["changes"].as_object().unwrap();
    assert!(
        changes.contains_key(module.as_str()) && changes.contains_key(warm.as_str()),
        "{edit}"
    );

    let typed = uri(&root, "charts/typing.av.sql");
    client.open(&typed, typing);
    let completion = json!({"textDocument": {"uri": typed}, "position": position(2, member)});
    let completions = client.request::<Completion>(completion);
    let labels: Vec<&str> = (completions["items"].as_array().unwrap().iter())
        .filter_map(|item| item["label"].as_str())
        .collect();
    // The fields of a column of the qualifier's name, if any, and the module's members.
    assert!(labels.contains(&"warm"), "{completions}");
}

#[test]
fn return_and_a_first_token_indent_their_lines() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("alone.av.sql");
    // The editor's own indentation put the cursor at 2.
    let text = "FROM sales\n|> AGGREGATE\n  \n";
    std::fs::write(&path, text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let alone = Url::from_file_path(&path).unwrap();
    client.open(&alone, text);
    client.until("diagnostics", |c| c.diagnostics.contains_key(&alone));
    let edits = client.request::<OnTypeFormatting>(json!({
        "textDocument": {"uri": alone}, "position": position(2, 2), "ch": "\n",
        "options": {"tabSize": 2, "insertSpaces": true}
    }));
    assert_eq!(
        edits,
        json!([{"range": {"start": position(2, 0), "end": position(2, 0)}, "newText": "  "}])
    );
    // A `)` in the middle of a line takes no edit.
    let edits = client.request::<OnTypeFormatting>(json!({
        "textDocument": {"uri": alone}, "position": position(0, 4), "ch": ")",
        "options": {"tabSize": 2, "insertSpaces": true}
    }));
    assert_eq!(edits, Value::Null);
}

#[test]
fn a_new_line_of_an_indented_cell_takes_the_fence_s_indent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("alone.qmd");
    let text = "- An item\n\n  ```{avenger-sql}\n  FROM sales\n  |> AGGREGATE\n    \n  ```\n";
    std::fs::write(&path, text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let alone = Url::from_file_path(&path).unwrap();
    client.open(&alone, text);
    client.until("diagnostics", |c| c.diagnostics.contains_key(&alone));
    let edits = client.request::<OnTypeFormatting>(json!({
        "textDocument": {"uri": alone}, "position": position(5, 4), "ch": "\n",
        "options": {"tabSize": 2, "insertSpaces": true}
    }));
    assert_eq!(
        edits,
        json!([{"range": {"start": position(5, 0), "end": position(5, 0)}, "newText": "  "}])
    );
    // The space after `GROUP` at the items' depth deletes a space at the start of the line, to
    // the depth of the fence's indent and the operator's keyword.
    let path = directory.path().canonicalize().unwrap().join("group.qmd");
    let text = "- An item\n\n  ```{avenger-sql}\n  FROM sales\n  |> AGGREGATE\n      n,\n      GROUP \n  ```\n";
    std::fs::write(&path, text).unwrap();
    let group = Url::from_file_path(&path).unwrap();
    client.open(&group, text);
    client.until("diagnostics", |c| c.diagnostics.contains_key(&group));
    let edits = client.request::<OnTypeFormatting>(json!({
        "textDocument": {"uri": group}, "position": position(6, 12), "ch": " ",
        "options": {"tabSize": 4, "insertSpaces": true}
    }));
    assert_eq!(
        edits,
        json!([{"range": {"start": position(6, 0), "end": position(6, 1)}, "newText": ""}])
    );
}

#[test]
fn a_pipe_operator_completes_with_its_pipe_and_filter_text() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("alone.av.sql");
    let text = "FROM sales\nwh";
    std::fs::write(&path, text).unwrap();
    let mut client = Client::start(ClientCapabilities::default());
    let alone = Url::from_file_path(&path).unwrap();
    client.open(&alone, text);
    client.until("diagnostics", |c| c.diagnostics.contains_key(&alone));
    let completions = client.request::<Completion>(json!({
        "textDocument": {"uri": alone}, "position": position(1, 2)
    }));
    let item = (completions["items"].as_array().unwrap().iter())
        .find(|item| item["label"] == "|> WHERE")
        .unwrap_or_else(|| panic!("{completions}"));
    assert_eq!(item["filterText"], "WHERE");
    assert_eq!(item["textEdit"]["newText"], "|> WHERE ");
}
