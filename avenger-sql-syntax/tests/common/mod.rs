//! Helpers and the inputs that the tests share.

#![allow(dead_code)]

use std::path::PathBuf;

use avenger_sql_syntax::{parse, parse_module, Diagnostic, Parse, SyntaxKind, SyntaxNode};

/// Parses `text` as a script, and checks that it has no diagnostics and that its tree holds the
/// text.
pub fn accepts(text: &str) -> SyntaxNode {
    checked(text, parse(text))
}

/// Parses `text` as a module, and checks it as [`accepts`] does.
pub fn accepts_module(text: &str) -> SyntaxNode {
    checked(text, parse_module(text))
}

fn checked(text: &str, parse: Parse) -> SyntaxNode {
    let root = parse.syntax();
    assert_eq!(root.text().to_string(), text, "the tree loses text");
    if let Some(diagnostic) = parse.diagnostics().first() {
        panic!("{text}\n{}", describe(text, diagnostic));
    }
    root
}

/// The diagnostic at its 1-based line and column, which counts characters, and its text.
pub fn describe(text: &str, diagnostic: &Diagnostic) -> String {
    let before = &text[..usize::from(diagnostic.range.start())];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let column = before[line_start..].chars().count() + 1;
    format!(
        "{line}:{column}: {diagnostic} at `{}`",
        &text[diagnostic.range]
    )
}

/// The first diagnostic of `parse`: its message and the text it covers.
pub fn first_error(text: &str, parse: Parse) -> (String, String) {
    assert_eq!(
        parse.syntax().text().to_string(),
        text,
        "the tree loses text"
    );
    let diagnostic = parse
        .diagnostics()
        .first()
        .unwrap_or_else(|| panic!("no diagnostic for {text}"));
    (diagnostic.to_string(), text[diagnostic.range].to_string())
}

/// Checks that the first diagnostic of the script `text` holds `message` and covers `at`.
pub fn rejects(text: &str, message: &str, at: &str) {
    let (found, range) = first_error(text, parse(text));
    assert!(found.contains(message), "{text}: {found}");
    assert_eq!(range, at, "{text}: {found}");
}

/// Checks that the first diagnostic of the module `text` holds `message` and covers `at`.
pub fn rejects_module(text: &str, message: &str, at: &str) {
    let (found, range) = first_error(text, parse_module(text));
    assert!(found.contains(message), "{text}: {found}");
    assert_eq!(range, at, "{text}: {found}");
}

/// The nodes of `kind` under `root`, in order.
pub fn nodes(root: &SyntaxNode, kind: SyntaxKind) -> Vec<SyntaxNode> {
    root.descendants()
        .filter(|node| node.kind() == kind)
        .collect()
}

/// The text of the first node of `kind` under `root`, without its trivia at the edges.
pub fn text_of(root: &SyntaxNode, kind: SyntaxKind) -> String {
    nodes(root, kind)
        .first()
        .unwrap_or_else(|| panic!("no {kind:?} in {root:#?}"))
        .text()
        .to_string()
        .trim()
        .to_string()
}

/// The condition of `FROM t |> WHERE <text>`.
pub fn condition(text: &str) -> SyntaxNode {
    let root = accepts(&format!("FROM t |> WHERE {text}"));
    let pipe = nodes(&root, SyntaxKind::PIPE_WHERE).remove(0);
    pipe.children().next().expect("a condition")
}

/// An expression written with parentheses around each operator, to check precedence.
pub fn shape(node: &SyntaxNode) -> String {
    let children: Vec<String> = node.children().map(|child| shape(&child)).collect();
    let tokens: Vec<String> = node
        .children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_ascii_uppercase())
        .collect();
    match node.kind() {
        SyntaxKind::BIN_EXPR => {
            format!("({} {} {})", children[0], tokens.join(""), children[1])
        }
        SyntaxKind::PREFIX_EXPR => format!("({} {})", tokens.join(" "), children[0]),
        SyntaxKind::BETWEEN_EXPR => {
            format!("({} BETWEEN {} {})", children[0], children[1], children[2])
        }
        SyntaxKind::IS_EXPR => format!("({} {})", children[0], tokens.join(" ")),
        SyntaxKind::FIELD_EXPR => format!("{}.{}", children[0], children[1]),
        SyntaxKind::CALL_EXPR => format!("{}(..)", children[0]),
        _ => node.text().to_string().trim().to_string(),
    }
}

/// Compares the tree of `text` with the baseline `name` in `tests/baselines/`. A missing or
/// different baseline fails, and the tree goes to `tests/output/`, from which a copy accepts
/// it.
pub fn assert_baseline(name: &str, root: &SyntaxNode) {
    assert_text_baseline(name, &format!("{root:#?}"));
}

/// Compares `dump` with the baseline `name`, as [`assert_baseline`] does.
pub fn assert_text_baseline(name: &str, dump: &str) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let baseline = manifest.join("tests/baselines").join(format!("{name}.txt"));
    let expected = std::fs::read_to_string(&baseline).unwrap_or_default();
    if expected != dump {
        let output = manifest.join("tests/output");
        std::fs::create_dir_all(&output).unwrap();
        let written = output.join(format!("{name}.txt"));
        std::fs::write(&written, dump).unwrap();
        panic!(
            "{name} differs from {}; the new text is in {}",
            baseline.display(),
            written.display()
        );
    }
}

