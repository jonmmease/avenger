//! `lump`: each case through [`lump::plan`] against hand-written SQL in the shape of its plan (a
//! grouped SELECT of the categories, joined back to the rows), aggregates grouped by lumps, and
//! the errors of the plan. The calls are written `lump(category, name => value, ...) OVER
//! (PARTITION BY ...)`, which the tests' adapter plans.

mod common;

use common::{calls, session};

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

/// A case: the category, the named arguments, and their parts for the hand-written SQL.
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
            options: "n => 2",
            keep: N(2),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "n 2 by sum",
            category: "port",
            options: "n => 2, weight => sum(tons)",
            keep: N(2),
            weight: "sum(tons)",
            other: "Other",
        },
        Case {
            name: "keep a share",
            category: "port",
            options: "keep => count(*) >= 0.2 * sum(count(*)) OVER ()",
            keep: Cond("count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY {keys})"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "keep two measures",
            category: "port",
            options: "keep => sum(tons) > 10 AND count(*) >= 2",
            keep: Cond("sum(tons) > 10 AND count(*) >= 2"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "n 2 by avg (second pass)",
            category: "port",
            options: "n => 2, weight => avg(tons)",
            keep: N(2),
            weight: "avg(tons)",
            other: "Other",
        },
        Case {
            name: "by max, keep max",
            category: "port",
            options: "weight => max(tons), keep => max(tons) >= 5",
            keep: Cond("max(tons) >= 5"),
            weight: "max(tons)",
            other: "Other",
        },
        Case {
            name: "other label",
            category: "port",
            options: "n => 1, other => 'Rest'",
            keep: N(1),
            weight: "count(*)",
            other: "Rest",
        },
        Case {
            name: "NULL condition drops",
            category: "port",
            options: "keep => CASE WHEN port = 'b' THEN NULL ELSE true END",
            keep: Cond("CASE WHEN port = 'b' THEN NULL ELSE true END"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "category expression",
            category: "lower(port)",
            options: "n => 2",
            keep: N(2),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "Pareto cut",
            category: "port",
            options: "keep => sum(count(*)) OVER (ORDER BY count(*) DESC, port ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER ()",
            keep: Cond(
                "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC, port ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
            ),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "keep median (no state)",
            category: "port",
            options: "keep => median(tons) >= 2",
            keep: Cond("median(tons) >= 2"),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "by stddev",
            category: "port",
            options: "n => 2, weight => stddev_pop(tons)",
            keep: N(2),
            weight: "stddev_pop(tons)",
            other: "Other",
        },
        Case {
            name: "ranked: top 2 by weight",
            category: "port",
            options: "weight => sum(tons), keep => row_number() OVER ranked <= 2",
            keep: N(2),
            weight: "sum(tons)",
            other: "Other",
        },
        Case {
            name: "ranked: ties",
            category: "port",
            options: "weight => sum(tons), keep => rank() OVER ranked <= 2",
            keep: Cond(
                "rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 2",
            ),
            weight: "sum(tons)",
            other: "Other",
        },
        Case {
            name: "ranked: Pareto cut",
            category: "port",
            options: "keep => sum(count(*)) OVER (ranked ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER ()",
            keep: Cond(
                "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
            ),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "ranked: bottom 2",
            category: "port",
            options: "keep => row_number() OVER ranked > count(*) OVER () - 2",
            keep: Cond(
                "row_number() OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST) > count(*) OVER (PARTITION BY {keys}) - 2",
            ),
            weight: "count(*)",
            other: "Other",
        },
        Case {
            name: "ranked: percent rank",
            category: "port",
            options: "weight => max(tons), keep => percent_rank() OVER ranked <= 0.5",
            keep: Cond(
                "percent_rank() OVER (PARTITION BY {keys} ORDER BY max(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 0.5",
            ),
            weight: "max(tons)",
            other: "Other",
        },
        Case {
            name: "rank",
            category: "port",
            options: "keep => rank() OVER (ORDER BY sum(tons) DESC) <= 2",
            keep: Cond("rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC) <= 2"),
            weight: "count(*)",
            other: "Other",
        },
    ]
}

/// The window clause, and the partition keys.
const WINDOWS: [(&str, &str); 2] = [("OVER ()", ""), ("OVER (PARTITION BY region)", "region")];

/// The modes of the NULL category: the default, and each mode by name.
const NULLS: [Option<&str>; 3] = [None, Some("other"), Some("drop")];

/// The view of the lump of `c` over `table`, as a query.
async fn lump_query(
    ctx: &SessionContext,
    table: &str,
    c: &Case,
    over: &str,
    nulls: Option<&str>,
) -> Result<String> {
    let nulls = nulls.map_or(String::new(), |mode| format!(", null_category => '{mode}'"));
    let query = format!(
        "SELECT *, lump({}, {}{nulls}) {over} AS l FROM {table}",
        c.category, c.options
    );
    let view = calls::lump_view(ctx, &query).await?;
    Ok(format!("SELECT * FROM {view}"))
}

/// Hand-written SQL with the same result: the categories, the decision, the Other weight,
/// and the join back to the rows. The categories that are not NULL are ranked and decided
/// alone, and the NULL category is kept, in the Other group, or without a lump by `nulls`.
fn hand_query(table: &str, c: &Case, keys: &str, nulls: Option<&str>) -> String {
    let null_kept = match nulls {
        None | Some("keep") => "true",
        Some("other") => "false",
        _ => "CAST(NULL AS BOOLEAN)",
    };
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
        "SELECT {}CAST({} AS DOUBLE) AS ow FROM t2 r JOIN cats k ON {}(r.cat IS NOT DISTINCT FROM k.cat) WHERE k.kept IS FALSE{}",
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
         cats AS (SELECT {k}{cat} AS cat, CAST({weight} AS DOUBLE) AS w, coalesce({keep}, false) AS kept FROM {table} WHERE {cat} IS NOT NULL GROUP BY {k}{cat}
                  UNION ALL SELECT {k}{cat} AS cat, CAST({weight} AS DOUBLE) AS w, {null_kept} AS kept FROM {table} WHERE {cat} IS NULL GROUP BY {k}{cat}),
         oth AS ({other_from}),
         dropped AS (SELECT {k}count(*) AS n FROM cats WHERE kept IS FALSE{dropped_by})
         SELECT t2.*, CASE WHEN k.kept IS NOT NULL THEN named_struct(
                                   'value', CASE WHEN k.kept IS TRUE THEN t2.cat END,
                                   'other', CASE WHEN k.kept IS TRUE THEN NULL ELSE '{other}' END,
                                   'weight', CASE WHEN k.kept IS TRUE THEN k.w ELSE o.ow END,
                                   'category_count', CASE WHEN k.kept IS TRUE THEN 1 ELSE dc.n END) END AS l
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
    let ctx = context(session(true)).await?;
    for c in cases() {
        for (over, keys) in WINDOWS {
            for nulls in NULLS {
                let lump = lump_query(&ctx, "c", &c, over, nulls).await?;
                let ours = rows(&ctx, &lump, COLUMNS).await?;
                let hand = rows(&ctx, &hand_query("c", &c, keys, nulls), COLUMNS).await?;
                assert_eq!(ours, hand, "{} {over} {nulls:?}", c.name);
            }
        }
    }
    Ok(())
}

/// Aggregates grouped by lumps, against the hand-written SQL.
#[tokio::test]
async fn aggregates() -> Result<()> {
    let ctx = context(session(true)).await?;
    let aggs = "count(*) AS n, count(tons) AS nt, sum(tons) AS s, min(tons) AS lo, max(tons) AS hi, avg(tons) AS mean, var_samp(tons) AS vs, stddev_pop(tons) AS sp";
    let cols = "CAST(l['value'] AS VARCHAR) AS v, l['other'] AS o, round(l['weight'], 9) AS w, l['category_count'] AS c, n, nt, round(s, 9) AS s, lo, hi, round(mean, 9) AS mean, round(vs, 9) AS vs, round(sp, 9) AS sp";
    for c in cases() {
        for (over, keys) in WINDOWS {
            for extra in ["", ", region"] {
                for nulls in NULLS {
                    let grouped =
                        |q: String| format!("SELECT l{extra}, {aggs} FROM ({q}) GROUP BY l{extra}");
                    let cols = format!("{cols}{}", if extra.is_empty() { "" } else { ", region" });
                    let what = format!("{} {over}{extra} {nulls:?}", c.name);
                    let lump = grouped(lump_query(&ctx, "c", &c, over, nulls).await?);
                    assert_eq!(
                        rows(&ctx, &lump, &cols).await?,
                        rows(&ctx, &grouped(hand_query("c", &c, keys, nulls)), &cols).await?,
                        "{what}"
                    );
                }
            }
        }
    }
    Ok(())
}

/// An aggregate grouped by a lump, and by the lump's partition keys, reads its input once
/// through the fusion, and the plan of the lump alone reads it more than once.
#[tokio::test]
async fn an_aggregate_grouped_by_a_lump_reads_its_input_once() -> Result<()> {
    let ctx = context(session(true)).await?;
    let scans = |plan: &datafusion::logical_expr::LogicalPlan| {
        let mut n = 0;
        plan.apply(|node| {
            if matches!(node, datafusion::logical_expr::LogicalPlan::TableScan(_)) {
                n += 1;
            }
            Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
        })
        .unwrap();
        n
    };
    use datafusion::common::tree_node::TreeNode;
    for c in cases() {
        for (over, keys) in WINDOWS {
            let lump = lump_query(&ctx, "c", &c, over, None).await?;
            let extra = if keys.is_empty() {
                String::new()
            } else {
                format!(", {keys}")
            };
            let grouped = format!(
                "SELECT l{extra}, count(*) AS n, sum(tons) AS s, avg(tons) AS a FROM ({lump}) GROUP BY l{extra}"
            );
            let plan = ctx.sql(&grouped).await?.into_optimized_plan()?;
            assert_eq!(scans(&plan), 1, "{} {over}\n{plan}", c.name);
        }
    }
    // A median doesn't merge from states, so the lump plans as it does alone.
    let lump = lump_query(&ctx, "c", &cases()[0], "OVER ()", None).await?;
    let grouped = format!("SELECT l, median(tons) AS m FROM ({lump}) GROUP BY l");
    let plan = ctx.sql(&grouped).await?.into_optimized_plan()?;
    assert!(scans(&plan) > 1, "{plan}");
    Ok(())
}

/// The number of categories of each lump value: 1 for a kept category, and for the Other
/// group the number of categories that the lump lumps in its window partition. In region N,
/// the 2 largest of the ports a, b, c, and 'Other' are a and b, and the NULL port is a category
/// of its own, in the Other group with `null_category => 'other'`, or without a lump with
/// `null_category => 'drop'`; in region S, of a, b, c, d, and e, they are b and c; the NULL
/// region has 2 ports.
#[tokio::test]
async fn category_count() -> Result<()> {
    let cols = "region, port IS NULL AS null_port, l IS NULL AS dropped, l['other'] AS other, \
                l['category_count'] AS category_count";
    let cases = [
        (
            "",
            vec![
                "N | false | false | NULL | 1",
                "N | false | false | Other | 2",
                "N | true | false | NULL | 1",
                "NULL | false | false | NULL | 1",
                "S | false | false | NULL | 1",
                "S | false | false | Other | 3",
            ],
        ),
        (
            ", null_category => 'other'",
            vec![
                "N | false | false | NULL | 1",
                "N | false | false | Other | 3",
                "N | true | false | Other | 3",
                "NULL | false | false | NULL | 1",
                "S | false | false | NULL | 1",
                "S | false | false | Other | 3",
            ],
        ),
        (
            ", null_category => 'drop'",
            vec![
                "N | false | false | NULL | 1",
                "N | false | false | Other | 2",
                "N | true | true | NULL | NULL",
                "NULL | false | false | NULL | 1",
                "S | false | false | NULL | 1",
                "S | false | false | Other | 3",
            ],
        ),
    ];
    let ctx = context(session(true)).await?;
    for (nulls, want) in &cases {
        let view = calls::lump_view(
            &ctx,
            &format!("SELECT *, lump(port, n => 2{nulls}) OVER (PARTITION BY region) AS l FROM c"),
        )
        .await?;
        let mut got = rows(&ctx, &format!("SELECT * FROM {view}"), cols).await?;
        got.dedup();
        assert_eq!(&got, want, "{nulls}");
    }
    Ok(())
}

/// The errors of the plan, for options that a front end planned.
#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context(session(true)).await?;
    let errors = [
        (
            "lump(port, n => 2, weight => sum(tons) / sum(sum(tons)) OVER ()) OVER ()",
            "the weight of lump is the weight of one category, so it has no window",
        ),
        (
            "lump(port, n => 2, weight => tons) OVER ()",
            "the weight of lump is an aggregate of the rows of a category",
        ),
        (
            "lump(port, keep => count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY port)) OVER ()",
            "takes no PARTITION BY",
        ),
        (
            "lump(port, keep => count(*)) OVER ()",
            "the keep of lump is a condition",
        ),
    ];
    for (call, message) in errors {
        let e = calls::lump_view(&ctx, &format!("SELECT *, {call} AS l FROM c"))
            .await
            .err()
            .unwrap_or_else(|| panic!("no error: {call}"))
            .to_string();
        assert!(e.contains(message), "{call}: {e}");
    }
    Ok(())
}

