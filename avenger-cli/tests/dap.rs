//! `avenger dap --headless` over pipes, on a copy of avenger-sql-project's fixture project: a
//! session's launch, breakpoints, stops, frames, scopes, variables, and steps, as a debugger in an
//! editor drives them.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

/// A copy of the fixture project in a temporary directory, by its canonical root, with a chart of
/// a declared variable.
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
    std::fs::write(root.join("charts/heavy.av.sql"), HEAVY).unwrap();
    std::fs::write(root.join("charts/broken.av.sql"), BROKEN).unwrap();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    std::fs::write(root.join("reports/overview.qmd"), REPORT).unwrap();
    (directory, root)
}

/// A chart whose variable a `SET` changes, and whose temporary table's query fails at its second
/// operator.
const BROKEN: &str = "\
DECLARE min_weight INT64 DEFAULT 3000;
SET min_weight = 4000;
CREATE TEMP TABLE heavy AS (
    FROM vega_datasets.cars
    |> WHERE Weight_in_lbs > min_weight
    |> SELECT Origin, nope
    |> LIMIT 5
);
FROM heavy
|> AGGREGATE count(*) AS cars GROUP BY Origin
";

/// A document whose labeled cell starts at its fourth line.
const REPORT: &str = "# Report\n\n```{avenger-sql}\n--| label: fig-weather\nFROM wet_days\n|> AGGREGATE mark_bar(x => weather, y => COUNT(*)) AS chart\n```\n";

const HEAVY: &str = "\
DECLARE min_weight INT64 DEFAULT 3000;
FROM vega_datasets.cars
|> WHERE Weight_in_lbs > min_weight
|> AGGREGATE count(*) AS cars GROUP BY Origin
|> AGGREGATE mark_bar(x => Origin, y => cars) AS chart
";

/// A client of `avenger dap --headless` in the project at `root`.
struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    seq: i64,
}

impl Client {
    fn start(root: &Path) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_avenger"))
            .args(["dap", "--headless"])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Client {
            child,
            input,
            output,
            seq: 0,
        }
    }

    fn send(&mut self, command: &str, arguments: Value) -> i64 {
        self.seq += 1;
        let body = json!({"seq": self.seq, "type": "request", "command": command,
                          "arguments": arguments})
        .to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.input.flush().unwrap();
        self.seq
    }

    fn read(&mut self) -> Value {
        let mut length = 0;
        loop {
            let mut line = String::new();
            self.output.read_line(&mut line).unwrap();
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        self.output.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// The response to the request `command`, after the events before it.
    fn request(&mut self, command: &str, arguments: Value) -> Value {
        let seq = self.send(command, arguments);
        loop {
            let message = self.read();
            if message["type"] == "response" && message["request_seq"] == seq {
                return message;
            }
        }
    }

    /// The next event `event`, past the messages before it.
    fn event(&mut self, event: &str) -> Value {
        loop {
            let message = self.read();
            if message["event"] == event {
                return message;
            }
        }
    }

    /// The children of the variable reference `reference`.
    fn variables(&mut self, reference: &Value) -> Vec<Value> {
        let response = self.request("variables", json!({"variablesReference": reference}));
        response["body"]["variables"].as_array().unwrap().clone()
    }

    /// The scopes of the current stop, by name, with their references.
    fn scopes(&mut self) -> Vec<(String, Value)> {
        let response = self.request("scopes", json!({"frameId": 1}));
        (response["body"]["scopes"].as_array().unwrap().iter())
            .map(|scope| {
                let name = scope["name"].as_str().unwrap().to_string();
                (name, scope["variablesReference"].clone())
            })
            .collect()
    }

    /// The name and the line of the current frame.
    fn frame(&mut self) -> (String, u64) {
        let response = self.request("stackTrace", json!({"threadId": 1}));
        let frame = &response["body"]["stackFrames"][0];
        (
            frame["name"].as_str().unwrap().to_string(),
            frame["line"].as_u64().unwrap(),
        )
    }

    /// Steps by `command` and gives the reason of the stop and its frame.
    fn step(&mut self, command: &str) -> (String, (String, u64)) {
        let response = self.request(command, json!({"threadId": 1}));
        assert_eq!(response["success"], true, "{response}");
        let stopped = self.event("stopped");
        let reason = stopped["body"]["reason"].as_str().unwrap().to_string();
        (reason, self.frame())
    }
}

/// The named child of `variables`.
fn named<'a>(variables: &'a [Value], name: &str) -> &'a Value {
    (variables.iter())
        .find(|variable| variable["name"] == name)
        .unwrap_or_else(|| panic!("no `{name}` in {variables:?}"))
}

