//! The pipeline of `avenger_sql::sql`: a script over the tables of a session, and its errors as a
//! `ScriptError` of diagnostics with ranges.

mod common;

use avenger_sql::ScriptError;
use common::{charts, session};
use datafusion::common::DataFusionError;

/// The diagnostics of `text`, each as its message and the text at its range.
async fn errors(text: &str) -> Vec<(String, String)> {
    let ctx = session();
    let error = match avenger_sql::sql(&ctx, text).await {
        Ok(_) => panic!("`{text}` plans"),
        Err(error) => error,
    };
    let DataFusionError::External(error) = error else {
        panic!("{error} is no ScriptError");
    };
    let error = error.downcast_ref::<ScriptError>().expect("a ScriptError");
    (error.diagnostics.iter())
        .map(|diagnostic| {
            let range = std::ops::Range::<usize>::from(diagnostic.range);
            (diagnostic.message.clone(), text[range].to_string())
        })
        .collect()
}

/// The number of rows of the result of `text`.
async fn count(text: &str) -> usize {
    let ctx = session();
    let batches = avenger_sql::sql(&ctx, text)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    batches.iter().map(|batch| batch.num_rows()).sum()
}

#[tokio::test]
async fn a_query_reads_the_tables_of_the_session() {
    assert_eq!(
        count("FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY species").await,
        3
    );
    // Names match the session's tables without regard to case.
    assert_eq!(count("FROM PENGUINS |> WHERE island = 'Dream'").await, 124);
}

#[tokio::test]
async fn a_named_argument_after_an_omitted_one_binds() {
    let query = "FROM penguins |> AGGREGATE symbol(x => body_mass_g, y => body_mass_g, fill => species) AS chart";
    assert_eq!(charts(query).await.len(), 1);
}

#[tokio::test]
async fn the_result_is_the_last_query_of_a_script_after_its_variables() {
    let script = "DECLARE heavy FLOAT64 DEFAULT 5000;
SET heavy = heavy + 500;
FROM penguins |> WHERE body_mass_g >= heavy";
    assert_eq!(count(script).await, 33);
    assert_eq!(
        count("FROM penguins |> LIMIT 1; FROM penguins |> LIMIT 2").await,
        2
    );
}

#[tokio::test]
async fn the_errors_of_a_script_are_its_diagnostics_at_their_text() {
    assert_eq!(
        errors("FROM penguins |> AGGREGATE symbol(x => body_mass_g, colour => species) AS chart")
            .await,
        [(
            "symbol has no parameter named `colour`".to_string(),
            "colour => species".to_string()
        )]
    );
    // The analysis reports each error of a script.
    let found = errors("FROM penguins |> WHERE nope = 1; FROM nowhere").await;
    assert_eq!(
        found,
        [
            ("Unrecognized name: nope".to_string(), "nope".to_string()),
            (
                "Table not found: nowhere".to_string(),
                "nowhere".to_string()
            )
        ]
    );
}

#[tokio::test]
async fn a_syntax_error_comes_before_the_analysis() {
    let found = errors("FROM penguins |> AGGREGATE symbol(x => ) AS chart").await;
    assert_eq!(found.len(), 1, "{found:?}");
    // The message gives the line and column of each diagnostic.
    let ctx = session();
    let message = match avenger_sql::sql(&ctx, "FROM penguins\n|> WHERE nope").await {
        Ok(_) => panic!("the query plans"),
        Err(error) => error.to_string(),
    };
    assert!(message.contains("[at 2:10]"), "{message}");
}

#[tokio::test]
async fn an_ordered_aggregate_reads_a_string_column_of_any_layout() {
    // The session maps strings to views, and the column of the table is `Utf8`: the argument
    // stays the column, which the `ORDER BY` of the call names.
    let ctx = session();
    let batches = avenger_sql::sql(
        &ctx,
        "FROM penguins |> AGGREGATE STRING_AGG(DISTINCT island, ',' ORDER BY island) AS islands",
    )
    .await
    .unwrap()
    .collect()
    .await
    .unwrap();
    let text = datafusion::arrow::util::pretty::pretty_format_batches(&batches)
        .unwrap()
        .to_string();
    assert!(text.contains("Biscoe,Dream,Torgersen"), "{text}");
}

#[tokio::test]
async fn a_script_imports_the_modules_that_its_host_gives() {
    let modules = |names: &[String]| {
        match names.join(".").as_str() {
        "util" => Some(
            "MODULE util;\nIMPORT MODULE base;\nCREATE PUBLIC FUNCTION heavy(x FLOAT64) AS (x > base.heavy_mass);\n"
                .to_string(),
        ),
        "base" => Some("MODULE base;\nCREATE PUBLIC CONSTANT heavy_mass = 5000;\n".to_string()),
        _ => None,
    }
    };
    let ctx = session();
    let batches = avenger_sql::sql_with_modules(
        &ctx,
        "IMPORT MODULE util; FROM penguins |> WHERE util.heavy(body_mass_g) |> AGGREGATE COUNT(*) AS n",
        &modules,
    )
    .await
    .unwrap()
    .collect()
    .await
    .unwrap();
    let text = datafusion::arrow::util::pretty::pretty_format_batches(&batches)
        .unwrap()
        .to_string();
    assert!(text.contains("| 61 |"), "{text}");
    // Without the host's modules, the import is an error.
    let found = errors("IMPORT MODULE util; FROM penguins").await;
    assert_eq!(found[0].0, "Module not found: util");
}
