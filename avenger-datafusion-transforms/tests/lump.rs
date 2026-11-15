//! `lump`: each case through [`lump::plan`] against hand-written SQL in the shape of its plan (a
//! grouped SELECT of the categories, joined back to the rows), aggregates grouped by lumps, and
//! the errors of the plan. Each case builds a [`LumpCall`]: the category, the partition, the
//! weight, and the conditions of `keep` are expressions in DataFusion's SQL over the rows of `c`.
//! The conditions over the window `ranked` write out its order, and one case builds that order
//! with [`lump::ranked`]. A lump beside other windows of one projection plans through
//! [`plan_windows`].

mod common;

use common::{exec, session, view};

use avenger_datafusion_transforms::lump::{self, Keep, LumpCall, Nulls};
use avenger_datafusion_transforms::plan_windows;
use datafusion::common::{Column, Result};
use datafusion::functions_window::expr_fn::row_number;
use datafusion::logical_expr::{Expr, ExprFunctionExt, LogicalPlan, LogicalPlanBuilder, lit};
use datafusion::prelude::SessionContext;

/// The input of every lump.
const INPUT: &str = "SELECT * FROM c";

/// A session with the table `c`.
async fn context() -> Result<SessionContext> {
    let ctx = session();
    exec(
        &ctx,
        "CREATE TABLE c (region VARCHAR, port VARCHAR, tons DOUBLE) AS VALUES
        ('N', 'a', 1.0), ('N', 'a', 2.0), ('N', 'a', 3.0), ('N', 'b', 4.0), ('N', 'b', 5.0),
        ('N', 'c', 6.0), ('N', 'Other', 0.5), ('N', NULL, 7.0), ('N', NULL, 1.5),
        ('S', 'a', 9.0), ('S', 'b', 1.0), ('S', 'b', 1.0), ('S', 'c', 2.0), ('S', 'c', NULL),
        ('S', 'd', 12.0), ('S', 'd', 0.5), ('S', 'e', 3.0),
        (NULL, 'a', 4.0), (NULL, 'z', 1.0)",
    )
    .await?;
    Ok(ctx)
}