/// A lump beside a window over the rows, and two lumps over one input.
#[tokio::test]
async fn mixed() -> Result<()> {
    let ctx = context(session(true)).await?;
    let cols = "region, port, tons, l['value'] AS value, l['other'] AS other, round(l['weight'], 9) AS weight, l['category_count'] AS category_count, t";
    let mixed = calls::lump_view(
        &ctx,
        "SELECT *, lump(port, keep => count(*) >= 2) OVER () AS l, sum(tons) OVER () AS t FROM c",
    )
    .await?;
    let a = rows(&ctx, &format!("SELECT * FROM {mixed}"), cols).await?;
    let single = calls::lump_view(
        &ctx,
        "SELECT *, lump(port, keep => count(*) >= 2) OVER () AS l, (SELECT sum(tons) FROM c) AS t FROM c",
    )
    .await?;
    let b = rows(&ctx, &format!("SELECT * FROM {single}"), cols).await?;
    assert_eq!(a.len(), 19);
    assert_eq!(a, b);

    let two = calls::lump_view(
        &ctx,
        "SELECT *, lump(port, n => 1) OVER () AS l, lump(port, keep => sum(tons) > 5) OVER () AS m FROM c",
    )
    .await?;
    let got = rows(
        &ctx,
        &format!("SELECT * FROM {two}"),
        "port, l['other'] AS lo, m['other'] AS mo",
    )
    .await?;
    assert_eq!(got.len(), 19);
    Ok(())
}
