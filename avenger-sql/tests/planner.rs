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