/// A script that holds every pipe operator.
pub const OPERATORS: &str = "
FROM `my-project`.sales.orders AS o
|> WHERE o.amount > 0
|> EXTEND amount * 2 AS doubled, ROW_NUMBER() OVER w AS n WINDOW w AS (ORDER BY day)
|> SET amount = amount + 1
|> DROP doubled,
|> RENAME n AS position, amount total
|> AS sales
|> SELECT DISTINCT day, total,
|> AGGREGATE SUM(total) AS sum DESC, COUNT(*) GROUP AND ORDER BY day DESC NULLS LAST
|> DISTINCT
|> ORDER BY day DESC NULLS FIRST, sum
|> LIMIT 10 OFFSET 5
|> UNION ALL (FROM other), (FROM third)
|> LEFT JOIN customers USING (id)
|> CROSS JOIN UNNEST(items) AS item WITH OFFSET AS i
|> CALL bin(x, maxbins => 20) AS binned
|> TABLESAMPLE SYSTEM (10 PERCENT)
|> WITH a AS (FROM t1), b AS (FROM t2),
|> PIVOT (SUM(sales) AS total FOR quarter IN ('Q1' AS q1, 'Q2'))
|> UNPIVOT INCLUDE NULLS (sales FOR quarter IN (q1 AS 'Q1', q2))
|> FULL OUTER UNION ALL BY NAME (FROM x)
|> INTERSECT DISTINCT STRICT CORRESPONDING (FROM y)
";

/// A script that holds every statement.
pub const STATEMENTS: &str = "
DECLARE lo, hi INT64 DEFAULT 0;
DECLARE label DEFAULT 'x';
IMPORT MODULE charts.scales AS s OPTIONS (a = 1);
SET hi = (FROM t |> AGGREGATE MAX(v));
SET (lo, label) = (1, 'y');
CREATE TEMP TABLE big AS (FROM t |> WHERE v > hi);
CREATE TEMPORARY TABLE IF NOT EXISTS shaped (a INT64 NOT NULL, b ARRAY<STRING>)
    OPTIONS (description = 'shaped');
-- Scales a value.
CREATE TEMP FUNCTION scaled(x FLOAT64, k ANY TYPE) RETURNS FLOAT64 AS (x * k)
    OPTIONS (description = 'scales');
CREATE TEMP AGGREGATE FUNCTION mean_over(x FLOAT64, d FLOAT64 NOT AGGREGATE) AS (AVG(x) / d);
CREATE TEMP TABLE FUNCTION recent(input TABLE<day DATE, v INT64>, n INT64)
    RETURNS TABLE<day DATE, v INT64>
    AS FROM input |> ORDER BY day DESC |> LIMIT n;
CREATE TABLE FUNCTION any_rows(input ANY TABLE) AS (FROM input);
FROM recent(TABLE big, 10) |> EXTEND scaled(v, 2) AS w;
";

/// A module that holds every statement of a module.
pub const MODULE: &str = "MODULE charts.scales OPTIONS (description = 'scales');
IMPORT MODULE charts.math AS math;
CREATE PRIVATE CONSTANT base = 10;
CREATE PUBLIC FUNCTION scaled(x FLOAT64, k ANY TYPE) AS (math.round(x * k / base));";

/// A query whose sources hold every form of `FROM`.
pub const SOURCES: &str = "FROM a NATURAL LEFT OUTER JOIN b, c AS c2 WITH OFFSET AS i
  JOIN LATERAL (FROM d |> WHERE d.k = c2.k) AS e ON e.k = a.k
  JOIN f(TABLE a, n => TABLE b, 3)
    TABLESAMPLE RESERVOIR (10 ROWS PARTITION BY k) WITH WEIGHT AS w REPEATABLE (5) USING (k)
  CROSS JOIN UNNEST([1, 2] AS x, [3] AS y, mode => 'PAD') AS z WITH OFFSET
|> AGGREGATE COUNT(*) GROUP BY (), ROLLUP(a, b), CUBE(c), GROUPING SETS((a, b), (), ROLLUP(d)), x NULLS FIRST
|> PIVOT (SUM(v) AS total FOR k IN ('a' AS a, 'b')) AS p
|> UNPIVOT ((v1, v2) FOR k IN ((a, b) AS 'ab', c 1)) AS u";

