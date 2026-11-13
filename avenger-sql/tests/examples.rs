//! The documented examples of the library's functions through the pass, against the results that
//! the docs record: a scalar example as `FROM one |> SELECT <example> AS v`, and an example that
//! is a query, of an aggregate, a window function, or a function of `MATCH_RECOGNIZE`, as the
//! query it is, whose rows the docs write as lines of values separated by ` | `, after a line of
//! the column names.

mod eval;
mod pass;

use avenger_sql::library::{FunctionKind, Library};

#[tokio::test]
async fn scalar_examples_return_their_results_through_the_pass() {
    let library = Library::new();
    let mut failures = Vec::new();
    let mut checked = 0;
    for function in library.functions() {
        if function.kind != FunctionKind::Scalar || function.examples.is_empty() {
            continue;
        }
        for example in &function.examples {
            // The examples of avenger's constructors build fragments of messages, which the
            // tests of the constructors check.
            if example.query.contains("|>") || example.result.contains('\n') {
                continue;
            }
            let text = format!("FROM one |> SELECT {} AS v", example.query);
            let batches = match pass::run(&text).await {
                Ok(batches) => batches,
                Err(errors) => {
                    failures.push(format!("{}: {errors:?}", example.query));
                    continue;
                }
            };
            let found = eval::literal(batches[0].column(0), 0);
            if found != example.result {
                failures.push(format!(
                    "{}: the docs say {}, the pass {found}",
                    example.query, example.result
                ));
            }
            checked += 1;
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(checked > 100, "{checked} examples");
}

#[tokio::test]
async fn query_examples_return_their_rows_through_the_pass() {
    let library = Library::new();
    let mut failures = Vec::new();
    let mut checked = 0;
    for function in library.functions() {
        for example in &function.examples {
            // A scalar function's example is a query for the functions of `MATCH_RECOGNIZE`.
            if !example.query.starts_with("FROM UNNEST") {
                continue;
            }
            let batches = match pass::run(&example.query).await {
                Ok(batches) => batches,
                Err(errors) => {
                    failures.push(format!("{}: {errors:?}", example.query));
                    continue;
                }
            };
            let mut lines = example.result.lines();
            let header = lines.next().unwrap_or_default();
            let mut want: Vec<String> = lines.map(String::from).collect();
            want.sort();
            let schema = batches[0].schema();
            let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
            let mut found = Vec::new();
            for batch in &batches {
                for row in 0..batch.num_rows() {
                    let cells: Vec<String> = (batch.columns().iter())
                        .map(|column| eval::literal(column, row))
                        .collect();
                    found.push(cells.join(" | "));
                }
            }
            found.sort();
            if names.join(" | ") != header || found != want {
                failures.push(format!(
                    "{}: the docs say {header} {want:?}, the pass {} {found:?}",
                    example.query,
                    names.join(" | ")
                ));
            }
            checked += 1;
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(checked > 20, "{checked} examples");
}
