//! The formatter: snapshots of the style's rules, and its properties over the corpora of the
//! tests. Each formatted text parses as its input does, holds the same tokens apart from case and
//! the commas after the last items of lists, holds the same comments, and formats to itself.
//!
//! A snapshot that differs goes to `tests/output/`, and copying it to `tests/baselines/`
//! accepts it.

mod common;

use std::path::PathBuf;

use avenger_sql_syntax::{format::format, Role, SyntaxElement, SyntaxKind};

/// The tokens of `text`, a text of `role`, that are not whitespace or comments, in lower case,
/// without the commas after the last item of a list, and the texts of its comments.
fn tokens(text: &str, role: Role) -> (Vec<String>, Vec<String>) {
    let root = common::parse_as(text, role).syntax();
    let all: Vec<_> = (root.descendants_with_tokens())
        .filter_map(SyntaxElement::into_token)
        .collect();
    let solid: Vec<_> = all.iter().filter(|t| !t.kind().is_trivia()).collect();
    let mut kept = Vec::new();
    for (index, token) in solid.iter().enumerate() {
        let next = solid.get(index + 1).map(|t| t.kind());
        let last_item = matches!(
            next,
            None | Some(
                SyntaxKind::R_PAREN
                    | SyntaxKind::R_BRACK
                    | SyntaxKind::PIPE
                    | SyntaxKind::SEMICOLON
                    | SyntaxKind::GROUP_KW
                    | SyntaxKind::WINDOW_KW
                    | SyntaxKind::ORDER_KW
                    | SyntaxKind::HAVING_KW
                    | SyntaxKind::LIMIT_KW
                    | SyntaxKind::IGNORE_KW
                    | SyntaxKind::RESPECT_KW
                    | SyntaxKind::MEASURES_KW
                    | SyntaxKind::AFTER_KW
                    | SyntaxKind::PATTERN_KW
                    | SyntaxKind::DEFINE_KW
                    | SyntaxKind::OPTIONS_KW
            )
        );
        if token.kind() == SyntaxKind::COMMA && last_item {
            continue;
        }
        kept.push(token.text().to_ascii_lowercase());
    }
    let comments = (all.iter())
        .filter(|t| t.kind() == SyntaxKind::COMMENT)
        .map(|t| t.text().trim_end().to_string())
        .collect();
    (kept, comments)
}

/// Checks the properties of the formatter on `text`, a text of `role`.
fn check(text: &str, role: Role) {
    let formatted = format(text, role);
    assert_eq!(
        format(&formatted, role),
        formatted,
        "formatting again changes the text of\n{text}\nformatted as\n{formatted}"
    );
    assert_eq!(
        tokens(&formatted, role),
        tokens(text, role),
        "the tokens differ in\n{text}\nformatted as\n{formatted}"
    );
    let before = common::parse_as(text, role).diagnostics().len();
    let after = common::parse_as(&formatted, role).diagnostics().len();
    assert_eq!(
        after, before,
        "the formatted text has {after} errors and its input {before}:\n{text}\nformatted as\n{formatted}"
    );
    // A statement with an error keeps its text, spaces at the ends of its lines included.
    assert!(
        before > 0 || !formatted.lines().any(|line| line.ends_with(' ')),
        "a line ends with a space:\n{formatted}"
    );
}

#[test]
fn the_corpora_keep_their_tokens_and_comments_and_format_to_themselves() {
    let mut texts: Vec<&str> = vec![common::OPERATORS, common::STATEMENTS, common::SOURCES];
    texts.extend(common::EXPRESSIONS.iter().copied());
    texts.extend(common::QUERIES.iter().copied());
    texts.extend(common::TRAILING_COMMAS.iter().copied());
    for text in texts {
        check(text, Role::Script);
    }
    check(common::MODULE, Role::Module);
    check(common::DATA, Role::Data);
    for (_, role, text) in SNAPSHOTS {
        check(text, *role);
    }
}