/// Expressions of every form.
pub const EXPRESSIONS: &[&str] = &[
    "x IN (1, 2, 3)",
    "x NOT IN (FROM t |> SELECT y)",
    "x IN UNNEST(arr)",
    "s LIKE ANY ('a%', 'b%')",
    "s NOT LIKE 'a%'",
    "a IS DISTINCT FROM b",
    "a IS NOT UNKNOWN",
    "EXISTS (FROM u |> WHERE u.k = t.k)",
    "(FROM u |> AGGREGATE MAX(v)) > 3",
    "CASE WHEN a THEN 1 WHEN b THEN 2 ELSE 3 END = 1",
    "CASE x WHEN 1 THEN TRUE END",
    "CAST(x AS NUMERIC(10, 2)) > 0",
    "SAFE_CAST(s AS DATE FORMAT 'YYYY-MM-DD') IS NULL",
    "CAST(s AS ARRAY<STRUCT<a INT64, b ARRAY<STRING>>>) IS NULL",
    "EXTRACT(WEEK(MONDAY) FROM d AT TIME ZONE 'UTC') = 1",
    "d > DATE '2024-01-01' AND ts < TIMESTAMP '2024-01-01 00:00:00+00'",
    "d + INTERVAL 1 DAY > d - INTERVAL '1-2' YEAR TO MONTH",
    "ARRAY<INT64>[1, 2][OFFSET(0)] = arr[SAFE_ORDINAL(1)]",
    "STRUCT<x INT64>(1 AS x).x = (1, 2).a",
    "ARRAY_LENGTH(ARRAY(FROM u |> SELECT v)) > 0",
    "IF(a, b, c) AND LEFT(s, 2) = 'ab'",
    "SAFE.PARSE_DATE('%Y', s) IS NULL",
    "ANY_VALUE(x HAVING MAX y) = 1",
    "STRING_AGG(DISTINCT s, ',' IGNORE NULLS ORDER BY s DESC LIMIT 3) = ''",
    "LAST_VALUE(x IGNORE NULLS) OVER (PARTITION BY k ORDER BY t ROWS BETWEEN UNBOUNDED PRECEDING AND 1 FOLLOWING) > 0",
    "SUM(x) OVER (w RANGE 3 PRECEDING) > 0",
    "x = @param AND y = ? AND z = ?",
    "'a' 'b' \"c\" = r'\\d' AND b'x' = rb'\\x'",
    "RANGE<DATE> '[2024-01-01, UNBOUNDED)' IS NOT NULL",
    "`group`.`order` = t.group",
    "0x1F + 1.5e3 + .5 + 3. > 0",
    "a = (NOT b)",
    "NOT NOT a",
    "a AND NOT b",
    "d + INTERVAL n * 7 DAY > d",
    "RANGE(d1, d2) IS NULL",
    "DECIMAL '1.5' = BIGDECIMAL '2'",
    "CAST(s AS STRING(MAX)) IS NULL",
    "'a' LIKE ANY (FROM u |> SELECT p)",
    "a[OFFSET(1) + 1] = 0",
    "t.select = 1",
    "((FROM u) |> WHERE true |> AGGREGATE COUNT(*)) > 0",
    "((FROM u) + 1) > 0",
    "COUNT(*) OVER (PARTITION BY k ORDER BY b ROWS BETWEEN x + 1 PRECEDING AND CURRENT ROW) > 0",
    "a <> b AND c >> 1 = 0 AND CAST(x AS STRUCT<>) IS NULL",
    "CAST(x AS ARRAY<ARRAY<INT64>>) IS NULL",
    "CURRENT_DATE > d",
    "x.2_3.2abc3 = @p.123",
    "x.1e+5 > 0",
];

