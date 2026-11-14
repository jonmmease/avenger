//! The split of a query over `events` for a streaming runtime: the fronts that it accepts, and why
//! it rejects the rest, whose caller runs the batch plan.

use std::sync::Arc;

use avenger_sql::{
    analysis::{analyze, Options},
    catalog::MemoryCatalog,
    library::Library,
    streaming::{split, Front, Split},
};
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema, TimeUnit},
    datasource::MemTable,
};

fn catalog() -> (MemoryCatalog, Arc<dyn datafusion::catalog::TableProvider>) {
    let events = Arc::new(Schema::new(vec![
        Field::new("seq", DataType::Int64, false),
        Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("type", DataType::Utf8, false),
        Field::new("x", DataType::Float64, true),
        Field::new("target", DataType::Utf8, true),
    ]));
    let other = Arc::new(Schema::new(vec![Field::new(
        "target",
        DataType::Utf8,
        true,
    )]));
    let events: Arc<dyn datafusion::catalog::TableProvider> =
        Arc::new(MemTable::try_new(events, vec![vec![]]).unwrap());
    let catalog = MemoryCatalog::new()
        .with_table(&["events"], events.clone())
        .with_table(
            &["targets"],
            Arc::new(MemTable::try_new(other, vec![vec![]]).unwrap()),
        );
    (catalog, events)
}

fn split_of(query: &str) -> Result<Split, String> {
    let parse = avenger_sql_syntax::parse(query);
    assert!(parse.ok(), "{query}: {:?}", parse.diagnostics());
    let (catalog, events) = catalog();
    let analysis = analyze(
        &parse.syntax(),
        &catalog,
        &Library::new(),
        &Options::default(),
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{query}: {:?}",
        analysis.diagnostics
    );
    split(&analysis.plan.unwrap(), &events).map_err(|rejection| rejection.message)
}

#[test]
fn the_fronts_split() {
    let cases = [
        ("FROM events |> AGGREGATE COUNT(*) AS n", "aggregate of 0 groups and 1 aggregates"),
        (
            "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS n, MAX(x) AS hi GROUP BY target |> WHERE n >= 3 |> AGGREGATE COUNT(*) AS hot",
            "aggregate of 1 groups and 2 aggregates",
        ),
        (
            "FROM events |> EXTEND x * 2 AS double |> AGGREGATE ARRAY_AGG(double ORDER BY seq LIMIT 3) AS last GROUP BY type",
            "aggregate of 1 groups and 1 aggregates",
        ),
        ("FROM events |> ORDER BY seq DESC |> LIMIT 1", "limit of 1 after 0, by 1 keys"),
        (
            "FROM events |> WHERE x IS NOT NULL |> ORDER BY x DESC, seq |> LIMIT 3 OFFSET 2 |> SELECT seq, x",
            "limit of 3 after 2, by 2 keys",
        ),
        (
            "FROM events |> AS e |> SELECT e.seq AS id, e.x |> ORDER BY x, id |> LIMIT 1",
            "limit of 1 after 0, by 2 keys",
        ),
        (
            "FROM events AS e |> WHERE e.type = 'click' |> AGGREGATE COUNT(*) AS n",
            "aggregate of 0 groups and 1 aggregates",
        ),
        // MATCH_RECOGNIZE, with the reach of PREV and NEXT.
        (
            "FROM events |> WHERE type = 'move' |> MATCH_RECOGNIZE(PARTITION BY target ORDER BY seq MEASURES COUNT(*) AS n PATTERN (a b+) DEFINE a AS TRUE, b AS x > PREV(x, 2) AND x < NEXT(x)) |> AGGREGATE MAX(n) AS longest",
            "match of 2 variables in 1 partitions, 2 rows back and 1 ahead",
        ),
        // A table function downstream of the front.
        (
            "FROM events |> AGGREGATE COUNT(*) AS n GROUP BY type, target |> CALL impute(type, [target], n, value => 0)",
            "aggregate of 2 groups and 1 aggregates",
        ),
        // Several reads of events, each with a front, which the downstream joins or unions.
        (
            "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS clicks GROUP BY target |> JOIN (FROM events |> WHERE type = 'move' |> AGGREGATE COUNT(*) AS moves GROUP BY target) AS m USING (target)",
            "aggregate of 1 groups and 1 aggregates; aggregate of 1 groups and 1 aggregates",
        ),
        (
            "FROM events |> ORDER BY seq DESC |> LIMIT 1 |> SELECT seq |> UNION ALL (FROM events |> AGGREGATE MAX(seq) AS seq)",
            "limit of 1 after 0, by 1 keys; aggregate of 0 groups and 1 aggregates",
        ),
        (
            "FROM events |> MATCH_RECOGNIZE(ORDER BY seq MEASURES COUNT(*) AS n PATTERN (a b+) DEFINE a AS type = 'down', b AS type = 'move') |> UNION ALL (FROM events |> MATCH_RECOGNIZE(ORDER BY seq MEASURES COUNT(*) AS n PATTERN (e) DEFINE e AS type = 'escape'))",
            "match of 2 variables in 0 partitions, 0 rows back and 0 ahead; match of 1 variables in 0 partitions, 0 rows back and 0 ahead",
        ),
        // Equal reads, each with a front of its own.
        (
            "FROM events |> AGGREGATE COUNT(*) AS n |> CROSS JOIN (FROM events |> AGGREGATE COUNT(*) AS n) AS again",
            "aggregate of 0 groups and 1 aggregates; aggregate of 0 groups and 1 aggregates",
        ),
    ];
    for (query, want) in cases {
        let found = match split_of(query) {
            Ok(split) => (split.fronts.iter())
                .map(|front| match &front.front {
                    Front::Aggregate { groups, aggregates } => format!(
                        "aggregate of {} groups and {} aggregates",
                        groups.len(),
                        aggregates.len()
                    ),
                    Front::Limit { order, skip, fetch } => {
                        format!("limit of {fetch} after {skip}, by {} keys", order.len())
                    }
                    Front::MatchRecognize(front) => format!(
                        "match of {} variables in {} partitions, {} rows back and {} ahead",
                        front.call.variables.len(),
                        front.partition.len(),
                        front.back,
                        front.ahead
                    ),
                })
                .collect::<Vec<_>>()
                .join("; "),
            Err(message) => message,
        };
        assert_eq!(found, want, "{query}");
    }
}

