//! The pass's plans of the sources and the core operators, against the rows of hand-written
//! standard queries that DataFusion plans, ported from the probes and tests of the earlier
//! planner.

mod pass;

use std::sync::Arc;

use arrow::{
    array::{AsArray, Int64Array, RecordBatch},
    datatypes::{DataType, Field, Int64Type, Schema},
};
use avenger_chart::{charts_from_arrow, Decode};
use datafusion::{
    datasource::MemTable,
    prelude::{SessionConfig, SessionContext},
};
use pass::{errors, rows, run, run_in, standard_rows, table_text};

/// A catalog with the table `big` of 200,000 rows, `i` and `y`, whose `y` is a permutation of
/// `i`, in 8 partitions, and a session with 8 partitions.
fn big() -> (SessionContext, avenger_sql::catalog::MemoryCatalog) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("i", DataType::Int64, false),
        Field::new("y", DataType::Int64, false),
    ]));
    let partitions = (0..8)
        .map(|p| {
            let i: Vec<i64> = (p * 25_000..(p + 1) * 25_000).collect();
            let y: Vec<i64> = i.iter().map(|i| (i * 7919) % 200_000).collect();
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(Int64Array::from(i)), Arc::new(Int64Array::from(y))],
            )
            .unwrap();
            vec![batch]
        })
        .collect();
    let table = Arc::new(MemTable::try_new(schema, partitions).unwrap());
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(8));
    (ctx, pass::catalog().with_table(&["big"], table))
}

/// The column `name` of the result of `query` over `big`, in the order of its rows.
async fn column(query: &str, name: &str) -> Vec<i64> {
    let (ctx, catalog) = big();
    let batches = run_in(&ctx, &catalog, query)
        .await
        .unwrap_or_else(|errors| panic!("{query}: {errors:?}"));
    let mut values = Vec::new();
    for batch in batches {
        let column = batch.column_by_name(name).expect("the column");
        let column = arrow::compute::cast(column, &DataType::Int64).unwrap();
        values.extend(column.as_primitive::<Int64Type>().values().iter().copied());
    }
    values
}

fn descending(values: &[i64]) -> bool {
    values.windows(2).all(|pair| pair[0] >= pair[1])
}

#[tokio::test]
async fn the_order_of_the_rows_survives_computed_projections() {
    let query = "FROM big |> ORDER BY y DESC |> SET i = i * 2 |> EXTEND i + 1 AS w";
    let y = column(query, "y").await;
    assert_eq!(y.len(), 200_000);
    assert!(descending(&y), "{query} is not in order");
    let query = "FROM big |> ORDER BY y |> SELECT i * 3 AS k, y |> DROP y";
    let k = column(query, "k").await;
    let expected: Vec<i64> = {
        let mut pairs: Vec<(i64, i64)> = (0..200_000)
            .map(|i| ((i * 7919) % 200_000, i * 3))
            .collect();
        pairs.sort();
        pairs.into_iter().map(|(_, k)| k).collect()
    };
    assert_eq!(k, expected, "a dropped key orders the rows");
}

#[tokio::test]
async fn a_limit_takes_the_first_rows_in_the_order() {
    let query = "FROM big |> ORDER BY y DESC |> LIMIT 5 |> EXTEND y * 2 AS z";
    assert_eq!(
        column(query, "y").await,
        [199_999, 199_998, 199_997, 199_996, 199_995]
    );
    let query = "FROM big |> ORDER BY y |> LIMIT 2 OFFSET 3";
    assert_eq!(column(query, "y").await, [3, 4]);
}

#[tokio::test]
async fn the_operators_that_keep_the_order_keep_it() {
    for query in [
        "FROM big |> ORDER BY y DESC |> RENAME y AS v |> SELECT v AS y, i",
        "FROM big |> ORDER BY y DESC |> AS b |> SELECT b.y, b.i + 1 AS j",
        "FROM big |> ORDER BY y DESC |> SET y = y * 1 |> DROP i",
        "FROM big |> ORDER BY y DESC |> WHERE i >= 0",
        "FROM big |> ORDER BY MOD(i * 7919, 200000) DESC |> SELECT y",
        "FROM big |> ORDER BY 2 DESC",
    ] {
        let y = column(query, "y").await;
        assert_eq!(y.len(), 200_000, "{query}");
        assert!(descending(&y), "{query} is not in order");
    }
    let query = "FROM big |> ORDER BY y |> SET y = 200000 - y |> SELECT y";
    assert!(
        descending(&column(query, "y").await),
        "the key is the value before SET"
    );
}

#[tokio::test]
async fn a_window_function_orders_the_rows() {
    let text =
        table_text("FROM t |> ORDER BY ROW_NUMBER() OVER (ORDER BY x DESC) |> SELECT x").await;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(&lines[3..6], ["| 3 |", "| 2 |", "| 1 |"]);
}

#[tokio::test]
async fn select_extend_set_drop_and_rename() {
    assert_eq!(
        rows("FROM penguins |> SELECT species, body_mass_g / 1000 AS kg |> EXTEND kg * 2 AS twice |> SET kg = ROUND(kg, 1) |> DROP twice |> RENAME kg AS mass").await,
        standard_rows("SELECT species, round(body_mass_g / 1000, 1) AS mass FROM penguins").await,
    );
    assert_eq!(
        rows("FROM penguins |> LIMIT 3 |> SELECT * EXCEPT (sex) REPLACE (UPPER(island) AS island)").await,
        standard_rows("SELECT species, upper(island) AS island, bill_length_mm, bill_depth_mm, flipper_length_mm, body_mass_g FROM (SELECT * FROM penguins LIMIT 3)").await,
    );
}

#[tokio::test]
async fn rename_and_set_keep_the_place_of_their_column() {
    let text =
        table_text("FROM penguins |> LIMIT 1 |> RENAME island AS isle |> SET species = 'x'").await;
    let header = text.lines().nth(1).unwrap();
    assert!(header.starts_with("| species | isle "), "{header}");
}

#[tokio::test]
async fn errors_of_the_operators_point_at_their_text() {
    for (query, message, at) in [
        (
            "FROM penguins |> SET nope = 1",
            "Column nope in pipe SET does not exist",
            "nope",
        ),
        (
            "FROM penguins |> RENAME nope AS x",
            "Column nope in pipe RENAME does not exist",
            "nope",
        ),
        (
            "FROM penguins |> DROP nope",
            "Column nope in pipe DROP does not exist",
            "nope",
        ),
        (
            "FROM penguins |> SELECT * REPLACE (1 AS nope)",
            "Column nope in SELECT * REPLACE list does not exist",
            "nope",
        ),
        (
            "FROM penguins |> SELECT species, COUNT(*) AS n",
            "Aggregate function COUNT not allowed in pipe SELECT",
            "COUNT(*)",
        ),
        (
            "FROM penguins |> EXTEND MAX(body_mass_g) AS m",
            "Aggregate function MAX not allowed in pipe EXTEND",
            "MAX(body_mass_g)",
        ),
        (
            "FROM penguins |> AGGREGATE species GROUP BY species",
            "Pipe AGGREGATE cannot include non-aggregate expressions",
            "species",
        ),
        (
            "FROM penguins |> AGGREGATE COUNT(*) GROUP BY 1",
            "GROUP BY ordinal not allowed in pipe AGGREGATE",
            "1",
        ),
        (
            "FROM penguins |> WHERE body_mass_g",
            "WHERE clause should return type BOOL, but returns FLOAT64",
            "body_mass_g",
        ),
        (
            "FROM penguins |> LIMIT 1.5",
            "LIMIT ... OFFSET ... expects an INT64 literal or parameter, got FLOAT64",
            "1.5",
        ),
        ("FROM nope", "Table not found: nope", "nope"),
        (
            "FROM penguins |> WHERE species = 1",
            "No matching signature for operator = for argument types: STRING, INT64",
            "species = 1",
        ),
        (
            "FROM penguins |> AGGREGATE SUM(MAX(body_mass_g))",
            "Aggregations of aggregations are not allowed",
            "MAX(body_mass_g)",
        ),
        (
            "FROM penguins |> AGGREGATE SUM(body_mass_g) + bill_depth_mm GROUP BY species",
            "The item reads the column bill_depth_mm, which is neither grouped nor aggregated",
            "SUM(body_mass_g) + bill_depth_mm",
        ),
        (
            "FROM penguins AS p |> JOIN penguins AS p ON TRUE",
            "Duplicate table alias p in the same FROM clause",
            "p",
        ),
        (
            "FROM penguins AS a NATURAL JOIN penguins AS b",
            "Natural join not supported",
            "NATURAL",
        ),
    ] {
        let found = errors(query);
        assert_eq!(
            found.first(),
            Some(&(message.to_string(), at.to_string())),
            "{query}"
        );
    }
}