/// Queries of the forms that GoogleSQL accepts.
pub const QUERIES: &[&str] = &[
    "FROM t # a comment to the line end",
    "(FROM t) AS q |> WHERE true",
    "(FROM t) |> WHERE x",
    "((FROM t))",
    "FROM ((FROM t) |> WHERE x)",
    "FROM t |> UNION ALL ((FROM u))",
    "FROM ((FROM t) JOIN u USING (k))",
    "FROM (a JOIN b USING (k)) TABLESAMPLE RESERVOIR (10 ROWS) REPEATABLE (5)",
    "FROM t TABLESAMPLE BERNOULLI (1.5 PERCENT) WITH WEIGHT w REPEATABLE (@seed)",
    "FROM t TABLESAMPLE SYSTEM (CAST(@n AS INT64) ROWS PARTITION BY k)",
    "FROM t.arr WITH OFFSET pos",
    "FROM UNNEST([1, 2] AS a, [3] AS b, mode => 'PAD') AS x WITH OFFSET",
    "FROM a NATURAL JOIN b |> NATURAL LEFT JOIN c",
    "FROM a |> JOIN LATERAL (FROM b |> WHERE b.k = a.k) |> LEFT JOIN LATERAL f(a.k) AS g ON true",
    "FROM t |> SELECT ALL a",
    "FROM t |> AGGREGATE",
    "FROM t |> AGGREGATE COUNT(*) GROUP AND ORDER BY ROLLUP(x)",
    "FROM t |> INTERSECT ALL (FROM u) |> EXCEPT ALL (FROM v)",
    "FROM t |> OUTER UNION ALL CORRESPONDING (FROM u)",
    "FROM t |> UNION ALL (FROM u), (FROM v),",
    "FROM t PIVOT (SUM(v) FOR (k) IN (1, 2)) AS p",
    "FROM f(TABLE t, other => TABLE u, 1)",
    "FROM a.b.c.123",
    "FROM t |> SELECT x.2_3.2abc3, @p.123, (x).1e2",
    "FROM t |> SELECT 1.x",
    "FROM\u{0b}t\u{a0}|>\u{3000}WHERE true",
    "FROM t -- note\r|> WHERE true",
    "FROM t AS graph_table",
    "FROM t |> WHERE x = @ name",
    "FROM t |> WHERE x = 'a' /* c */ 'b'",
    "FROM my-project-2.dataset.table",
    "FROM a, b JOIN c USING (k)",
    "",
    "-- nothing",
    "FROM t; IMPORT MODULE m",
    "CREATE TEMP FUNCTION f(x INT64) OPTIONS (description = 'f') AS (x + 1)",
    "CREATE TEMP FUNCTION f(x INT64) AS (x + 1) OPTIONS (hash = 1, proto = 2)",
    "CREATE TEMP TABLE FUNCTION g(n INT64) AS (FROM t) |> LIMIT n",
    "CREATE TEMP TABLE `function` (x INT64)",
    "FROM t |> SELECT DISTINCT AS STRUCT a, b + 1 |> SELECT AS VALUE v",
    "FROM t |> SELECT f(x).*, (s).* EXCEPT (a) |> AGGREGATE ANY_VALUE(s).* REPLACE (1 AS b), COUNT(*) AS n",
    "CREATE TEMP TABLE my-proj.ds.t AS FROM u",
    "DECLARE k DEFAULT 2; IMPORT MODULE charts.scales AS s; IMPORT MODULE geo OPTIONS (a = 1); IMPORT MODULE module.`all`; FROM t |> EXTEND s.scaled(v, k) AS w; IMPORT MODULE late",
    // Near misses of the triggers of GoogleSQL's syntax outside the subset, which parse.
    "FROM t |> SELECT IF(a, 1, 2) AS v, a * b AS p, CASE WHEN a THEN 1 END AS c",
    "FROM t AS `lookup` |> WHERE `lookup`.x > 1 |> LIMIT 10 OFFSET 2",
    "FROM t |> PIVOT (SUM(x) FOR k IN ('a', 'b'))",
    "FROM t |> EXTEND begin + 1 AS x, `table` AS y",
    "FROM t |> WHERE x IN (FROM u |> SELECT x) AND EXISTS (FROM u)",
    "DECLARE x INT64 DEFAULT 1; SET x = 2; CREATE TEMP TABLE u AS (FROM t)",
    "FROM t |> CALL f(TABLE u, 1) AS r",
    // MATCH_RECOGNIZE, as a pipe operator and after a source, with each clause and pattern form.
    "FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a b+) DEFINE a AS TRUE, b AS x < PREV(x)) AS m",
    "FROM t |> MATCH_RECOGNIZE (PARTITION BY k, f(j) ORDER BY x DESC NULLS LAST, y MEASURES MAX(a.x) AS hi, MATCH_NUMBER() AS id AFTER MATCH SKIP TO NEXT ROW PATTERN (^ (a | b)+? c{2,} d{,3}? e{3} || f?? $) DEFINE a AS x > NEXT(x, 2), b AS TRUE, c AS TRUE, d AS TRUE, e AS TRUE, f AS TRUE OPTIONS (use_longest_match = TRUE)) m",
    "FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a{@lo, ?} (() | a)* ((^))*) DEFINE a AS TRUE) |> WHERE n > 0",
    "FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN () DEFINE a AS TRUE)",
    "FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (| a |) DEFINE `select` AS TRUE, a AS FALSE)",
    "FROM t MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE) AS m TABLESAMPLE BERNOULLI (10 PERCENT)",
    "FROM (FROM t) AS s MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE) MATCH_RECOGNIZE (ORDER BY n MEASURES SUM(n) AS total PATTERN (b*) DEFINE b AS TRUE)",
    "FROM t, u MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE) JOIN (v JOIN w USING (k)) MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE) USING (n)",
    "FROM f(1) MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)",
    "FROM t |> SELECT `match_recognize`, measures, pattern, `define` AS skip, past, after",
];

