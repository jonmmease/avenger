//! `lump`: each case, with the rule, against hand-written SQL in the shape of a table
//! function (a grouped SELECT of the categories, joined back to the rows); the cases with
//! `'n'` and the default weight also through the window's own evaluator. Then aggregates
//! grouped by lumps, fused and unfused, and errors.

mod common;

use common::session;

use datafusion::arrow::array::{Array, ArrayRef};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

async fn context(ctx: SessionContext) -> Result<SessionContext> {
    ctx.sql(
        "CREATE TABLE c (region VARCHAR, port VARCHAR, tons DOUBLE) AS VALUES
        ('N', 'a', 1.0), ('N', 'a', 2.0), ('N', 'a', 3.0), ('N', 'b', 4.0), ('N', 'b', 5.0),
        ('N', 'c', 6.0), ('N', 'Other', 0.5), ('N', NULL, 7.0), ('N', NULL, 1.5),
        ('S', 'a', 9.0), ('S', 'b', 1.0), ('S', 'b', 1.0), ('S', 'c', 2.0), ('S', 'c', NULL),
        ('S', 'd', 12.0), ('S', 'd', 0.5), ('S', 'e', 3.0),
        (NULL, 'a', 4.0), (NULL, 'z', 1.0)",
    )
    .await?
    .collect()
    .await?;
    Ok(ctx)
}

/// A session with the rules but without the fusion of aggregates.
async fn unfused() -> Result<SessionContext> {
    let ctx = session(true);
    ctx.remove_optimizer_rule("lump_aggregate_fusion");
    context(ctx).await
}

/// A case: the category, the options as SQL, and their parts for the hand-written SQL.
struct Case {
    name: &'static str,
    category: &'static str,
    options: &'static str,
    /// `n`, or the `keep` condition.
    keep: Keep,
    weight: &'static str,
    other: &'static str,
}

