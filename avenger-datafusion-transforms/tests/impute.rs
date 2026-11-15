//! `impute` against reference SQL: the key values, the groups, the join of each group to the
//! key values, an anti join to the input, the imputed value, and a `UNION ALL` with the input.
//! Then the errors of its arguments, at their arguments, and its edge cases.
//!
//! Each case builds an `ImputeCall` with `common::impute_call` from the names of its columns and
//! the SQL of its named arguments over the schema of its input query, plans it with
//! `impute::plan`, and registers the plan as a view, which DataFusion's SQL reads.

mod common;

use avenger_datafusion_transforms::table_function::ArgAt;
use common::{exec, impute_call, impute_error, imputed, rows, session};
use datafusion::arrow::datatypes::DataType;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

/// The table `s`.
async fn context() -> Result<SessionContext> {
    let ctx = session();
    exec(
        &ctx,
        "CREATE TABLE s (month BIGINT, region VARCHAR, product VARCHAR, units DOUBLE, note VARCHAR, iunits BIGINT) AS VALUES
         (1, 'North', 'A', 10.0, 'a', 10),
         (2, 'North', 'A', 12.5, NULL, 12),
         (4, 'North', 'A', 7.0, 'b', 7),
         (1, 'North', 'B', 3.0, NULL, 3),
         (3, 'North', 'B', 4.5, NULL, 4),
         (3, 'North', 'B', 1.0, 'duplicate key', 1),
         (2, 'South', 'A', 8.0, NULL, 8),
         (5, 'South', 'A', NULL, 'null measure', NULL),
         (1, 'South', 'B', 6.0, NULL, 6),
         (6, 'South', 'B', 2.5, NULL, 2),
         (2, NULL, 'A', 9.0, 'null region', 9),
         (NULL, 'North', 'A', 11.0, 'null key', 11),
         (4, 'East', 'B', 5.5, NULL, 5)",
    )
    .await?;
    Ok(ctx)
}

/// The rows that `sql` reads from the view `imputed` of the call of `impute` over the query
/// `input`.
async fn read(
    ctx: &SessionContext,
    input: &str,
    key: &str,
    groups: &[&str],
    value: &str,
    named: Named<'_>,
    sql: &str,
) -> Result<Vec<Vec<String>>> {
    let call = impute_call(ctx, input, key, groups, value, named).await?;
    imputed(ctx, "imputed", input, call).await?;
    rows(ctx, sql).await
}

/// The column names and types of the query `sql`, and its rows.
async fn typed_rows(ctx: &SessionContext, sql: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let fields = ctx.sql(sql).await?.schema().fields().clone();
    let schema = fields
        .iter()
        .map(|f| format!("{} {}", f.name(), f.data_type()));
    Ok((schema.collect(), rows(ctx, sql).await?))
}

/// The SQL of the named argument `name`.
fn arg<'a>(named: &[(&str, &'a str)], name: &str) -> Option<&'a str> {
    named.iter().find(|(n, _)| *n == name).map(|(_, sql)| *sql)
}

/// Named arguments: their names and their SQL.
type Named<'a> = &'a [(&'a str, &'a str)];

/// A case of the key `month`: its name, its input query, its group and value columns, and its
/// named arguments.
type Case = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    Named<'static>,
);

const ALL: &str = "SELECT * FROM s";
const AGG: &str = "SELECT month, region, sum(units) AS units FROM s WHERE month IS NOT NULL GROUP BY month, region";
const REGION: &[&str] = &["region"];