/// Texts with a trailing comma in each kind of list, which avenger-sql takes and GoogleSQL takes
/// only in the lists of the pipe operators and the columns of `CREATE TABLE`.
pub const TRAILING_COMMAS: &[&str] = &[
    "FROM t |> WHERE f(a, b,) = 1 AND COUNT(*,) > 0",
    "FROM t |> WHERE ARRAY_AGG(x ORDER BY a,) IS NULL",
    "FROM t |> WHERE ARRAY_AGG(x, ORDER BY a, LIMIT 1) IS NULL",
    "FROM t |> WHERE x IN (1, 2,)",
    "FROM t |> SELECT [1, 2,] AS a, ARRAY<INT64>[3,] AS b",
    "FROM t |> SELECT STRUCT(1 AS a, 2 AS b,) AS s, (1, 2,) AS p",
    "FROM t |> SELECT CAST(x AS STRUCT<a INT64, b ARRAY<STRUCT<c STRING,>>,>) AS s",
    "FROM t |> SELECT CAST(x AS NUMERIC(10, 2,)) AS n",
    "FROM t |> EXTEND SUM(x) OVER (PARTITION BY a, ORDER BY b, ROWS UNBOUNDED PRECEDING) AS s",
    "FROM t |> EXTEND SUM(x) OVER (PARTITION BY a, b,) AS s",
    "FROM t |> SELECT * EXCEPT (a, b,) REPLACE (c AS d,)",
    "FROM a JOIN b USING (k, l,)",
    "FROM t |> UNION ALL CORRESPONDING BY (a, b,) (FROM u)",
    "FROM a, b,",
    "FROM a, b, |> WHERE true",
    "FROM UNNEST([1], [2],) AS x",
    "FROM f(TABLE t, 1,)",
    "FROM t |> AGGREGATE COUNT(*) GROUP BY ROLLUP(a, b,), CUBE(c,), GROUPING SETS ((a, b,), (c),)",
    "FROM t |> PIVOT (SUM(a) AS s, FOR k IN (1, 2,))",
    "FROM t |> UNPIVOT ((v, w,) FOR k IN ((a, b,), (c, d),))",
    "FROM t |> TABLESAMPLE RESERVOIR (10 ROWS PARTITION BY a, b,)",
    "FROM t |> MATCH_RECOGNIZE (PARTITION BY a, ORDER BY x, MEASURES COUNT(*) AS n, PATTERN (a) DEFINE a AS TRUE,)",
    "FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n, AFTER MATCH SKIP PAST LAST ROW PATTERN (a) DEFINE a AS TRUE)",
    // A sort key named `measures`, and a measure that calls `pattern`, aren't the clauses.
    "FROM t |> MATCH_RECOGNIZE (ORDER BY x, measures MEASURES pattern(x) AS p, COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)",
    "SET (a, b,) = (1, 2)",
    "CREATE TEMP FUNCTION f(a INT64, b INT64,) AS (a)",
    "CREATE TEMP TABLE FUNCTION f(t TABLE<a INT64, b STRING,>) AS (FROM t)",
    "CREATE TEMP TABLE t (a INT64,) OPTIONS (a = 1,)",
];

