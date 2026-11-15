//! `lump`: each case through [`lump::plan`] against hand-written SQL in the shape of its plan (a
//! grouped SELECT of the categories, joined back to the rows), aggregates grouped by lumps, and
//! the errors of the plan. Each case builds a [`LumpCall`]: the category, the partition, the
//! weight, and the conditions of `keep` are expressions in DataFusion's SQL over the rows of `c`,
//! and the conditions over the window `ranked` are built with [`lump::ranked`]. A lump beside
//! other windows of one projection plans through [`plan_windows`].

mod common;

use common::{session, view};

use avenger_datafusion_transforms::lump::{self, Keep, LumpCall, Nulls};
use avenger_datafusion_transforms::plan_windows;
use datafusion::arrow::array::{Array, ArrayRef};
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::common::{Column, Result, ScalarValue};
use datafusion::functions_aggregate::count::count_udaf;
use datafusion::functions_aggregate::sum::sum_udaf;
use datafusion::functions_window::rank::{percent_rank_udwf, rank_udwf};
use datafusion::functions_window::row_number::row_number_udwf;
use datafusion::logical_expr::expr::WindowFunction;
use datafusion::logical_expr::{
    Expr, LogicalPlan, LogicalPlanBuilder, SortExpr, WindowFrame, WindowFrameBound,
    WindowFrameUnits, WindowFunctionDefinition, lit,
};
use datafusion::prelude::SessionContext;

/// The input of every lump.
const INPUT: &str = "SELECT * FROM c";

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

/// The parts of a call over the rows of `c`, with its expressions in DataFusion's SQL.
struct Call {
    category: &'static str,
    /// The weight, or `count(*)` without one.
    weight: Option<&'static str>,
    keep: Kept,
    other: &'static str,
}

/// A call of the category `port` and the label `Other`.
fn port(weight: Option<&'static str>, keep: Kept) -> Call {
    Call {
        category: "port",
        weight,
        keep,
        other: "Other",
    }
}

/// A case: a call, and the decision of the hand-written SQL.
struct Case {
    name: &'static str,
    call: Call,
    hand: Hand,
}