const CASES: &[Case] = &[
    (
        "value 0, groups [region]",
        ALL,
        REGION,
        "units",
        &[("value", "0")],
    ),
    (
        "two groups and a flag",
        ALL,
        &["region", "product"],
        "units",
        &[("value", "0"), ("flag", "imputed")],
    ),
    ("no groups", ALL, &[], "units", &[("value", "-1")]),
    (
        "value NULL",
        ALL,
        REGION,
        "units",
        &[("value", "NULL"), ("flag", "imputed")],
    ),
    ("avg", ALL, REGION, "units", &[("method", "'avg'")]),
    (
        "median",
        ALL,
        &["region", "product"],
        "units",
        &[("method", "'median'")],
    ),
    ("min", ALL, REGION, "units", &[("method", "'min'")]),
    ("max", ALL, &["product"], "units", &[("method", "'max'")]),
    (
        "keys",
        ALL,
        REGION,
        "units",
        &[("value", "0"), ("keys", "[1, 2, 3, 4]")],
    ),
    (
        "extra keys",
        ALL,
        REGION,
        "units",
        &[("value", "0"), ("extra_keys", "[7, 2, 9]")],
    ),
    (
        "keys and extra keys",
        ALL,
        REGION,
        "units",
        &[
            ("value", "0"),
            ("keys", "[3, 1]"),
            ("extra_keys", "[8, 1, 5]"),
        ],
    ),
    (
        "integer value column",
        ALL,
        REGION,
        "iunits",
        &[("value", "0")],
    ),
    (
        "frame avg [-1, 1]",
        AGG,
        REGION,
        "units",
        &[("method", "'avg'"), ("frame", "[-1, 1]")],
    ),
    (
        "frame max [NULL, 0]",
        AGG,
        REGION,
        "units",
        &[("method", "'max'"), ("frame", "[NULL, 0]")],
    ),
    (
        "frame median [-2, 2], keys",
        AGG,
        REGION,
        "units",
        &[
            ("method", "'median'"),
            ("frame", "[-2, 2]"),
            ("keys", "[6, 5, 4, 3, 2, 1]"),
        ],
    ),
    (
        "frame min [1, NULL], no groups",
        AGG,
        &[],
        "units",
        &[("method", "'min'"), ("frame", "[1, NULL]")],
    ),
    (
        "avg of integers is a DOUBLE",
        ALL,
        REGION,
        "iunits",
        &[("method", "'avg'")],
    ),
    (
        "median of integers stays BIGINT",
        ALL,
        REGION,
        "iunits",
        &[("method", "'median'")],
    ),
    (
        "min of strings",
        ALL,
        REGION,
        "note",
        &[("method", "'min'")],
    ),
    (
        "frame avg of integers",
        ALL,
        REGION,
        "iunits",
        &[("method", "'avg'"), ("frame", "[-1, 1]")],
    ),
    (
        "value 0.5 widens integers to DOUBLE",
        ALL,
        REGION,
        "iunits",
        &[("value", "0.5")],
    ),
    (
        "subquery input",
        "SELECT month, product, units FROM s WHERE region = 'North'",
        &["product"],
        "units",
        &[("value", "0")],
    ),
];

fn frame_bound(b: Option<i64>, start: bool) -> String {
    match b {
        None if start => "UNBOUNDED PRECEDING".into(),
        None => "UNBOUNDED FOLLOWING".into(),
        Some(0) => "CURRENT ROW".into(),
        Some(b) if b < 0 => format!("{} PRECEDING", -b),
        Some(b) => format!("{b} FOLLOWING"),
    }
}

