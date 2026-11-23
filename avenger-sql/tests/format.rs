//! `FORMAT` against GoogleSQL's own answers, which `tests/data/format_oracle.py` records in
//! `tests/data/format.jsonl` with GoogleSQL's `execute_query`. Each case runs twice: with its
//! arguments as written, where a literal pattern that fails is a diagnostic, and with each
//! argument from a column, where a pattern that fails is an error when the query runs.

mod pass;

use datafusion::arrow::{
    array::{Array, AsArray},
    compute::cast,
    datatypes::DataType,
};
use serde_json::Value;

/// The result of the script `text`, as the record writes it: a string or `null`, or the
/// messages of its diagnostics, or of its error when it runs.
async fn run(text: &str) -> Result<Value, Vec<String>> {
    let batches = pass::run_in(&pass::session(), &pass::catalog(), text).await?;
    let column = batches[0].column(0).clone();
    if column.is_null(0) {
        return Ok(Value::Null);
    }
    let text = cast(&column, &DataType::Utf8).unwrap();
    Ok(Value::String(text.as_string::<i32>().value(0).to_string()))
}

/// Whether the messages `found` hold GoogleSQL's message `expected`. A precision over its limit
/// reads otherwise, since GoogleSQL's message runs the limit into its text.
fn same_error(found: &[String], expected: &str) -> bool {
    match expected.strip_prefix("Precision value is too big") {
        Some(limit) => found
            .iter()
            .any(|message| message.contains(&format!("is too big; limit {limit}"))),
        None => found.iter().any(|message| message.contains(expected)),
    }
}

#[tokio::test]
async fn format_gives_googlesql_s_answers() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/format.jsonl");
    let mut failures = Vec::new();
    let mut cases = 0;
    for line in std::fs::read_to_string(path).unwrap().lines() {
        cases += 1;
        let case: Value = serde_json::from_str(line).unwrap();
        let args: Vec<&str> = (case["args"].as_array().unwrap().iter())
            .map(|arg| arg.as_str().unwrap())
            .collect();
        let literals = format!("FROM one |> SELECT FORMAT({}) AS r", args.join(", "));
        let fields: Vec<String> = (args.iter().enumerate())
            .map(|(index, arg)| format!("{arg} AS a{index}"))
            .collect();
        let names: Vec<String> = (0..args.len()).map(|index| format!("a{index}")).collect();
        let columns = format!(
            "FROM one |> SELECT {} |> SELECT FORMAT({}) AS r",
            fields.join(", "),
            names.join(", ")
        );
        for script in [literals, columns] {
            let found = run(&script).await;
            let ok = match (&found, case.get("error")) {
                (Err(messages), Some(expected)) => same_error(messages, expected.as_str().unwrap()),
                (Ok(value), None) => value == &case["result"],
                _ => false,
            };
            if !ok {
                let expected = case.get("error").unwrap_or(&case["result"]);
                failures.push(format!(
                    "{script}\n  GoogleSQL {expected}\n  avenger   {found:?}"
                ));
            }
        }
    }
    assert!(cases > 150, "the record holds {cases} cases");
    assert!(
        failures.is_empty(),
        "{} of {} runs differ:\n{}",
        failures.len(),
        cases * 2,
        failures.join("\n")
    );
}

/// A literal pattern that fails is a diagnostic at the argument that fails it, or at the
/// pattern.
#[test]
fn a_literal_pattern_s_error_is_at_its_argument() {
    let text = "FROM one |> SELECT FORMAT('%d and %s', 1, 2.5) AS r";
    let analysis = pass::analysis_in(&pass::catalog(), text);
    let [diagnostic] = analysis.diagnostics.as_slice() else {
        panic!("{:?}", analysis.diagnostics);
    };
    assert_eq!(
        diagnostic.message,
        "Invalid type for argument 3 to FORMAT; Expected STRING; Got FLOAT64"
    );
    assert_eq!(&text[diagnostic.range], "2.5");
    let text = "FROM one |> SELECT FORMAT('%d %d', 1) AS r";
    let analysis = pass::analysis_in(&pass::catalog(), text);
    let [diagnostic] = analysis.diagnostics.as_slice() else {
        panic!("{:?}", analysis.diagnostics);
    };
    assert_eq!(&text[diagnostic.range], "'%d %d'");
}