#[test]
fn the_data_corpus_is_formatted_as_a_data_script() {
    // A data script that parses as one, so that no statement stays as written.
    assert!(common::parse_as(common::DATA, Role::Data).ok());
    // As a chart script, a statement of a data script has an error and stays as written.
    let text = "create  external table t options (format = 'CSV', uris = ['t.csv'])";
    assert_eq!(format(text, Role::Script), text);
    assert_eq!(
        format(text, Role::Data),
        "CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = ['t.csv'])"
    );
    // A chart script's own tables of files format as a data script's do.
    for text in [
        "create  temp external table t options (format = 'CSV', uris = ['t.csv'])",
        "load data  overwrite temp table t from files (format = 'CSV', uris = ['t.csv'])",
    ] {
        assert_eq!(
            format(text, Role::Script),
            format(text, Role::Data),
            "{text}"
        );
        assert_ne!(format(text, Role::Script), text, "{text}");
    }
}

/// The inputs of GoogleSQL's parser tests in the subset, when `tests/conformance/extract.py` has
/// written them: an ignored test, since the corpus lives outside the repository.
#[test]
#[ignore]
fn the_googlesql_corpus_keeps_its_tokens_and_formats_to_itself() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scratch/tools/googlesql/parser-corpus.txt");
    let Ok(corpus) = std::fs::read_to_string(&path) else {
        eprintln!("no corpus at {}", path.display());
        return;
    };
    let mut input = String::new();
    for line in corpus.lines().skip(1).chain(["## end"]) {
        match line.starts_with("## ") {
            true => {
                if !input.trim().is_empty() {
                    check(&input, Role::Script);
                }
                input.clear();
            }
            false => {
                input.push_str(line);
                input.push('\n');
            }
        }
    }
}