/// The reference SQL of a case.
/// `out` is the type of the value column in the result: the type of `COALESCE` of the column and
/// the fill.
fn reference(case: &Case, columns: &[(String, DataType)], out: &str) -> String {
    let (_, input, groups, value, named) = *case;
    let q = |n: &str| format!("\"{n}\"");
    let key = q("month");
    let val_type = &columns.iter().find(|(n, _)| n == value).unwrap().1;
    let gs: Vec<String> = groups.iter().map(|g| q(g)).collect();
    let keyvals = match (arg(named, "keys"), arg(named, "extra_keys")) {
        (Some(k), Some(e)) => format!(
            "SELECT DISTINCT __k FROM (SELECT unnest({k}) AS __k UNION ALL SELECT unnest({e}))"
        ),
        (Some(k), None) => format!("SELECT DISTINCT __k FROM (SELECT unnest({k}) AS __k)"),
        (None, Some(e)) => format!(
            "SELECT DISTINCT __k FROM (SELECT {key} AS __k FROM input WHERE {key} IS NOT NULL UNION ALL SELECT unnest({e}))"
        ),
        (None, None) => format!("SELECT DISTINCT {key} AS __k FROM input WHERE {key} IS NOT NULL"),
    };
    let grp = if gs.is_empty() {
        "SELECT DISTINCT 1 AS __one FROM input".to_string()
    } else {
        format!("SELECT DISTINCT {} FROM input", gs.join(", "))
    };
    let on_groups = |a: &str, b: &str| -> String {
        gs.iter()
            .map(|g| format!("({a}.{g} IS NOT DISTINCT FROM {b}.{g}) AND "))
            .collect()
    };
    let present_groups: String = gs.iter().map(|g| format!("{g}, ")).collect();
    let mut sql = format!(
        "WITH input AS (SELECT * FROM ({input})),\n\
         keyvals AS ({keyvals}),\n\
         grp AS ({grp}),\n\
         cand AS (SELECT grp.*, keyvals.__k FROM grp CROSS JOIN keyvals),\n\
         present AS (SELECT DISTINCT {present_groups}{key} AS __k, true AS __present FROM input),\n\
         added AS (SELECT cand.* FROM cand LEFT JOIN present p ON {on}cand.__k = p.__k WHERE p.__present IS NULL)",
        on = on_groups("cand", "p"),
    );
    let method = arg(named, "method").map(|m| m.trim_matches('\''));
    let frame = arg(named, "frame").map(|f| {
        let (lo, hi) = f.trim_matches(['[', ']']).split_once(',').unwrap();
        (lo.trim().parse().ok(), hi.trim().parse().ok())
    });
    // The value of each added row, and what the added rows are selected from.
    let (imputed, from) = match (arg(named, "value"), method, frame) {
        (Some(v), _, _) => (format!("arrow_cast({v}, '{out}')"), "added".to_string()),
        (None, Some(agg), None) => {
            let by = if gs.is_empty() {
                String::new()
            } else {
                format!(" GROUP BY {}", gs.join(", "))
            };
            sql += &format!(
                ",\nst AS (SELECT {present_groups}{agg}({v}) AS __imp FROM input{by})",
                v = q(value)
            );
            let join = if gs.is_empty() {
                "added CROSS JOIN st".to_string()
            } else {
                let on: String = on_groups("added", "st");
                format!("added LEFT JOIN st ON {}", on.trim_end_matches(" AND "))
            };
            (format!("arrow_cast(st.__imp, '{out}')"), join)
        }
        (None, Some(agg), Some((lo, hi))) => {
            let order = match arg(named, "keys") {
                Some(k) => format!("array_position({k}, __k) ASC NULLS LAST, __k"),
                None => "__k".to_string(),
            };
            let part = if gs.is_empty() {
                String::new()
            } else {
                format!("PARTITION BY {} ", gs.join(", "))
            };
            let gsel: String = gs.iter().map(|g| format!("i.{g}, ")).collect();
            sql += &format!(
                ",\ncombined AS (SELECT {gsel}i.{key} AS __k, i.{v} AS __v, false AS __added FROM input i JOIN keyvals k ON i.{key} = k.__k\n\
                   UNION ALL SELECT {present_groups}__k, arrow_cast(NULL, '{val_type}'), true FROM added),\n\
                 framed AS (SELECT *, {agg}(__v) OVER ({part}ORDER BY {order} ROWS BETWEEN {lo} AND {hi}) AS __imp FROM combined)",
                v = q(value),
                lo = frame_bound(lo, true),
                hi = frame_bound(hi, false),
            );
            (
                format!("arrow_cast(__imp, '{out}')"),
                "framed WHERE __added".to_string(),
            )
        }
        _ => panic!("a case needs a value or a method"),
    };
    let flag = arg(named, "flag");
    let flag_in = flag.map_or(String::new(), |f| format!(", false AS {f}"));
    let flag_add = flag.map_or(String::new(), |f| format!(", true AS {f}"));
    let cols_in: Vec<String> = columns
        .iter()
        .map(|(n, _)| match n == value {
            true => format!("arrow_cast({}, '{out}') AS {}", q(n), q(n)),
            false => q(n),
        })
        .collect();
    let qualifier = match from.starts_with("added ") {
        true => "added.",
        false => "",
    };
    let cols_add: Vec<String> = columns
        .iter()
        .map(|(n, t)| {
            if n == "month" {
                format!("__k AS {}", q(n))
            } else if groups.contains(&n.as_str()) {
                format!("{qualifier}{}", q(n))
            } else if n == value {
                format!("{imputed} AS {}", q(n))
            } else {
                format!("arrow_cast(NULL, '{t}') AS {}", q(n))
            }
        })
        .collect();
    sql += &format!(
        "\nSELECT {}{flag_in} FROM input\nUNION ALL\nSELECT {}{flag_add} FROM {from}",
        cols_in.join(", "),
        cols_add.join(", ")
    );
    sql
}

