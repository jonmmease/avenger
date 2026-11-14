//! The documented examples of the library's scalar functions, planned through the pass as
//! `FROM one |> SELECT <example> AS v`, against the results that the docs record. The examples
//! of the aggregate and window functions read `UNNEST`, which the pass plans in a later phase.

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
