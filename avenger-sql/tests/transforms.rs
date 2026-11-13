//! The transforms of avenger-datafusion-transforms through the pass: a chart of `bin`'s bins,
//! `lump` with its scopes, `|> CALL impute(...)` and `FROM impute(...)`, and the errors of each
//! at their text.

mod pass;

use pass::{errors, rows};

/// The rows of `text` and the line of its column names, sorted, as cells separated by ` | `,
/// with `NULL` for an empty cell.
async fn lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = (rows(text).await.into_iter())
        .filter(|line| line.starts_with('|'))
        .map(|line| {
            let cells: Vec<&str> = (line.trim_matches('|').split('|'))
                .map(|cell| match cell.trim() {
                    "" => "NULL",
                    cell => cell,
                })
                .collect();
            cells.join(" | ")
        })
        .collect();
    lines.sort();
    lines
}

#[tokio::test]
async fn a_chart_reads_the_bins_of_a_transform() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> EXTEND bin(body_mass_g) OVER () AS b
|> AGGREGATE COUNT(*) AS n GROUP BY b
|> AGGREGATE bar(x => b, y => n) AS chart";
    let charts = pass::charts(query).await;
    assert_eq!(charts.len(), 1);
    let chart = &charts[0];
    let Some(avenger_chart::proto::node::Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot")
    };
    let layer = avenger_chart::LayerRef::of(plot).remove(0);
    let table = chart.table(layer.table().unwrap());
    assert!(table.num_rows() > 5, "the bins of the body masses");
    let bins = table
        .schema()
        .field_with_name("b")
        .unwrap()
        .data_type()
        .clone();
    assert!(
        matches!(bins, arrow::datatypes::DataType::Struct(_)),
        "{bins}"
    );
}

#[tokio::test]
async fn lump_keeps_the_heaviest_categories_of_each_partition() {
    // Adelie lives on three islands, the most of them on Dream; the others on one each.
    let found = lines(
        "FROM penguins
|> EXTEND lump(island, n => 1) OVER (PARTITION BY species) AS l
|> AGGREGATE COUNT(*) AS c, MAX(l.category_count) AS k GROUP BY species, l.value AS kept, l.other AS other",
    )
    .await;
    assert_eq!(
        found,
        [
            "Adelie | Dream | NULL | 56 | 1",
            "Adelie | NULL | Other | 96 | 2",
            "Chinstrap | Dream | NULL | 68 | 1",
            "Gentoo | Biscoe | NULL | 124 | 1",
            "species | kept | other | c | k",
        ]
    );
}

#[tokio::test]
async fn the_keep_of_lump_reads_aggregates_and_windows_of_the_categories() {
    // Gentoo and Adelie weigh the most; Chinstrap is a fifth of the penguins.
    let want = [
        "Adelie | NULL | 152",
        "Gentoo | NULL | 124",
        "NULL | Rest | 68",
        "kept | other | c",
    ];
    for keep in [
        "weight => SUM(body_mass_g), keep => ROW_NUMBER() OVER ranked <= 2",
        "keep => COUNT(*) >= 0.3 * SUM(COUNT(*)) OVER ()",
        "keep => SUM(COUNT(*)) OVER (ranked ROWS UNBOUNDED PRECEDING) - COUNT(*) < 0.6 * SUM(COUNT(*)) OVER ()",
        "keep => species != 'Chinstrap' AND COUNT(*) > 100",
    ] {
        let text = format!(
            "FROM penguins
|> EXTEND lump(species, {keep}, other => 'Rest') OVER () AS l
|> AGGREGATE COUNT(*) AS c GROUP BY l.value AS kept, l.other AS other"
        );
        assert_eq!(lines(&text).await, want, "{keep}");
    }
}

#[tokio::test]
async fn a_lump_filters_the_rows_in_a_pipe_where() {
    // A pipe WHERE computes its windows over the rows before it, as GoogleSQL's does.
    let found = lines(
        "FROM penguins |> WHERE lump(island, n => 1) OVER ().value IS NOT NULL |> AGGREGATE COUNT(*) AS n",
    )
    .await;
    assert_eq!(found, ["168", "n"]);
}

#[tokio::test]
async fn lumps_beside_other_windows_have_columns_of_their_own() {
    let found = lines(
        "FROM penguins
|> EXTEND lump(island, n => 1) OVER () AS a, lump(island, n => 2) OVER () AS b, COUNT(*) OVER () AS total
|> AGGREGATE COUNT(*) AS c GROUP BY a.value AS a, b.value AS b, total",
    )
    .await;
    assert_eq!(
        found,
        [
            "Biscoe | Biscoe | 344 | 168",
            "NULL | Dream | 344 | 124",
            "NULL | NULL | 344 | 52",
            "a | b | total | c",
        ]
    );
}