#[tokio::test]
async fn a_query_in_parentheses_names_its_columns_without_its_tables() {
    assert_eq!(
        rows("FROM penguins |> JOIN (FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY species) USING (species) |> AGGREGATE MAX(n) AS most").await,
        ["+------+", "+------+", "+------+", "| 152  |", "| most |"],
    );
    assert_eq!(
        rows("FROM penguins |> CROSS JOIN (FROM penguins |> LIMIT 1 |> SELECT species AS other) |> AGGREGATE COUNT(*) AS n").await,
        ["+-----+", "+-----+", "+-----+", "| 344 |", "| n   |"],
    );
    assert_eq!(
        rows("(FROM t |> WHERE x > 1) AS u |> SELECT u.x").await,
        ["+---+", "+---+", "+---+", "| 2 |", "| 3 |", "| x |"],
    );
}

#[tokio::test]
async fn each_kind_of_join() {
    let target =
        "(FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY island |> WHERE island <> 'Dream')";
    let standard =
        "(SELECT island, count(*) AS n FROM penguins GROUP BY island HAVING island <> 'Dream')";
    for (kind, standard_kind) in [
        ("JOIN", "JOIN"),
        ("INNER JOIN", "INNER JOIN"),
        ("LEFT JOIN", "LEFT JOIN"),
        ("LEFT OUTER JOIN", "LEFT OUTER JOIN"),
        ("RIGHT JOIN", "RIGHT JOIN"),
        ("FULL OUTER JOIN", "FULL OUTER JOIN"),
    ] {
        assert_eq!(
            rows(&format!("FROM penguins AS p |> {kind} {target} AS c ON p.island = c.island |> AGGREGATE COUNT(*) AS k, SUM(n) AS s GROUP BY species")).await,
            standard_rows(&format!("SELECT species, count(*) AS k, sum(n) AS s FROM penguins AS p {standard_kind} {standard} AS c ON p.island = c.island GROUP BY species")).await,
            "{kind}",
        );
    }
    assert_eq!(
        rows("FROM t AS a, t AS b |> AGGREGATE COUNT(*) AS n").await,
        ["+---+", "+---+", "+---+", "| 9 |", "| n |"],
    );
}

#[tokio::test]
async fn a_join_using_has_each_key_once() {
    let using = "FROM t AS a |> JOIN (FROM t |> SELECT x, s AS y) AS b USING (x)";
    let text = table_text(&format!("{using} |> AS j |> SELECT * |> ORDER BY x")).await;
    assert_eq!(text.lines().nth(1).unwrap(), "| x | s | y |");
    assert_eq!(text.lines().nth(3).unwrap(), "| 1 | a | a |");
    let text = table_text(&format!("{using} |> RENAME x AS key |> ORDER BY 1")).await;
    assert_eq!(text.lines().nth(1).unwrap(), "| key | s | y |");
    let left = "(FROM t |> WHERE x < 3)";
    let right = "(FROM t |> WHERE x > 1 |> SELECT x, s AS y)";
    assert_eq!(
        rows(&format!(
            "FROM {left} AS a |> FULL OUTER JOIN {right} AS b USING (x) |> SELECT x"
        ))
        .await,
        ["+---+", "+---+", "+---+", "| 1 |", "| 2 |", "| 3 |", "| x |"],
    );
    assert_eq!(
        rows(&format!(
            "FROM {left} AS a |> RIGHT JOIN {right} AS b USING (x) |> SELECT x, a.x AS ax"
        ))
        .await,
        [
            "+---+----+",
            "+---+----+",
            "+---+----+",
            "| 2 | 2  |",
            "| 3 |    |",
            "| x | ax |"
        ],
    );
    // The keys take their common type: an INT64 and a FLOAT64 join as FLOAT64s.
    assert_eq!(
        rows("FROM t |> JOIN (FROM t |> SELECT CAST(x AS FLOAT64) AS x) USING (x) |> AGGREGATE COUNT(*) AS n").await,
        ["+---+", "+---+", "+---+", "| 3 |", "| n |"],
    );
}

#[tokio::test]
async fn aggregate_suffixes_order_the_rows() {
    let text = table_text("FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY species DESC").await;
    assert_eq!(text.lines().nth(3).unwrap(), "| Gentoo    | 124 |");
    let text = table_text("FROM penguins |> AGGREGATE COUNT(*) AS n DESC GROUP BY island").await;
    assert_eq!(text.lines().nth(3).unwrap(), "| Biscoe    | 168 |");
    let text =
        table_text("FROM penguins |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY island").await;
    assert_eq!(text.lines().nth(3).unwrap(), "| Biscoe    | 168 |");
}

#[tokio::test]
async fn aggregates_marks_and_windows_plan_in_items() {
    assert_eq!(
        rows("FROM penguins |> AGGREGATE COUNT(*) AS n, COUNTIF(sex = 'FEMALE') AS f, SUM(body_mass_g) / COUNT(body_mass_g) AS mean GROUP BY species").await,
        standard_rows("SELECT species, count(*) AS n, count(*) FILTER (WHERE sex = 'FEMALE') AS f, sum(body_mass_g) / count(body_mass_g) AS mean FROM penguins GROUP BY species").await,
    );
    assert_eq!(
        rows("FROM penguins |> AGGREGATE SUM(body_mass_g) + flipper_length_mm AS s GROUP BY flipper_length_mm |> WHERE flipper_length_mm = 172").await,
        standard_rows("SELECT flipper_length_mm, sum(body_mass_g) + flipper_length_mm AS s FROM penguins WHERE flipper_length_mm = 172 GROUP BY flipper_length_mm").await,
    );
    let batches = run("FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY species |> AGGREGATE bar(x => species, y => n ORDER BY species) AS chart")
        .await
        .unwrap();
    let column = batches[0].column_by_name("chart").expect("a chart column");
    let charts = charts_from_arrow(column, Decode::Auto).unwrap();
    assert_eq!(charts.len(), 1);
    assert!(charts[0].is_some());
    assert_eq!(
        rows("FROM penguins |> EXTEND ROW_NUMBER() OVER (PARTITION BY species ORDER BY body_mass_g, bill_length_mm) AS r |> WHERE r = 1 |> SELECT species, island, body_mass_g").await,
        // GoogleSQL orders NULL first, where DataFusion's default orders it last.
        standard_rows("SELECT species, island, body_mass_g FROM (SELECT *, row_number() OVER (PARTITION BY species ORDER BY body_mass_g NULLS FIRST, bill_length_mm NULLS FIRST) AS r FROM penguins) WHERE r = 1").await,
    );
    assert_eq!(
        rows("FROM t |> EXTEND SUM(x) OVER (ORDER BY x ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) AS w |> SELECT x, w").await,
        ["+---+---+", "+---+---+", "+---+---+", "| 1 | 1 |", "| 2 | 3 |", "| 3 | 5 |", "| x | w |"],
    );
}

#[tokio::test]
async fn as_keeps_the_key_of_the_order_among_columns_of_one_name() {
    let query = "FROM t |> CROSS JOIN (FROM t |> SELECT x * 10 AS x) AS z |> ORDER BY z.x DESC, t.x |> AS w";
    let text = table_text(query).await;
    let z: Vec<&str> = (text.lines().skip(3).take(9))
        .map(|line| {
            line.trim_matches('|')
                .split('|')
                .next_back()
                .unwrap()
                .trim()
        })
        .collect();
    assert_eq!(
        z,
        ["30", "30", "30", "20", "20", "20", "10", "10", "10"],
        "{text}"
    );
    // The two columns named x make `w.x` ambiguous, as in GoogleSQL.
    assert_eq!(
        errors(&format!("{query} |> SELECT w.x")),
        [("Column name x is ambiguous".to_string(), "x".to_string())]
    );
}