/// The `keep` of a call.
#[derive(Clone, Copy)]
enum Kept {
    /// [`Keep::First`].
    First(i64),
    /// A condition in DataFusion's SQL, where `PARTITION BY {keys}` stands for the partition of
    /// the hand-written SQL. The call has no partition in its windows.
    Sql(&'static str),
    /// The first `n` categories by `row_number()` over the order of [`lump::ranked`].
    Ranked(i64),
}

use Kept::*;

/// A case: its name, and the category, weight, Other label, and `keep` of its call.
type Case = (
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
    Kept,
);

const CASES: [Case; 18] = [
    ("n 2", "port", None, "Other", First(2)),
    ("n 2 by sum", "port", Some("sum(tons)"), "Other", First(2)),
    (
        "keep a share",
        "port",
        None,
        "Other",
        Sql("count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY {keys})"),
    ),
    (
        "keep two measures",
        "port",
        None,
        "Other",
        Sql("sum(tons) > 10 AND count(*) >= 2"),
    ),
    (
        "n 2 by avg (second pass)",
        "port",
        Some("avg(tons)"),
        "Other",
        First(2),
    ),
    (
        "by max, keep max",
        "port",
        Some("max(tons)"),
        "Other",
        Sql("max(tons) >= 5"),
    ),
    ("other label", "port", None, "Rest", First(1)),
    (
        "NULL condition drops",
        "port",
        None,
        "Other",
        Sql("CASE WHEN port = 'b' THEN NULL ELSE true END"),
    ),
    (
        "category expression",
        "lower(port)",
        None,
        "Other",
        First(2),
    ),
    (
        "Pareto cut",
        "port",
        None,
        "Other",
        Sql(
            "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC, port ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
        ),
    ),
    (
        "keep median (no state)",
        "port",
        None,
        "Other",
        Sql("median(tons) >= 2"),
    ),
    (
        "by stddev",
        "port",
        Some("stddev_pop(tons)"),
        "Other",
        First(2),
    ),
    (
        "ranked: top 2 by weight",
        "port",
        Some("sum(tons)"),
        "Other",
        Ranked(2),
    ),
    (
        "ranked: ties",
        "port",
        Some("sum(tons)"),
        "Other",
        Sql(
            "rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 2",
        ),
    ),
    (
        "ranked: Pareto cut",
        "port",
        None,
        "Other",
        Sql(
            "sum(count(*)) OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST ROWS UNBOUNDED PRECEDING) - count(*) < 0.6 * sum(count(*)) OVER (PARTITION BY {keys})",
        ),
    ),
    (
        "ranked: bottom 2",
        "port",
        None,
        "Other",
        Sql(
            "row_number() OVER (PARTITION BY {keys} ORDER BY count(*) DESC NULLS LAST, port ASC NULLS LAST) > count(*) OVER (PARTITION BY {keys}) - 2",
        ),
    ),
    (
        "ranked: percent rank",
        "port",
        Some("max(tons)"),
        "Other",
        Sql(
            "percent_rank() OVER (PARTITION BY {keys} ORDER BY max(tons) DESC NULLS LAST, port ASC NULLS LAST) <= 0.5",
        ),
    ),
    (
        "rank",
        "port",
        None,
        "Other",
        Sql("rank() OVER (PARTITION BY {keys} ORDER BY sum(tons) DESC) <= 2"),
    ),
];

/// The partition keys: none, and the region.
const KEYS: [&str; 2] = ["", "region"];

/// The modes of the NULL category.
const NULLS: [Nulls; 3] = [Nulls::Keep, Nulls::Other, Nulls::Drop];

/// The call of `case` over the partitions of `keys`, with the NULL category `nulls`.
async fn call(ctx: &SessionContext, case: &Case, keys: &str, nulls: Nulls) -> Result<LumpCall> {
    let (_, category, weight, other, keep) = *case;
    let sql = async |sql: &str| common::expr(ctx, INPUT, sql).await;
    let category = sql(category).await?;
    let weight = match weight {
        Some(weight) => Some(sql(weight).await?),
        None => None,
    };
    let keep = match keep {
        First(n) => Keep::First(n),
        Sql(cond) => Keep::Condition(sql(&cond.replace("PARTITION BY {keys}", "")).await?),
        Ranked(n) => {
            let weight = weight.clone().unwrap_or_else(lump::count_star);
            let order = lump::ranked(&weight, &category);
            Keep::Condition(row_number().order_by(order).build()?.lt_eq(lit(n)))
        }
    };
    let partition = match keys {
        "" => Vec::new(),
        keys => vec![sql(keys).await?],
    };
    Ok(LumpCall {
        category,
        partition,
        weight,
        keep,
        other: other.to_string(),
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
    let named = (windows.iter())
        .map(|(name, w)| Expr::from(Column::from_name(w.schema_name().to_string())).alias(*name));
    LogicalPlanBuilder::from(plan)
        .project(columns.chain(named))?
        .build()
}

/// Hand-written SQL with the same result: the categories, the decision, the Other weight, and
/// the join back to the rows. The rows partition by `pk`, the keys or `true`. The categories that
/// are not NULL are ranked and decided alone, and the NULL category is kept, in the Other group,
/// or without a lump by `nulls`.
fn hand_query(case: &Case, keys: &str, nulls: Nulls) -> String {
    let (_, cat, weight, other, keep) = *case;
    let weight = weight.unwrap_or("count(*)");
    let pk = if keys.is_empty() { "true" } else { keys };
    let keep = match keep {
        Sql(cond) => cond.replace("{keys}", "pk"),
        First(n) | Ranked(n) => format!(
            "row_number() OVER (PARTITION BY pk ORDER BY {weight} DESC NULLS LAST, {cat} ASC NULLS LAST) <= {n}"
        ),
    };
    let null_kept = match nulls {
        Nulls::Keep => "true",
        Nulls::Other => "false",
        Nulls::Drop => "CAST(NULL AS BOOLEAN)",
    };
    format!(
        "WITH t AS (SELECT *, {pk} AS pk FROM c),
         cats AS (SELECT pk, {cat} AS cat, CAST({weight} AS DOUBLE) AS w, coalesce({keep}, false) AS kept FROM t WHERE {cat} IS NOT NULL GROUP BY pk, {cat}
                  UNION ALL SELECT pk, NULL, CAST({weight} AS DOUBLE), {null_kept} FROM t WHERE {cat} IS NULL GROUP BY pk),
         j AS (SELECT t.*, k.cat, k.w, k.kept FROM t JOIN cats k ON (t.pk IS NOT DISTINCT FROM k.pk) AND ({cat} IS NOT DISTINCT FROM k.cat)),
         o AS (SELECT pk, CAST({weight} AS DOUBLE) AS ow FROM j WHERE kept IS FALSE GROUP BY pk),
         d AS (SELECT pk, count(*) AS n FROM cats WHERE kept IS FALSE GROUP BY pk)
         SELECT j.*, CASE WHEN kept IS NOT NULL THEN named_struct(
                'value', CASE WHEN kept THEN j.cat END,
                'other', CASE WHEN kept THEN NULL ELSE '{other}' END,
                'weight', CASE WHEN kept THEN w ELSE o.ow END,
                'category_count', CASE WHEN kept THEN 1 ELSE d.n END) END AS l
         FROM j LEFT JOIN o ON j.pk IS NOT DISTINCT FROM o.pk LEFT JOIN d ON j.pk IS NOT DISTINCT FROM d.pk"
    )
}

/// The sorted rows of the columns `columns` of the query `sql`.
async fn rows(ctx: &SessionContext, sql: &str, columns: &str) -> Result<Vec<Vec<String>>> {
    common::rows(ctx, &format!("SELECT {columns} FROM ({sql})")).await
}

const COLUMNS: &str = "region, port, tons, l['value'] AS value, l['other'] AS other, round(l['weight'], 9) AS weight, l['category_count'] AS category_count";

#[tokio::test]
async fn hand_sql() -> Result<()> {
    let ctx = context().await?;
    for case in &CASES {
        for keys in KEYS {
            for nulls in NULLS {
                let lump = lumped(&ctx, call(&ctx, case, keys, nulls).await?).await?;
                let ours = rows(&ctx, &lump, COLUMNS).await?;
                let hand = rows(&ctx, &hand_query(case, keys, nulls), COLUMNS).await?;
                assert_eq!(ours, hand, "{} by '{keys}' {nulls:?}", case.0);
            }
        }
    }
    Ok(())
}

/// Aggregates grouped by lumps, against the hand-written SQL.
#[tokio::test]
async fn aggregates() -> Result<()> {
    let ctx = context().await?;
    let aggs = "count(*) AS n, count(tons) AS nt, sum(tons) AS s, min(tons) AS lo, max(tons) AS hi, avg(tons) AS mean, var_samp(tons) AS vs, stddev_pop(tons) AS sp";
    let cols = "CAST(l['value'] AS VARCHAR) AS v, l['other'] AS o, round(l['weight'], 9) AS w, l['category_count'] AS c, n, nt, round(s, 9) AS s, lo, hi, round(mean, 9) AS mean, round(vs, 9) AS vs, round(sp, 9) AS sp";
    for case in &CASES {
        for (keys, extra) in KEYS.into_iter().flat_map(|k| [(k, ""), (k, ", region")]) {
            let grouped = |q| format!("SELECT l{extra}, {aggs} FROM ({q}) GROUP BY l{extra}");
            let cols = format!("{cols}{extra}");
            for nulls in NULLS {
                let lump = grouped(lumped(&ctx, call(&ctx, case, keys, nulls).await?).await?);
                let hand = grouped(hand_query(case, keys, nulls));
                assert_eq!(
                    rows(&ctx, &lump, &cols).await?,
                    rows(&ctx, &hand, &cols).await?,
                    "{} by '{keys}'{extra} {nulls:?}",
                    case.0
                );
            }
        }
    }
    Ok(())
}

/// The joins of `plan`.
fn joins(plan: &LogicalPlan) -> usize {
    use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
    let mut n = 0;
    plan.apply(|node| {
        n += usize::from(matches!(node, LogicalPlan::Join(_)));
        Ok(TreeNodeRecursion::Continue)
    })
    .expect("a plan");
    n
}

/// An aggregate grouped by a lump, and by the lump's partition keys, merges the states of the
/// categories without joining the lump to the rows, which `aggregates` checks against the
/// hand-written SQL. An aggregate that doesn't merge from states, such as a median, joins.
#[tokio::test]
async fn an_aggregate_grouped_by_a_lump_merges_its_categories() -> Result<()> {
    let ctx = context().await?;
    for case in &CASES {
        for (keys, extra) in KEYS.into_iter().flat_map(|k| [(k, ""), (k, ", region")]) {
            let lump = lumped(&ctx, call(&ctx, case, keys, Nulls::Keep).await?).await?;
            let grouped = format!(
                "SELECT l{extra}, count(*) AS n, sum(tons) AS s, avg(tons) AS a FROM ({lump}) GROUP BY l{extra}"
            );
            let plan = ctx.sql(&grouped).await?.into_optimized_plan()?;
            let fused = extra.is_empty() || keys == "region";
            assert_eq!(
                joins(&plan) == 0,
                fused,
                "{} by '{keys}'{extra}\n{plan}",
                case.0
            );
        }
    }
    let lump = lumped(&ctx, call(&ctx, &CASES[0], "", Nulls::Keep).await?).await?;
    let grouped = format!("SELECT l, median(tons) AS m FROM ({lump}) GROUP BY l");
    let plan = ctx.sql(&grouped).await?.into_optimized_plan()?;
    assert!(joins(&plan) > 0, "{plan}");
    Ok(())
}

/// The number of categories of each lump value: 1 for a kept category, and for the Other
/// group the number of categories that the lump lumps in its window partition. In region N,
/// the 2 largest of the ports a, b, c, and 'Other' are a and b, and the NULL port is a category
/// of its own, in the Other group with [`Nulls::Other`], or without a lump with [`Nulls::Drop`].
/// In region S, of a, b, c, d, and e, they are b and c. The NULL region has 2 ports.
#[tokio::test]
async fn category_count() -> Result<()> {
    let cols = "region, port IS NULL AS null_port, l IS NULL AS dropped, l['other'] AS other, \
                l['category_count'] AS category_count";
    let cases = [
        (
            Nulls::Keep,
            [
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
            [
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
            [
                "N | false | false | NULL | 1",
                "N | false | false | Other | 2",
                "N | true | true | NULL | NULL",
                "NULL | false | false | NULL | 1",
                "S | false | false | NULL | 1",
                "S | false | false | Other | 3",
            ],
        ),
    ];
    let ctx = context().await?;
    let n2 = ("", "port", None, "Other", First(2));
    for (nulls, want) in cases {
        let lump = lumped(&ctx, call(&ctx, &n2, "region", nulls).await?).await?;
        let mut got: Vec<_> = (rows(&ctx, &lump, cols).await?.iter())
            .map(|r| r.join(" | "))
            .collect();
        got.sort();
        got.dedup();
        assert_eq!(got, want, "{nulls:?}");
    }
    Ok(())
}

/// The errors of the plan, for options that a front end planned.
#[tokio::test]
async fn errors() -> Result<()> {
    let ctx = context().await?;
    let errors = [
        (
            Some("sum(tons) / sum(sum(tons)) OVER ()"),
            First(2),
            "the weight of lump is the weight of one category, so it has no window",
        ),
        (
            Some("tons"),
            First(2),
            "the weight of lump is an aggregate of the rows of a category",
        ),
        (
            None,
            Sql("count(*) >= 0.2 * sum(count(*)) OVER (PARTITION BY port)"),
            "takes no PARTITION BY",
        ),
        (None, Sql("count(*)"), "the keep of lump is a condition"),
    ];
    for (weight, keep, message) in errors {
        let call = call(&ctx, &("", "port", weight, "Other", keep), "", Nulls::Keep).await?;
        let input = common::input(&ctx, INPUT).await?;
        let e = lump::plan(input, call, Column::from_name("l"));
        let e = e
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
    let ctx = context().await?;
    let port = |keep| ("", "port", None, "Other", keep);
    let lump = call(&ctx, &port(Sql("count(*) >= 2")), "", Nulls::Keep).await?;
    let total = common::expr(&ctx, INPUT, "sum(tons) OVER ()").await?;
    let windows = vec![("l", lump::window(lump.clone(), "lump_l")), ("t", total)];
    view(&ctx, "mixed", INPUT, |plan| windowed(plan, windows)).await?;
    let cols = format!("{COLUMNS}, t");
    let a = rows(&ctx, "SELECT * FROM mixed", &cols).await?;
    let single = lumped(&ctx, lump).await?;
    let single = format!("SELECT *, (SELECT sum(tons) FROM c) AS t FROM ({single})");
    assert_eq!(a.len(), 19);
    assert_eq!(a, rows(&ctx, &single, &cols).await?);

    let n1 = call(&ctx, &port(First(1)), "", Nulls::Keep).await?;
    let over_5 = call(&ctx, &port(Sql("sum(tons) > 5")), "", Nulls::Keep).await?;
    let windows = vec![
        ("l", lump::window(n1, "lump_l")),
        ("m", lump::window(over_5, "lump_m")),
    ];
    view(&ctx, "two", INPUT, |plan| windowed(plan, windows)).await?;
    let got = rows(
        &ctx,
        "SELECT * FROM two",
        "port, l['other'] AS lo, m['other'] AS mo",
    )
    .await?;
    assert_eq!(got.len(), 19);
    Ok(())
}
