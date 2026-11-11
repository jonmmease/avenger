//! The token pass and the syntax pass: the reserved prefix, queries that start with `FROM`,
//! windows in pipe operators, the option pairs of the transforms, omitted named arguments, and the
//! spans of errors. Each item lowers to the statement that `expand` prints, and the lowered
//! statement returns the rows of a standard query.

mod common;

use arrow::util::pretty::pretty_format_batches;
use common::{charts, session};
use datafusion::common::DataFusionError;

/// The expansion of `query`.
fn expand(query: &str) -> String {
    avenger_sql::expand(&session().state(), query).unwrap()
}

/// The error of planning `query`.
async fn error(query: &str) -> String {
    planning_error(query).await.to_string()
}

/// The error of planning `query`, with its diagnostic.
async fn planning_error(query: &str) -> DataFusionError {
    let ctx = session();
    match avenger_sql::sql(&ctx, query).await {
        Ok(_) => panic!("`{query}` plans"),
        Err(error) => error,
    }
}

/// The rows of `query`, as sorted lines of text.
async fn rows(query: &str) -> Vec<String> {
    let ctx = session();
    let batches = avenger_sql::sql(&ctx, query)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let text = pretty_format_batches(&batches).unwrap().to_string();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    lines.sort();
    lines
}

/// The line and the column, both from 1, of the diagnostic of `error`.
fn position(error: &DataFusionError) -> (u64, u64) {
    let span = (error.diagnostic())
        .and_then(|diagnostic| diagnostic.span)
        .unwrap_or_else(|| panic!("a span: {error}"));
    (span.start.line, span.start.column)
}