#[tokio::test]
async fn distinct_gives_each_row_once() {
    assert_eq!(
        rows("FROM penguins |> SELECT DISTINCT species").await,
        standard_rows("SELECT DISTINCT species FROM penguins").await,
    );
    assert_eq!(
        rows("FROM penguins |> SELECT island, species |> DISTINCT |> AGGREGATE COUNT(*) AS n")
            .await,
        ["+---+", "+---+", "+---+", "| 5 |", "| n |"],
    );
}

#[tokio::test]
async fn names_resolve_by_googlesql_rules() {
    // A range variable reads its row as its source gives it, after SET, DROP and EXTEND.
    assert_eq!(
        rows("FROM t |> WHERE x = 1 |> SET x = 5 |> EXTEND 3 AS y |> DROP s |> SELECT t.x, t.s, x, y").await,
        ["+---+---+---+---+", "+---+---+---+---+", "+---+---+---+---+", "| 1 | a | 5 | 3 |", "| x | s | x | y |"],
    );
    // Names match without regard to case, and an output column keeps the case it's written in.
    let text = table_text("FROM t AS T |> WHERE X = 1 |> SELECT T.X, S").await;
    assert_eq!(text.lines().nth(1).unwrap(), "| X | S |");
    // A range variable alone is its row, as a struct.
    let text = table_text("FROM t |> WHERE x = 1 |> SELECT t").await;
    assert_eq!(text.lines().nth(3).unwrap(), "| {x: 1, s: a} |");
}

#[tokio::test]
async fn set_operations_by_quantifier_and_name() {
    assert_eq!(
        rows("FROM penguins |> SELECT island |> INTERSECT DISTINCT (FROM penguins |> WHERE species = 'Gentoo' |> SELECT island)").await,
        standard_rows("SELECT island FROM penguins INTERSECT SELECT island FROM penguins WHERE species = 'Gentoo'").await,
    );
    assert_eq!(
        rows("FROM penguins |> SELECT species, island |> UNION ALL BY NAME (FROM penguins |> SELECT island, species) |> AGGREGATE COUNT(*) AS n GROUP BY species").await,
        standard_rows("SELECT species, count(*) * 2 AS n FROM penguins GROUP BY species").await,
    );
    assert_eq!(
        rows("FROM penguins |> SELECT species |> UNION DISTINCT (FROM penguins |> SELECT island) |> AGGREGATE COUNT(*) AS n").await,
        standard_rows("SELECT count(*) AS n FROM (SELECT species FROM penguins UNION SELECT island FROM penguins)").await,
    );
    assert_eq!(
        rows("FROM penguins |> SELECT island |> EXCEPT DISTINCT (FROM penguins |> WHERE species = 'Gentoo' |> SELECT island)").await,
        standard_rows("SELECT island FROM penguins EXCEPT SELECT island FROM penguins WHERE species = 'Gentoo'").await,
    );
}

#[tokio::test]
async fn rollup_and_cube_group_by_grouping_sets() {
    assert_eq!(
        rows("FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY ROLLUP (species, island)").await,
        standard_rows(
            "SELECT species, island, count(*) AS n FROM penguins GROUP BY ROLLUP (species, island)"
        )
        .await,
    );
    assert_eq!(
        rows("FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY CUBE (species, island)").await,
        standard_rows(
            "SELECT species, island, count(*) AS n FROM penguins GROUP BY CUBE (species, island)"
        )
        .await,
    );
    assert_eq!(
        rows("FROM penguins |> AGGREGATE SUM(body_mass_g) AS total GROUP BY sex, ROLLUP (species, island)").await,
        standard_rows("SELECT sex, species, island, sum(body_mass_g) AS total FROM penguins GROUP BY sex, ROLLUP (species, island)").await,
    );
}

#[tokio::test]
async fn a_sample_keeps_about_its_percent_and_repeats_with_a_seed() {
    let count = |text: String| async move { column(&text, "n").await[0] };
    let n =
        count("FROM big |> TABLESAMPLE BERNOULLI (10 PERCENT) |> AGGREGATE COUNT(*) AS n".into())
            .await;
    assert!((19_000..21_000).contains(&n), "{n} of 200,000 rows");
    let n = count("FROM big |> TABLESAMPLE RESERVOIR (100 ROWS PARTITION BY MOD(i, 4)) |> AGGREGATE COUNT(*) AS n".into()).await;
    assert_eq!(n, 400);
    let sample = "FROM t |> ORDER BY x |> TABLESAMPLE BERNOULLI (50 PERCENT) REPEATABLE (3) |> AGGREGATE STRING_AGG(s, ',' ORDER BY s) AS kept";
    assert_eq!(table_text(sample).await, table_text(sample).await);
}

#[tokio::test]
async fn a_correlated_subquery_reads_the_outer_row() {
    assert_eq!(
        rows("FROM penguins AS p |> WHERE body_mass_g > (FROM penguins AS q |> WHERE q.species = p.species |> AGGREGATE AVG(body_mass_g) AS m) |> AGGREGATE COUNT(*) AS n GROUP BY species").await,
        standard_rows("SELECT species, count(*) AS n FROM penguins AS p WHERE body_mass_g > (SELECT avg(body_mass_g) FROM penguins AS q WHERE q.species = p.species) GROUP BY species").await,
    );
    assert_eq!(
        rows("FROM penguins AS p |> WHERE EXISTS (FROM penguins AS q |> WHERE q.island = p.island |> WHERE q.species = 'Gentoo') |> AGGREGATE COUNT(*) AS n").await,
        standard_rows("SELECT count(*) AS n FROM penguins AS p WHERE EXISTS (SELECT 1 FROM penguins AS q WHERE q.island = p.island AND q.species = 'Gentoo')").await,
    );
}

#[tokio::test]
async fn a_name_of_with_is_in_scope_after_it() {
    assert_eq!(
        rows("FROM penguins |> WITH heavy AS (FROM penguins |> WHERE body_mass_g > 5000) |> WHERE species IN (FROM heavy |> SELECT species) |> AGGREGATE COUNT(*) AS n").await,
        standard_rows("SELECT count(*) AS n FROM penguins WHERE species IN (SELECT species FROM penguins WHERE body_mass_g > 5000)").await,
    );
    assert_eq!(
        rows("FROM penguins |> WITH counts AS (FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY species), |> JOIN counts USING (species) |> AGGREGATE MAX(n) AS most").await,
        ["+------+", "+------+", "+------+", "| 152  |", "| most |"],
    );
    assert_eq!(
        errors("FROM penguins |> JOIN later USING (species) |> WITH later AS (FROM penguins)")[0],
        ("Table not found: later".to_string(), "later".to_string()),
    );
}

#[tokio::test]
async fn a_limit_in_a_join_target_takes_the_first_rows_in_its_order() {
    assert_eq!(
        rows("FROM one |> CROSS JOIN (FROM penguins |> WHERE body_mass_g IS NOT NULL |> ORDER BY body_mass_g DESC |> LIMIT 2 |> SELECT body_mass_g)").await,
        ["+-----+-------------+", "+-----+-------------+", "+-----+-------------+", "| 1   | 6050.0      |", "| 1   | 6300.0      |", "| one | body_mass_g |"],
    );
}

#[tokio::test]
async fn unnest_gives_a_row_for_each_element() {
    assert_eq!(
        rows("FROM penguins |> LIMIT 2 |> CROSS JOIN UNNEST(['a', 'b', 'c']) AS letter WITH OFFSET |> AGGREGATE COUNT(*) AS n, SUM(offset) AS offsets").await,
        ["+---+---------+", "+---+---------+", "+---+---------+", "| 6 | 6       |", "| n | offsets |"],
    );
    assert_eq!(
        rows("FROM penguins |> AGGREGATE ARRAY_AGG(DISTINCT island) AS islands |> CROSS JOIN UNNEST(islands) AS island |> AGGREGATE COUNT(*) AS n").await,
        ["+---+", "+---+", "+---+", "| 3 |", "| n |"],
    );
}