#[tokio::test]
async fn the_null_category_of_lump() {
    // `keep` keeps the NULL category as a category of its own, and `drop` gives its rows no lump.
    for (mode, want) in [
        (
            "keep",
            [
                "MALE | NULL | 168",
                "NULL | NULL | 11",
                "NULL | Other | 165",
                "kept | other | c",
            ],
        ),
        (
            "other",
            [
                "MALE | NULL | 168",
                "NULL | Other | 176",
                "kept | other | c",
                "",
            ],
        ),
        (
            "drop",
            [
                "MALE | NULL | 168",
                "NULL | NULL | 11",
                "NULL | Other | 165",
                "kept | other | c",
            ],
        ),
    ] {
        let text = format!(
            "FROM penguins
|> EXTEND lump(sex, n => 1, null_category => '{mode}') OVER () AS l
|> AGGREGATE COUNT(*) AS c GROUP BY l.value AS kept, l.other AS other"
        );
        let want: Vec<&str> = want.into_iter().filter(|line| !line.is_empty()).collect();
        assert_eq!(lines(&text).await, want, "{mode}");
    }
    // Dropped, the NULL category's rows have no lump.
    let found = lines(
        "FROM penguins
|> EXTEND lump(sex, n => 1, null_category => 'drop') OVER () AS l
|> WHERE l IS NULL
|> AGGREGATE COUNT(*) AS c",
    )
    .await;
    assert_eq!(found, ["11", "c"]);
}

