//! Pipe queries against GoogleSQL: each query of `data/googlesql_pipe_*.txt`, which GoogleSQL's
//! `execute_query` ran over the same rows as the tests' tables, through the pass, with its columns,
//! their types and its rows, or its error.

mod googlesql;
mod pass;

use avenger_sql::types;
use googlesql::cell;

/// The files of the recorded queries.
const DATA: &[(&str, &str)] = &[
    (
        "googlesql_pipe_5a.txt",
        include_str!("data/googlesql_pipe_5a.txt"),
    ),
    (
        "googlesql_pipe_5b.txt",
        include_str!("data/googlesql_pipe_5b.txt"),
    ),
    (
        "googlesql_pipe_5c.txt",
        include_str!("data/googlesql_pipe_5c.txt"),
    ),
];

/// The cases where the pass differs from GoogleSQL, and why.
const DIFFERENCES: &[(&str, &str)] = &[
    (
        "FROM t |> SELECT x, ARRAY(FROM u |> WHERE u.x >= t.x |> SELECT x |> ORDER BY x DESC) AS xs",
        "DataFusion 55 decorrelates only correlations of equality",
    ),
    (
        "FROM t, LATERAL (FROM u |> WHERE u.x <= t.x |> AGGREGATE COUNT(*) AS n) |> SELECT x, n",
        "DataFusion 55 decorrelates only correlations of equality",
    ),
    (
        "FROM t |> JOIN LATERAL (FROM u |> WHERE u.x < t.x |> SELECT x AS y) ON TRUE |> SELECT x, y",
        "DataFusion 55 decorrelates only correlations of equality",
    ),
    (
        "FROM t AS a |> WHERE EXISTS (FROM u |> WHERE EXISTS (FROM t AS c |> WHERE c.x = u.x AND c.x = a.x))",
        "DataFusion 55 doesn't decorrelate a reference to a query two levels out",
    ),
    (
        "FROM arrays |> SELECT k, (FROM UNNEST(xs) AS x |> AGGREGATE SUM(x)) AS total",
        "DataFusion 55 doesn't decorrelate an outer reference outside a filter, as in UNNEST",
    ),
];

#[tokio::test]
async fn pipe_queries_return_googlesqls_rows() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for (file, data) in DATA {
        for line in data
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
        {
            let parts: Vec<&str> = line.split('\t').collect();
            let (query, columns, expected) = (parts[0], parts[1], parts[2]);
            if DIFFERENCES.iter().any(|(text, _)| *text == query) {
                continue;
            }
            checked += 1;
            let result = pass::run(query).await;
            if columns == "ERROR" {
                match result {
                    // GoogleSQL can add a hint after the message, and the pass a reason.
                    Err(errors)
                        if expected.starts_with(&errors[0]) || errors[0].starts_with(expected) => {}
                    Err(errors) => failures.push(format!(
                        "{file}: {query}: wants the error {expected:?}, has {:?}",
                        errors[0]
                    )),
                    Ok(_) => failures.push(format!(
                        "{file}: {query}: wants the error {expected:?}, has rows"
                    )),
                }
                continue;
            }
            let batches = match result {
                Ok(batches) => batches,
                Err(errors) => {
                    failures.push(format!("{file}: {query}: {errors:?}"));
                    continue;
                }
            };
            let schema = pass::analysis(query).plan.expect("a plan").schema().clone();
            // A record of a script has the names of its columns, without their types.
            let typed = serde_free::strings(columns)
                .iter()
                .any(|column| column.contains(' '));
            let found: Vec<String> = (schema.fields().iter())
                .map(|field| match typed {
                    true => format!(
                        "{} {}",
                        field.name(),
                        types::googlesql_name(field.data_type())
                    ),
                    false => field.name().clone(),
                })
                .collect();
            let want: Vec<String> = (serde_free::strings(columns).into_iter())
                .map(|column| column.replace("DOUBLE", "FLOAT64"))
                .collect();
            // GoogleSQL names the column of a value table, such as `$value`, internally.
            let same = found.len() == want.len()
                && (found.iter().zip(&want)).all(|(found, want)| {
                    found == want
                        || (want.starts_with("`$")
                            && found.split(' ').nth(1) == want.split(' ').nth(1))
                });
            if !same {
                failures.push(format!(
                    "{file}: {query}: the columns {found:?}, not {want:?}"
                ));
            }
            let mut rows: Vec<String> = Vec::new();
            for batch in &batches {
                for row in 0..batch.num_rows() {
                    let cells: Vec<String> = (batch.columns().iter())
                        .map(|column| serde_free::quote(&cell(column, row)))
                        .collect();
                    rows.push(format!("[{}]", cells.join(", ")));
                }
            }
            rows.sort();
            let rows = format!("[{}]", rows.join(", "));
            if rows != expected {
                failures.push(format!(
                    "{file}: {query}:\n  the pass {rows}\n  GoogleSQL {expected}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(checked > 50, "{checked} queries");
}

/// The JSON of the data: arrays of strings, as Python's `json.dumps` writes them.
mod serde_free {
    /// The strings of a JSON array of strings without escapes other than `\"` and `\\`.
    pub fn strings(json: &str) -> Vec<String> {
        let inner = json.trim().trim_start_matches('[').trim_end_matches(']');
        let mut out = Vec::new();
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            let mut text = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => text.extend(chars.next()),
                    '"' => break,
                    c => text.push(c),
                }
            }
            out.push(text);
        }
        out
    }

    /// `text` as a JSON string, as `json.dumps` writes it.
    pub fn quote(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }
}