#[tokio::test]
async fn an_empty_array_literal_coerces_to_any_array() {
    // `[]` is an `ARRAY<INT64>` alone, and an array of the type beside it.
    let analysis = pass::analysis("FROM t |> SELECT [] AS e, IF(x = 1, [], ['a']) AS s");
    let plan = analysis
        .plan
        .unwrap_or_else(|| panic!("{:?}", analysis.diagnostics));
    let types: Vec<String> = (plan.schema().fields().iter())
        .map(|field| avenger_sql::types::googlesql_name(field.data_type()))
        .collect();
    assert_eq!(types, ["ARRAY<INT64>", "ARRAY<STRING>"]);
    assert!(table_text("FROM t |> SELECT IF(x = 1, [], ['a']) AS s")
        .await
        .contains("[]"));
}

#[tokio::test]
async fn a_cast_to_an_integer_rounds_half_away_from_zero() {
    let text = table_text(
        "FROM one |> SELECT CAST(2.5 AS INT64) AS a, CAST(-2.5 AS INT64) AS b, CAST(1.4 AS INT64) AS c, SAFE_CAST(NUMERIC '0.5' AS INT64) AS d",
    )
    .await;
    let row = text.lines().nth(3).expect("a row");
    let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
    assert_eq!(cells, ["3", "-3", "1", "1"]);
}

#[tokio::test]
async fn a_parenthesized_name_names_its_column() {
    let analysis = pass::analysis(
        "FROM sales |> AGGREGATE COUNT(*) AS n GROUP BY GROUPING SETS ((sp), (isl), ()) |> SELECT (sp), ((isl)), n",
    );
    let plan = analysis.plan.expect("a plan");
    let names: Vec<&str> = (plan.schema().fields().iter())
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(names, ["sp", "isl", "n"]);
}