#[test]
fn the_errors_of_lump_are_at_their_text() {
    let cases = [
        ("lump(island) OVER ()", "lump takes exactly one of n and keep", "lump(island) OVER ()"),
        ("lump(island, n => 1, keep => TRUE) OVER ()", "lump takes exactly one of n and keep", "lump(island, n => 1, keep => TRUE) OVER ()"),
        ("lump(island, n => 0) OVER ()", "The n of lump is a positive INT64 literal", "0"),
        ("lump(island, top => 1) OVER ()", "lump has no parameter named `top`", "top => 1"),
        ("lump(island, n => 1, n => 2) OVER ()", "The argument n of lump is given twice", "n => 2"),
        ("lump(island, n => 1, null_category => 'rank') OVER ()", "The null_category of lump is 'keep', 'other' or 'drop'; found 'rank'", "null_category => 'rank'"),
        ("lump(island, n => 1, other => 1) OVER ()", "The argument other of lump must be a STRING literal", "1"),
        ("lump(island, n => 1, weight => body_mass_g) OVER ()", "The weight of lump is an aggregate of the rows of each category, such as SUM(tons)", "body_mass_g"),
        ("lump(island, n => 1, weight => SUM(body_mass_g) + body_mass_g) OVER ()", "The weight of lump reads the rows of each category through aggregates, and body_mass_g is not in one", "SUM(body_mass_g) + body_mass_g"),
        ("lump(island, n => 1, weight => MAX(species)) OVER ()", "The weight of lump is a number; found STRING", "MAX(species)"),
        ("lump(island, n => 1, weight => COUNT(*) OVER ()) OVER ()", "Analytic function COUNT not allowed in the weight of lump", "COUNT(*) OVER ()"),
        ("lump(island, keep => COUNT(*)) OVER ()", "The keep of lump is a condition, a BOOL; found INT64", "COUNT(*)"),
        ("lump(island, keep => body_mass_g > 5000) OVER ()", "The keep of lump reads the rows of each category through aggregates, such as SUM(tons) > 10, and body_mass_g is not in one", "body_mass_g > 5000"),
        ("lump(island, keep => MAX(SUM(body_mass_g)) > 1) OVER ()", "Aggregations of aggregations are not allowed", "SUM(body_mass_g)"),
        ("lump(island, keep => COUNT(*) >= 0.2 * SUM(COUNT(*)) OVER (PARTITION BY species)) OVER ()", "A window in the keep of lump runs over the categories of the lump's partition, so it takes no PARTITION BY", "OVER (PARTITION BY species)"),
        ("lump(island, keep => RANK() OVER (ranked ORDER BY island) <= 2) OVER ()", "ORDER BY cannot be specified when the named window ranked has one", "(ranked ORDER BY island)"),
        ("lump(island, keep => RANK() OVER w <= 2) OVER ()", "Unrecognized window alias w; the keep of lump has the window ranked", "w"),
        ("lump(DISTINCT island, n => 1) OVER ()", "lump takes its category and named arguments, and no other clause", "lump(DISTINCT island, n => 1) OVER ()"),
        ("lump(island, n => 1) OVER (ORDER BY island)", "Window ORDER BY is not allowed for analytic function lump", "OVER (ORDER BY island)"),
        ("lump(island, n => 1)", "Analytic function lump cannot be called without an OVER clause", "lump(island, n => 1)"),
    ];
    let mut failures = Vec::new();
    for (call, message, at) in cases {
        let text = format!("FROM penguins |> EXTEND {call} AS l");
        let found = errors(&text);
        let want = vec![(message.to_string(), at.to_string())];
        if found != want {
            failures.push(format!("{call}:\n  found {found:?}\n  want  {want:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The counts of the penguins of each species and island.
const COUNTS: &str = "FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY species, island";

#[tokio::test]
async fn a_pipe_call_of_impute_adds_the_missing_rows() {
    // The penguins have 5 of the 9 pairs of a species and an island, and `impute` adds the rest.
    let found = lines(&format!(
        "{COUNTS} |> CALL impute(island, [species], n, value => 0, flag => 'imputed') |> AGGREGATE COUNT(*) AS c, SUM(n) AS s GROUP BY imputed"
    ))
    .await;
    assert_eq!(
        found,
        ["false | 5 | 344", "imputed | c | s", "true | 4 | 0"]
    );
    // Each partition has its own keys and groups.
    let found = lines(
        "FROM penguins |> WHERE sex IS NOT NULL
|> AGGREGATE COUNT(*) AS n GROUP BY sex, species, island
|> CALL impute(island, [species], n, value => 0, partition_by => [sex]) AS i
|> AGGREGATE COUNT(*) AS c GROUP BY i.sex",
    )
    .await;
    assert_eq!(found, ["FEMALE | 9", "MALE | 9", "sex | c"]);
}

#[tokio::test]
async fn impute_in_from_takes_a_table_or_a_query() {
    let script = format!(
        "CREATE TEMP TABLE counts AS ({COUNTS});
FROM impute(TABLE counts, island, [species], n, method => 'max') AS i
|> WHERE i.species = 'Chinstrap'
|> SELECT i.island, i.n
|> ORDER BY island"
    );
    let found = pass::table_text(&script).await;
    let found: Vec<&str> = found.lines().filter(|line| line.starts_with('|')).collect();
    assert_eq!(
        found,
        [
            "| island    | n  |",
            "| Biscoe    | 68 |",
            "| Dream     | 68 |",
            "| Torgersen | 68 |",
        ]
    );
    let found = lines(&format!(
        "FROM impute(({COUNTS}), island, [species], n, value => 0) |> AGGREGATE COUNT(*) AS c"
    ))
    .await;
    assert_eq!(found, ["9", "c"]);
}

#[test]
fn the_errors_of_impute_are_at_their_text() {
    let cases = [
        ("FROM penguins |> CALL impute(island, [species], n, value => 0)", "Unrecognized name: n", "n"),
        ("FROM t |> CALL impute(x + 1, [], s, value => 'z')", "The key of impute is a column of its input", "x + 1"),
        ("FROM t |> CALL impute(x, s, s, value => 'z')", "The group columns of impute are an array of columns, such as [region]", "s"),
        ("FROM t |> CALL impute(x, [], s)", "Impute needs exactly one of `value` and `method`; found neither", "|> CALL impute(x, [], s)"),
        ("FROM t |> CALL impute(s, [], x, value => 'z')", "`value` 'z' does not coerce to the type of x, Int64", "value => 'z'"),
        ("FROM t |> CALL impute(x, [x], s, value => 'z')", "The key x of impute is also a group column", "x"),
        ("FROM t |> CALL impute(x, [], s, value => 'z', nope => 1)", "Unknown argument `nope` of impute: the named arguments are value, method, frame, keys, extra_keys, fill_gaps, flag, partition_by", "nope => 1"),
        ("FROM t |> CALL impute(x, [], s, value => 'z', flag => 1)", "The argument flag of impute must be a STRING literal", "1"),
        ("FROM t |> CALL impute(x, [], s, value => 'z', flag => 'x')", "The flag column `x` is already a column of the input", "flag => 'x'"),
        ("FROM t |> CALL impute(s, [], x, method => 'avg', frame => [1.5, 2])", "A bound of `frame` must be an integer or NULL", "1.5"),
        ("FROM t |> CALL impute(x, [])", "impute takes 3 positional arguments, the key, the group columns and the column to impute; found 2", "|> CALL impute(x, [])"),
        ("FROM t |> CALL nope(1)", "Table-valued function not found: nope", "nope"),
        ("FROM impute(1, x, [], s, value => 'z')", "The first argument of impute in FROM is its input, as TABLE name or a query in parentheses", "1"),
        ("FROM impute(TABLE nowhere, x, [], s, value => 'z')", "Table not found: nowhere", "TABLE nowhere"),
    ];
    let mut failures = Vec::new();
    for (text, message, at) in cases {
        let found = errors(text);
        let want = vec![(message.to_string(), at.to_string())];
        if found != want {
            failures.push(format!("{text}:\n  found {found:?}\n  want  {want:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
