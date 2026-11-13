//! Conformance with GoogleSQL's planner: an ignored test that runs each pipe query of
//! `conformance/queries.txt` through `avenger_sql::sql` and through GoogleSQL's `execute_query`
//! over the same CSV tables, and compares the names of the columns and the rows, or the errors.
//!
//! `execute_query` reads a CSV file as a table of `STRING` columns, so both read the CSV files
//! as strings, and each query runs after `conformance/tables.sql`, which types them. A difference
//! that `conformance/known.txt` lists is explained; the test fails on any other, and writes its
//! report to `tests/output/conformance.md`. Without the binary, it skips with a message.
//!
//! ```bash
//! cargo test --release -p avenger-sql --test conformance -- --ignored
//! ```

mod googlesql;

use std::{fmt::Write as _, path::PathBuf, process::Command, sync::Arc};

use arrow::{
    array::{ArrayRef, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};
use avenger_sql::ScriptError;
use datafusion::{common::DataFusionError, datasource::MemTable, prelude::SessionContext};

/// The CSV tables of the queries, by their names, and their files under the crate's root.
const TABLES: &[(&str, &str)] = &[
    ("penguins_csv", "tests/data/penguins.csv"),
    ("sales_csv", "tests/conformance/sales.csv"),
    ("events_csv", "tests/conformance/events.csv"),
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The table of the CSV file at `path`, of `STRING` columns that aren't `NULL`, as
/// `execute_query` reads one: an empty field is the empty string. The files hold no quotes.
fn csv(path: &PathBuf) -> RecordBatch {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains('"'), "{} holds no quotes", path.display());
    let mut lines = text.lines();
    let names: Vec<&str> = lines.next().unwrap().split(',').collect();
    let rows: Vec<Vec<&str>> = lines.map(|line| line.split(',').collect()).collect();
    let schema = Arc::new(Schema::new(
        (names.iter())
            .map(|name| Field::new(*name, DataType::Utf8, false))
            .collect::<Vec<_>>(),
    ));
    let columns: Vec<ArrayRef> = (0..names.len())
        .map(|i| {
            let values: StringArray = rows.iter().map(|row| Some(row[i])).collect();
            Arc::new(values) as ArrayRef
        })
        .collect();
    RecordBatch::try_new(schema, columns).unwrap()
}

/// A query of the corpus, and its label.
struct Query {
    label: String,
    text: String,
}

/// The queries of `text`, each after a line `## <label>`.
fn queries(text: &str) -> Vec<Query> {
    let mut queries: Vec<Query> = Vec::new();
    for line in text.lines() {
        if let Some(label) = line.strip_prefix("## ") {
            queries.push(Query {
                label: label.to_string(),
                text: String::new(),
            });
        } else if let Some(query) = queries.last_mut() {
            if !query.text.is_empty() {
                query.text.push('\n');
            }
            query.text.push_str(line);
        }
    }
    queries
}

/// What a query returns: the names of its columns and its rows, sorted, as `execute_query`'s
/// table mode prints their values, or the message of its first error.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Error(String),
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Rows { columns, rows } => {
                writeln!(f, "| {} |", columns.join(" | "))?;
                for row in rows {
                    writeln!(f, "| {} |", row.join(" | "))?;
                }
                Ok(())
            }
            Outcome::Error(message) => writeln!(f, "error: {message}"),
        }
    }
}

/// The message of an error of `execute_query`, without its place and its type URL.
fn message(line: &str) -> String {
    let line = line.trim();
    let line = line.strip_prefix("ERROR: ").unwrap_or(line);
    // A status, such as `INVALID_ARGUMENT:` or `OUT_OF_RANGE:`.
    let line = match line.split_once(": ") {
        Some((status, rest)) if status.chars().all(|c| c.is_ascii_uppercase() || c == '_') => rest,
        _ => line,
    };
    let line = line.split(" [type.googleapis").next().unwrap_or(line);
    let line = match line.rfind(" [at ") {
        Some(at) if line.ends_with(']') => &line[..at],
        _ => line,
    };
    // The tool names types in GoogleSQL's internal mode, as `DOUBLE`, and avenger in its
    // external mode, as `FLOAT64`.
    line.replace("DOUBLE", "FLOAT64")
}