/// Scripts that GoogleSQL rejects: the text, a part of the first diagnostic's message, and the
/// text that it covers.
pub const REJECTED: &[(&str, &str, &str)] = &[
    (
        "FROM t extra tokens",
        "expected `|>`, `;` or the end of the script",
        "tokens",
    ),
    ("FROM t |> WHERE a > > 1", "expected an expression", ">"),
    ("SELECT a FROM t", "expected `FROM`", "SELECT"),
    ("FROM t |> WHERE a < b < c", "comparisons don't chain", "<"),
    (
        "FROM t |> SELECT AS mytype a",
        "no protocol buffer types",
        "AS mytype",
    ),
    ("FROM t |> SELECT 5abc", "can't start with a digit", "abc"),
    ("FROM t |> SELECT 'a\nb'", "has no closing quote", "'a"),
    (
        "FROM t |> WHERE x = 1 |> QUALIFY y",
        "expected a pipe operator",
        "QUALIFY",
    ),
    (
        "FROM t |> SELECT select",
        "expected an expression",
        "select",
    ),
    ("IF x THEN FROM t; END IF", "have no IF statement", "IF"),
    (
        "CREATE TEMP FUNCTION f(x STRING) RETURNS STRING LANGUAGE js AS 'x'",
        "take no LANGUAGE",
        "LANGUAGE",
    ),
    (
        "CREATE TEMP TABLE t (a INT64) PARTITION BY a",
        "take no PARTITION BY",
        "PARTITION BY",
    ),
    (
        "CREATE TEMP TABLE t (a INT64 DEFAULT 1)",
        "takes a name, a type and NOT NULL",
        "DEFAULT",
    ),
    (
        "CREATE VIEW v AS FROM t",
        "avenger-sql's scripts create no VIEW",
        "VIEW",
    ),
    (
        "MODULE m; FROM t",
        "starts a module, not a script",
        "MODULE",
    ),
    ("IMPORT PROTO 'x.proto'", "no IMPORT PROTO", "PROTO"),
    (
        "IMPORT MODULE foo bar",
        "expected `;` or the end of the script",
        "bar",
    ),
    ("IMPORT MODULE select", "expected a name", "select"),
    (
        "IMPORT MODULE foo.bar AS foo.baz",
        "expected `;` or the end of the script",
        ".",
    ),
    ("FROM t |> DROP order", "expected a name", "order"),
    ("FROM t |> WHERE x = 'a\rb'", "has no closing quote", "'a"),
    ("FROM t |> WHERE x = 0x", "takes digits after 0x", "x"),
    ("FROM t |> WHERE x = @@sys", "no system variables", "@@sys"),
    (
        "FROM t |> WHERE x = 'a''b'",
        "need whitespace or a comment",
        "'b'",
    ),
    ("FROM t |> SELECT 1.5abc", "can't start with a digit", "abc"),
    (
        "FROM my-default.t",
        "`default` is reserved, so it can't follow a dash in a table name",
        "default",
    ),
    (
        "FROM t |> WHERE a = NOT b",
        "this NOT needs parentheses",
        "NOT",
    ),
    (
        "FROM t |> WHERE x BETWEEN NOT a AND b",
        "this NOT needs parentheses",
        "NOT",
    ),
    (
        "FROM t |> WHERE -NOT b",
        "this NOT needs parentheses",
        "NOT",
    ),
    (
        "FROM t |> WHERE CAST(s AS STRING(x)) IS NULL",
        "expected a literal or `MAX`",
        "x",
    ),
    ("FROM t |> WHERE f(a, *) = 1", "expected an expression", "*"),
    ("FROM t |> WHERE f(TABLE t) = 1", "expected `)`", "t"),
    (
        "FROM t |> WHERE (x).select = 1",
        "expected a name",
        "select",
    ),
    (
        "(FROM t) q |> WHERE true",
        "expected `|>`, `;` or the end of the script",
        "q",
    ),
    ("FROM (t)", "expected a JOIN", ")"),
    ("FROM (a, b)", "expected a JOIN", ","),
    (
        "FROM t |> AGGREGATE COUNT(*) GROUP BY ROLLUP(x AS y)",
        "expected `)`",
        "AS",
    ),
    (
        "FROM t PIVOT (SUM(v) FOR k IN (1)) PIVOT (SUM(v) FOR k IN (1))",
        "expected `|>`",
        "PIVOT",
    ),
    (
        "FROM (a JOIN b USING (k)) PIVOT (SUM(v) FOR k IN (1))",
        "expected `|>`",
        "PIVOT",
    ),
    (
        "FROM t |> UNPIVOT (v FOR n IN (a AS 1.1))",
        "a string or an integer",
        "1.1",
    ),
    (
        "FROM t TABLESAMPLE SYSTEM (-1 ROWS)",
        "a number, a query parameter",
        "-",
    ),
    ("FROM t |> EXTEND *", "takes no `*`", "*"),
    (
        "FROM t |> GROUP BY x",
        "GROUP BY goes in a pipe AGGREGATE",
        "GROUP",
    ),
    ("FROM t |> LOG", "has no pipe LOG", "LOG"),
    ("FROM f(*)", "takes no `*` argument", "*"),
    (
        "IMPORT MODULE 'm.sqlm'",
        "names its module by a path",
        "'m.sqlm'",
    ),
    ("IMPORT MODULE m INTO n", "with AS, not INTO", "INTO"),
    // A `(` that the text ends inside.
    (
        "FROM (FROM t",
        "expected `)`, found the end of the text",
        "",
    ),
    (
        "FROM t |> WHERE f(1",
        "expected `)`, found the end of the text",
        "",
    ),
    (
        "FROM t |> PIVOT(x FOR y IN (0)",
        "expected `)`, found the end of the text",
        "",
    ),
    // Errors where GoogleSQL finds them.
    (
        "FROM t |> DROP t.x",
        "names a column alone, not a path",
        "t.x",
    ),
    (
        "FROM t |> RENAME x.abc AS y",
        "names a column alone, not a path",
        "x.abc",
    ),
    (
        "FROM t |> SET t.y = x",
        "names a column alone, not a path",
        "t.y",
    ),
    ("SET a, b = (4, 5)", "takes them in parentheses", "a, b"),
    ("SET a, b) = y", "expected `=`", ")"),
    ("DECLARE x;", "takes a type, a DEFAULT, or both", ""),
    (
        "FROM t |> UNION ALL STRICT BY NAME (FROM u)",
        "STRICT takes CORRESPONDING, not BY NAME",
        "STRICT",
    ),
    (
        "FROM t |> UNPIVOT EXCLUDE NULL (a FOR c IN (x, y))",
        "expected `NULLS`",
        "NULL",
    ),
    // `TABLE FUNCTION` is a table function.
    (
        "CREATE TEMP TABLE function (x INT64)",
        "expected a name",
        "(",
    ), // `.*` follows only the whole expression of a select or aggregate item.
    ("FROM t |> EXTEND 1 + s.*", "expected a name", "*"),
    ("FROM t |> EXTEND SUM(s.*) OVER ()", "expected a name", "*"),
    ("FROM t |> AGGREGATE GROUP BY s.*", "expected a name", "*"),
    // GoogleSQL's syntax outside the subset, which avenger-sql reports as unsupported.
    (
        "ALTER TABLE t ADD COLUMN x INT64",
        "scripts have no ALTER statement",
        "ALTER",
    ),
    (
        "BEGIN FROM t; FROM u; END; FROM v",
        "scripts have no BEGIN statement",
        "BEGIN",
    ),
    (
        "L1: LOOP LEAVE L1; END LOOP L1",
        "scripts have no LOOP statement",
        "L1",
    ),
    (
        "INSERT INTO t VALUES (1)",
        "scripts have no INSERT statement",
        "INSERT",
    ),
    ("CREATE TEMP TABLE u LIKE t", "tables take no LIKE", "LIKE"),
    ("TABLE t", "a query of a table is `FROM t`", "TABLE"),
    ("|> WHERE true", "no subpipelines on their own", "|>"),
    ("FROM t @{hint=1} |> WHERE true", "no hints", "@{hint=1}"),
    ("@1 BEGIN END", "no hints", "@1"),
    ("FROM t |> LIMIT ALL", "LIMIT takes a count", "ALL"),
    (
        "FROM t HASH JOIN u USING (k)",
        "take no HASH or LOOKUP",
        "HASH",
    ),
    (
        "FROM t |> LEFT LOOKUP JOIN u USING (k)",
        "take no HASH or LOOKUP",
        "LOOKUP",
    ),
    (
        "FROM 'a.csv'",
        "names tables with names, not strings",
        "'a.csv'",
    ),
    ("FROM t FOR UPDATE", "without locking them", "FOR UPDATE"),
    (
        "FROM t |> CALL f(MODEL m)",
        "take no MODEL argument",
        "MODEL",
    ),
    (
        "FROM t |> CALL f() PARTITION BY a",
        "take no PARTITION BY",
        "PARTITION",
    ),
    ("FROM t |> SELECT **s", "no `**` unpack", "**"),
    (
        "FROM t |> SELECT WITH(a AS 1, a + 1) AS v",
        "no WITH expressions",
        "WITH",
    ),
    ("FROM t |> DROP **s", "no `**` unpack", "**"),
    (
        "FROM (t1 JOIN t2 JOIN t3 USING (x) USING (y))",
        "joins take one condition each",
        "USING",
    ),
    (
        "SET TRANSACTION READ ONLY",
        "no transactions",
        "TRANSACTION",
    ),
    (
        "CREATE TEMP FUNCTION f(INT64) AS (1)",
        "a parameter of avenger-sql has a name",
        "INT64",
    ),
    (
        "CREATE TEMP FUNCTION f(x INT64) RETURNS INT64",
        "functions have bodies",
        "INT64",
    ),
    (
        "CREATE TEMP FUNCTION f(x FUNCTION<INT64 -> INT64>) AS (1)",
        "no function types",
        "FUNCTION",
    ),
    (
        "CREATE TEMP FUNCTION f(x a.b) AS (1)",
        "no protocol buffer or enum types",
        "a",
    ),
    (
        "CREATE TEMP FUNCTION f(x INT64) DETERMINISTIC AS (1)",
        "take no attributes",
        "DETERMINISTIC",
    ),
    (
        "CREATE TEMP FUNCTION f(x INT64 AS T) AS (1)",
        "no type aliases",
        "AS",
    ),
    (
        "CREATE TEMP TABLE u OPTIONS (a += 1) AS FROM t",
        "set each option with `=`",
        "+",
    ),
    (
        "CREATE TEMP TABLE u OPTIONS (a = 1)",
        "takes columns, AS and a query",
        "OPTIONS (a = 1)",
    ),
    // The forms of MATCH_RECOGNIZE that GoogleSQL rejects, at GoogleSQL's positions.
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE) MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)", "expected `|>`", "MATCH_RECOGNIZE"),
    ("FROM t MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE) PIVOT (SUM(x) FOR y IN (1))", "expected `|>`", "("),
    ("FROM t |> MATCH_RECOGNIZE (MEASURES COUNT(*) AS n PATTERN (a) DEFINE a AS TRUE)", "expected `ORDER`", "MEASURES"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) n PATTERN (a) DEFINE a AS TRUE)", "expected `AS`", "n"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n ONE ROW PER MATCH PATTERN (a) DEFINE a AS TRUE)", "expected `PATTERN`", "ONE"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n AFTER MATCH SKIP TO FIRST a PATTERN (a) DEFINE a AS TRUE)", "expected `NEXT`", "FIRST"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN ({- a -}) DEFINE a AS TRUE)", "expected `)`", "{"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (^*) DEFINE a AS TRUE)", "expected `)`", "*"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a{2}?) DEFINE a AS TRUE)", "expected `)`", "?"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a**) DEFINE a AS TRUE)", "expected `)`", "*"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a{-1}) DEFINE a AS TRUE)", "expected `,`", "-"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (select) DEFINE a AS TRUE)", "expected `)`", "select"),
    ("FROM t |> MATCH_RECOGNIZE (ORDER BY x MEASURES COUNT(*) AS n PATTERN (a) SUBSET u = (a) DEFINE a AS TRUE)", "expected `DEFINE`", "SUBSET"),
    ("FROM t |> SELECT 1 AS match_recognize", "expected a name", "match_recognize"),
    // avenger-sql's own error: a value in parentheses takes no trailing comma.
    ("FROM t |> SELECT (x,) AS a", "takes no trailing comma", ","),
];

