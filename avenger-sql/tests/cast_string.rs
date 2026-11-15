//! `CAST(x AS STRING)` against GoogleSQL: each case of `data/googlesql_cast_string.txt`, which
//! GoogleSQL's `execute_query` recorded in its default time zone, America/Los_Angeles, which a
//! `TIMESTAMP`'s text follows. A line holds the case, a tab, and GoogleSQL's value in brackets.

mod pass;

use datafusion::arrow::array::{Array, AsArray};

#[tokio::test]
async fn casts_to_strings_write_values_as_googlesql_does() {
    let options = avenger_sql::analysis::Options {
        time_zone: "America/Los_Angeles".to_string(),
        ..Default::default()
    };
    let data = include_str!("data/googlesql_cast_string.txt");
    let mut failures = Vec::new();
    for line in data.lines() {
        let (expr, expected) = line.split_once('\t').expect("a case, a tab, and its value");
        let text = format!("FROM one |> SELECT CONCAT('[', {expr}, ']') AS v");
        let parse = avenger_sql_syntax::parse(&text);
        let analysis = avenger_sql::analyze(
            &parse.syntax(),
            &pass::catalog(),
            &avenger_sql::library::Library::new(),
            &options,
        );
        assert!(
            analysis.diagnostics.is_empty(),
            "{text}: {:?}",
            analysis.diagnostics
        );
        let batches = analysis.run(&pass::session()).await.unwrap();
        let column = batches[0].column(0).as_string::<i32>().clone();
        let found = match column.is_null(0) {
            true => "NULL".to_string(),
            false => column.value(0).to_string(),
        };
        if found != expected {
            failures.push(format!(
                "{expr}\n  GoogleSQL {expected}\n  avenger   {found}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
