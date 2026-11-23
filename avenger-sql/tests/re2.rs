//! The regular expressions of `REGEXP_CONTAINS`, `REGEXP_EXTRACT`, and `REGEXP_REPLACE` against
//! RE2's own answers, which `tests/data/re2_oracle.py` records in `tests/data/re2.jsonl` with
//! RE2's Python binding. Each case runs twice: with literal arguments, which compile when the
//! query plans, and with the arguments as columns, which compile when it runs.

mod pass;

use datafusion::arrow::{array::Array, compute::cast, datatypes::DataType};
use serde_json::Value;

/// `text` as a string literal of avenger-sql.
fn literal(text: &str) -> String {
    let mut out = String::from("'");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\x0B' => out.push_str("\\v"),
            '\x0C' => out.push_str("\\f"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// The result of the script `text`, one value, as RE2's record writes it, or `None` for an error.
async fn run(text: &str) -> Option<Value> {
    let batches = pass::run_in(&pass::session(), &pass::catalog(), text)
        .await
        .ok()?;
    let column = batches.first()?.column(0).clone();
    if column.is_null(0) {
        return Some(Value::Null);
    }
    Some(match column.data_type() {
        DataType::Boolean => {
            let values = column
                .as_any()
                .downcast_ref::<datafusion::arrow::array::BooleanArray>()?;
            Value::Bool(values.value(0))
        }
        _ => {
            let text = cast(&column, &DataType::Utf8).ok()?;
            let text = text
                .as_any()
                .downcast_ref::<datafusion::arrow::array::StringArray>()?;
            Value::String(text.value(0).to_string())
        }
    })
}

/// The patterns whose results differ from RE2's by design.
const DIFFERENCES: &[&str] = &[r"\C"];

#[tokio::test]
async fn the_regex_functions_give_re2_s_answers() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/re2.jsonl");
    let mut failures = Vec::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let (function, pattern, text) = (
            case["function"].as_str().unwrap(),
            case["pattern"].as_str().unwrap(),
            case["text"].as_str().unwrap(),
        );
        let name = match function {
            "contains" => "REGEXP_CONTAINS",
            "extract" => "REGEXP_EXTRACT",
            _ => "REGEXP_REPLACE",
        };
        let replacement = case["replacement"].as_str();
        let mut arguments = vec![literal(text), literal(pattern)];
        arguments.extend(replacement.map(literal));
        let literals = format!("FROM one |> SELECT {name}({}) AS r", arguments.join(", "));
        let mut fields = vec![
            format!("{} AS s", literal(text)),
            format!("{} AS p", literal(pattern)),
        ];
        fields.extend(replacement.map(|r| format!("{} AS w", literal(r))));
        let columns = match replacement {
            Some(_) => "s, p, w",
            None => "s, p",
        };
        let rows = format!(
            "FROM UNNEST([STRUCT({})]) |> SELECT {name}({columns}) AS r",
            fields.join(", ")
        );
        let expected = match case.get("error") {
            Some(Value::Bool(true)) => None,
            // `\C`, any byte, which RE2 accepts, can't match alone in a pattern over strings, so
            // avenger-sql rejects it.
            _ if DIFFERENCES.contains(&pattern) => None,
            _ => Some(case["result"].clone()),
        };
        for (path, script) in [("literals", &literals), ("columns", &rows)] {
            let found = run(script).await;
            if found != expected {
                failures.push(format!(
                    "{path}: {script}\n  RE2 {expected:?}, avenger-sql {found:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_literal_replacement_s_errors_are_at_its_argument() {
    let error = |replacement: &str| {
        let text = format!("FROM one |> SELECT REGEXP_REPLACE('abc', r'(b)', {replacement}) AS r");
        let analysis = pass::analysis(&text);
        let diagnostic = analysis.diagnostics.first().expect("an error");
        (
            text[diagnostic.range].to_string(),
            diagnostic.message.clone(),
        )
    };
    assert_eq!(
        error(r"r'\2'"),
        (
            r"r'\2'".to_string(),
            "the replacement reads group 2, and the pattern has 1".to_string()
        )
    );
    assert_eq!(
        error(r"r'\x'").1,
        r"the replacement has the escape `\x`; its escapes are `\0` to `\9` and `\\`"
    );
    assert_eq!(
        error(r"'a\\'").1,
        "the replacement ends in a lone backslash"
    );
}

#[test]
fn a_pattern_that_re2_rejects_has_re2_s_error_at_its_argument() {
    let error = |pattern: &str| {
        let text = format!("FROM one |> SELECT REGEXP_CONTAINS('a', r'{pattern}') AS r");
        let analysis = pass::analysis(&text);
        let diagnostic = analysis.diagnostics.first().expect("an error");
        assert_eq!(&text[diagnostic.range], format!("r'{pattern}'"));
        diagnostic.message.clone()
    };
    for (pattern, message) in [
        (r"(a)\1", r"invalid escape sequence: \1"),
        (r"\8", r"invalid escape sequence: \8"),
        (r"\p{Letter}", r"invalid character class range: \p{Letter}"),
        (r"[[:foo:]]", r"invalid character class range: [:foo:]"),
        (r"[z-a]", r"invalid character class range: z-a"),
        (r"[ab", r"missing ]: [ab"),
        (r"(?x)a", r"invalid perl operator: (?x"),
        (r"(?i-)a", r"invalid perl operator: (?i-)"),
        (r"a**", r"bad repetition operator: **"),
        (r"x{2}{3}", r"bad repetition operator: {2}{3}"),
        (r"a{1001}", r"invalid repetition size: {1001}"),
        (r"(?:a{500}){3}", r"invalid repetition size: {3}"),
        (
            r"\C",
            r"`\C` matches one byte, which a pattern over strings can't match alone",
        ),
    ] {
        assert_eq!(
            error(pattern),
            format!("the pattern doesn't compile: {message}"),
            "{pattern}"
        );
    }
}