#[tokio::test]
async fn reference_sql() -> Result<()> {
    let ctx = context().await?;
    for case in CASES {
        let (name, input, groups, value, named) = *case;
        let probe = ctx.sql(&format!("SELECT * FROM ({input}) LIMIT 0")).await?;
        let columns: Vec<(String, DataType)> = (probe.schema().fields().iter())
            .map(|f| (f.name().clone(), f.data_type().clone()))
            .collect();
        // The type of `COALESCE` of the value column and the fill.
        let fill = match (arg(named, "value"), arg(named, "method")) {
            (Some(v), _) => v.to_string(),
            (None, m) => format!(
                "(SELECT {}({value}) FROM ({input}))",
                m.unwrap().trim_matches('\'')
            ),
        };
        let typed = rows(
            &ctx,
            &format!("SELECT arrow_typeof(coalesce({value}, {fill})) FROM ({input}) LIMIT 1"),
        )
        .await?;
        let reference_sql = reference(case, &columns, &typed[0][0]);
        let call = impute_call(&ctx, input, "month", groups, value, named).await?;
        imputed(&ctx, "imputed", input, call).await?;
        let ours = typed_rows(&ctx, "SELECT * FROM imputed").await?;
        let expected = typed_rows(&ctx, &reference_sql).await?;
        assert_eq!(ours, expected, "{name}");
    }
    Ok(())
}