enum Keep {
    N(i64),
    Cond(&'static str),
}

fn cases() -> Vec<Case> {
    use Keep::*;
    vec![
        Case {
            name: "n 2",
            category: "port",
            options: "'n', 2",
            keep: N(2),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "n 2 by sum",
            category: "port",
            options: "'n', 2, 'weight', 'sum(tons)'",
            keep: N(2),
            weight: "sum(tons)",
            other: "Other",
        },
        Case {
            name: "keep a share",
            category: "port",
            options: "'keep', 'count(*) >= 0.2 * sum(count(*)) OVER ()'",
            keep: Cond("count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY {keys})"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "keep two measures",
            category: "port",
            options: "'keep', 'sum(tons) > 10 AND count(*) >= 2'",
            keep: Cond("sum(tons) > 10 AND count(*) >= 2"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "n 2 by avg (second pass)",
            category: "port",
            options: "'n', 2, 'weight', 'avg(tons)'",
            keep: N(2),
            weight: "avg(tons)",
            other: "Other",
        },
        Case {
            name: "by max, keep max",
            category: "port",
            options: "'weight', 'max(tons)', 'keep', 'max(tons) >= 5'",
            keep: Cond("max(tons) >= 5"),
            weight: "max(tons)",
            other: "Other",
        },
        Case {
            name: "other label",
            category: "port",
            options: "'n', 1, 'other', 'Rest'",
            keep: N(1),
            weight: "count(*)",
            other: "Rest",
        },
        Case {
            name: "NULL condition drops",
            category: "port",
            options: "'keep', 'CASE WHEN port = ''b'' THEN NULL ELSE true END'",
            keep: Cond("CASE WHEN port = 'b' THEN NULL ELSE true END"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "category expression",
            category: "lower(port)",
            options: "'n', 2",
            keep: N(2),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "Pareto cut",
            category: "port",
            options: "'keep', 'sum(count(*)) OVER (ORDER BY count(*) DESC, port ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER ()'",
            keep: Cond(
                "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC, port ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
            ),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "keep median (no state)",
            category: "port",
            options: "'keep', 'median(tons) >= 2'",
            keep: Cond("median(tons) >= 2"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "by stddev",
            category: "port",
            options: "'n', 2, 'weight', 'stddev_pop(tons)'",
            keep: N(2),
            weight: "stddev_pop(tons)",
            other: "Other",
        },
        Case {
            name: "ranked: top 2 by weight",
            category: "port",
            options: "'weight', 'sum(tons)', 'keep', 'row_number() OVER ranked <= 2'",
            keep: N(2),
            weight: "sum(tons)",
            other: "Other",
        },
        Case {
            name: "ranked: ties",
            category: "port",
            options: "'weight', 'sum(tons)', 'keep', 'rank() OVER ranked <= 2'",
            keep: Cond(
                "rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 2",
            ),
            weight: "sum(tons)",
            other: "Other",
        },
        Case {
            name: "ranked: Pareto cut",
            category: "port",
            options: "'keep', 'sum(count(*)) OVER (ranked ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER ()'",
            keep: Cond(
                "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
            ),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "ranked: bottom 2",
            category: "port",
            options: "'keep', 'row_number() OVER ranked > count(*) OVER () - 2'",
            keep: Cond(
                "row_number() OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST) > count(*) OVER (PARTITION BY {keys}) - 2",
            ),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "ranked: percent rank",
            category: "port",
            options: "'weight', 'max(tons)', 'keep', 'percent_rank() OVER ranked <= 0.5'",
            keep: Cond(
                "percent_rank() OVER (PARTITION BY {keys} ORDER BY max(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 0.5",
            ),
            weight: "max(tons)",
            other: "Other",
        },
        Case {
            name: "rank",
            category: "port",
            options: "'keep', 'rank() OVER (ORDER BY sum(tons) DESC) <= 2'",
            keep: Cond("rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC) <= 2"),
            weight: "count(*)",
            other: "Other",
        },
    ]
}

/// The window clause, and the partition keys.
const WINDOWS: [(&str, &str); 2] = [("OVER ()", ""), ("OVER (PARTITION BY region)", "region")];

fn lump_query(table: &str, c: &Case, over: &str) -> String {
    format!(
        "SELECT *, lump({}, {}) {over} AS l FROM {table}",
        c.category, c.options
    )
}

/// Hand-written SQL with the same result: the categories, the decision, the Other weight,
/// and the join back to the rows.
fn hand_query(table: &str, c: &Case, keys: &str) -> String {
    let kc = |p: &str| {
        if keys.is_empty() {
            String::new()
        } else {
            format!("{p}{keys}, ")
        }
    };
    let part = if keys.is_empty() {
        String::new()
    } else {
        format!("PARTITION BY {keys}")
    };
    let keep = match &c.keep {
        Keep::N(n) => format!(
            "row_number() OVER ({part} ORDER BY {} DESC NULLS LAST, {} ASC NULLS LAST) <= {n}",
            c.weight, c.category
        ),
        Keep::Cond(k) => {
            if keys.is_empty() {
                k.replace("PARTITION BY {keys}", "")
            } else {
                k.replace("{keys}", keys)
            }
        }
    };
    let key_on = |a: &str, b: &str| {
        if keys.is_empty() {
            String::new()
        } else {
            format!("({a}.{keys} IS NOT DISTINCT FROM {b}.{keys}) AND ")
        }
    };
    let other_from = format!(
        "SELECT {}CAST({} AS DOUBLE) AS ow FROM t2 r JOIN cats k ON {}(r.cat IS NOT DISTINCT FROM k.cat) WHERE k.kept IS NOT TRUE{}",
        kc("r."),
        c.weight,
        key_on("r", "k"),
        if keys.is_empty() {
            String::new()
        } else {
            format!(" GROUP BY r.{keys}")
        }
    );
    format!(
        "WITH t2 AS (SELECT *, {cat} AS cat FROM {table}),
         cats AS (SELECT {k}{cat} AS cat, CAST({weight} AS DOUBLE) AS w, {keep} AS kept FROM {table} GROUP BY {k}{cat}),
         oth AS ({other_from}),
         dropped AS (SELECT {k}count(*) AS n FROM cats WHERE kept IS NOT TRUE{dropped_by})
         SELECT t2.*, named_struct('value', CASE WHEN k.kept IS TRUE THEN t2.cat END,
                                   'other', CASE WHEN k.kept IS TRUE THEN NULL ELSE '{other}' END,
                                   'weight', CASE WHEN k.kept IS TRUE THEN k.w ELSE o.ow END,
                                   'category_count', CASE WHEN k.kept IS TRUE THEN 1 ELSE dc.n END) AS l
         FROM t2 JOIN cats k ON {}(t2.cat IS NOT DISTINCT FROM k.cat)
         LEFT JOIN oth o ON {}
         LEFT JOIN dropped dc ON {}",
        key_on("t2", "k"),
        if keys.is_empty() { "true".to_string() } else { format!("t2.{keys} IS NOT DISTINCT FROM o.{keys}") },
        if keys.is_empty() { "true".to_string() } else { format!("t2.{keys} IS NOT DISTINCT FROM dc.{keys}") },
        dropped_by = if keys.is_empty() { String::new() } else { format!(" GROUP BY {keys}") },
        cat = c.category,
        k = kc(""),
        weight = c.weight,
        other = c.other,
    )
}

/// The rows as sorted strings of the given columns.
async fn rows(ctx: &SessionContext, sql: &str, columns: &str) -> Result<Vec<String>> {
    let batches = ctx
        .sql(&format!("SELECT {columns} FROM ({sql})"))
        .await?
        .collect()
        .await?;
    let mut out = Vec::new();
    for b in &batches {
        for i in 0..b.num_rows() {
            out.push(
                b.columns()
                    .iter()
                    .map(|c: &ArrayRef| {
                        if c.is_null(i) {
                            "NULL".into()
                        } else {
                            array_value_to_string(c, i).unwrap()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
        }
    }
    out.sort();
    Ok(out)
}

const COLUMNS: &str = "region, port, tons, l['value'] AS value, l['other'] AS other, round(l['weight'], 9) AS weight, l['category_count'] AS category_count";

#[tokio::test]
async fn hand_sql() -> Result<()> {
    let rules = unfused().await?;
    let plain = context(session(false)).await?;
    for c in cases() {
        for (over, keys) in WINDOWS {
            let ours = rows(&rules, &lump_query("c", &c, over), COLUMNS).await?;
            let hand = rows(&rules, &hand_query("c", &c, keys), COLUMNS).await?;
            assert_eq!(ours, hand, "{} {over}", c.name);
            if matches!(c.keep, Keep::N(_)) && c.weight == "count(*)" {
                let eval = rows(&plain, &lump_query("c", &c, over), COLUMNS).await?;
                assert_eq!(eval, hand, "{} {over}, the window evaluator", c.name);
            }
        }
    }
    Ok(())
}

/// Aggregates grouped by lumps: the fused plan (one pass) against the unfused plan (the lump on each
/// row, then the aggregate) and against the hand-written SQL.
#[tokio::test]
async fn aggregates() -> Result<()> {
    let fused = context(session(true)).await?;
    let joined = unfused().await?;
    let aggs = "count(*) AS n, count(tons) AS nt, sum(tons) AS s, min(tons) AS lo, max(tons) AS hi, avg(tons) AS mean, var_samp(tons) AS vs, stddev_pop(tons) AS sp";
    let cols = "CAST(l['value'] AS VARCHAR) AS v, l['other'] AS o, round(l['weight'], 9) AS w, l['category_count'] AS c, n, nt, round(s, 9) AS s, lo, hi, round(mean, 9) AS mean, round(vs, 9) AS vs, round(sp, 9) AS sp";
    for c in cases() {
        for (over, keys) in WINDOWS {
            for extra in ["", ", region"] {
                let grouped =
                    |q: String| format!("SELECT l{extra}, {aggs} FROM ({q}) GROUP BY l{extra}");
                let cols = format!("{cols}{}", if extra.is_empty() { "" } else { ", region" });
                let what = format!("{} {over}{extra}", c.name);
                let a = rows(&fused, &grouped(lump_query("c", &c, over)), &cols).await?;
                assert_eq!(
                    a,
                    rows(&joined, &grouped(lump_query("c", &c, over)), &cols).await?,
                    "{what}: fused and unfused"
                );
                assert_eq!(
                    a,
                    rows(&joined, &grouped(hand_query("c", &c, keys)), &cols).await?,
                    "{what}: fused and hand"
                );
                assert!(
                    fuses(&fused, &grouped(lump_query("c", &c, over))).await?,
                    "{what} does not fuse"
                );
            }
        }
    }
    Ok(())
}

async fn fuses(ctx: &SessionContext, sql: &str) -> Result<bool> {
    let plan = ctx.sql(&format!("EXPLAIN {sql}")).await?.collect().await?;
    Ok(
        datafusion::arrow::util::pretty::pretty_format_batches(&plan)?
            .to_string()
            .contains("_merge("),
    )
}

/// The number of categories of each lump value: 1 for a kept category, and for the Other
/// group the number of categories that the lump drops in its window partition, where a NULL
/// category is one. In region N, the 2 largest of the ports a, b, c, 'Other', and NULL are a
/// and b; in region S, of a, b, c, d, and e, they are b and c; the NULL region has 2 ports.
#[tokio::test]
async fn category_count() -> Result<()> {
    let sql = "SELECT *, lump(port, 'n', 2) OVER (PARTITION BY region) AS l FROM c";
    let cols = "region, l['other'] AS other, l['category_count'] AS category_count";
    let want = [
        "N | NULL | 1",
        "N | Other | 3",
        "NULL | NULL | 1",
        "S | NULL | 1",
        "S | Other | 3",
    ];
    // Through the rule, and through the window's own evaluator.
    for ctx in [unfused().await?, context(session(false)).await?] {
        let mut got = rows(&ctx, sql, cols).await?;
        got.dedup();
        assert_eq!(got, want);
    }
    Ok(())
}

#[tokio::test]
async fn errors() -> Result<()> {
    let rules = context(session(true)).await?;
    let plain = context(session(false)).await?;
    let errors = [
        (
            &rules,
            "lump(port) OVER ()",
            "lump needs exactly one of the options 'n' and 'keep'",
        ),
        (
            &rules,
            "lump(port, 'n', 2, 'keep', 'true') OVER ()",
            "lump needs exactly one of the options 'n' and 'keep'",
        ),
        (
            &rules,
            "lump(port, 'n', 2, 'other', false) OVER ()",
            "lump cannot remove rows, so 'other' cannot be false",
        ),
        (
            &rules,
            "lump(port, 'top', 2) OVER ()",
            "unknown lump option 'top'",
        ),
        (
            &rules,
            "lump(port, 'n', 0) OVER ()",
            "the lump option 'n' must be a positive integer, not 0",
        ),
        (
            &rules,
            "lump(port, 'n', 2, 'weight', 'sum(') OVER ()",
            "the lump option 'weight' is not an expression",
        ),
        (
            &rules,
            "lump(port, 'keep', 'sum(nosuch) > 1') OVER ()",
            "No field named nosuch",
        ),
        (
            &rules,
            "lump(port, 'keep', 'tons > 5') OVER ()",
            "must appear in the GROUP BY clause or must be part of an aggregate function",
        ),
        (
            &rules,
            "lump(port, 'n', 2) OVER (ORDER BY tons)",
            "the window of lump cannot have an ORDER BY",
        ),
        (
            &rules,
            "lump(port, 'keep', 'count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY port)') OVER ()",
            "remove its PARTITION BY",
        ),
        (
            &rules,
            "lump(port, 'keep', 'count(*) >= 0.2 * sum(count(*))') OVER ()",
            "`sum(count(*))` is an aggregate of aggregates; in the 'keep' of lump, an aggregate over the categories is a window, such as `sum(count(*)) OVER ()`",
        ),
        (
            &rules,
            "lump(port, 'keep', 'max(sum(count(*))) > 1') OVER ()",
            "is an aggregate of aggregates",
        ),
        (
            &rules,
            "lump(port, 'n', 2, 'weight', 'sum(count(*))') OVER ()",
            "cannot be an aggregate of aggregates",
        ),
        (
            &plain,
            "lump(port, 'n', 2, 'weight', 'sum(tons)') OVER ()",
            "needs the lump rewrite rule",
        ),
        (
            &rules,
            "lump(port, 'n', 2, 'weight', 'sum(tons) garbage') OVER ()",
            "the lump option 'weight' is not an expression",
        ),
        (
            &rules,
            "lump(port, 'n', 2, 'weight', 'sum(tons) / sum(sum(tons)) OVER ()') OVER ()",
            "it cannot have a window",
        ),
        (
            &rules,
            "lump(port, 'keep', 'rank() OVER (ranked ORDER BY port) <= 2') OVER ()",
            "the window `ranked` in the 'keep' of lump has an ORDER BY already",
        ),
        (
            &rules,
            "lump(port, 'keep', 'rank() OVER (ranked PARTITION BY port) <= 2') OVER ()",
            "remove its PARTITION BY",
        ),
        (
            &rules,
            "lump(port, 'keep', 'rank() OVER w <= 2') OVER ()",
            "unknown window `w` in the 'keep' of lump; its windows are `ranked`",
        ),
        (
            &rules,
            "lump(port, 'keep', 'sum(count(*)) OVER (ROWS UNBOUNDED PRECEDING) < 10') OVER ()",
            "a window frame in the 'keep' of lump needs an order",
        ),
        (
            &rules,
            "lump(port, 'n', 2, 'by', 'sum(tons)') OVER ()",
            "unknown lump option 'by'",
        ),
        (
            &rules,
            "lump(port, 'keep', 'count(*)') OVER ()",
            "the 'keep' of lump must be a condition, not an expression of type Int64",
        ),
        (
            &rules,
            "lump(DISTINCT port, 'n', 1) OVER ()",
            "lump takes no DISTINCT",
        ),
        (
            &rules,
            "lump(port, 'n', 1) OVER (ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)",
            "it cannot have a frame",
        ),
    ];
    for (ctx, call, message) in errors {
        let sql = format!("SELECT *, {call} AS l FROM c");
        let e = common::error(ctx, &sql)
            .await
            .unwrap_or_else(|| panic!("no error: {call}"));
        assert!(e.contains(message), "{call}: {e}");
    }
    Ok(())
}

/// A window node with a lump and other functions, two lumps in one window node, and inputs
/// that the rules must not read two times.
#[tokio::test]
async fn mixed_and_volatile() -> Result<()> {
    let rules = unfused().await?;
    let cols = "region, port, tons, l['value'] AS value, l['other'] AS other, round(l['weight'], 9) AS weight, l['category_count'] AS category_count, t";
    let mixed =
        "SELECT *, lump(port, 'keep', 'count(*) >= 2') OVER () AS l, sum(tons) OVER () AS t FROM c";
    let single = "SELECT *, lump(port, 'keep', 'count(*) >= 2') OVER () AS l, (SELECT sum(tons) FROM c) AS t FROM c";
    let a = rows(&rules, mixed, cols).await?;
    let b = rows(&rules, single, cols).await?;
    assert_eq!(a.len(), 19);
    assert_eq!(a, b);

    let two = "SELECT *, lump(port, 'n', 1) OVER () AS l, lump(port, 'keep', 'sum(tons) > 5') OVER () AS m FROM c";
    // Selecting a field of each, such as `l['other']`, fails in DataFusion 55.2's
    // push_down_leaf_projections with an ambiguous reference.
    let got = rows(&rules, two, "port, l, m").await?;
    assert_eq!(got.len(), 19);

    // With 'keep' or 'weight', a volatile input is an error; with 'n', the window evaluator reads
    // the input once.
    let e = common::error(&rules, "SELECT *, lump(port, 'keep', 'count(*) >= 2') OVER () AS l FROM (SELECT * FROM c WHERE random() < 2)").await;
    assert!(e.is_some_and(|e| e.contains("cannot have a volatile function")));
    let n = rules
        .sql("SELECT *, lump(port, 'n', 2) OVER () AS l FROM (SELECT * FROM c WHERE random() < 2)")
        .await?
        .count()
        .await?;
    assert_eq!(n, 19);
    Ok(())
}

/// The fusion falls back to the lump on each row where it cannot fuse. A `by` with a
/// subquery fuses the aggregate, and its decisions read the input again.
#[tokio::test]
async fn aggregates_that_do_not_fuse() -> Result<()> {
    let fused = context(session(true)).await?;
    let joined = unfused().await?;
    for (grouped, fuse) in [
        (
            "SELECT l, count(DISTINCT tons) AS n FROM (SELECT *, lump(port, 'n', 2) OVER () AS l FROM c) GROUP BY l",
            false,
        ),
        (
            "SELECT l, count(NULL) AS n FROM (SELECT *, lump(port, 'n', 2) OVER () AS l FROM c) GROUP BY l",
            false,
        ),
        (
            "SELECT l, sum(tons) FILTER (WHERE tons > 2) AS n FROM (SELECT *, lump(port, 'n', 2) OVER () AS l FROM c) GROUP BY l",
            false,
        ),
        (
            "SELECT l, count(*) AS n FROM (SELECT *, lump(port, 'n', 2, 'weight', 'sum(tons) / (SELECT sum(tons) FROM __input)') OVER () AS l FROM c) GROUP BY l",
            true,
        ),
        (
            "SELECT l, count(1) AS n FROM (SELECT *, lump(port, 'n', 2) OVER () AS l FROM c) GROUP BY l",
            true,
        ),
    ] {
        let cols = "CAST(l['value'] AS VARCHAR) AS v, l['other'] AS o, l['category_count'] AS c, n";
        assert_eq!(
            rows(&fused, grouped, cols).await?,
            rows(&joined, grouped, cols).await?,
            "{grouped}"
        );
        assert_eq!(fuses(&fused, grouped).await?, fuse, "{grouped}");
    }
    Ok(())
}
