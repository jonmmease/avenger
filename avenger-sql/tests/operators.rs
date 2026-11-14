//! The operators against GoogleSQL: each case of `data/googlesql_operators.txt`, which GoogleSQL's
//! `execute_query` recorded, planned through the pass as `FROM one |> SELECT <case> AS v`, with
//! the form of its outermost operator, the type of its result, and its value.

mod pass;

use avenger_sql::types;
use avenger_sql_syntax::ast::{self, AstNode};

/// A case of the data: its text, the form of its outermost operator, its type, and its value.
struct Case {
    text: String,
    form: String,
    ty: String,
    value: String,
}

fn cases() -> Vec<Case> {
    let data = include_str!("data/googlesql_operators.txt");
    data.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            Case {
                text: parts[0].to_string(),
                form: parts[1].to_string(),
                ty: parts.get(2).unwrap_or(&"").to_string(),
                value: parts.get(3).unwrap_or(&"").to_string(),
            }
        })
        .collect()
}

/// GoogleSQL's form in avenger-sql's terms: `DOUBLE` is `FLOAT64`, and the arguments of
/// `repeated(n) T` are `n` arguments of `T`.
fn normalized(form: &str) -> String {
    let form = form.replace("DOUBLE", "FLOAT64");
    let Some(open) = form.find('(') else {
        return form;
    };
    let close = form.rfind(") -> ").expect("a result");
    let args: Vec<String> = form[open + 1..close]
        .split(", ")
        .flat_map(|arg| match arg.strip_prefix("repeated(") {
            Some(rest) => {
                let (n, ty) = rest.split_once(") ").expect("a repeated argument");
                vec![ty.to_string(); n.parse().expect("a count")]
            }
            None => vec![arg.to_string()],
        })
        .collect();
    // GoogleSQL writes the functions of `||` in lower case.
    let name = match form.starts_with('$') {
        true => form[..open].to_string(),
        false => form[..open].to_ascii_uppercase(),
    };
    format!("{name}({}) -> {}", args.join(", "), &form[close + 5..])
}

/// The cases where avenger-sql differs from GoogleSQL's `execute_query`, and why.
const DIFFERENCES: &[(&str, &str)] = &[(
    "DATETIME '2024-01-01 00:00:00' = DATE '2024-01-01'",
    "execute_query 2026.11.1 casts DATE '2024-01-01' to the DATETIME 1970-01-01 00:00:00, where \
     BigQuery gives 2024-01-01 00:00:00",
)];

#[tokio::test]
async fn operators_resolve_and_compute_as_googlesql_does() {
    let mut failures = Vec::new();
    for case in cases() {
        if DIFFERENCES.iter().any(|(text, _)| *text == case.text) {
            continue;
        }
        let text = format!("FROM one |> SELECT {} AS v", case.text);
        let analysis = pass::analysis(&text);
        if case.form == "ERROR" {
            let message = analysis.diagnostics.first().map(|d| d.message.clone());
            match message {
                Some(message) if case.value.contains(&message) => {}
                other => failures.push(format!(
                    "{}: wants the error {}, has {other:?}",
                    case.text, case.value
                )),
            }
            continue;
        }
        if !analysis.diagnostics.is_empty() {
            failures.push(format!("{}: {:?}", case.text, analysis.diagnostics));
            continue;
        }
        // The form of the outermost operator, at the range of the case's expression.
        let parse = avenger_sql_syntax::parse(&text);
        let item = parse
            .syntax()
            .descendants()
            .find_map(ast::SelectItem::cast)
            .and_then(|item| item.expr())
            .expect("an item");
        let form = analysis
            .model
            .call_at(item.syntax().text_range())
            .unwrap_or("-")
            .to_string();
        if form != normalized(&case.form) {
            failures.push(format!(
                "{}: the form {form}, not {}",
                case.text,
                normalized(&case.form)
            ));
        }
        let batches = match pass::run(&text).await {
            Ok(batches) => batches,
            Err(errors) => {
                failures.push(format!("{}: {errors:?}", case.text));
                continue;
            }
        };
        let column = batches[0].column(0);
        let ty = types::googlesql_name(column.data_type());
        let want_ty = case.ty.replace("DOUBLE", "FLOAT64");
        if ty != want_ty {
            failures.push(format!("{}: the type {ty}, not {want_ty}", case.text));
        }
        let got = pass::values::json(column, 0);
        if got != case.value {
            failures.push(format!("{}: {got}, not {}", case.text, case.value));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