#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context().await?;
    // The groups, the value column, and the named arguments of a call, the place of its error,
    // and the start of its message.
    let errors: [(&[&str], &str, Named, ArgAt, &str); 14] = [
        (
            REGION,
            "units",
            &[],
            ArgAt::Call,
            "Impute needs exactly one of `value` and `method`; found neither",
        ),
        (
            REGION,
            "units",
            &[("value", "0"), ("method", "'avg'")],
            ArgAt::Named(1),
            "Impute needs exactly one of `value` and `method`; found both",
        ),
        (
            REGION,
            "units",
            &[("method", "'linear'")],
            ArgAt::Named(0),
            "`method` must be 'avg', 'median', 'min', or 'max'",
        ),
        (
            REGION,
            "units",
            &[("value", "0"), ("frame", "[-1, 1]")],
            ArgAt::Named(1),
            "`frame` needs a method",
        ),
        (
            REGION,
            "units",
            &[("method", "'avg'"), ("frame", "[2, 1]")],
            ArgAt::Named(1),
            "The start of `frame`, 2, is after its end, 1",
        ),
        (
            REGION,
            "units",
            &[("value", "0"), ("fill", "1")],
            ArgAt::Named(1),
            "Unknown argument `fill` of impute",
        ),
        (
            REGION,
            "units",
            &[("value", "'zero'")],
            ArgAt::Named(0),
            "`value` 'zero' does not coerce to the type of units",
        ),
        (
            REGION,
            "units",
            &[("value", "0"), ("extra_keys", "['a']")],
            ArgAt::NamedElement(1, 0),
            "The value 'a' of `extra_keys` does not coerce",
        ),
        (
            REGION,
            "units",
            &[("value", "0"), ("flag", "note")],
            ArgAt::Named(1),
            "The flag column `note` is already a column of the input",
        ),
        (
            REGION,
            "note",
            &[("method", "'avg'")],
            ArgAt::Named(0),
            "`method => 'avg'` does not apply to note",
        ),
        (
            &["month"],
            "units",
            &[("value", "0")],
            ArgAt::Element(1, 0),
            "The key month of impute is also a group column",
        ),
        (
            REGION,
            "units",
            &[("value", "0"), ("keys", "[1, NULL]")],
            ArgAt::NamedElement(1, 1),
            "A value of `keys` cannot be NULL",
        ),
        (
            REGION,
            "units",
            &[("value", "0"), ("keys", "[1.5]")],
            ArgAt::NamedElement(1, 0),
            "The value 1.5 of `keys` does not coerce",
        ),
        (
            REGION,
            "units",
            &[("method", "'avg'"), ("frame", "[0, 18446744073709551615]")],
            ArgAt::NamedElement(1, 1),
            "A bound of `frame` must fit in an INT64",
        ),
    ];
    for (groups, value, named, at, message) in errors {
        let call = impute_call(&ctx, ALL, "month", groups, value, named).await?;
        let e = impute_error(&ctx, ALL, call).await?;
        assert!(
            e.at == at && e.message.starts_with(message),
            "{message}: {e:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn edge_cases() -> Result<()> {
    let ctx = context().await?;
    let by_region = async |input: &str, named: Named<'_>| {
        let all = "SELECT * FROM imputed";
        read(&ctx, input, "month", REGION, "units", named, all).await
    };
    // A constant that needs type coercion.
    let a = by_region(ALL, &[("value", "1 + 0.5")]).await?;
    assert!(a.iter().any(|r| r[3] == "1.5"));
    // A frame of the whole group is no frame.
    let avg = [("method", "'avg'"), ("keys", "[1, 2, 3]")];
    let no_frame = by_region(ALL, &avg).await?;
    let whole = by_region(ALL, &[avg[0], avg[1], ("frame", "[NULL, NULL]")]).await?;
    assert_eq!(no_frame, whole);
    // A frame bound past the end of every group.
    let max = ("method", "'max'");
    let wide = by_region(ALL, &[max, ("frame", "[0, 9223372036854775807]")]).await?;
    let open = by_region(ALL, &[max, ("frame", "[0, NULL]")]).await?;
    assert_eq!(wide, open);
    // An empty input.
    let empty = by_region("SELECT * FROM s WHERE false", &[("value", "0")]).await?;
    assert!(empty.is_empty());
    // A struct key, as the result of timeunit or bin, sorted in key order.
    let input = "SELECT named_struct('m', month) AS k, region AS g, units AS v FROM s WHERE month IS NOT NULL";
    let sql = "SELECT k['m'] AS m, g, v FROM imputed";
    // 12 rows, and the missing months of 4 regions: 2, 2, 5, and 5.
    let all = read(&ctx, input, "k", &["g"], "v", &[("value", "0")], sql).await?;
    assert_eq!(all.len(), 26);
    let east: Vec<&str> = (all.iter().filter(|r| r[1] == "East"))
        .map(|r| r[0].as_str())
        .collect();
    assert_eq!(east, ["1", "2", "3", "4", "5", "6"]);
    Ok(())
}
