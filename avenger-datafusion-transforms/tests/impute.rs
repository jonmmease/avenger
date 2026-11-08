//! `impute` against reference SQL: the key values, the groups, the join of each group to the key values, an anti join to the input,
//! the imputed value, and a `UNION ALL` with the input. Then its errors, at their places.

mod common;

use avenger_datafusion_transforms::session;
use datafusion::arrow::array::{Array, ArrayRef};
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

async fn context() -> Result<SessionContext> {
    let ctx = session(true);
    ctx.sql(
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
    .await?
    .collect()
    .await?;
    Ok(ctx)
}

enum Fill {
    Value(&'static str),
    Gap,
    Stat(&'static str, Option<(Option<i64>, Option<i64>)>),
}

struct Case {
    name: &'static str,
    input: &'static str,
    key: &'static str,
    /// The groups argument as written, and its columns.
    groups: (&'static str, &'static [&'static str]),
    val: &'static str,
    /// The named arguments as written.
    named: &'static str,
    fill: Fill,
    keys: Option<&'static str>,
    extra: Option<&'static str>,
    flag: Option<&'static str>,
}

const AGG: &str = "(SELECT month, region, sum(units) AS units FROM s WHERE month IS NOT NULL GROUP BY month, region)";

fn cases() -> Vec<Case> {
    let c = |name, input, key, groups, val, named, fill, keys, extra, flag| Case {
        name,
        input,
        key,
        groups,
        val,
        named,
        fill,
        keys,
        extra,
        flag,
    };
    vec![
        c(
            "value 0, groups [region]",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "units",
            "value => 0",
            Fill::Value("0"),
            None,
            None,
            None,
        ),
        c(
            "two groups and a flag",
            "(SELECT * FROM s)",
            "month",
            ("[region, product]", &["region", "product"]),
            "units",
            "value => 0, flag => imputed",
            Fill::Value("0"),
            None,
            None,
            Some("imputed"),
        ),
        c(
            "no groups",
            "(SELECT * FROM s)",
            "month",
            ("[]", &[]),
            "units",
            "value => -1",
            Fill::Value("-1"),
            None,
            None,
            None,
        ),
        c(
            "one group without brackets",
            "(SELECT * FROM s)",
            "month",
            ("product", &["product"]),
            "units",
            "value => 0",
            Fill::Value("0"),
            None,
            None,
            None,
        ),
        c(
            "gap",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "units",
            "method => 'gap', flag => imputed",
            Fill::Gap,
            None,
            None,
            Some("imputed"),
        ),
        c(
            "mean",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "units",
            "method => 'mean'",
            Fill::Stat("avg", None),
            None,
            None,
            None,
        ),
        c(
            "median",
            "(SELECT * FROM s)",
            "month",
            ("[region, product]", &["region", "product"]),
            "units",
            "method => 'median'",
            Fill::Stat("median", None),
            None,
            None,
            None,
        ),
        c(
            "min",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "units",
            "method => 'min'",
            Fill::Stat("min", None),
            None,
            None,
            None,
        ),
        c(
            "max",
            "(SELECT * FROM s)",
            "month",
            ("[product]", &["product"]),
            "units",
            "method => 'max'",
            Fill::Stat("max", None),
            None,
            None,
            None,
        ),
        c(
            "keys",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "units",
            "value => 0, keys => [1, 2, 3, 4]",
            Fill::Value("0"),
            Some("1, 2, 3, 4"),
            None,
            None,
        ),
        c(
            "extra keys",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "units",
            "value => 0, extra_keys => [7, 2, 9]",
            Fill::Value("0"),
            None,
            Some("7, 2, 9"),
            None,
        ),
        c(
            "keys and extra keys",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "units",
            "value => 0, keys => [3, 1], extra_keys => [8, 1, 5]",
            Fill::Value("0"),
            Some("3, 1"),
            Some("8, 1, 5"),
            None,
        ),
        c(
            "integer value column",
            "(SELECT * FROM s)",
            "month",
            ("[region]", &["region"]),
            "iunits",
            "value => 0",
            Fill::Value("0"),
            None,
            None,
            None,
        ),
        c(
            "frame mean [-1, 1]",
            AGG,
            "month",
            ("[region]", &["region"]),
            "units",
            "method => 'mean', frame => [-1, 1]",
            Fill::Stat("avg", Some((Some(-1), Some(1)))),
            None,
            None,
            None,
        ),
        c(
            "frame max [NULL, 0]",
            AGG,
            "month",
            ("[region]", &["region"]),
            "units",
            "method => 'max', frame => [NULL, 0]",
            Fill::Stat("max", Some((None, Some(0)))),
            None,
            None,
            None,
        ),
        c(
            "frame median [-2, 2], keys",
            AGG,
            "month",
            ("[region]", &["region"]),
            "units",
            "method => 'median', frame => [-2, 2], keys => [6, 5, 4, 3, 2, 1]",
            Fill::Stat("median", Some((Some(-2), Some(2)))),
            Some("6, 5, 4, 3, 2, 1"),
            None,
            None,
        ),
        c(
            "frame min [1, NULL], no groups",
            AGG,
            "month",
            ("[]", &[]),
            "units",
            "method => 'min', frame => [1, NULL]",
            Fill::Stat("min", Some((Some(1), None))),
            None,
            None,
            None,
        ),
        c(
            "subquery input",
            "(SELECT month, product, units FROM s WHERE region = 'North')",
            "month",
            ("product", &["product"]),
            "units",
            "value => 0",
            Fill::Value("0"),
            None,
            None,
            None,
        ),
    ]
}

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
fn reference(c: &Case, columns: &[(String, DataType)]) -> String {
    let q = |n: &str| format!("\"{n}\"");
    let key = q(c.key);
    let val_type = &columns.iter().find(|(n, _)| n == c.val).unwrap().1;
    let gs: Vec<String> = c.groups.1.iter().map(|g| q(g)).collect();
    let keyvals = match (c.keys, c.extra) {
        (Some(k), Some(e)) => format!(
            "SELECT DISTINCT __k FROM (SELECT unnest(make_array({k})) AS __k UNION ALL SELECT unnest(make_array({e})))"
        ),
        (Some(k), None) => {
            format!("SELECT DISTINCT __k FROM (SELECT unnest(make_array({k})) AS __k)")
        }
        (None, Some(e)) => format!(
            "SELECT DISTINCT __k FROM (SELECT {key} AS __k FROM input WHERE {key} IS NOT NULL UNION ALL SELECT unnest(make_array({e})))"
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
        "WITH input AS (SELECT * FROM {input}),\n\
         keyvals AS ({keyvals}),\n\
         grp AS ({grp}),\n\
         cand AS (SELECT grp.*, keyvals.__k FROM grp CROSS JOIN keyvals),\n\
         present AS (SELECT DISTINCT {present_groups}{key} AS __k, true AS __present FROM input),\n\
         added AS (SELECT cand.* FROM cand LEFT JOIN present p ON {on}cand.__k = p.__k WHERE p.__present IS NULL)",
        input = c.input,
        on = on_groups("cand", "p"),
    );
    // The value of each added row, and what the added rows are selected from.
    let (imputed, from) = match &c.fill {
        Fill::Value(v) => (
            format!("arrow_cast({v}, '{val_type}')"),
            "added".to_string(),
        ),
        Fill::Gap => (
            format!("arrow_cast(NULL, '{val_type}')"),
            "added".to_string(),
        ),
        Fill::Stat(agg, None) => {
            let by = if gs.is_empty() {
                String::new()
            } else {
                format!(" GROUP BY {}", gs.join(", "))
            };
            sql += &format!(
                ",\nst AS (SELECT {present_groups}{agg}(arrow_cast({v}, 'Float64')) AS __imp FROM input{by})",
                v = q(c.val)
            );
            let join = if gs.is_empty() {
                "added CROSS JOIN st".to_string()
            } else {
                let on: String = on_groups("added", "st");
                format!("added LEFT JOIN st ON {}", on.trim_end_matches(" AND "))
            };
            (format!("arrow_cast(st.__imp, '{val_type}')"), join)
        }
        Fill::Stat(agg, Some((lo, hi))) => {
            let order = match c.keys {
                Some(k) => format!("array_position(make_array({k}), __k) ASC NULLS LAST, __k"),
                None => "__k".to_string(),
            };
            let part = if gs.is_empty() {
                String::new()
            } else {
                format!("PARTITION BY {} ", gs.join(", "))
            };
            let gsel: String = gs.iter().map(|g| format!("i.{g}, ")).collect();
            let gadd: String = gs.iter().map(|g| format!("{g}, ")).collect();
            sql += &format!(
                ",\ncombined AS (SELECT {gsel}i.{key} AS __k, arrow_cast(i.{v}, 'Float64') AS __v, false AS __added FROM input i JOIN keyvals k ON i.{key} = k.__k\n\
                   UNION ALL SELECT {gadd}__k, arrow_cast(NULL, 'Float64'), true FROM added),\n\
                 framed AS (SELECT *, {agg}(__v) OVER ({part}ORDER BY {order} ROWS BETWEEN {lo} AND {hi}) AS __imp FROM combined)",
                v = q(c.val),
                lo = frame_bound(*lo, true),
                hi = frame_bound(*hi, false),
            );
            (
                format!("arrow_cast(__imp, '{val_type}')"),
                "framed WHERE __added".to_string(),
            )
        }
    };
    let flag_in = c.flag.map_or(String::new(), |f| format!(", false AS {f}"));
    let flag_add = c.flag.map_or(String::new(), |f| format!(", true AS {f}"));
    let cols_in: Vec<String> = columns.iter().map(|(n, _)| q(n)).collect();
    let cols_add: Vec<String> = columns
        .iter()
        .map(|(n, t)| {
            if n == c.key {
                format!("__k AS {}", q(n))
            } else if c.groups.1.contains(&n.as_str()) {
                let qualifier =
                    if from.starts_with("added LEFT JOIN") || from.starts_with("added CROSS") {
                        "added."
                    } else {
                        ""
                    };
                format!("{qualifier}{}", q(n))
            } else if n == c.val {
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

fn ours(c: &Case) -> String {
    format!(
        "SELECT * FROM impute({}, {}, {}, {}, {})",
        c.input, c.key, c.groups.0, c.val, c.named
    )
}

/// The rows as sorted strings, and the column names and types.
async fn rows(ctx: &SessionContext, sql: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let df = ctx.sql(sql).await?;
    let schema: Vec<String> = df
        .schema()
        .fields()
        .iter()
        .map(|f| format!("{} {}", f.name(), f.data_type()))
        .collect();
    let mut out = Vec::new();
    for b in df.collect().await? {
        for i in 0..b.num_rows() {
            out.push(
                b.columns()
                    .iter()
                    .map(|c: &ArrayRef| {
                        if c.is_null(i) {
                            "NULL".to_string()
                        } else {
                            array_value_to_string(c, i).unwrap()
                        }
                    })
                    .collect::<Vec<_>>(),
            );
        }
    }
    out.sort();
    Ok((schema, out))
}

#[tokio::test]
async fn reference_sql() -> Result<()> {
    let ctx = context().await?;
    for c in cases() {
        let probe = ctx
            .sql(&format!("SELECT * FROM {} LIMIT 0", c.input))
            .await?;
        let columns: Vec<(String, DataType)> = probe
            .schema()
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.data_type().clone()))
            .collect();
        let reference_sql = reference(&c, &columns);
        let ours = rows(&ctx, &ours(&c)).await?;
        let expected = rows(&ctx, &reference_sql).await?;
        assert_eq!(ours, expected, "{}", c.name);
    }
    Ok(())
}

#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context().await?;
    // The arguments, the start of the message, and the column of the place in the arguments,
    // or 0 for the name `impute`.
    let errors = [
        (
            "s, month, [region], units, value => 0",
            "impute cannot read the table s by name",
            1,
        ),
        (
            "(SELECT * FROM s), month, [region], units",
            "impute needs exactly one of `value` and `method`; found neither",
            0,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 0, method => 'mean'",
            "found both",
            66,
        ),
        (
            "(SELECT * FROM s), month, [region], units, method => 'linear'",
            "`method` must be 'mean', 'median', 'min', 'max', or 'gap'",
            54,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 0, frame => [-1, 1]",
            "`frame` needs a method",
            67,
        ),
        (
            "(SELECT * FROM s), month, [region], units, method => 'gap', frame => [-1, 1]",
            "`frame` needs a method",
            72,
        ),
        (
            "(SELECT * FROM s), month, [region], units, method => 'mean', frame => [2, 1]",
            "the start of `frame`, 2, is after its end, 1",
            72,
        ),
        (
            "(SELECT * FROM s), month + 1, [region], units, value => 0",
            "the key of impute must be a column of its input",
            20,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 0, fill => 1",
            "unknown argument `fill` of impute",
            56,
        ),
        (
            "42, month, [region], units, value => 0",
            "the input of impute must be a table name or a subquery",
            1,
        ),
        (
            "(SELECT * FROM s), month, units, value => 0",
            "impute takes 4 positional arguments",
            0,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 'zero'",
            "`value` 'zero' does not coerce to the type of s.units",
            53,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 0, extra_keys => ['a']",
            "the value 'a' of `extra_keys` does not coerce",
            71,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 0, flag => note",
            "the flag column `note` is already a column of the input",
            64,
        ),
        (
            "(SELECT * FROM s), month, [region], note, method => 'mean'",
            "`method => 'mean'` needs a numeric value column",
            53,
        ),
        (
            "(SELECT * FROM s), month, [regoin], units, value => 0",
            "the group of impute must be a column of its input",
            28,
        ),
        (
            "(SELECT * FROM s), month, [month], units, value => 0",
            "the key s.month of impute is also a group column",
            28,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 0, keys => [1, NULL]",
            "a value of `keys` cannot be NULL",
            68,
        ),
        (
            "(SELECT * FROM s), month, [region], iunits, value => 0.5",
            "`value` 0.5 does not coerce to the type of s.iunits",
            54,
        ),
        (
            "(SELECT * FROM s), month, [region], units, value => 0, keys => [1.5]",
            "the value 1.5 of `keys` does not coerce",
            65,
        ),
        (
            "(SELECT * FROM s), month, [region], units, method => 'mean', frame => [0, 18446744073709551615]",
            "a bound of `frame` must fit in a BIGINT",
            75,
        ),
    ];
    let prefix = "SELECT * FROM impute(".len() as u64;
    for (args, message, column) in errors {
        let sql = format!("SELECT * FROM impute({args})");
        let e = ctx
            .sql(&sql)
            .await
            .err()
            .unwrap_or_else(|| panic!("no error: {args}"));
        let place = e
            .diagnostic()
            .and_then(|d| d.span)
            .map(|s| s.start.column.saturating_sub(prefix))
            .unwrap_or(u64::MAX);
        assert!(e.to_string().contains(message), "{args}: {e}");
        assert_eq!(place, column, "{args}: {e}");
    }
    Ok(())
}

/// The rows of a query, sorted.
async fn sorted(ctx: &SessionContext, sql: &str) -> Result<Vec<Vec<String>>> {
    Ok(rows(ctx, sql).await?.1)
}

#[tokio::test]
async fn edge_cases() -> Result<()> {
    let ctx = context().await?;
    // A constant that needs type coercion.
    let a = sorted(
        &ctx,
        "SELECT * FROM impute((SELECT * FROM s), month, [region], units, value => 1 + 0.5)",
    )
    .await?;
    assert!(a.iter().any(|r| r[3] == "1.5"));
    // A frame of the whole group is no frame.
    let no_frame = sorted(&ctx, "SELECT * FROM impute((SELECT * FROM s), month, [region], units, method => 'mean', keys => [1, 2, 3])").await?;
    let whole = sorted(&ctx, "SELECT * FROM impute((SELECT * FROM s), month, [region], units, method => 'mean', keys => [1, 2, 3], frame => [NULL, NULL])").await?;
    assert_eq!(no_frame, whole);
    // A frame bound past the end of every group.
    let wide = sorted(&ctx, "SELECT * FROM impute((SELECT * FROM s), month, [region], units, method => 'max', frame => [0, 9223372036854775807])").await?;
    let open = sorted(&ctx, "SELECT * FROM impute((SELECT * FROM s), month, [region], units, method => 'max', frame => [0, NULL])").await?;
    assert_eq!(wide, open);
    // An empty input.
    let empty = sorted(
        &ctx,
        "SELECT * FROM impute((SELECT * FROM s WHERE false), month, [region], units, value => 0)",
    )
    .await?;
    assert!(empty.is_empty());
    // A struct key, as the result of timeunit or bin, sorted in key order.
    let sql = "SELECT k['m'] AS m, g, v FROM impute((SELECT named_struct('m', month) AS k, region AS g, units AS v FROM s WHERE month IS NOT NULL), k, [g], v, value => 0)";
    // 12 rows, and the missing months of 4 regions: 2, 2, 5, and 5.
    let all = sorted(&ctx, sql).await?;
    assert_eq!(all.len(), 26);
    let east: Vec<&str> = all
        .iter()
        .filter(|r| r[1] == "East")
        .map(|r| r[0].as_str())
        .collect();
    assert_eq!(east, ["1", "2", "3", "4", "5", "6"]);
    Ok(())
}