/// Modules that GoogleSQL rejects, as [`REJECTED`].
pub const REJECTED_MODULES: &[(&str, &str, &str)] = &[
    (
        "CREATE PUBLIC FUNCTION f() AS (1)",
        "expected `MODULE`",
        "CREATE",
    ),
    (
        "MODULE m; FROM t",
        "holds IMPORT MODULE and CREATE PUBLIC or PRIVATE",
        "FROM",
    ),
    (
        "MODULE m; DECLARE x DEFAULT 1",
        "holds IMPORT MODULE and CREATE PUBLIC or PRIVATE",
        "DECLARE",
    ),
    (
        "MODULE m; MODULE n",
        "one MODULE statement, its first",
        "MODULE",
    ),
];

/// Scripts that the validation pass rejects, with the message and the text of the first
/// diagnostic: the checks that GoogleSQL makes after its parser.
pub const VALIDATED: &[(&str, &str, &str)] = &[
    ("FROM t |> SELECT `` ", "can't be empty", "``"),
    (
        "FROM t |> CROSS JOIN u ON true",
        "takes no condition",
        "ON true",
    ),
    ("DECLARE x", "takes a type, a DEFAULT, or both", ""),
    (
        "FROM t; DECLARE x INT64",
        "comes before the other statements",
        "DECLARE x INT64",
    ),
    (
        "CREATE TEMP FUNCTION f(t TABLE<a INT64>) AS (1)",
        "belongs to a TABLE FUNCTION",
        "TABLE<a INT64>",
    ),
    (
        "CREATE TEMP FUNCTION f(x INT64 NOT AGGREGATE) AS (x)",
        "belongs to a parameter of an AGGREGATE",
        "NOT AGGREGATE",
    ),
    (
        "CREATE TABLE FUNCTION f() RETURNS TABLE<INT64> AS FROM t",
        "each take a name and a type",
        "<INT64>",
    ),
    (
        "CREATE PUBLIC FUNCTION f() AS (1)",
        "belong to the CREATE statements of a module",
        "PUBLIC",
    ),
    (
        "CREATE TEMP CONSTANT c = 1",
        "a CONSTANT belongs to a module",
        "CONSTANT",
    ),
    ("FROM my-proj.ds.tvf(1)", "has no dashes", "my-proj.ds.tvf"),
    (
        "FROM t |> WHERE COUNT(*, a) = 1",
        "is a call's only argument",
        "*",
    ),
    (
        "FROM a, b RIGHT JOIN c ON true",
        "after a comma join needs parentheses",
        "RIGHT JOIN",
    ),
    (
        "FROM t |> FULL UNION ALL STRICT CORRESPONDING (FROM u)",
        "STRICT can't go with the mode",
        "STRICT",
    ),
    (
        "IMPORT MODULE m; DECLARE x INT64",
        "a DECLARE comes before",
        "DECLARE x INT64",
    ),
    (
        "CREATE OR REPLACE TEMP FUNCTION IF NOT EXISTS f() AS (1)",
        "don't go together",
        "IF NOT EXISTS",
    ),
    (
        "FROM t |> WHERE x = 'a\\q'",
        "`\\q` is not an escape",
        "\\q",
    ),
    (
        "FROM t |> WHERE x = '\\xff'",
        "don't form UTF-8 text",
        "'\\xff'",
    ),
    (
        "FROM t |> WHERE x = b'\\u0041'",
        "`\\u` is not an escape",
        "\\u",
    ),
    (
        "FROM t |> WHERE d = DATE b'2024-01-01'",
        "takes a string, not bytes",
        "b'2024-01-01'",
    ),
    ("FROM t |> WHERE x = 'a' b'b'", "don't concatenate", "b'b'"),
    ("FROM LATERAL t", "LATERAL comes before a query", "LATERAL"),
    (
        "FROM UNNEST([1], depth => 2)",
        "one named argument, `mode`",
        "depth",
    ),
    (
        "FROM t |> LEFT UNION ALL (FROM u)",
        "goes with BY NAME or CORRESPONDING",
        "LEFT",
    ),
    (
        "CREATE TEMP TABLE t OPTIONS (a = 1)",
        "takes columns, AS and a query, or both",
        "OPTIONS (a = 1)",
    ),
];

/// Modules that the validation pass rejects, as [`VALIDATED`].
pub const VALIDATED_MODULES: &[(&str, &str, &str)] = &[
    (
        "MODULE m; CREATE FUNCTION f() AS (1)",
        "is PUBLIC or PRIVATE",
        "FUNCTION",
    ),
    (
        "MODULE m; CREATE TEMP FUNCTION f() AS (1)",
        "is PUBLIC or PRIVATE",
        "TEMP",
    ),
    (
        "MODULE m; CREATE OR REPLACE PUBLIC FUNCTION f() AS (1)",
        "no OR REPLACE",
        "OR REPLACE",
    ),
    (
        "MODULE m; CREATE PUBLIC TABLE t AS FROM u",
        "not tables",
        "TABLE",
    ),
    (
        "MODULE m; CREATE PUBLIC FUNCTION IF NOT EXISTS f() AS (1)",
        "no IF NOT EXISTS",
        "IF NOT EXISTS",
    ),
    (
        "MODULE m; CREATE PUBLIC FUNCTION a.f() AS (1)",
        "one-part name",
        "a.f",
    ),
];