/// The outcome of the script `text` in GoogleSQL's `binary`, over the CSV tables.
fn theirs(binary: &PathBuf, text: &str) -> Outcome {
    let tables: Vec<String> = (TABLES.iter())
        .map(|(name, path)| format!("{name}=csv:{}", crate_root().join(path).display()))
        .collect();
    let output = Command::new(binary)
        .args([
            "--mode=execute",
            "--sql_mode=script",
            "--enabled_language_features=MAXIMUM",
            &format!("--table_spec={}", tables.join(",")),
            text,
        ])
        .output()
        .expect("execute_query runs");
    let out = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = out.lines().collect();
    // The statements before the query, which return no table, log lines, so a query that returns
    // one has its table at the end, and a query that fails has a line of its error.
    if !lines.iter().any(|line| line.starts_with('┌')) {
        // Each log ends with a line of its source, and the error follows the last.
        let start = (lines.iter())
            .rposition(|line| line.starts_with("googlesql/"))
            .map_or(0, |i| i + 1);
        let error = (lines[start..].iter())
            .find(|line| !line.trim().is_empty())
            .copied()
            .unwrap_or("no output");
        return Outcome::Error(message(error));
    }
    // The table of the last statement.
    let start = lines.iter().rposition(|line| line.starts_with('┌'));
    let table: Vec<Vec<String>> = (lines[start.map_or(lines.len(), |s| s + 1)..].iter())
        .filter(|line| line.starts_with('│'))
        .map(|line| {
            (line.trim().trim_matches('│').split('│'))
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect();
    let Some((columns, rows)) = table.split_first() else {
        return Outcome::Rows {
            columns: Vec::new(),
            rows: Vec::new(),
        };
    };
    let mut rows = rows.to_vec();
    rows.sort();
    Outcome::Rows {
        columns: columns.clone(),
        rows,
    }
}

/// The outcome of the script `text` through `avenger_sql::sql` in `ctx`.
async fn ours(ctx: &SessionContext, text: &str) -> Outcome {
    let error = |error: DataFusionError| {
        let message = match &error {
            DataFusionError::External(inner) => match inner.downcast_ref::<ScriptError>() {
                Some(script) => script.diagnostics[0].message.clone(),
                None => error.strip_backtrace(),
            },
            _ => error.strip_backtrace(),
        };
        // An error of a function at run time, which DataFusion writes after its kind.
        let message =
            (message.strip_prefix("Execution error: ")).map_or(message.clone(), String::from);
        Outcome::Error(message)
    };
    let frame = match avenger_sql::sql(ctx, text).await {
        Ok(frame) => frame,
        Err(e) => return error(e),
    };
    let schema = frame.schema().clone();
    let batches = match frame.collect().await {
        Ok(batches) => batches,
        Err(e) => return error(e),
    };
    let columns = (schema.fields().iter())
        .map(|field| field.name().clone())
        .collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for batch in &batches {
        for row in 0..batch.num_rows() {
            rows.push(
                (batch.columns().iter())
                    .map(|column| googlesql::cell(column, row))
                    .collect(),
            );
        }
    }
    rows.sort();
    Outcome::Rows { columns, rows }
}

/// Whether two outcomes agree: the same columns and rows, or errors whose messages start the
/// same, since either can add a reason or a hint after a message.
fn agree(ours: &Outcome, theirs: &Outcome) -> bool {
    match (ours, theirs) {
        // The parser's conformance check compares syntax errors by their places.
        (Outcome::Error(_), Outcome::Error(b)) if b.starts_with("Syntax error") => true,
        (Outcome::Error(a), Outcome::Error(b)) => {
            a.starts_with(b.as_str()) || b.starts_with(a.as_str())
        }
        (
            Outcome::Rows { columns, rows },
            Outcome::Rows {
                columns: their_columns,
                rows: their_rows,
            },
        ) => {
            columns == their_columns
                && rows.len() == their_rows.len()
                && (rows.iter().zip(their_rows)).all(|(a, b)| {
                    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_cell(a, b))
                })
        }
        _ => false,
    }
}

/// Whether two cells hold the same value: the same text without the spaces at its ends, which
/// the table mode's text loses, or floats within a relative 1e-12, since GoogleSQL prints a
/// float with up to 17 digits and can sum in another order.
fn same_cell(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a == b {
        return true;
    }
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(a), Ok(b)) => (a - b).abs() <= 1e-12 * a.abs().max(b.abs()),
        _ => false,
    }
}

