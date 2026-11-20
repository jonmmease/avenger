//! The formatter: snapshots of the style's rules, and its properties over the corpora of the
//! tests. Each formatted text parses as its input does, holds the same tokens apart from case and
//! the commas after the last items of lists, holds the same comments, and formats to itself.
//!
//! A snapshot that differs goes to `tests/output/`, and copying it to `tests/baselines/`
//! accepts it.

mod common;

use std::path::PathBuf;

use avenger_sql_syntax::{format::format, parse, SyntaxElement, SyntaxKind};

/// The tokens of `text` that are not whitespace or comments, in lower case, without the commas
/// after the last item of a list, and the texts of its comments.
fn tokens(text: &str) -> (Vec<String>, Vec<String>) {
    let root = parse(text).syntax();
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

/// Checks the properties of the formatter on `text`.
fn check(text: &str) {
    let formatted = format(text);
    assert_eq!(
        format(&formatted),
        formatted,
        "formatting again changes the text of\n{text}\nformatted as\n{formatted}"
    );
    assert_eq!(
        tokens(&formatted),
        tokens(text),
        "the tokens differ in\n{text}\nformatted as\n{formatted}"
    );
    let before = parse(text).diagnostics().len();
    let after = parse(&formatted).diagnostics().len();
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
        check(text);
    }
    for (_, text) in SNAPSHOTS {
        check(text);
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
                    check(&input);
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

const SNAPSHOTS: &[(&str, &str)] = &[
    (
        "format-comments",
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
        "FROM t |> WHERE species = 'Gentoo' AND body_mass_g IS NOT NULL AND flipper_length_mm > 200 AND bill_length_mm < 50
|> EXTEND '$mu = ' || CAST(CAST(ROUND(ANY_VALUE(mu)) AS INT64) AS STRING) || ', quad sigma = ' || CAST(sigma AS STRING) || '$' AS label
|> EXTEND CASE WHEN x > 0 THEN 'positive' WHEN x < 0 THEN 'negative' ELSE 'zero' END AS sign, -x AS negated, NOT flag AS unflagged
|> EXTEND CASE WHEN body_mass_g > 5000 THEN 'heavy' WHEN body_mass_g > 4000 THEN 'medium' ELSE 'light' END AS weight_class
|> EXTEND row_number() OVER (PARTITION BY species ORDER BY body_mass_g DESC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS place
|> WHERE x IN (1, 2, 3) AND y BETWEEN 1 AND 2 AND z LIKE 'a%' AND d > DATE '2020-01-01' AND arr[OFFSET(0)] > 1 AND a >> 2 > 0",
    ),
    (
        "format-statements",
        "DECLARE lo, hi INT64 DEFAULT 3;
DECLARE xs ARRAY<STRUCT<a INT64, b STRING>>;
SET lo = (FROM t |> AGGREGATE MIN(x));
CREATE TEMP FUNCTION twice(v INT64) RETURNS INT64 AS (v * 2);
CREATE TEMP TABLE FUNCTION evens(k INT64) AS (FROM t |> WHERE MOD(x, k) = 0);
FROM evens(2) AS e |> JOIN (FROM u |> WHERE b > 0) AS u ON e.x = u.x |> CALL impute(x, [g], y, value => 0)
|> UNION ALL (FROM v), (FROM w |> WHERE c);
FROM t |> WHERE (;
FROM t   |>   LIMIT 3",
    ),
];

#[test]
fn snapshots_of_the_style() {
    for (name, text) in SNAPSHOTS {
        common::assert_text_baseline(name, &format(text));
    }
}