#[test]
fn the_other_shapes_are_rejected() {
    let cases = [
        ("FROM targets |> AGGREGATE COUNT(*) AS n", "the query doesn't read events"),
        // The review's cases: a descending order, subqueries, and the clock.
        (
            "FROM events |> MATCH_RECOGNIZE(ORDER BY seq DESC MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)",
            "a MATCH_RECOGNIZE over events whose ORDER BY isn't seq ascending, the order in which events arrive",
        ),
        (
            "FROM events |> WHERE type = 'click' |> AGGREGATE COUNT(*) AS n |> SELECT n, (FROM events |> AGGREGATE COUNT(*) AS c) AS total",
            "a subquery of an expression reads events",
        ),
        (
            "FROM events |> MATCH_RECOGNIZE(ORDER BY seq MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS x > (FROM targets |> AGGREGATE COUNT(*) AS c))",
            "a subquery in the front over events, which the runtime doesn't evaluate",
        ),
        (
            "FROM events |> AGGREGATE COUNTIF(target IN (FROM targets |> SELECT target)) AS known",
            "a subquery in the front over events, which the runtime doesn't evaluate",
        ),
        (
            "FROM events |> AGGREGATE COUNTIF(ts <= CURRENT_TIMESTAMP()) AS past",
            "the query reads the function now, whose value changes as time passes",
        ),
        (
            "FROM events |> AGGREGATE MAX(ts) AS last |> SELECT TIMESTAMP_DIFF(CURRENT_TIMESTAMP(), last, SECOND) AS idle",
            "the query reads the function now, whose value changes as time passes",
        ),
        (
            "FROM events |> JOIN events AS other USING (seq) |> AGGREGATE COUNT(*) AS n",
            "a join over events",
        ),
        // One read of several without a front of its own.
        (
            "FROM events |> AGGREGATE COUNT(*) AS n |> CROSS JOIN (FROM events |> WHERE type = 'click' |> SELECT seq)",
            "a join over events",
        ),
        (
            "FROM events |> JOIN targets USING (target) |> AGGREGATE COUNT(*) AS n",
            "a join over events",
        ),
        (
            "FROM events |> WHERE type = 'click'",
            "the query has no front over events, so its result grows with every event",
        ),
        (
            "FROM events |> ORDER BY seq DESC",
            "an ORDER BY over events without a LIMIT, whose result grows with every event",
        ),
        (
            "FROM events |> ORDER BY x DESC |> LIMIT 1",
            "an ORDER BY ... LIMIT over events whose last key isn't seq, so ties have no order of their own",
        ),
        (
            "FROM events |> ORDER BY seq DESC |> LIMIT @n",
            "a LIMIT or OFFSET over events that isn't a literal",
        ),
        (
            "FROM events |> AGGREGATE COUNT(*) AS n GROUP BY ROLLUP (type, target)",
            "an AGGREGATE over events with ROLLUP, CUBE or GROUPING SETS",
        ),
        (
            "FROM events |> EXTEND ROW_NUMBER() OVER (ORDER BY seq) AS n |> AGGREGATE MAX(n) AS m",
            "a window over events, which reads every row, as a window function or HAVING MAX does",
        ),
        (
            "FROM events |> AGGREGATE ANY_VALUE(target HAVING MAX x) AS rightmost",
            "a window over events, which reads every row, as a window function or HAVING MAX does",
        ),
        (
            "FROM events |> WHERE ts > TIMESTAMP_SUB(CURRENT_TIMESTAMP(), INTERVAL 1 MINUTE) |> AGGREGATE COUNT(*) AS n",
            "a stage over events reads the function now, whose value a stream reads at another time than the batch plan does",
        ),
        (
            "FROM events |> WHERE target IN (FROM targets |> SELECT target) |> AGGREGATE COUNT(*) AS n",
            "a stage over events reads a subquery, whose value a stream reads at another time than the batch plan does",
        ),
        (
            "FROM events |> MATCH_RECOGNIZE(ORDER BY ts MEASURES COUNT(*) AS n PATTERN (a+) DEFINE a AS TRUE)",
            "a MATCH_RECOGNIZE over events whose ORDER BY isn't seq ascending, the order in which events arrive",
        ),
    ];
    for (query, want) in cases {
        let found = match split_of(query) {
            Ok(_) => "split".to_string(),
            Err(message) => message,
        };
        assert_eq!(found, want, "{query}");
    }
}