#[tokio::test]
async fn functions_fail_where_googlesql_fails_and_safe_gives_null() {
    let cases = [
        ("SQRT(-1)", "Argument to SQRT cannot be negative: -1"),
        (
            "SQRT(NUMERIC '-4')",
            "SQRT is undefined for negative value: SQRT(-4)",
        ),
        ("LN(0)", "Floating point error in function: LN(0)"),
        ("LOG(0)", "Floating point error in function: LN(0)"),
        ("LOG(8, 1)", "Floating point error in function: LOG(8, 1)"),
        (
            "LOG10(-2.25)",
            "Floating point error in function: LOG10(-2.25)",
        ),
        (
            "POW(-8, 0.5)",
            "Floating point error in function: POW(-8, 0.5)",
        ),
        ("POW(NUMERIC '0', -1)", "division by zero: POW(0, -1)"),
        ("[1, 2][OFFSET(5)]", "Array index 5 is out of bounds"),
        ("[1, 2][ORDINAL(0)]", "Array index 0 is out of bounds"),
        (
            "REPEAT('ab', -1)",
            "Second argument (repeat count) for REPEAT cannot be negative",
        ),
        (
            "LPAD('abc', -1)",
            "Second argument (output size) for LPAD/RPAD cannot be negative",
        ),
        (
            "RPAD('abc', 5, '')",
            "Third argument (pad pattern) for LPAD/RPAD cannot be empty",
        ),
        (
            "LEFT('abc', -1)",
            "Second argument in LEFT() cannot be negative",
        ),
        (
            "SUBSTRING('abc', 1, -1)",
            "Third argument in SUBSTR() cannot be negative",
        ),
    ];
    let mut failures = Vec::new();
    for (expr, message) in cases {
        match run(&format!("FROM one |> SELECT {expr} AS v")).await {
            Err(errors) if errors[0].contains(message) => {}
            other => failures.push(format!("{expr}: {other:?}")),
        }
        // `SAFE.` returns `NULL` where the call fails.
        if !expr.starts_with('[') {
            let text = table_text(&format!("FROM one |> SELECT SAFE.{expr} AS v")).await;
            let row = text.lines().nth(3).unwrap_or_default();
            if !row.trim_matches(|c| c == '|' || c == ' ').is_empty() {
                failures.push(format!("SAFE.{expr}: {row}"));
            }
        }
    }
    // The `SAFE_` subscripts are `NULL` out of the array, and a branch that no row takes doesn't
    // fail.
    let text = table_text(
        "FROM t |> WHERE x = 1 |> SELECT [1][SAFE_OFFSET(3)] AS a, IF(x = 2, SQRT(-1), 0.0) AS b",
    )
    .await;
    assert!(text.contains("|   | 0.0 |"), "{text}");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The forms that GoogleSQL plans and the pass planned only in part: windows in a pipe `WHERE`,
/// `DISTINCT` windows, `LIKE ANY UNNEST`, array paths in `FROM`, `LEFT JOIN UNNEST ... ON`,
/// `STRICT` unnests, `HAVING MAX` with grouping sets, `BYTES` bitwise operators, pipe `CALL` of
/// a temporary table function, `.*` of an expression, and `SELECT AS STRUCT` and `AS VALUE`.
#[tokio::test]
async fn the_pass_plans_googlesqls_forms() {
    let cases = [
        (
            "FROM t |> WHERE ROW_NUMBER() OVER (ORDER BY x DESC) = 1 |> SELECT x",
            "| x |\n| 3 |",
        ),
        (
            "FROM t |> EXTEND COUNT(DISTINCT s) OVER () AS n |> SELECT x, n",
            "| x | n |\n| 1 | 2 |\n| 2 | 2 |\n| 3 | 2 |",
        ),
        (
            "FROM t |> SELECT x, s LIKE ANY UNNEST(['a%', 'b']) AS any_, s NOT LIKE ALL UNNEST(['a', 'c']) AS all_",
            "| x | any_ | all_  |\n| 1 | true | false |\n| 2 | true | true  |\n| 3 |      |       |",
        ),
        (
            "FROM arrays, arrays.xs AS v WITH OFFSET AS o |> SELECT k, v, o",
            "| k | v | o |\n| 1 | 1 | 0 |\n| 1 | 2 | 1 |",
        ),
        (
            "FROM arrays LEFT JOIN UNNEST(arrays.xs) AS v ON v > 1 |> SELECT k, v",
            "| k | v |\n| 1 | 2 |\n| 2 |   |\n| 3 |   |",
        ),
        (
            "FROM UNNEST([1, 2] AS a, [3, 4] AS b, mode => 'STRICT') |> SELECT a, b",
            "| a | b |\n| 1 | 3 |\n| 2 | 4 |",
        ),
        (
            "FROM sales |> AGGREGATE ANY_VALUE(isl HAVING MAX m) AS top, SUM(m) AS total, GROUPING(sp) AS g GROUP BY ROLLUP(sp)",
            "| sp | top | total | g |\n|    | x   | 11    | 1 |\n| A  | y   | 3     | 0 |\n| B  | x   | 8     | 0 |",
        ),
        (
            r"FROM one |> SELECT b'\x0f' & b'\x3c' AS a, ~b'\x0f' AS n, b'\x01\x00' >> 1 AS r, -8 >> 60 AS i",
            "| a  | n  | r    | i  |\n| 0c | f0 | 0080 | 15 |",
        ),
        (
            "CREATE TEMP TABLE FUNCTION top_n(src ANY TABLE, k INT64) AS (FROM src |> ORDER BY x DESC |> LIMIT k); FROM t |> CALL top_n(2) |> SELECT x",
            "| x |\n| 2 |\n| 3 |",
        ),
        (
            "FROM t |> SELECT STRUCT(x AS a, s AS b, x * 2 AS c).* EXCEPT (b) REPLACE (x AS c)",
            "| a | c |\n| 1 | 1 |\n| 2 | 2 |\n| 3 | 3 |",
        ),
        (
            "FROM t |> AGGREGATE ARRAY_AGG(STRUCT(x AS a, s AS b) ORDER BY x DESC LIMIT 1)[OFFSET(0)].*",
            "| a | b |\n| 3 |   |",
        ),
        (
            "FROM t |> SELECT AS STRUCT x, s |> WHERE x > 1 |> SELECT *",
            "| x | s |\n| 2 | b |\n| 3 |   |",
        ),
        (
            "FROM t |> SELECT AS VALUE x |> SELECT *",
            "| $col_1 |\n| 1      |\n| 2      |\n| 3      |",
        ),
    ];
    let mut failures = Vec::new();
    for (query, expected) in cases {
        let found: Vec<String> = (rows(query).await.into_iter())
            .filter(|line| line.starts_with('|'))
            .collect();
        let mut expected: Vec<String> = expected.lines().map(String::from).collect();
        expected.sort();
        if found != expected {
            failures.push(format!("{query}\n{}", found.join("\n")));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// The errors of those forms, as GoogleSQL's.
#[tokio::test]
async fn the_forms_fail_where_googlesql_fails() {
    let cases = [
        (
            "FROM UNNEST([1, 2] AS a, [3] AS b, mode => 'STRICT')",
            "Unnested arrays under STRICT mode must have equal lengths",
        ),
        (
            r"FROM one |> SELECT b'\x0f' | b'\x3c\x00' AS a",
            "Bitwise binary operator for BYTES requires equal length of the inputs. Got 1 bytes on the left hand side and 2 bytes on the right hand side.",
        ),
        (
            "FROM one |> SELECT 8 >> -1 AS a",
            "Bitwise shift by negative offset.",
        ),
        (
            "CREATE TEMP TABLE FUNCTION f(k INT64) AS (FROM t |> LIMIT k); FROM t |> CALL f(2)",
            "Table-valued function cannot be called with pipe CALL syntax because its signature does not include a table-typed argument",
        ),
        (
            "FROM t |> EXTEND COUNT(DISTINCT s) OVER (ORDER BY x) AS n",
            "Window ORDER BY is not allowed if DISTINCT is specified",
        ),
        (
            "FROM t |> SELECT AS VALUE x, s",
            "SELECT AS VALUE query must have exactly one column",
        ),
        ("FROM t WITH OFFSET", "WITH OFFSET can only be used with array scans"),
    ];
    let mut failures = Vec::new();
    for (query, message) in cases {
        match run(query).await {
            Err(errors) if errors[0].contains(message) => {}
            other => failures.push(format!("{query}: {other:?}")),
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

// --- MATCH_RECOGNIZE ---
//
// The queries and their rows are the cases of `execute_query` in the research notes of
// `MATCH_RECOGNIZE`, with the rows as GoogleSQL's `TO_JSON_STRING` writes them.

/// The rows of `text` as a JSON array of objects, in their order.
async fn json(text: &str) -> String {
    use datafusion::arrow::json::{writer::JsonArray, WriterBuilder};
    let batches = run(text)
        .await
        .unwrap_or_else(|errors| panic!("{text}: {errors:?}"));
    let mut writer = WriterBuilder::new()
        .with_explicit_nulls(true)
        .build::<_, JsonArray>(Vec::new());
    for batch in &batches {
        writer.write(batch).unwrap();
    }
    writer.finish().unwrap();
    match String::from_utf8(writer.into_inner()).unwrap() {
        text if text.is_empty() => "[]".to_string(),
        text => text,
    }
}

#[tokio::test]
async fn the_clause_follows_a_source_or_a_pipe_operator() {
    // Both forms, an alias, and a clause after another.
    let cases = [
        // pipe_basic
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(a.id) AS s, MAX(b.id) AS e, COUNT(*) AS n PATTERN (a b+) DEFINE a AS TRUE, b AS price < PREV(price)) |> ORDER BY s"#,
            r#"[{"s":1,"e":3,"n":3},{"s":5,"e":7,"n":3}]"#,
        ),
        // from_form
        (
            r#"FROM (FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT *) MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(a.id) AS s, MAX(b.id) AS e PATTERN (a b+) DEFINE a AS TRUE, b AS price < PREV(price)) AS mr |> SELECT mr.s, mr.e |> ORDER BY s"#,
            r#"[{"s":1,"e":3},{"s":5,"e":7}]"#,
        ),
        // pipe_alias
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(id) AS s PATTERN (a b+) DEFINE a AS TRUE, b AS price < PREV(price)) AS mr |> SELECT mr.s |> ORDER BY s"#,
            r#"[{"s":1},{"s":5}]"#,
        ),
        // pipe_mr_twice
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(id) AS s, COUNT(*) AS n PATTERN (a b) DEFINE a AS TRUE, b AS TRUE) |> MATCH_RECOGNIZE(ORDER BY s MEASURES SUM(n) AS total PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"[{"total":6}]"#,
        ),
    ];
    for (query, rows) in cases {
        assert_eq!(json(query).await, rows, "{query}");
    }
}

#[tokio::test]
async fn partitions_match_apart_in_their_order() {
    // Partitions, a NULL key, partition columns in measures, anchors per partition, and NULL order keys first.
    let cases = [
        // partition_by
        (
            r#"FROM UNNEST([STRUCT('a' AS sym, 1 AS id, 5 AS price), STRUCT('b', 2, 50), STRUCT('a', 3, 4), STRUCT('b', 4, 40), STRUCT('a', 5, 6), STRUCT('b', 6, 45), STRUCT('a', 7, 3)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY sym ORDER BY id MEASURES MIN(id) AS s, MAX(id) AS e, MATCH_NUMBER() AS m PATTERN (a b+) DEFINE a AS TRUE, b AS price < PREV(price)) |> ORDER BY sym, s"#,
            r#"[{"sym":"a","s":1,"e":3,"m":1},{"sym":"a","s":5,"e":7,"m":2},{"sym":"b","s":2,"e":4,"m":1}]"#,
        ),
        // partition_by_null_key
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 'x' AS k, 1 AS v), STRUCT(2, 'x', NULL), STRUCT(3, NULL, 3), STRUCT(4, NULL, 4), STRUCT(NULL, 'x', 5)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY k ORDER BY id MEASURES COUNT(*) AS n, ARRAY_AGG(id) AS ids PATTERN (a+) DEFINE a AS TRUE) |> ORDER BY k"#,
            r#"[{"k":null,"n":2,"ids":[3,4]},{"k":"x","n":3,"ids":[null,1,2]}]"#,
        ),
        // partition_column_in_measure
        (
            r#"FROM UNNEST([STRUCT('a' AS sym, 1 AS id, 5 AS price), STRUCT('b', 2, 50), STRUCT('a', 3, 4), STRUCT('b', 4, 40), STRUCT('a', 5, 6), STRUCT('b', 6, 45), STRUCT('a', 7, 3)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY sym ORDER BY id MEASURES sym || CAST(COUNT(*) AS STRING) AS label PATTERN (a+) DEFINE a AS TRUE) |> ORDER BY sym"#,
            r#"[{"sym":"a","label":"a4"},{"sym":"b","label":"b3"}]"#,
        ),
        // anchor_start_per_partition
        (
            r#"FROM UNNEST([STRUCT('a' AS sym, 1 AS id, 5 AS price), STRUCT('b', 2, 50), STRUCT('a', 3, 4), STRUCT('b', 4, 40), STRUCT('a', 5, 6), STRUCT('b', 6, 45), STRUCT('a', 7, 3)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY sym ORDER BY id MEASURES MIN(id) AS first_id PATTERN (^ x) DEFINE x AS TRUE) |> ORDER BY sym"#,
            r#"[{"sym":"a","first_id":1},{"sym":"b","first_id":2}]"#,
        ),
        // order_by_nulls_first_default
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 'x' AS k, 1 AS v), STRUCT(2, 'x', NULL), STRUCT(3, NULL, 3), STRUCT(4, NULL, 4), STRUCT(NULL, 'x', 5)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ARRAY_AGG(v) AS vs PATTERN (a+) DEFINE a AS TRUE)"#,
            r#"[{"vs":[5,1,null,3,4]}]"#,
        ),
    ];
    for (query, rows) in cases {
        assert_eq!(json(query).await, rows, "{query}");
    }
}

