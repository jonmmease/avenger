//! The rewrites of the pipeline: queries that start with `FROM`, and omitted named arguments.

mod common;

use common::{charts, session};

/// The expansion of `query`.
fn expand(query: &str) -> String {
    avenger_sql::expand(&session().state(), query).unwrap()
}

/// The error of planning `query`.
async fn error(query: &str) -> String {
    let ctx = session();
    match avenger_sql::sql(&ctx, query).await {
        Ok(_) => panic!("`{query}` plans"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_query_that_starts_with_from_selects_every_column() {
    assert_eq!(
        expand("FROM penguins |> AGGREGATE count(*) AS n GROUP BY species"),
        "SELECT * FROM penguins |> AGGREGATE count(*) AS n GROUP BY species"
    );
    assert_eq!(
        expand("SELECT species FROM penguins"),
        "SELECT species FROM penguins"
    );
}

#[test]
fn an_omitted_parameter_before_a_named_one_is_null() {
    assert_eq!(
        expand("SELECT symbol(x => body_mass_g, y => body_mass_g, fill => species) FROM penguins"),
        "SELECT symbol(x => body_mass_g, y => body_mass_g, fill => species, color => NULL) FROM penguins"
    );
    assert_eq!(
        expand("SELECT symbol(body_mass_g, y => body_mass_g) FROM penguins"),
        "SELECT symbol(body_mass_g, y => body_mass_g) FROM penguins",
        "a call that omits only trailing parameters stays as it is"
    );
}

#[tokio::test]
async fn a_named_argument_after_an_omitted_one_binds() {
    let query = "FROM penguins |> AGGREGATE symbol(x => body_mass_g, y => body_mass_g, fill => species) AS chart";
    assert_eq!(charts(query).await.len(), 1);
}

#[tokio::test]
async fn an_unknown_parameter_keeps_the_error_of_datafusion() {
    let message = error("SELECT symbol(x => body_mass_g, colour => species) FROM penguins").await;
    assert!(
        message.contains("Unknown parameter name 'colour'"),
        "{message}"
    );
}

#[tokio::test]
async fn a_parser_error_points_at_the_text() {
    let message = error("FROM penguins |> AGGREGATE symbol(x => ) AS chart").await;
    assert!(message.contains("Line: 1, Column"), "{message}");
}

#[tokio::test]
async fn a_query_holds_one_statement() {
    let message = error("SELECT 1; SELECT 2").await;
    assert!(message.contains("one statement, not 2"), "{message}");
}