const SNAPSHOTS: &[(&str, Role, &str)] = &[
    (
        "format-comments",
        Role::Script,
        "-- the penguins
FROM samples.penguins -- every row
|> AGGREGATE mark_bar(
     -- the x column
     x => species,

     -- the aggregation
     y => count(*), -- a count
     -- after the last argument
   ) AS chart
   GROUP BY island;


-- a second statement
FROM t /* inline */ |> WHERE a > 0
-- at the end
",
    ),
    (
        "format-lists",
        Role::Script,
        "from samples.penguins |> aggregate MARK_BAR(x => species, y => COUNT(*), );
FROM samples.penguins |> AGGREGATE mark_symbol(x => x(flipper_length_mm, scale => scale(zero => FALSE)), y => body_mass_g, color => species);
FROM t |> AGGREGATE avg(body_mass_g) AS mu, stddev(body_mass_g) AS sigma, count(*) AS n, min(body_mass_g) AS low GROUP BY species, island;
FROM t |> AGGREGATE count(*) AS n, avg(body_mass_g) AS mean_body_mass_in_grams GROUP BY species, island, sex;
FROM t |> SELECT a, b, |> EXTEND [1, 2, 3] AS xs, STRUCT(1 AS a, 'x' AS b) AS s;
FROM t |> AGGREGATE count(DISTINCT x) AS n, array_agg(x IGNORE NULLS ORDER BY y LIMIT 3) AS xs;
FROM t |> SELECT - -x, -(-x), + -x, a - -b, -1 AS n",
    ),
    (
        "format-expressions",
        Role::Script,
        "FROM t |> WHERE species = 'Gentoo' AND body_mass_g IS NOT NULL AND flipper_length_mm > 200 AND bill_length_mm < 50
|> EXTEND '$mu = ' || CAST(CAST(ROUND(ANY_VALUE(mu)) AS INT64) AS STRING) || ', quad sigma = ' || CAST(sigma AS STRING) || '$' AS label
|> EXTEND CASE WHEN x > 0 THEN 'positive' WHEN x < 0 THEN 'negative' ELSE 'zero' END AS sign, -x AS negated, NOT flag AS unflagged
|> EXTEND CASE WHEN body_mass_g > 5000 THEN 'heavy' WHEN body_mass_g > 4000 THEN 'medium' ELSE 'light' END AS weight_class
|> EXTEND row_number() OVER (PARTITION BY species ORDER BY body_mass_g DESC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS place
|> WHERE x IN (1, 2, 3) AND y BETWEEN 1 AND 2 AND z LIKE 'a%' AND d > DATE '2020-01-01' AND arr[OFFSET(0)] > 1 AND a >> 2 > 0
|> EXTEND first_measurement_in_millimeters * scale_factor_of_the_chart || ' millimeters across' || suffix AS mixed
|> WHERE first_column_with_a_long_name << 2 <> second_column_with_a_long_name_and_more_text_after_it_to_break",
    ),
    (
        "format-statements",
        Role::Script,
        "DECLARE lo, hi INT64 DEFAULT 3;
DECLARE xs ARRAY<STRUCT<a INT64, b STRING>>;
declare brush selection options(resolution='union');
SET lo = (FROM t |> AGGREGATE MIN(x));
CREATE TEMP FUNCTION twice(v INT64) RETURNS INT64 AS (v * 2);
CREATE TEMP TABLE FUNCTION evens(k INT64) AS (FROM t |> WHERE MOD(x, k) = 0);
FROM evens(2) AS e |> JOIN (FROM u |> WHERE b > 0) AS u ON e.x = u.x |> CALL impute(x, [g], y, value => 0)
|> UNION ALL (FROM v), (FROM w |> WHERE c);
FROM t |> WHERE (;
FROM t   |>   LIMIT 3",
    ),
    (
        "format-data",
        Role::Data,
        "create schema if not exists vega_datasets options (description = 'The datasets of the Vega examples');
create external table vega_datasets.cars options (format = 'JSON', uris = ['data/files/vega_datasets/cars.json']);
CREATE EXTERNAL TABLE trips OPTIONS (format = 'PARQUET', uris = ['data/files/trips/*.parquet']);
CREATE TEMP EXTERNAL TABLE IF NOT EXISTS raw_events (id INT64 NOT NULL, kind STRING, observed_at TIMESTAMP, payload STRING)
    WITH PARTITION COLUMNS (year INT64, month STRING)
    OPTIONS (format = 'PARQUET', uris = ['data/files/events/2024/', 'data/files/events/2025/', 'data/files/events/2026/', 'data/files/events/2027/']);
CREATE EXTERNAL TABLE logs -- every log
WITH PARTITION COLUMNS OPTIONS (format = 'JSON', uris = ['logs/*.json'],);

-- Rates, loaded once.
LOAD DATA INTO TEMP TABLE staging.rates (day DATE, rate FLOAT64)
    -- the table's own options
    OPTIONS (description = 'daily rates')
    FROM FILES (format = 'CSV', uris = ['data/files/rates.csv'], skip_leading_rows = 1)
    WITH PARTITION COLUMNS (region STRING);
load data overwrite sales from files (format = 'CSV', uris = ['data/files/sales.csv']);
CREATE VIEW finance.recent OPTIONS (description = 'the orders of the last week') AS (FROM finance.orders |> WHERE day > DATE '2025-01-01');
CREATE TEMP VIEW kinds AS FROM raw_events |> AGGREGATE COUNT(*) GROUP BY kind;
CREATE TABLE events
-- Only the events with a kind.
AS (FROM raw_events |> WHERE kind IS NOT NULL);
CREATE OR REPLACE TABLE totals (n INT64) OPTIONS (description = 'counts') AS (FROM events |> AGGREGATE COUNT(*) AS n);
CREATE EXTERNAL TABLE broken OPTIONS (format = 'CSV', uris = ['a.csv']) WITH PARTITION   COLUMNS;
CREATE TEMP FUNCTION cleaned(s STRING) AS (TRIM(s))",
    ),
];

#[test]
fn snapshots_of_the_style() {
    for (name, role, text) in SNAPSHOTS {
        common::assert_text_baseline(name, &format(text, *role));
    }
}