#[tokio::test]
async fn matching_resumes_by_the_skip_mode() {
    // Both skip modes, and match numbers that count overlaps.
    let cases = [
        // skip_past_last_row_default
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ARRAY_AGG(id) AS ids PATTERN (u u) DEFINE u AS f = 1) |> ORDER BY ids[OFFSET(0)]"#,
            r#"[{"ids":[1,2]},{"ids":[4,5]}]"#,
        ),
        // skip_to_next_row_overlaps
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ARRAY_AGG(id) AS ids, MATCH_NUMBER() AS m AFTER MATCH SKIP TO NEXT ROW PATTERN (u u) DEFINE u AS f = 1) |> ORDER BY m"#,
            r#"[{"ids":[1,2],"m":1},{"ids":[4,5],"m":2},{"ids":[5,6],"m":3}]"#,
        ),
        // skip_to_next_row_greedy
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ARRAY_AGG(id) AS ids AFTER MATCH SKIP TO NEXT ROW PATTERN (u+) DEFINE u AS f = 1) |> ORDER BY ids[OFFSET(0)]"#,
            r#"[{"ids":[1,2]},{"ids":[2]},{"ids":[4,5,6]},{"ids":[5,6]},{"ids":[6]}]"#,
        ),
        // match_number_counts_overlaps
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MATCH_NUMBER() AS m, ARRAY_AGG(id) AS ids AFTER MATCH SKIP TO NEXT ROW PATTERN (u+) DEFINE u AS f = 1) |> ORDER BY m"#,
            r#"[{"m":1,"ids":[1,2]},{"m":2,"ids":[2]},{"m":3,"ids":[4,5,6]},{"m":4,"ids":[5,6]},{"m":5,"ids":[6]}]"#,
        ),
    ];
    for (query, rows) in cases {
        assert_eq!(json(query).await, rows, "{query}");
    }
}

#[tokio::test]
async fn an_empty_match_is_a_row_of_its_own() {
    // Empty matches, and a variable without rows.
    let cases = [
        // star_yields_empty_matches
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n, MIN(id) AS s, MATCH_NUMBER() AS m PATTERN (d*) DEFINE d AS f = 0) |> ORDER BY m"#,
            r#"[{"n":0,"s":null,"m":1},{"n":0,"s":null,"m":2},{"n":1,"s":3,"m":3},{"n":0,"s":null,"m":4},{"n":0,"s":null,"m":5},{"n":0,"s":null,"m":6},{"n":1,"s":7,"m":7}]"#,
        ),
        // measures_first_of_unmatched_var
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES FIRST(z.price) AS fz, COUNT(z.id) AS nz, SUM(z.price) AS sz, ARRAY_AGG(z.id) AS az, COUNT(*) AS n PATTERN (x z*) DEFINE x AS TRUE, z AS FALSE) |> ORDER BY n |> LIMIT 1"#,
            r#"[{"fz":null,"nz":0,"sz":null,"az":null,"n":1}]"#,
        ),
    ];
    for (query, rows) in cases {
        assert_eq!(json(query).await, rows, "{query}");
    }
}

#[tokio::test]
async fn measures_read_the_rows_of_the_match_or_of_a_variable() {
    // Each form of a measure.
    let cases = [
        // measures_var_scoped_aggs
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(dn.price) AS low, COUNT(dn.id) AS n_dn, SUM(up.price) AS up_sum, COUNT(*) AS n PATTERN (st dn+ up+) DEFINE st AS TRUE, dn AS price < PREV(price), up AS price > PREV(price))"#,
            r#"[{"low":6,"n_dn":2,"up_sum":21,"n":5}]"#,
        ),
        // measures_first_last
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES FIRST(price) AS f, LAST(price) AS l, FIRST(dn.id) AS fdn, LAST(dn.id) AS ldn, LAST(up.price) AS lup PATTERN (st dn+ up+) DEFINE st AS TRUE, dn AS price < PREV(price), up AS price > PREV(price))"#,
            r#"[{"f":10,"l":12,"fdn":2,"ldn":3,"lup":12}]"#,
        ),
        // measures_match_row_number_aggregated
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ARRAY_AGG(MATCH_ROW_NUMBER()) AS rn, ARRAY_AGG(CLASSIFIER()) AS cls, MAX(MATCH_ROW_NUMBER()) AS len PATTERN (st dn+ up) DEFINE st AS TRUE, dn AS price < PREV(price), up AS price > PREV(price))"#,
            r#"[{"rn":[1,2,3,4],"cls":["st","dn","dn","up"],"len":4}]"#,
        ),
        // measures_implicit_row_order
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY price MEASURES ARRAY_AGG(id) AS by_match, ARRAY_AGG(id ORDER BY id) AS by_id, STRING_AGG(CAST(price AS STRING), "<") AS prices PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"[{"by_match":[3,7,2,4,1,6,5],"by_id":[1,2,3,4,5,6,7],"prices":"6<7<8<9<10<11<12"}]"#,
        ),
        // measures_having_and_limit
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ANY_VALUE(id HAVING MAX price) AS id_of_max, STRING_AGG(CAST(id AS STRING), "," LIMIT 3) AS first3 PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"[{"id_of_max":5,"first3":"1,2,3"}]"#,
        ),
        // classifier_spelling_from_define
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES STRING_AGG(CLASSIFIER(), ",") AS cls PATTERN (UP dOwN) DEFINE Up AS f = 1, DOWN AS f = 0)"#,
            r#"[{"cls":"Up,DOWN"},{"cls":"Up,DOWN"}]"#,
        ),
        // measures_input_range_variable
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> AS t |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(t.price) AS mx, MAX(x.price) AS mxx PATTERN (x y) DEFINE x AS TRUE, y AS t.price > 0) |> ORDER BY mx"#,
            r#"[{"mx":9,"mxx":6},{"mx":10,"mxx":10},{"mx":12,"mxx":12}]"#,
        ),
        // measures_var_shadows_range_variable
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> AS x |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(x.price) AS mx, MAX(price) AS m PATTERN (x y) DEFINE x AS price > 9, y AS TRUE) |> ORDER BY mx"#,
            r#"[{"mx":10,"m":10},{"mx":12,"m":12}]"#,
        ),
        // measures_distinct_agg
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(DISTINCT f) AS nd, ARRAY_AGG(DISTINCT f ORDER BY f) AS fs PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"[{"nd":2,"fs":[0,1]}]"#,
        ),
    ];
    for (query, rows) in cases {
        assert_eq!(json(query).await, rows, "{query}");
    }
}

#[tokio::test]
async fn define_reads_the_rows_around_the_row() {
    // PREV at an offset, and use_longest_match.
    let cases = [
        // define_prev_offset
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(id) AS s PATTERN (p) DEFINE p AS price > PREV(price, 2)) |> ORDER BY s"#,
            r#"[{"s":4},{"s":5},{"s":6}]"#,
        ),
        // longest_match_option
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ARRAY_AGG(id) AS ids PATTERN (u | u u) DEFINE u AS f = 1 OPTIONS (use_longest_match = TRUE)) |> ORDER BY ids[OFFSET(0)]"#,
            r#"[{"ids":[1,2]},{"ids":[4,5]},{"ids":[6]}]"#,
        ),
    ];
    for (query, rows) in cases {
        assert_eq!(json(query).await, rows, "{query}");
    }
}