/// The `keep` of a call.
enum Kept {
    /// [`Keep::First`].
    First(i64),
    /// A condition in DataFusion's SQL.
    Sql(&'static str),
    /// A condition over the window `ranked`, of the order of [`lump::ranked`].
    Ranked(fn(Vec<SortExpr>) -> Expr),
}

/// The decision of each category in the hand-written SQL: the first `n` categories in weight
/// order, or a condition, where `{keys}` stands for the partition keys.
enum Hand {
    First(i64),
    Cond(&'static str),
}

/// The window `function` of `args` over the categories, in the order `order`, over the frame
/// `frame`.
fn over(
    function: impl Into<WindowFunctionDefinition>,
    args: Vec<Expr>,
    order: Vec<SortExpr>,
    frame: WindowFrame,
) -> Expr {
    let mut window = WindowFunction::new(function, args);
    window.params.order_by = order;
    window.params.window_frame = frame;
    Expr::from(window)
}

/// The frame from the first row to the current row.
fn rows_to_current() -> WindowFrame {
    WindowFrame::new_bounds(
        WindowFrameUnits::Rows,
        WindowFrameBound::Preceding(ScalarValue::UInt64(None)),
        WindowFrameBound::CurrentRow,
    )
}

/// The frame of a window with an order and no frame of its own.
fn ordered() -> WindowFrame {
    WindowFrame::new(Some(false))
}

/// The frame of all rows of the partition.
fn all() -> WindowFrame {
    WindowFrame::new(None)
}

fn cases() -> Vec<Case> {
    use Hand::Cond;
    use Kept::*;
    vec![
        Case {
            name: "n 2",
            call: port(None, First(2)),
            hand: Hand::First(2),
        },
        Case {
            name: "n 2 by sum",
            call: port(Some("sum(tons)"), First(2)),
            hand: Hand::First(2),
        },
        Case {
            name: "keep a share",
            call: port(None, Sql("count(*) >= 0.2 * sum(count(*)) OVER ()")),
            hand: Cond("count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY {keys})"),
        },
        Case {
            name: "keep two measures",
            call: port(None, Sql("sum(tons) > 10 AND count(*) >= 2")),
            hand: Cond("sum(tons) > 10 AND count(*) >= 2"),
        },
        Case {
            name: "n 2 by avg (second pass)",
            call: port(Some("avg(tons)"), First(2)),
            hand: Hand::First(2),
        },
        Case {
            name: "by max, keep max",
            call: port(Some("max(tons)"), Sql("max(tons) >= 5")),
            hand: Cond("max(tons) >= 5"),
        },
        Case {
            name: "other label",
            call: Call {
                other: "Rest",
                ..port(None, First(1))
            },
            hand: Hand::First(1),
        },
        Case {
            name: "NULL condition drops",
            call: port(None, Sql("CASE WHEN port = 'b' THEN NULL ELSE true END")),
            hand: Cond("CASE WHEN port = 'b' THEN NULL ELSE true END"),
        },
        Case {
            name: "category expression",
            call: Call {
                category: "lower(port)",
                ..port(None, First(2))
            },
            hand: Hand::First(2),
        },
        Case {
            name: "Pareto cut",
            call: port(
                None,
                Sql(
                    "sum(count(*)) OVER (ORDER BY count(*) DESC, port ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER ()",
                ),
            ),
            hand: Cond(
                "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC, port ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
            ),
        },
        Case {
            name: "keep median (no state)",
            call: port(None, Sql("median(tons) >= 2")),
            hand: Cond("median(tons) >= 2"),
        },
        Case {
            name: "by stddev",
            call: port(Some("stddev_pop(tons)"), First(2)),
            hand: Hand::First(2),
        },
        Case {
            name: "ranked: top 2 by weight",
            call: port(
                Some("sum(tons)"),
                Ranked(|ranked| {
                    over(row_number_udwf(), vec![], ranked, ordered()).lt_eq(lit(2i64))
                }),
            ),
            hand: Hand::First(2),
        },
        Case {
            name: "ranked: ties",
            call: port(
                Some("sum(tons)"),
                Ranked(|ranked| over(rank_udwf(), vec![], ranked, ordered()).lt_eq(lit(2i64))),
            ),
            hand: Cond(
                "rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 2",
            ),
        },
        Case {
            name: "ranked: Pareto cut",
            call: port(
                None,
                Ranked(|ranked| {
                    let running = over(
                        sum_udaf(),
                        vec![lump::count_star()],
                        ranked,
                        rows_to_current(),
                    );
                    let total = over(sum_udaf(), vec![lump::count_star()], vec![], all());
                    (running - lump::count_star()).lt(lit(0.6) * total)
                }),
            ),
            hand: Cond(
                "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
            ),
        },
        Case {
            name: "ranked: bottom 2",
            call: port(
                None,
                Ranked(|ranked| {
                    let categories = over(count_udaf(), vec![lit(1i64)], vec![], all());
                    over(row_number_udwf(), vec![], ranked, ordered()).gt(categories - lit(2i64))
                }),
            ),
            hand: Cond(
                "row_number() OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST) > count(*) OVER (PARTITION BY {keys}) - 2",
            ),
        },
        Case {
            name: "ranked: percent rank",
            call: port(
                Some("max(tons)"),
                Ranked(|ranked| {
                    over(percent_rank_udwf(), vec![], ranked, ordered()).lt_eq(lit(0.5))
                }),
            ),
            hand: Cond(
                "percent_rank() OVER (PARTITION BY {keys} ORDER BY max(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 0.5",
            ),
        },
        Case {
            name: "rank",
            call: port(None, Sql("rank() OVER (ORDER BY sum(tons) DESC) <= 2")),
            hand: Cond("rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC) <= 2"),
        },
    ]
}

/// The partition keys: none, and the region.
const KEYS: [&str; 2] = ["", "region"];

/// The modes of the NULL category.
const NULLS: [Nulls; 3] = [Nulls::Keep, Nulls::Other, Nulls::Drop];

/// The call `c` over the partitions of `keys`, with the NULL category `nulls`.
async fn call(ctx: &SessionContext, c: &Call, keys: &str, nulls: Nulls) -> Result<LumpCall> {
    let category = common::expr(ctx, INPUT, c.category).await?;
    let weight = match c.weight {
        Some(weight) => Some(common::expr(ctx, INPUT, weight).await?),
        None => None,
    };
    let keep = match c.keep {
        Kept::First(n) => Keep::First(n),
        Kept::Sql(condition) => Keep::Condition(common::expr(ctx, INPUT, condition).await?),
        Kept::Ranked(condition) => {
            let weight = weight.clone().unwrap_or_else(lump::count_star);
            Keep::Condition(condition(lump::ranked(&weight, &category)))
        }
    };
    let partition = match keys {
        "" => Vec::new(),
        keys => vec![common::expr(ctx, INPUT, keys).await?],
    };
    Ok(LumpCall {
        category,
        partition,
        weight,
        keep,
        other: c.other.to_string(),
        null_category: nulls,
    })
}

/// The rows of `c` with their lump by `call` as the column `l`, as a query of the view `lumped`.
async fn lumped(ctx: &SessionContext, call: LumpCall) -> Result<String> {
    view(ctx, "lumped", INPUT, |plan| {
        lump::plan(plan, call, Column::from_name("l"))
    })
    .await?;
    Ok("SELECT * FROM lumped".to_string())
}

/// `input` with a column for each of `windows`, under its name: `plan_windows` plans the lumps,
/// and a window node over its plan the other windows.
fn windowed(input: LogicalPlan, windows: Vec<(&str, Expr)>) -> Result<LogicalPlan> {
    let columns = input.schema().columns().into_iter().map(Expr::Column);
    let (plan, rest) = plan_windows(input, windows.iter().map(|(_, w)| w.clone()).collect())?;
    let plan = match rest.is_empty() {
        true => plan,
        false => LogicalPlanBuilder::window_plan(plan, rest)?,
    };
    let named = windows.iter().map(|(name, window)| {
        Expr::Column(Column::from_name(window.schema_name().to_string())).alias(*name)
    });
    LogicalPlanBuilder::from(plan)
        .project(columns.chain(named))?
        .build()
}

/// Hand-written SQL with the same result: the categories, the decision, the Other weight,
/// and the join back to the rows. The categories that are not NULL are ranked and decided
/// alone, and the NULL category is kept, in the Other group, or without a lump by `nulls`.
fn hand_query(table: &str, c: &Case, keys: &str, nulls: Nulls) -> String {
    let null_kept = match nulls {
        Nulls::Keep => "true",
        Nulls::Other => "false",
        Nulls::Drop => "CAST(NULL AS BOOLEAN)",
    };
    let weight = c.call.weight.unwrap_or("count(*)");
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
    let keep = match &c.hand {
        Hand::First(n) => format!(
            "row_number() OVER ({part} ORDER BY {weight} DESC NULLS LAST, {} ASC NULLS LAST) <= {n}",
            c.call.category
        ),
        Hand::Cond(k) => {
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
        "SELECT {}CAST({weight} AS DOUBLE) AS ow FROM t2 r JOIN cats k ON {}(r.cat IS NOT DISTINCT FROM k.cat) WHERE k.kept IS FALSE{}",
        kc("r."),
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
        cat = c.call.category,
        k = kc(""),
        other = c.call.other,
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
    let ctx = context(session()).await?;
    for c in cases() {
        for keys in KEYS {
            for nulls in NULLS {
                let lump = lumped(&ctx, call(&ctx, &c.call, keys, nulls).await?).await?;
                let ours = rows(&ctx, &lump, COLUMNS).await?;
                let hand = rows(&ctx, &hand_query("c", &c, keys, nulls), COLUMNS).await?;
                assert_eq!(ours, hand, "{} by '{keys}' {nulls:?}", c.name);
            }
        }
    }
    Ok(())
}

/// Aggregates grouped by lumps, against the hand-written SQL.
#[tokio::test]
async fn aggregates() -> Result<()> {
    let ctx = context(session()).await?;
    let aggs = "count(*) AS n, count(tons) AS nt, sum(tons) AS s, min(tons) AS lo, max(tons) AS hi, avg(tons) AS mean, var_samp(tons) AS vs, stddev_pop(tons) AS sp";
    let cols = "CAST(l['value'] AS VARCHAR) AS v, l['other'] AS o, round(l['weight'], 9) AS w, l['category_count'] AS c, n, nt, round(s, 9) AS s, lo, hi, round(mean, 9) AS mean, round(vs, 9) AS vs, round(sp, 9) AS sp";
    for c in cases() {
        for keys in KEYS {
            for extra in ["", ", region"] {
                for nulls in NULLS {
                    let grouped =
                        |q: String| format!("SELECT l{extra}, {aggs} FROM ({q}) GROUP BY l{extra}");
                    let cols = format!("{cols}{}", if extra.is_empty() { "" } else { ", region" });
                    let what = format!("{} by '{keys}'{extra} {nulls:?}", c.name);
                    let lump =
                        grouped(lumped(&ctx, call(&ctx, &c.call, keys, nulls).await?).await?);
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

/// The number of categories of each lump value: 1 for a kept category, and for the Other
/// group the number of categories that the lump lumps in its window partition. In region N,
/// the 2 largest of the ports a, b, c, and 'Other' are a and b, and the NULL port is a category
/// of its own, in the Other group with [`Nulls::Other`], or without a lump with [`Nulls::Drop`];
/// in region S, of a, b, c, d, and e, they are b and c; the NULL region has 2 ports.
#[tokio::test]
async fn category_count() -> Result<()> {
    let cols = "region, port IS NULL AS null_port, l IS NULL AS dropped, l['other'] AS other, \
                l['category_count'] AS category_count";
    let cases = [
        (
            Nulls::Keep,
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
            Nulls::Other,
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
            Nulls::Drop,
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
    let ctx = context(session()).await?;
    let n2 = port(None, Kept::First(2));
    for (nulls, want) in &cases {
        let lump = lumped(&ctx, call(&ctx, &n2, "region", *nulls).await?).await?;
        let mut got = rows(&ctx, &lump, cols).await?;
        got.dedup();
        assert_eq!(&got, want, "{nulls:?}");
    }
    Ok(())
}

/// The errors of the plan, for options that a front end planned.
#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context(session()).await?;
    let errors = [
        (
            Some("sum(tons) / sum(sum(tons)) OVER ()"),
            Kept::First(2),
            "the weight of lump is the weight of one category, so it has no window",
        ),
        (
            Some("tons"),
            Kept::First(2),
            "the weight of lump is an aggregate of the rows of a category",
        ),
        (
            None,
            Kept::Sql("count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY port)"),
            "takes no PARTITION BY",
        ),
        (
            None,
            Kept::Sql("count(*)"),
            "the keep of lump is a condition",
        ),
    ];
    for (weight, keep, message) in errors {
        let call = call(&ctx, &port(weight, keep), "", Nulls::Keep).await?;
        let input = common::input(&ctx, INPUT).await?;
        let e = lump::plan(input, call, Column::from_name("l"))
            .err()
            .unwrap_or_else(|| panic!("no error: {weight:?}"))
            .to_string();
        assert!(e.contains(message), "{weight:?}: {e}");
    }
    Ok(())
}

/// A lump beside a window over the rows, and two lumps over one input, in one projection.
#[tokio::test]
async fn mixed() -> Result<()> {
    let ctx = context(session()).await?;
    let cols = "region, port, tons, l['value'] AS value, l['other'] AS other, round(l['weight'], 9) AS weight, l['category_count'] AS category_count, t";
    let at_least_2 = port(None, Kept::Sql("count(*) >= 2"));
    let lump = call(&ctx, &at_least_2, "", Nulls::Keep).await?;
    let total = common::expr(&ctx, INPUT, "sum(tons) OVER ()").await?;
    view(&ctx, "mixed", INPUT, |plan| {
        windowed(
            plan,
            vec![("l", lump::window(lump.clone(), "lump_l")), ("t", total)],
        )
    })
    .await?;
    let a = rows(&ctx, "SELECT * FROM mixed", cols).await?;
    let single = lumped(&ctx, lump).await?;
    let b = rows(
        &ctx,
        &format!("SELECT *, (SELECT sum(tons) FROM c) AS t FROM ({single})"),
        cols,
    )
    .await?;
    assert_eq!(a.len(), 19);
    assert_eq!(a, b);

    let n1 = call(&ctx, &port(None, Kept::First(1)), "", Nulls::Keep).await?;
    let over_5 = port(None, Kept::Sql("sum(tons) > 5"));
    let over_5 = call(&ctx, &over_5, "", Nulls::Keep).await?;
    view(&ctx, "two", INPUT, |plan| {
        windowed(
            plan,
            vec![
                ("l", lump::window(n1, "lump_l")),
                ("m", lump::window(over_5, "lump_m")),
            ],
        )
    })
    .await?;
    let got = rows(
        &ctx,
        "SELECT * FROM two",
        "port, l['other'] AS lo, m['other'] AS mo",
    )
    .await?;
    assert_eq!(got.len(), 19);
    Ok(())
}