/// The column, from 1, of the first `needle` in `text`.
fn column_of(text: &str, needle: &str) -> u64 {
    text.find(needle).expect("the needle") as u64 + 1
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

#[tokio::test]
async fn the_prefix_of_the_passes_is_reserved() {
    let query = "SELECT body_mass_g AS __avenger_x FROM penguins";
    let error = planning_error(query).await;
    assert!(
        error
            .to_string()
            .contains("`__avenger_x` starts with `__avenger_`"),
        "{error}"
    );
    assert_eq!(position(&error), (1, column_of(query, "__avenger_x")));
    let error = planning_error("SELECT \"__AVENGER_branch\" FROM penguins").await;
    assert!(error.to_string().contains("reserves"), "{error}");
}

#[tokio::test]
async fn a_window_of_a_pipe_extend_reads_a_derived_table() {
    let query = "FROM penguins |> EXTEND row_number() OVER (ORDER BY body_mass_g, species, island, sex) AS r |> WHERE r <= 3 |> SELECT species, r";
    assert_eq!(
        expand(query),
        "SELECT *, row_number() OVER (ORDER BY body_mass_g, species, island, sex) AS r FROM (SELECT * FROM penguins) |> WHERE r <= 3 |> SELECT species, r"
    );
    assert_eq!(
        rows(query).await,
        rows("SELECT species, r FROM (SELECT *, row_number() OVER (ORDER BY body_mass_g, species, island, sex) AS r FROM penguins) WHERE r <= 3").await
    );
    // A pipe SELECT, and two windows in a row.
    let query = "FROM penguins |> SELECT species, rank() OVER (ORDER BY species) AS a |> EXTEND dense_rank() OVER (ORDER BY a) AS b |> AGGREGATE count(*) AS n GROUP BY species, a, b";
    assert_eq!(
        expand(query),
        "SELECT *, dense_rank() OVER (ORDER BY a) AS b FROM (SELECT species, rank() OVER (ORDER BY species) AS a FROM (SELECT * FROM penguins)) |> AGGREGATE count(*) AS n GROUP BY species, a, b"
    );
    assert_eq!(
        rows(query).await,
        rows("SELECT species, a, b, count(*) AS n FROM (SELECT *, dense_rank() OVER (ORDER BY a) AS b FROM (SELECT species, rank() OVER (ORDER BY species) AS a FROM penguins)) GROUP BY species, a, b").await
    );
}

#[tokio::test]
async fn a_derived_table_keeps_the_qualifiers_and_the_tables_of_the_query() {
    // The qualifier `p` stays valid after the derived table.
    let query = "FROM penguins AS p |> EXTEND row_number() OVER (ORDER BY p.body_mass_g, p.species, p.island, p.sex) AS r |> WHERE r <= 2 |> SELECT p.species, r";
    assert_eq!(
        rows(query).await,
        rows("SELECT species, r FROM (SELECT *, row_number() OVER (ORDER BY body_mass_g, species, island, sex) AS r FROM penguins) WHERE r <= 2").await
    );
    // The CTEs of the query stay readable after the window.
    let query = "WITH t AS (SELECT * FROM penguins) FROM t |> EXTEND row_number() OVER () AS r |> AGGREGATE count(*) AS n |> CROSS JOIN (SELECT count(*) AS total FROM t)";
    assert_eq!(
        rows(query).await,
        rows("SELECT 344 AS n, 344 AS total").await
    );
}

#[tokio::test]
async fn unnest_in_a_pipe_extend_reads_a_derived_table() {
    let query = "SELECT 1 AS a |> EXTEND unnest([10, 20]) AS v";
    assert_eq!(
        expand(query),
        "SELECT *, unnest([10, 20]) AS v FROM (SELECT 1 AS a)"
    );
    assert_eq!(
        rows(query).await,
        rows("SELECT 1 AS a, 10 AS v UNION ALL SELECT 1, 20").await
    );
}

#[tokio::test]
async fn a_named_option_of_a_transform_is_an_option_pair() {
    let query = "FROM penguins |> EXTEND bin(body_mass_g, maxbins => 20) OVER () AS b |> AGGREGATE count(*) AS n GROUP BY b";
    assert_eq!(
        expand(query),
        "SELECT *, bin(body_mass_g, 'maxbins', 20) OVER () AS b FROM (SELECT * FROM penguins) |> AGGREGATE count(*) AS n GROUP BY b"
    );
    assert_eq!(
        rows(query).await,
        rows("SELECT b, count(*) AS n FROM (SELECT bin(body_mass_g, 'maxbins', 20) OVER () AS b FROM penguins) GROUP BY b").await
    );
    // The options keep the order of the call, after the positional arguments.
    assert_eq!(
        expand("SELECT bin(body_mass_g, maxbins => 20, 'nice', false) OVER () FROM penguins"),
        "SELECT bin(body_mass_g, 'nice', false, 'maxbins', 20) OVER () FROM penguins"
    );
}

#[tokio::test]
async fn the_weight_of_a_lump_is_the_text_of_its_expression() {
    let query = "FROM penguins |> EXTEND lump(island, n => 1, weight => sum(body_mass_g)) OVER () AS l |> AGGREGATE count(*) AS c GROUP BY l";
    assert_eq!(
        expand(query),
        "SELECT *, lump(island, 'n', 1, 'weight', 'sum(body_mass_g)') OVER () AS l FROM (SELECT * FROM penguins) |> AGGREGATE count(*) AS c GROUP BY l"
    );
    assert_eq!(
        rows(query).await,
        rows("SELECT l, count(*) AS c FROM (SELECT lump(island, 'n', 1, 'weight', 'sum(body_mass_g)') OVER () AS l FROM penguins) GROUP BY l").await
    );
}

#[tokio::test]
async fn an_unknown_column_in_a_lowered_operator_points_to_its_text() {
    let query = "FROM penguins |> EXTEND row_number() OVER (ORDER BY body_mas) AS r";
    let error = planning_error(query).await;
    assert!(error.to_string().contains("body_mas"), "{error}");
    assert_eq!(position(&error), (1, column_of(query, "body_mas")));
}

#[tokio::test]
async fn a_pipe_call_is_the_call_of_its_table_function_over_the_query_before_it() {
    let query = "FROM penguins |> AGGREGATE count(*) AS n GROUP BY species, island |> CALL impute(island, [species], n, value => 0)";
    assert_eq!(
        expand(query),
        "SELECT * FROM impute((SELECT * FROM penguins |> AGGREGATE count(*) AS n GROUP BY species, island), island, [species], n, value => 0)"
    );
    let lines = rows(query).await;
    assert_eq!(
        lines,
        rows("SELECT * FROM impute((SELECT species, island, count(*) AS n FROM penguins GROUP BY species, island), island, [species], n, value => 0)").await
    );
    // The 5 pairs of a species and an island, and 4 that impute adds.
    assert_eq!(lines.len() - 4, 9, "{lines:#?}");
}

#[tokio::test]
async fn the_over_clause_of_a_pipe_call_is_its_partition() {
    let query = "FROM penguins |> AGGREGATE count(*) AS n GROUP BY sex, species, island |> CALL impute(island, [species], n, value => 0) OVER (PARTITION BY sex) AS filled |> WHERE filled.n = 0 |> AGGREGATE count(*) AS added GROUP BY sex";
    assert_eq!(
        expand(query),
        "SELECT * FROM impute((SELECT * FROM penguins |> AGGREGATE count(*) AS n GROUP BY sex, species, island), island, [species], n, value => 0, partition_by => [sex]) AS filled |> WHERE filled.n = 0 |> AGGREGATE count(*) AS added GROUP BY sex"
    );
    assert_eq!(
        rows(query).await,
        rows("SELECT sex, count(*) AS added FROM impute((SELECT sex, species, island, count(*) AS n FROM penguins GROUP BY sex, species, island), island, [species], n, value => 0, partition_by => [sex]) WHERE n = 0 GROUP BY sex").await
    );
}

#[tokio::test]
async fn the_over_clause_of_a_pipe_call_has_only_a_partition() {
    let query = "FROM penguins |> CALL impute(island, [species], body_mass_g, value => 0) OVER (ORDER BY species)";
    let error = planning_error(query).await;
    assert!(
        error.to_string().contains(
            "the OVER clause of CALL impute has only a PARTITION BY list of columns, not an ORDER BY"
        ),
        "{error}"
    );
    assert_eq!(position(&error), (1, column_of(query, "species)")));
    let query = "FROM penguins |> CALL impute(island, [species], body_mass_g, value => 0) OVER (PARTITION BY upper(sex))";
    let error = planning_error(query).await;
    assert!(
        (error.to_string()).contains("a partition of CALL impute is a column, not `upper(sex)`"),
        "{error}"
    );
    assert_eq!(position(&error), (1, column_of(query, "upper(sex)")));
}

#[tokio::test]
async fn the_arguments_of_a_pipe_call_are_lowered() {
    // A named option of a transform in an argument of the call is an option pair.
    let query = "FROM penguins |> CALL impute(island, [species], body_mass_g, value => (SELECT max(b) FROM (SELECT bin(body_mass_g, maxbins => 5) OVER () AS b FROM penguins) |> SELECT b.start AS b))";
    assert!(
        expand(query).contains("bin(body_mass_g, 'maxbins', 5)"),
        "{}",
        expand(query)
    );
}

#[tokio::test]
async fn the_aggregate_items_of_chart_join_its_charts() {
    let query =
        "FROM penguins |> CHART bar(x => species, y => count(*)), count(*) AS n GROUP BY island";
    let expanded = expand(query);
    assert!(
        expanded.starts_with("SELECT __avenger_aggregates.island, __avenger_charts.chart, __avenger_aggregates.n FROM (SELECT * FROM penguins |> AGGREGATE count(*) AS n GROUP BY island) AS __avenger_aggregates LEFT OUTER JOIN __avenger_chart("),
        "{expanded}"
    );
    assert!(
        expanded.ends_with("AS __avenger_charts ON __avenger_aggregates.island IS NOT DISTINCT FROM __avenger_charts.island"),
        "{expanded}"
    );
    // Without groups, the join is ON true. Without aggregate items, there is no join.
    let expanded = expand("FROM penguins |> CHART bar(x => species, y => count(*)), count(*) AS n");
    assert!(
        expanded.ends_with("AS __avenger_charts ON true"),
        "{expanded}"
    );
    let expanded =
        expand("FROM penguins |> CHART bar(x => species, y => count(*)) GROUP BY island");
    assert!(!expanded.contains("JOIN"), "{expanded}");
    assert_eq!(
        rows("FROM (FROM penguins |> CHART bar(x => species, y => count(*)), count(*) AS n GROUP BY island) |> SELECT island, n").await,
        rows("FROM penguins |> AGGREGATE count(*) AS n GROUP BY island").await
    );
}

#[tokio::test]
async fn the_mark_items_of_chart_join_their_charts() {
    let expanded = expand("FROM penguins |> CHART bar(x => species, y => count(*)) AS a, bar(x => sex, y => count(*)) AS b GROUP BY island");
    assert!(
        expanded.starts_with("SELECT __avenger_charts.island, __avenger_charts.a, __avenger_charts_2.b FROM __avenger_chart("),
        "{expanded}"
    );
    assert!(
        expanded.ends_with("AS __avenger_charts_2 ON __avenger_charts.island IS NOT DISTINCT FROM __avenger_charts_2.island"),
        "{expanded}"
    );
}

#[tokio::test]
async fn a_trailing_comma_ends_a_call_a_list_or_the_items_of_an_operator() {
    // The arguments of a call, and a list.
    assert_eq!(expand("SELECT abs(-1,), [1, 2,]"), "SELECT abs(-1), [1, 2]");
    assert_eq!(
        rows("SELECT abs(-1,) AS a").await,
        rows("SELECT 1 AS a").await
    );
    // The items of a pipe AGGREGATE before GROUP BY, and a GROUP BY list before the next operator.
    let query = "FROM penguins |> AGGREGATE count(*) AS n, GROUP BY island, |> WHERE n > 100";
    assert_eq!(
        rows(query).await,
        rows("SELECT island, count(*) AS n FROM penguins GROUP BY island HAVING count(*) > 100")
            .await
    );
    // A select list before FROM, and the end of a statement.
    assert_eq!(
        rows("SELECT island, FROM penguins |> AGGREGATE count(*) AS n,").await,
        rows("SELECT count(*) AS n FROM penguins").await
    );
}

#[tokio::test]
async fn a_commented_out_last_argument_leaves_a_trailing_comma() {
    let query = "FROM penguins
|> CHART bar(
     x => species,
     y => count(*),
     -- color => island,
   ),
   count(*) AS n,
   GROUP BY island,
|> ORDER BY island";
    assert_eq!(
        rows(&format!("FROM ({query}) |> SELECT island, n")).await,
        rows("FROM penguins |> AGGREGATE count(*) AS n GROUP BY island |> ORDER BY island").await
    );
    // The tools of a mark, a list of calls.
    let query = "FROM penguins |> CHART symbol(x => bill_length_mm, y => bill_depth_mm, tools => [navigate(), tooltip(),],)";
    assert_eq!(charts(query).await.len(), 1);
}

#[tokio::test]
async fn a_comma_before_the_name_of_an_argument_stays() {
    // `order` and `limit` name arguments here, not clauses.
    let query = "FROM penguins |> CHART line(x => avg(bill_length_mm), y => avg(bill_depth_mm), color => species, order => body_mass_g)";
    assert_eq!(charts(query).await.len(), 1);
    let query = "FROM penguins |> CHART bar(x => species, y => count(*), impute => impute(value => 0, extra_keys => ['x'],),)";
    assert_eq!(charts(query).await.len(), 1);
}

#[tokio::test]
async fn a_comma_before_a_word_that_names_a_column_stays() {
    // WINDOW and QUALIFY can be bare column names, so a comma before them separates items.
    let query = r#"SELECT x, window, qualify FROM (SELECT 1 AS x, 2 AS "window", 3 AS "qualify")"#;
    assert_eq!(rows(query).await.len(), 5, "{:#?}", rows(query).await);
    assert!(rows(query)
        .await
        .iter()
        .any(|row| row.contains("| 1 | 2      | 3       |")));
}

#[tokio::test]
async fn an_error_after_a_trailing_comma_points_to_its_text() {
    let query = "SELECT abs(-1,), nosuch FROM penguins";
    let error = planning_error(query).await;
    assert!(error.to_string().contains("nosuch"), "{error}");
    assert_eq!(position(&error), (1, column_of(query, "nosuch")));
}