#[test]
fn the_errors_are_googlesqls() {
    // The messages of `execute_query` for each error of the clause that the pass reports.
    let cases = [
        // pipe_no_alias_no_name
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MIN(a.id) AS s PATTERN (a b+) DEFINE a AS TRUE, b AS price < PREV(price)) |> SELECT mr.s"#,
            r#"Unrecognized name: mr"#,
        ),
        // pipe_on_value_table
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 2 AS x)]) |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(a.x) AS m PATTERN (a) DEFINE a AS TRUE)"#,
            r#"Name a.x is a value table field, but a column was expected"#,
        ),
        // input_columns_not_passed_through
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a+) DEFINE a AS TRUE) |> SELECT price"#,
            r#"Unrecognized name: price"#,
        ),
        // order_by_ordinal
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY 1 MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)"#,
            r#"Ordinals are not allowed in MATCH_RECOGNIZE ORDER BY clause"#,
        ),
        // partition_by_ordinal
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY 1 ORDER BY id MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)"#,
            r#"Ordinals are not allowed in MATCH_RECOGNIZE PARTITION BY clause"#,
        ),
        // order_by_aggregate
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY MAX(id) MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)"#,
            r#"Aggregate function MAX not allowed in MATCH_RECOGNIZE ORDER BY clause"#,
        ),
        // partition_by_window
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY ROW_NUMBER() OVER () ORDER BY id MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)"#,
            r#"Analytic function not allowed in MATCH_RECOGNIZE PARTITION BY clause"#,
        ),
        // order_by_unorderable_type
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY STRUCT(id) MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)"#,
            r#"Ordering by expressions of type STRUCT is not allowed"#,
        ),
        // bounded_upper_below_lower
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (u{3,1}) DEFINE u AS f = 1)"#,
            r#"Quantifier upper bound cannot be smaller than the lower bound"#,
        ),
        // empty_pattern_alone_impossible
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN () DEFINE z AS TRUE)"#,
            r#"Pattern variable `z` is defined but not used in the pattern"#,
        ),
        // define_prev_negative_offset
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > PREV(price, -1))"#,
            r#"The offset to the function LAG must not be negative"#,
        ),
        // define_prev_column_offset
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > PREV(price, id))"#,
            r#"Argument 2 to PREV must be a literal or query parameter"#,
        ),
        // define_prev_null_offset
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > PREV(price, NULL))"#,
            r#"Argument 2 to PREV must be non-NULL"#,
        ),
        // define_prev_nested
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > PREV(NEXT(price)))"#,
            r#"function NEXT cannot be nested in another PREV() or NEXT() call"#,
        ),
        // define_safe_prev
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > SAFE.PREV(price))"#,
            r#"Function PREV does not support SAFE error mode"#,
        ),
        // define_prev_ignore_nulls
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > PREV(price IGNORE NULLS))"#,
            r#"IGNORE NULLS and RESPECT NULLS are not supported on scalar functions"#,
        ),
        // define_non_bool
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price)"#,
            r#"MATCH_RECOGNIZE DEFINE predicate should return type BOOL, but returns INT64"#,
        ),
        // define_aggregate
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > AVG(price))"#,
            r#"Aggregate function AVG not allowed in MATCH_RECOGNIZE DEFINE predicate"#,
        ),
        // define_window_function
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS price > LAG(price) OVER (ORDER BY id))"#,
            r#"Analytic function not allowed in MATCH_RECOGNIZE DEFINE predicate"#,
        ),
        // define_pattern_variable_column
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a b) DEFINE a AS TRUE, b AS price > a.price)"#,
            r#"Cannot access columns through pattern variables."#,
        ),
        // define_match_number
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS MATCH_NUMBER() > 0)"#,
            r#"function MATCH_NUMBER can only be used inside a MATCH_RECOGNIZE's MEASURES clause"#,
        ),
        // define_classifier
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS CLASSIFIER() = "p")"#,
            r#"function CLASSIFIER can only be used inside a MATCH_RECOGNIZE's MEASURES clause"#,
        ),
        // define_unknown_column
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (p) DEFINE p AS volume > 0)"#,
            r#"Unrecognized name: volume"#,
        ),
        // undefined_variable
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a b) DEFINE a AS TRUE)"#,
            r#"Pattern variable `b` is not defined in the DEFINE clause"#,
        ),
        // unused_define
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE, b AS TRUE)"#,
            r#"Pattern variable `b` is defined but not used in the pattern"#,
        ),
        // duplicate_define
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE, A AS FALSE)"#,
            r#"Pattern variable `A` is defined multiple times"#,
        ),
        // define_prev_in_measures
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(PREV(price)) AS n PATTERN (a) DEFINE a AS TRUE)"#,
            r#"function PREV must be directly inside a MATCH_RECOGNIZE's DEFINE clause, without any enclosing subquery"#,
        ),
        // prev_outside_mr
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> SELECT PREV(price) AS p"#,
            r#"function PREV must be directly inside a MATCH_RECOGNIZE's DEFINE clause, without any enclosing subquery"#,
        ),
        // measures_first_distinct
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES FIRST(DISTINCT price) AS f PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"Aggregate function FIRST does not support DISTINCT in arguments"#,
        ),
        // measures_first_order_by
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES FIRST(price ORDER BY id) AS f PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"FIRST does not support ORDER BY in arguments"#,
        ),
        // first_outside_mr
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> AGGREGATE FIRST(price) AS f"#,
            r#"aggregate function FIRST can only be used inside a MATCH_RECOGNIZE's MEASURES clause"#,
        ),
        // measures_match_row_number_unaggregated
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MATCH_ROW_NUMBER() AS r PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"Function MATCH_ROW_NUMBER must be aggregated"#,
        ),
        // measures_classifier_unaggregated
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES CLASSIFIER() AS c PATTERN (x+) DEFINE x AS TRUE)"#,
            r#"Function CLASSIFIER must be aggregated"#,
        ),
        // measures_non_aggregated_column
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES price AS p PATTERN (x) DEFINE x AS TRUE)"#,
            r#"MATCH_RECOGNIZE MEASURES clause expression references column price which is neither grouped nor aggregated"#,
        ),
        // measures_bare_var_column
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES x.price AS p PATTERN (x) DEFINE x AS TRUE)"#,
            r#"Cannot access columns through pattern variables."#,
        ),
        // measures_var_as_value
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES ANY_VALUE(x) AS p PATTERN (x) DEFINE x AS TRUE)"#,
            r#"Pattern variable x cannot be used as a value table"#,
        ),
        // measures_mixed_ranges_in_one_agg
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(x.price - price) AS p PATTERN (x y) DEFINE x AS TRUE, y AS TRUE)"#,
            r#"Column access ranges over all input rows in an expression that already ranges over a pattern variable x"#,
        ),
        // measures_two_vars_in_one_agg
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(x.price - y.price) AS p PATTERN (x y) DEFINE x AS TRUE, y AS TRUE)"#,
            r#"Column access ranges over pattern variable y in an expression that already ranges over a pattern variable x"#,
        ),
        // measures_window_function
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES SUM(price) OVER () AS w PATTERN (x) DEFINE x AS TRUE)"#,
            r#"Analytic function not allowed in MATCH_RECOGNIZE MEASURES clause"#,
        ),
        // measures_refer_previous_measure
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n, n + 1 AS m PATTERN (x) DEFINE x AS TRUE)"#,
            r#"Unrecognized name: n"#,
        ),
        // measures_grouping
        (
            r#"FROM UNNEST([STRUCT('a' AS sym, 1 AS id, 5 AS price), STRUCT('b', 2, 50), STRUCT('a', 3, 4), STRUCT('b', 4, 40), STRUCT('a', 5, 6), STRUCT('b', 6, 45), STRUCT('a', 7, 3)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY sym ORDER BY id MEASURES GROUPING(sym) AS g PATTERN (x) DEFINE x AS TRUE)"#,
            r#"GROUPING must have an argument that exists within the group-by expression list."#,
        ),
        // measures_order_by_column_unaggregated
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES id AS i PATTERN (x) DEFINE x AS TRUE)"#,
            r#"MATCH_RECOGNIZE MEASURES clause expression references column id which is neither grouped nor aggregated"#,
        ),
        // define_var_shadows_range_variable
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> AS x |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (x y) DEFINE x AS TRUE, y AS x.price > 0)"#,
            r#"Cannot access columns through pattern variables."#,
        ),
        // measures_var_shadows_column
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 2 AS a)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(a) AS m PATTERN (a) DEFINE a AS TRUE)"#,
            r#"Pattern variable a cannot be used as a value table"#,
        ),
        // match_number_outside_mr
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> SELECT MATCH_NUMBER() AS m"#,
            r#"function MATCH_NUMBER can only be used inside a MATCH_RECOGNIZE's MEASURES clause"#,
        ),
        // options_unknown
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (d) DEFINE d AS f = 0 OPTIONS (foo = TRUE))"#,
            r#"Unrecognized MATCH_RECOGNIZE option `foo`"#,
        ),
        // options_non_bool
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (d) DEFINE d AS f = 0 OPTIONS (use_longest_match = 1))"#,
            r#"MATCH_RECOGNIZE option `use_longest_match` requires a boolean value, but found INT64"#,
        ),
        // options_non_literal
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (d) DEFINE d AS f = 0 OPTIONS (use_longest_match = NOT FALSE))"#,
            r#"MATCH_RECOGNIZE option `use_longest_match` requires a boolean literal or parameter"#,
        ),
        // options_null
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (d) DEFINE d AS f = 0 OPTIONS (use_longest_match = CAST(NULL AS BOOL)))"#,
            r#"MATCH_RECOGNIZE option `use_longest_match` cannot be NULL"#,
        ),
        // options_duplicate
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 1 AS f), STRUCT(2, 1), STRUCT(3, 0), STRUCT(4, 1), STRUCT(5, 1), STRUCT(6, 1), STRUCT(7, 0)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (d) DEFINE d AS f = 0 OPTIONS (use_longest_match = TRUE, use_longest_match = FALSE))"#,
            r#"MATCH_RECOGNIZE option `use_longest_match` cannot be specified multiple times"#,
        ),
    ];
    let mut failures = Vec::new();
    for (query, message) in cases {
        let found = errors(query);
        if found.first().map(|(m, _)| m.as_str()) != Some(message) {
            failures.push(format!("{query}: wants {message:?}, has {found:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn the_output_is_the_partition_columns_and_the_measures() {
    // partition_by_expression_name, partition_by_duplicate_names and measures_types: GoogleSQL's
    // names and types, where an expression of `PARTITION BY` has no name.
    let cases = [
        (
            r#"FROM UNNEST([STRUCT('a' AS sym, 1 AS id, 5 AS price), STRUCT('b', 2, 50), STRUCT('a', 3, 4), STRUCT('b', 4, 40), STRUCT('a', 5, 6), STRUCT('b', 6, 45), STRUCT('a', 7, 3)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY UPPER(sym) ORDER BY id MEASURES COUNT(*) AS n PATTERN (a+) DEFINE a AS TRUE)"#,
            vec!["$col_1 STRING", "n INT64"],
        ),
        (
            r#"FROM UNNEST([STRUCT('a' AS sym, 1 AS id, 5 AS price), STRUCT('b', 2, 50), STRUCT('a', 3, 4), STRUCT('b', 4, 40), STRUCT('a', 5, 6), STRUCT('b', 6, 45), STRUCT('a', 7, 3)]) |> SELECT * |> MATCH_RECOGNIZE(PARTITION BY sym ORDER BY id MEASURES MAX(price) AS sym PATTERN (a+) DEFINE a AS TRUE)"#,
            vec!["sym STRING", "sym INT64"],
        ),
        (
            r#"FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9), STRUCT(5, 12), STRUCT(6, 11), STRUCT(7, 7)]) |> SELECT * |> MATCH_RECOGNIZE(ORDER BY id MEASURES AVG(price) AS avg_p, SUM(price) AS sum_p, MIN(CAST(price AS FLOAT64)) AS min_f, FIRST(CAST(id AS STRING)) AS first_s, MATCH_NUMBER() AS mn, MAX(MATCH_ROW_NUMBER()) AS mrn, ANY_VALUE(CLASSIFIER()) AS cls, ARRAY_AGG(STRUCT(id, price)) AS rws PATTERN (x x) DEFINE x AS TRUE)"#,
            vec![
                "avg_p FLOAT64",
                "sum_p INT64",
                "min_f FLOAT64",
                "first_s STRING",
                "mn INT64",
                "mrn INT64",
                "cls STRING",
                "rws ARRAY<STRUCT<id INT64, price INT64>>",
            ],
        ),
    ];
    for (query, want) in cases {
        let analysis = pass::analysis(query);
        let plan = (analysis.plan).unwrap_or_else(|| panic!("{query}: {:?}", analysis.diagnostics));
        let found: Vec<String> = (plan.schema().fields().iter())
            .map(|field| {
                let ty = avenger_sql::types::googlesql_name(field.data_type());
                format!("{} {ty}", field.name())
            })
            .collect();
        assert_eq!(found, want, "{query}");
    }
}

#[test]
fn a_pattern_over_the_cap_and_a_parameter_bound_are_errors() {
    // avenger-sql's own errors: GoogleSQL has no cap on a pattern, and reads a parameter bound
    // when the query runs.
    let data = "FROM UNNEST([STRUCT(1 AS id)]) |> SELECT *";
    let cases = [
        (
            format!("{data} |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a{{50000}} a{{50000}}) DEFINE a AS TRUE)"),
            "The PATTERN of MATCH_RECOGNIZE compiles to more than 100000 instructions; write fewer or smaller bounded quantifiers",
            "PATTERN (a{50000} a{50000})",
        ),
        (
            format!("{data} |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a{{@n}}) DEFINE a AS TRUE)"),
            "avenger-sql takes INT64 literals as the bounds of a quantifier, not query parameters",
            "@n",
        ),
    ];
    for (query, message, at) in cases {
        assert_eq!(
            errors(&query).first(),
            Some(&(message.to_string(), at.to_string())),
            "{query}"
        );
    }
}

#[tokio::test]
async fn repeated_names_repeated_partitions_and_sql_aggregates() {
    // The rows of `execute_query` for an input with two columns of one name, a repeated
    // partition column, an aggregate function of SQL in a measure, and an input column with the
    // name of a column of the node.
    let data = "FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9)]) |> SELECT *";
    let cases = [
        (
            format!("{data} |> AS l |> JOIN ({data}) AS r ON l.id = r.id |> MATCH_RECOGNIZE(ORDER BY l.id MEASURES COUNT(*) AS n PATTERN (a+) DEFINE a AS TRUE)"),
            r#"[{"n":4}]"#,
        ),
        (
            format!("{data} |> MATCH_RECOGNIZE(PARTITION BY id, id ORDER BY price MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE) |> ORDER BY 1"),
            r#"[{"id":1,"id":1,"n":1},{"id":2,"id":2,"n":1},{"id":3,"id":3,"n":1},{"id":4,"id":4,"n":1}]"#,
        ),
        (
            format!("CREATE TEMP AGGREGATE FUNCTION ag(x INT64) AS (MAX(x)); {data} |> MATCH_RECOGNIZE(ORDER BY id MEASURES ag(b.price) AS m, ag(price) AS n PATTERN (a b) DEFINE a AS TRUE, b AS TRUE) |> ORDER BY m"),
            r#"[{"m":8,"n":10},{"m":9,"n":9}]"#,
        ),
        (
            format!("{data} |> SELECT id, price AS `$match_number` |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS n PATTERN (a+) DEFINE a AS TRUE)"),
            r#"[{"n":4}]"#,
        ),
    ];
    for (query, rows) in cases {
        assert_eq!(json(&query).await, rows, "{query}");
    }
}

#[test]
fn errors_of_names_function_bodies_and_correlation() {
    // GoogleSQL's messages, from `execute_query`, but for the body of a function, whose error
    // avenger-sql writes after the function's name, and for a correlated clause, which
    // avenger-sql doesn't plan.
    let data = "FROM UNNEST([STRUCT(1 AS id, 10 AS price), STRUCT(2, 8), STRUCT(3, 6), STRUCT(4, 9)]) |> SELECT *";
    let cases = [
        (
            format!("{data} |> AS l |> JOIN ({data}) AS r ON l.id = r.id |> MATCH_RECOGNIZE(ORDER BY l.id MEASURES MAX(a.price) AS top PATTERN (a+) DEFINE a AS TRUE)"),
            "Name price is ambiguous inside a",
        ),
        (
            format!("{data} |> MATCH_RECOGNIZE(ORDER BY id MEASURES MAX(b.price.x) AS m PATTERN (a b) DEFINE a AS TRUE, b AS TRUE)"),
            "Cannot access field x on a value with type INT64",
        ),
        (
            format!("{data} |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS m PATTERN (a) DEFINE a AS a > 8)"),
            "Cannot access columns through pattern variables.",
        ),
        (
            format!("CREATE TEMP FUNCTION f(x ANY TYPE) AS (x > PREV(x)); {data} |> MATCH_RECOGNIZE(ORDER BY id MEASURES COUNT(*) AS m PATTERN (a) DEFINE a AS f(price))"),
            "In f: function PREV must be directly inside a MATCH_RECOGNIZE's DEFINE clause, without any enclosing subquery",
        ),
        (
            format!("{data} |> AS t |> WHERE EXISTS (FROM UNNEST([1, 2, 3]) AS x |> SELECT x |> MATCH_RECOGNIZE(ORDER BY x MEASURES COUNT(*) AS c PATTERN (a) DEFINE a AS x = t.id))"),
            "avenger-sql doesn't plan a MATCH_RECOGNIZE that reads a column of an enclosing query",
        ),
    ];
    for (query, message) in cases {
        let found = errors(&query);
        assert_eq!(found.len(), 1, "{query}: {found:?}");
        assert_eq!(found[0].0, message, "{query}");
    }
}