#[test]
fn a_session_stops_at_its_breakpoints_and_steps_through_its_stages() {
    let (_directory, root) = project();
    let mut client = Client::start(&root);
    let initialized = client.request("initialize", json!({"adapterID": "avenger"}));
    assert_eq!(initialized["body"]["supportsStepBack"], true);
    let launch = client.request("launch", json!({"target": "charts/heavy.av.sql"}));
    assert_eq!(launch["success"], true, "{launch}");
    client.event("initialized");
    let path = root.join("charts/heavy.av.sql");
    let set = client.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": [{"line": 3}, {"line": 5}, {"line": 1}]}),
    );
    let breakpoints = set["body"]["breakpoints"].as_array().unwrap();
    assert_eq!(breakpoints[0]["verified"], true);
    assert_eq!(breakpoints[1]["verified"], true);
    assert_eq!(breakpoints[2]["verified"], false);
    assert_eq!(
        breakpoints[2]["message"],
        "the statement at line 1 has no rows"
    );
    // The console says why, since a client may not show a breakpoint's message.
    let note = client.event("output");
    let text = note["body"]["output"].as_str().unwrap();
    assert!(
        text.contains("charts/heavy.av.sql:1: the breakpoint takes no stop"),
        "{text}"
    );
    client.request("configurationDone", json!({}));

    // The rows that enter `WHERE`, its columns, a chunk of its rows, and the script's variable.
    let stopped = client.event("stopped");
    assert_eq!(stopped["body"]["reason"], "breakpoint");
    assert_eq!(client.frame(), ("|> WHERE".to_string(), 3));
    let scopes = client.scopes();
    assert_eq!(scopes[0].0, "Stage");
    assert_eq!(scopes[1].0, "Script");
    let stage = client.variables(&scopes[0].1);
    assert_eq!(named(&stage, "rows")["value"], "406 rows");
    let columns = client.variables(&named(&stage, "columns")["variablesReference"]);
    assert_eq!(named(&columns, "Weight_in_lbs")["type"], "INT64");
    let chunks = client.variables(&named(&stage, "rows")["variablesReference"]);
    assert_eq!(chunks.len(), 5);
    assert_eq!(chunks[0]["name"], "rows 1-100");
    assert_eq!(chunks[4]["name"], "rows 401-406");
    let rows = client.variables(&chunks[0]["variablesReference"]);
    assert_eq!(rows.len(), 100);
    let fields = client.variables(&rows[0]["variablesReference"]);
    assert!(fields.iter().any(|field| field["name"] == "Origin"));
    let script = client.variables(&scopes[1].1);
    assert_eq!(named(&script, "min_weight")["value"], "3000");
    assert_eq!(named(&script, "min_weight")["type"], "INT64");

    // Over, back, and on to the next breakpoint.
    let aggregate = ("|> AGGREGATE".to_string(), 4);
    assert_eq!(client.step("next"), ("step".to_string(), aggregate.clone()));
    let scope = client.scopes()[0].1.clone();
    let stage = client.variables(&scope);
    assert_eq!(named(&stage, "rows")["value"], "174 rows");
    let where_ = ("|> WHERE".to_string(), 3);
    assert_eq!(client.step("stepBack"), ("breakpoint".to_string(), where_));
    let marks = ("|> AGGREGATE".to_string(), 5);
    assert_eq!(client.step("continue"), ("breakpoint".to_string(), marks));
    let scope = client.scopes()[0].1.clone();
    let stage = client.variables(&scope);
    assert_eq!(named(&stage, "rows")["value"], "2 rows");

    // The result, with its chart, and a `continue` there stops at the same frame.
    let result = ("result".to_string(), 2);
    assert_eq!(
        client.step("continue"),
        ("step".to_string(), result.clone())
    );
    let scopes = client.scopes();
    assert_eq!(scopes[0].0, "Result");
    let rows = client.variables(&scopes[0].1);
    let columns = client.variables(&named(&rows, "columns")["variablesReference"]);
    assert_eq!(named(&columns, "chart")["type"], "CHART");
    let seq = client.send("continue", json!({"threadId": 1}));
    let mut kinds = Vec::new();
    while kinds.last() != Some(&"stopped".to_string()) {
        let message = client.read();
        kinds.push(match message["type"].as_str() {
            Some("response") if message["request_seq"] == seq => "response".to_string(),
            _ => message["event"].as_str().unwrap_or_default().to_string(),
        });
    }
    assert_eq!(kinds, ["response", "continued", "stopped"]);
    assert_eq!(client.frame(), result);

    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn a_launch_without_a_target_fails_with_its_reason() {
    let (_directory, root) = project();
    let mut client = Client::start(&root);
    client.request("initialize", json!({}));
    let none = client.request("launch", json!({}));
    assert_eq!(none["success"], false);
    assert!(none["message"]
        .as_str()
        .unwrap()
        .contains("names no `target`"));
    let missing = client.request("launch", json!({"target": "charts/missing.av.sql"}));
    assert_eq!(missing["success"], false);
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

/// Launches `target` in the project at `root`, sets breakpoints at `lines` of `file`, and gives the
/// client at its first stop, with the breakpoints.
fn launched(root: &Path, target: &str, file: &str, lines: &[u64]) -> (Client, Vec<Value>) {
    let mut client = Client::start(root);
    client.request("initialize", json!({}));
    let launch = client.request("launch", json!({"target": target}));
    assert_eq!(launch["success"], true, "{launch}");
    client.event("initialized");
    let breakpoints: Vec<Value> = lines.iter().map(|line| json!({"line": line})).collect();
    let set = client.request(
        "setBreakpoints",
        json!({"source": {"path": root.join(file)}, "breakpoints": breakpoints}),
    );
    let breakpoints = set["body"]["breakpoints"].as_array().unwrap().clone();
    client.request("configurationDone", json!({}));
    client.event("stopped");
    (client, breakpoints)
}

#[test]
fn a_set_changes_a_variable_and_a_failing_stage_shows_its_error_at_its_place() {
    let (_directory, root) = project();
    let (mut client, _) = launched(&root, "charts/broken.av.sql", "charts/broken.av.sql", &[5]);
    assert_eq!(client.frame(), ("|> WHERE".to_string(), 5));
    let scopes = client.scopes();
    let script = client.variables(&scopes[1].1);
    assert_eq!(named(&script, "min_weight")["value"], "4000");
    // The rows that enter `SELECT`, past the `WHERE` of the `SET`'s value.
    assert_eq!(client.step("next").1, ("|> SELECT".to_string(), 6));
    let scope = client.scopes()[0].1.clone();
    let stage = client.variables(&scope);
    assert_eq!(named(&stage, "rows")["value"], "67 rows");
    // The stage after the `SELECT` fails, with its error at its place in the file, in the console
    // and in its scope.
    let seq = client.send("next", json!({"threadId": 1}));
    let mut output = String::new();
    loop {
        let message = client.read();
        if message["event"] == "output" {
            output.push_str(message["body"]["output"].as_str().unwrap());
        }
        if message["event"] == "stopped" {
            break;
        }
        if message["type"] == "response" {
            assert_eq!(message["request_seq"], seq);
        }
    }
    assert!(output.contains("charts/broken.av.sql:6:23"), "{output}");
    assert!(output.contains("nope"), "{output}");
    assert_eq!(client.frame(), ("|> LIMIT".to_string(), 7));
    let scope = client.scopes()[0].1.clone();
    let stage = client.variables(&scope);
    assert_eq!(stage[0]["name"], "error");
    // The stage before still has its rows.
    assert_eq!(client.step("stepBack").1, ("|> SELECT".to_string(), 6));
    let scope = client.scopes()[0].1.clone();
    let stage = client.variables(&scope);
    assert_eq!(named(&stage, "rows")["value"], "67 rows");
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn a_cell_s_stops_and_breakpoints_have_the_lines_of_its_document() {
    let (_directory, root) = project();
    let (mut client, breakpoints) = launched(
        &root,
        "reports/overview.qmd#fig-weather",
        "reports/overview.qmd",
        &[6, 1, 5],
    );
    assert_eq!(breakpoints[0]["verified"], true);
    assert_eq!(breakpoints[0]["line"], 6);
    assert_eq!(breakpoints[1]["verified"], false);
    assert!(breakpoints[1]["message"]
        .as_str()
        .unwrap()
        .contains("outside the cell"));
    assert_eq!(breakpoints[2]["line"], 5);
    let response = client.request("stackTrace", json!({"threadId": 1}));
    let frame = &response["body"]["stackFrames"][0];
    assert_eq!(frame["name"], "|> AGGREGATE");
    assert_eq!(frame["line"], 6);
    assert!(frame["source"]["path"]
        .as_str()
        .unwrap()
        .ends_with("reports/overview.qmd"));
    assert_eq!(client.step("continue").1, ("result".to_string(), 5));
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

/// The layers of the chart of the first row of the result at the client's stop, each by its name
/// and its value, the count of its rows.
fn layers_at_result(client: &mut Client) -> Vec<(String, String)> {
    let scope = client.scopes()[0].1.clone();
    let result = client.variables(&scope);
    let rows = client.variables(&named(&result, "rows")["variablesReference"]);
    let fields = client.variables(&rows[0]["variablesReference"]);
    let layers = client.variables(&named(&fields, "chart")["variablesReference"]);
    (layers.iter())
        .map(|layer| {
            let text = |key: &str| layer[key].as_str().unwrap().to_string();
            (text("name"), text("value"))
        })
        .collect()
}

#[test]
fn the_layers_of_a_chart_of_several_plots_name_their_plots_and_draw_their_own_rows() {
    let (_directory, root) = project();
    let text = "FROM vega_datasets.cars
|> WHERE Horsepower IS NOT NULL
|> AGGREGATE hconcat(
    layer(
        mark_symbol(x => Horsepower, y => Miles_per_Gallon, id => 'cars'),
        mark_symbol(x => avg(Horsepower), y => avg(Miles_per_Gallon), color => Origin),
    ),
    mark_bar(x => Origin, y => count(*), color => Origin),
   ) AS chart
";
    std::fs::write(root.join("charts/layers.av.sql"), text).unwrap();
    let path = "charts/layers.av.sql";
    let (mut client, _) = launched(&root, path, path, &[1]);
    assert_eq!(client.frame(), ("result".to_string(), 1));
    let layers = layers_at_result(&mut client);
    let layers: Vec<(&str, &str)> = (layers.iter())
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(layers.len(), 3, "{layers:?}");
    assert_eq!(layers[0].0, "plot 1 › cars (symbol)");
    assert_eq!(layers[1], ("plot 1 › layer 2 (symbol)", "3 rows"));
    assert_eq!(layers[2], ("plot 2 › layer 1 (bar)", "3 rows"));
    assert_ne!(layers[0].1, "3 rows");
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn the_layers_of_a_facet_name_their_cells_by_their_facet_values() {
    let (_directory, root) = project();
    let text = "FROM vega_datasets.cars
|> AGGREGATE mark_symbol(x => Horsepower, y => Miles_per_Gallon) AS chart GROUP BY Origin
|> AGGREGATE facet_column(chart, Origin) AS chart
";
    std::fs::write(root.join("charts/faceted.av.sql"), text).unwrap();
    let path = "charts/faceted.av.sql";
    let (mut client, _) = launched(&root, path, path, &[1]);
    let layers = layers_at_result(&mut client);
    let mut names: Vec<&str> = layers.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names.len(), 3, "{names:?}");
    for (name, origin) in names.iter().zip(["Europe", "Japan", "USA"]) {
        assert!(
            name.contains(&format!("(Origin = {origin}) › layer 1 (symbol)")),
            "{names:?}"
        );
    }
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn a_stage_after_a_mark_holds_its_charts() {
    let (_directory, root) = project();
    let text = "FROM vega_datasets.cars\n|> AGGREGATE mark_bar(x => Origin, y => COUNT(*)) AS chart GROUP BY Cylinders\n|> WHERE Cylinders > 4\n";
    std::fs::write(root.join("charts/cylinders.av.sql"), text).unwrap();
    let path = "charts/cylinders.av.sql";
    let (mut client, _) = launched(&root, path, path, &[3]);
    assert_eq!(client.frame(), ("|> WHERE".to_string(), 3));
    let scope = client.scopes()[0].1.clone();
    let stage = client.variables(&scope);
    assert_eq!(named(&stage, "rows")["value"], "5 rows");
    let columns = client.variables(&named(&stage, "columns")["variablesReference"]);
    assert_eq!(named(&columns, "chart")["type"], "CHART");
    let rows = client.variables(&named(&stage, "rows")["variablesReference"]);
    let fields = client.variables(&rows[0]["variablesReference"]);
    assert_eq!(named(&fields, "chart")["value"], "<chart>");
    // A chart opens into its layers, and a layer into the columns and the rows of its table.
    let chart = named(&fields, "chart");
    assert_eq!(chart["type"], "CHART");
    let layers = client.variables(&chart["variablesReference"]);
    assert_eq!(layers.len(), 1, "{layers:?}");
    assert_eq!(layers[0]["name"], "layer 1 (bar)");
    let value = layers[0]["value"].as_str().unwrap();
    assert!(value.ends_with(" rows") && value != "0 rows", "{value}");
    let table = client.variables(&layers[0]["variablesReference"]);
    let columns = client.variables(&named(&table, "columns")["variablesReference"]);
    // Each column names the channels that encode it.
    let names: Vec<&str> = (columns.iter())
        .map(|column| column["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"x: Origin"), "{names:?}");
    assert!(
        names.iter().any(|name| name.starts_with("y: ")),
        "{names:?}"
    );
    let rows = client.variables(&named(&table, "rows")["variablesReference"]);
    assert_eq!(rows.len().to_string() + " rows", value);
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn the_console_runs_pipe_operators_after_the_stop_and_queries_of_the_script() {
    let (_directory, root) = project();
    let (mut client, _) = launched(&root, "charts/heavy.av.sql", "charts/heavy.av.sql", &[3]);
    watchdog(&client.child);
    let evaluate = |client: &mut Client, expression: &str| {
        client.request(
            "evaluate",
            json!({"expression": expression, "context": "repl", "frameId": 1}),
        )
    };
    // Operators after the rows that enter `WHERE`, and their rows as a variable.
    let limited = evaluate(&mut client, "|> LIMIT 2");
    assert_eq!(limited["success"], true, "{limited}");
    assert_eq!(limited["body"]["result"], "2 rows, 9 columns");
    // The console prints the rows as a grid under the answer: a header and a line for each row.
    let grid = client.event("output");
    let grid = grid["body"]["output"].as_str().unwrap();
    let lines: Vec<&str> = grid.lines().collect();
    assert_eq!(lines.len(), 6, "{grid}");
    assert!(
        lines[0].starts_with("+-") && lines[1].contains("| Name "),
        "{grid}"
    );
    let rows = client.variables(&limited["body"]["variablesReference"]);
    assert_eq!(rows.len(), 2);
    // A grid of more rows than it prints says how many more there are.
    let named = evaluate(&mut client, "|> SELECT Name |> LIMIT 12");
    assert_eq!(named["body"]["result"], "12 rows, 1 columns");
    let grid = client.event("output");
    assert_eq!(grid["body"]["output"].as_str().unwrap().lines().count(), 14);
    let more = client.event("output");
    assert_eq!(more["body"]["output"], "… 2 more rows\n");
    // A query of its own, after the statements before the stop, which reads their variable.
    let counted = evaluate(
        &mut client,
        "FROM vega_datasets.cars |> WHERE Weight_in_lbs > min_weight |> AGGREGATE COUNT(*) AS n",
    );
    assert_eq!(counted["body"]["result"], "1 rows, 1 columns");
    // An error, and an expression that the console doesn't take.
    let failed = evaluate(&mut client, "|> WHERE nope");
    assert_eq!(failed["success"], false);
    assert!(
        failed["message"].as_str().unwrap().contains("nope"),
        "{failed}"
    );
    let other = evaluate(&mut client, "SELECT 1");
    assert_eq!(other["success"], false);
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

/// The value of the stop's row count, from its first scope.
fn row_count(client: &mut Client) -> String {
    let scope = client.scopes()[0].1.clone();
    let stage = client.variables(&scope);
    named(&stage, "rows")["value"].as_str().unwrap().to_string()
}

#[test]
fn a_variable_of_the_script_takes_a_value_of_its_type_and_the_stop_runs_again() {
    let (_directory, root) = project();
    let (mut client, _) = launched(&root, "charts/heavy.av.sql", "charts/heavy.av.sql", &[4]);
    assert_eq!(row_count(&mut client), "174 rows");
    let script = client.scopes()[1].1.clone();
    let set = client.request(
        "setVariable",
        json!({"variablesReference": script, "name": "min_weight", "value": "4000"}),
    );
    assert_eq!(set["success"], true, "{set}");
    assert_eq!(set["body"]["value"], "4000");
    assert_eq!(set["body"]["type"], "INT64");
    assert_eq!(client.event("stopped")["body"]["reason"], "pause");
    assert_eq!(row_count(&mut client), "67 rows");
    let script = client.scopes()[1].1.clone();
    let variables = client.variables(&script);
    assert_eq!(named(&variables, "min_weight")["value"], "4000");
    // Only the script's variables take values: the variables of the other scopes are read-only.
    assert!(named(&variables, "min_weight")["presentationHint"].is_null());
    let stage = client.scopes()[0].1.clone();
    let rows = client.variables(&stage);
    assert_eq!(
        named(&rows, "rows")["presentationHint"]["attributes"],
        json!(["readOnly"])
    );
    // A value of another type, and a variable of another scope.
    let wrong = client.request(
        "setVariable",
        json!({"variablesReference": script, "name": "min_weight", "value": "heavy"}),
    );
    assert_eq!(wrong["success"], false);
    assert!(
        wrong["message"].as_str().unwrap().contains("INT64"),
        "{wrong}"
    );
    let stage = client.scopes()[0].1.clone();
    let other = client.request(
        "setVariable",
        json!({"variablesReference": stage, "name": "rows", "value": "1"}),
    );
    assert_eq!(other["success"], false);
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn an_edit_runs_the_stop_again_and_restart_opens_the_project_again() {
    let (_directory, root) = project();
    let (mut client, _) = launched(&root, "charts/heavy.av.sql", "charts/heavy.av.sql", &[4]);
    watchdog(&client.child);
    assert_eq!(row_count(&mut client), "174 rows");
    // An edit of the chart's text runs the stop again, by its operator, and says so.
    let edited = HEAVY.replace("DEFAULT 3000", "DEFAULT 4000");
    std::fs::write(root.join("charts/heavy.av.sql"), edited).unwrap();
    let mut output = String::new();
    let stopped = loop {
        let message = client.read();
        if message["event"] == "output" {
            output.push_str(message["body"]["output"].as_str().unwrap());
        }
        if message["event"] == "stopped" {
            break message;
        }
    };
    assert_eq!(stopped["body"]["reason"], "pause");
    assert!(output.contains("heavy.av.sql changed"), "{output}");
    assert_eq!(client.frame(), ("|> AGGREGATE".to_string(), 4));
    assert_eq!(row_count(&mut client), "67 rows");
    // `restart` opens the project again, at the same stop.
    let restart = client.request("restart", json!({}));
    assert_eq!(restart["success"], true, "{restart}");
    client.event("stopped");
    assert_eq!(client.frame(), ("|> AGGREGATE".to_string(), 4));
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn a_data_script_s_statements_stop_at_their_stages_and_their_tables() {
    let (_directory, root) = project();
    let (mut client, _) = launched(
        &root,
        "data/summaries.av.sql:7",
        "data/summaries.av.sql",
        &[4],
    );
    // The rows that enter the temporary table's `WHERE`, and the table as it leaves it.
    assert_eq!(client.frame(), ("|> WHERE".to_string(), 4));
    assert_eq!(row_count(&mut client), "406 rows");
    assert_eq!(client.step("next").1, ("table heavy".to_string(), 2));
    assert_eq!(row_count(&mut client), "174 rows");
    // The next statement's stage reads the temporary table, and its result is its table.
    assert_eq!(client.step("next").1, ("|> AGGREGATE".to_string(), 9));
    assert_eq!(row_count(&mut client), "174 rows");
    assert_eq!(
        client.step("next").1,
        ("table heavy_by_origin".to_string(), 7)
    );
    assert_eq!(row_count(&mut client), "2 rows");
    let script = client.scopes()[1].1.clone();
    assert!(client.variables(&script).is_empty());
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn a_data_script_s_view_stops_at_its_stage_and_its_rows() {
    let (_directory, root) = project();
    let path = "data/summaries.av.sql";
    let (mut client, _) = launched(&root, "data/summaries.av.sql:12", path, &[14]);
    assert_eq!(client.frame(), ("|> WHERE".to_string(), 14));
    assert_eq!(row_count(&mut client), "1461 rows");
    assert_eq!(client.step("next").1, ("table wet_days".to_string(), 12));
    assert_eq!(row_count(&mut client), "623 rows");
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn a_data_script_s_loaded_table_is_its_result_and_its_console_runs_queries() {
    let (_directory, root) = project();
    let path = "data/vega_datasets.av.sql";
    // Without breakpoints, the session stops at its target's result: the table that it loads.
    let (mut client, _) = launched(&root, "data/vega_datasets.av.sql:7", path, &[]);
    assert_eq!(
        client.frame(),
        ("table vega_datasets.seattle_weather".to_string(), 7)
    );
    assert_eq!(row_count(&mut client), "1461 rows");
    // The console runs queries of the project's tables, and only those.
    let evaluate = |client: &mut Client, expression: &str| {
        client.request(
            "evaluate",
            json!({"expression": expression, "context": "repl", "frameId": 1}),
        )
    };
    let limited = evaluate(&mut client, "FROM vega_datasets.cars |> LIMIT 3");
    assert_eq!(limited["body"]["result"], "3 rows, 9 columns", "{limited}");
    let pipe = evaluate(&mut client, "|> LIMIT 1");
    assert_eq!(pipe["success"], false);
    assert!(pipe["message"].as_str().unwrap().contains("FROM"), "{pipe}");
    client.request("disconnect", json!({}));
    assert!(client.child.wait().unwrap().success());
}

/// Ends the adapter `child` after a minute, so a test that waits for an event that never comes,
/// such as a change of a file that the watch misses, fails at the end of its input in place of
/// waiting for ever.
fn watchdog(child: &Child) {
    let pid = child.id().to_string();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let _ = Command::new("kill").arg(&pid).status();
    });
}