#[test]
fn a_downstream_of_the_latest_matches_keeps_the_first_final_rows() {
    let brush = "FROM events |> WHERE type IN ('down', 'move', 'up') |> MATCH_RECOGNIZE(ORDER BY seq MEASURES FIRST(dn.x) AS x0, LAST(mv.x) AS x1, COUNT(mv.x) AS moves, MAX(seq) AS last_seq PATTERN (dn mv* up?) DEFINE dn AS type = 'down', mv AS type = 'move', up AS type = 'up')";
    let cases = [
        // Row-wise stages, then the latest match, then more of the query.
        (
            format!("{brush} |> WHERE moves > 0 |> ORDER BY last_seq DESC |> LIMIT 1 |> SELECT LEAST(x0, x1) AS lo"),
            Some(0),
        ),
        (format!("{brush} |> ORDER BY last_seq DESC |> LIMIT 2 OFFSET 1"), Some(1)),
        // An aggregate of every match, and a front whose output changes its rows.
        (format!("{brush} |> AGGREGATE COUNT(*) AS drags"), None),
        (
            "FROM events |> AGGREGATE COUNT(*) AS n GROUP BY type |> ORDER BY n DESC |> LIMIT 1".to_string(),
            None,
        ),
        // The latest matches of a query of several fronts.
        (
            format!("{brush} |> ORDER BY last_seq DESC |> LIMIT 1 |> UNION ALL ({brush} |> WHERE moves = 0 |> ORDER BY last_seq DESC |> LIMIT 1)"),
            None,
        ),
    ];
    for (query, skip) in cases {
        let split = split_of(&query).unwrap();
        assert_eq!(split.latest.map(|latest| latest.skip), skip, "{query}");
    }
}