/// The lines of `known.txt`: a label, a tab, and the reason of its difference.
fn known() -> Vec<(String, String)> {
    (include_str!("conformance/known.txt").lines())
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (label, reason) = line.split_once('\t').expect("a label, a tab, and a reason");
            (label.trim().to_string(), reason.trim().to_string())
        })
        .collect()
}

#[tokio::test]
#[ignore = "runs GoogleSQL's execute_query; see the module's docs"]
async fn conformance() {
    let binary = crate_root().join("../scratch/tools/googlesql/execute_query_macos");
    if !binary.exists() {
        eprintln!("skipped: the conformance check needs {}", binary.display());
        return;
    }
    let tables = include_str!("conformance/tables.sql");
    let queries = queries(include_str!("conformance/queries.txt"));
    // A script's DECLAREs come first, so a query's lines of DECLARE go before the tables.
    let scripts: Vec<String> = (queries.iter())
        .map(|query| {
            let (declares, rest): (Vec<&str>, Vec<&str>) =
                (query.text.lines()).partition(|line| line.starts_with("DECLARE"));
            format!("{}\n{tables}\n{}", declares.join("\n"), rest.join("\n"))
        })
        .collect();

    // GoogleSQL's outcomes, a process for each query, on a thread for each core.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = scripts.len().div_ceil(threads);
    let theirs: Vec<Outcome> = std::thread::scope(|scope| {
        let handles: Vec<_> = (scripts.chunks(chunk))
            .map(|scripts| {
                let binary = &binary;
                scope.spawn(move || {
                    (scripts.iter())
                        .map(|script| theirs(binary, script))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    let ctx = SessionContext::new();
    avenger_sql::register(&ctx).unwrap();
    for (name, path) in TABLES {
        let batch = csv(&crate_root().join(path));
        let table = MemTable::try_new(batch.schema(), vec![vec![batch]]).unwrap();
        ctx.register_table(*name, Arc::new(table)).unwrap();
    }

    let known = known();
    let (mut same, mut explained, mut unexplained) = (0, Vec::new(), Vec::new());
    for ((query, script), theirs) in queries.iter().zip(&scripts).zip(theirs) {
        let ours = ours(&ctx, script).await;
        if agree(&ours, &theirs) {
            same += 1;
            continue;
        }
        match known.iter().find(|(label, _)| *label == query.label) {
            Some((_, reason)) => explained.push((query, reason.clone(), ours, theirs)),
            None => unexplained.push((query, ours, theirs)),
        }
    }

    let mut report = String::from("# Conformance with GoogleSQL's planner\n\n");
    let _ = writeln!(
        report,
        "{} queries: {same} with the same result, {} explained differences, {} unexplained.\n",
        queries.len(),
        explained.len(),
        unexplained.len()
    );
    if !unexplained.is_empty() {
        report.push_str("## Unexplained differences\n\n");
        for (query, ours, theirs) in &unexplained {
            let _ = writeln!(
                report,
                "### `{}`\n\n```sql\n{}\n```\n\navenger:\n\n```text\n{ours}```\n\nGoogleSQL:\n\n```text\n{theirs}```\n",
                query.label, query.text
            );
        }
    }
    report.push_str("## Explained differences\n\n");
    for (query, reason, _, _) in &explained {
        let _ = writeln!(report, "- `{}`: {reason}", query.label);
    }
    let output = crate_root().join("tests/output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("conformance.md"), &report).unwrap();
    assert!(
        unexplained.is_empty(),
        "{} unexplained differences; the report is in tests/output/conformance.md",
        unexplained.len()
    );
}
