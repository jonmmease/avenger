//! The page's decoder: a plan that datafusion-proto encodes natively decodes with the page's
//! decoder of each operator and expression that the plans of charts hold, and runs to the rows of
//! the native plan, so a change of DataFusion's messages or of its decoding hooks fails here. An
//! operator outside the set fails to decode with its name.

#![cfg(not(target_arch = "wasm32"))]

use std::{collections::BTreeSet, sync::Arc};

use avenger_physical_graph::{
    cut::{CutInputExec, CutScan},
    kept::{substitute, Kept},
    page::{decode_plan, encode_plan, task_context},
};
#[expect(
    deprecated,
    reason = "the page decodes the plans that DataFusion encodes"
)]
use datafusion::physical_plan::coalesce_batches::CoalesceBatchesExec;
use datafusion::{
    arrow::{
        array::{Float64Array, Int64Array, ListArray, RecordBatch, StringArray},
        datatypes::{DataType, Field, Int64Type, Schema},
        util::pretty::pretty_format_batches,
    },
    common::tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    datasource::{memory::DataSourceExec, MemTable},
    execution::SessionStateBuilder,
    physical_plan::{
        coalesce_partitions::CoalescePartitionsExec, collect, coop::CooperativeExec,
        empty::EmptyExec, execution_plan::reset_plan_states, limit::LocalLimitExec,
        repartition::RepartitionExec, sorts::sort_preserving_merge::SortPreservingMergeExec,
        ExecutionPlan, ExecutionPlanProperties, Partitioning,
    },
    prelude::{SessionConfig, SessionContext},
};

/// A session of one partition and without dynamic filters, as a graph plans a page's plans, that
/// plans selections' operators, with a table `t` of keys, values, measures, and lists, and a table
/// `u` of keys and weights.
fn session() -> SessionContext {
    let mut config = SessionConfig::new().with_target_partitions(1);
    let optimizer = &mut config.options_mut().optimizer;
    optimizer.enable_dynamic_filter_pushdown = false;
    optimizer.enable_topk_dynamic_filter_pushdown = false;
    optimizer.enable_join_dynamic_filter_pushdown = false;
    optimizer.enable_aggregate_dynamic_filter_pushdown = false;
    let state = SessionStateBuilder::new()
        .with_config(config)
        .with_default_features()
        .with_query_planner(Arc::new(avenger_selection_datafusion::QueryPlanner))
        .build();
    let mut ctx = SessionContext::new_with_state(state);
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    let t = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("k", DataType::Utf8, true),
            Field::new("v", DataType::Int64, true),
            Field::new("d", DataType::Float64, true),
            Field::new(
                "tags",
                DataType::List(Arc::new(Field::new_list_field(DataType::Int64, true))),
                true,
            ),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                Some("a"),
                Some("b"),
                Some("a"),
                None,
                Some("c"),
                Some("b"),
            ])),
            Arc::new(Int64Array::from(vec![
                Some(1),
                Some(5),
                None,
                Some(3),
                Some(-2),
                Some(8),
            ])),
            Arc::new(Float64Array::from(vec![0.5, 1.5, 2.5, 3.5, 4.5, 5.5])),
            Arc::new(ListArray::from_iter_primitive::<Int64Type, _, _>(vec![
                Some(vec![Some(1), Some(2)]),
                Some(vec![]),
                None,
                Some(vec![Some(3)]),
                Some(vec![Some(4), None]),
                Some(vec![Some(5)]),
            ])),
        ],
    )
    .unwrap();
    let u = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("k", DataType::Utf8, false),
            Field::new("w", DataType::Int64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "d"])),
            Arc::new(Int64Array::from(vec![10, 20, 30])),
        ],
    )
    .unwrap();
    for (name, batch) in [("t", t), ("u", u)] {
        let table = MemTable::try_new(batch.schema(), vec![vec![batch]]).unwrap();
        ctx.register_table(name, Arc::new(table)).unwrap();
    }
    ctx
}

/// The physical plan of `sql` in `ctx`.
async fn plan(ctx: &SessionContext, sql: &str) -> Arc<dyn ExecutionPlan> {
    ctx.sql(sql)
        .await
        .unwrap()
        .create_physical_plan()
        .await
        .unwrap()
}

/// `plan` with each scan of a table behind a cut's leaf, as a page's plans read their inputs, and
/// the results of those scans.
async fn cut(
    ctx: &SessionContext,
    plan: Arc<dyn ExecutionPlan>,
) -> (Arc<dyn ExecutionPlan>, Vec<Option<Arc<Kept>>>) {
    let mut scans = Vec::new();
    let plan = (plan.transform_down(|node| {
        if node.downcast_ref::<DataSourceExec>().is_none() {
            return Ok(Transformed::no(node));
        }
        let cut = CutScan {
            input: scans.len(),
            schema: node.schema(),
            projection: None,
            limit: None,
        };
        scans.push(node.clone());
        let leaf: Arc<dyn ExecutionPlan> = Arc::new(CutInputExec::new(cut, node));
        Ok(Transformed::new(leaf, true, TreeNodeRecursion::Jump))
    }))
    .unwrap()
    .data;
    let mut results = Vec::new();
    for scan in scans {
        let columns = (0..scan.schema().fields().len()).collect();
        let kept = Kept::compute(scan, columns, ctx.task_ctx()).await.unwrap();
        results.push(Some(Arc::new(kept)));
    }
    (plan, results)
}

/// The names of the operators of `plan`.
fn operators(plan: &Arc<dyn ExecutionPlan>, names: &mut BTreeSet<String>) {
    names.insert(plan.name().to_string());
    for child in plan.children() {
        operators(child, names);
    }
}

/// Checks that `plan` decodes in a page and runs to the native rows, and adds the names of its
/// decoded operators to `names`. As a graph's export does, the native run executes a copy of the
/// plan with its states reset, and the plan encodes afterwards, so the states of a run, such as
/// the filter of a top-k sort, never travel.
async fn round_trip(
    ctx: &SessionContext,
    plan: Arc<dyn ExecutionPlan>,
    names: &mut BTreeSet<String>,
) {
    let (cut, results) = cut(ctx, plan.clone()).await;
    let expected = collect(reset_plan_states(plan).unwrap(), ctx.task_ctx())
        .await
        .unwrap();
    let bytes = encode_plan(cut, &Default::default()).unwrap();
    let page = Arc::new(task_context(Default::default()));
    let decoded = decode_plan(&bytes, &page).unwrap();
    operators(&decoded, names);
    let decoded = substitute(decoded, &results, 1).unwrap();
    let ran = collect(decoded, page).await.unwrap();
    assert_eq!(
        pretty_format_batches(&ran).unwrap().to_string(),
        pretty_format_batches(&expected).unwrap().to_string()
    );
}

const QUERIES: [&str; 15] = [
    // Projections, filters, and the expressions of the closed set.
    "SELECT k, v + 1 AS next, -v AS negated, CAST(v AS DOUBLE) / 2 AS half, \
     TRY_CAST(k AS BIGINT) AS number, upper(k) AS upper, abs(v) AS size, \
     CASE WHEN v > 2 THEN 'high' WHEN v IS NULL THEN 'none' ELSE 'low' END AS level \
     FROM t WHERE (v > 0 AND d < 5 AND k IS NOT NULL) OR NOT (k IN ('a', 'c')) OR k LIKE 'b%'",
    "SELECT k, v FROM t WHERE v IS NULL OR k IS NULL",
    // Aggregates.
    "SELECT k, count(*) AS n, sum(v) AS total, avg(d) AS mean, min(v) AS low FROM t \
     GROUP BY k ORDER BY k NULLS LAST",
    "SELECT count(DISTINCT k) AS keys, max(d) AS high FROM t",
    // Windows, bounded and unbounded.
    "SELECT k, d, row_number() OVER (PARTITION BY k ORDER BY d) AS rank, \
     sum(d) OVER (ORDER BY d ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) AS moving FROM t ORDER BY d",
    "SELECT k, d, sum(d) OVER () AS whole FROM t ORDER BY d",
    // Sorts and limits.
    "SELECT k, d FROM t ORDER BY d DESC LIMIT 3",
    "SELECT k, d FROM t ORDER BY d LIMIT 2 OFFSET 1",
    // Unions.
    "SELECT k FROM t UNION ALL SELECT k FROM u ORDER BY k NULLS LAST",
    // Joins.
    "SELECT t.k, t.d, u.w FROM t JOIN u ON t.k = u.k ORDER BY t.d",
    "SELECT t.d, u.w FROM t JOIN u ON t.v < u.w / 5 ORDER BY t.d, u.w",
    "SELECT t.d, u.w FROM t CROSS JOIN u ORDER BY t.d, u.w",
    // Scalar subqueries.
    "SELECT k, v FROM t WHERE v > (SELECT avg(v) FROM t) ORDER BY v",
    // Unnests and plans without tables.
    "SELECT d, unnest(tags) AS tag FROM t ORDER BY d, tag",
    "SELECT 1 AS one, 'x' AS letter",
];

#[tokio::test]
async fn a_page_decodes_the_operators_and_expressions_of_charts() {
    let ctx = session();
    let mut names = BTreeSet::new();
    for sql in QUERIES {
        round_trip(&ctx, plan(&ctx, sql).await, &mut names).await;
    }

    // The operators that the queries above do not plan in one partition.
    let sorted = plan(&ctx, "SELECT k, d FROM t ORDER BY d").await;
    let ordering = sorted.output_ordering().unwrap().clone();
    let wrapped: [Arc<dyn ExecutionPlan>; 5] = [
        Arc::new(SortPreservingMergeExec::new(ordering, sorted.clone())),
        Arc::new(CoalescePartitionsExec::new(sorted.clone())),
        #[expect(
            deprecated,
            reason = "the page decodes the plans that DataFusion encodes"
        )]
        Arc::new(CoalesceBatchesExec::new(sorted.clone(), 2)),
        Arc::new(CooperativeExec::new(sorted.clone())),
        Arc::new(LocalLimitExec::new(sorted.clone(), 2)),
    ];
    for plan in wrapped {
        round_trip(&ctx, plan, &mut names).await;
    }
    round_trip(&ctx, Arc::new(EmptyExec::new(sorted.schema())), &mut names).await;

    // A `VALUES` list, which a graph keeps in the node that reads it, travels as its scan.
    let values = plan(
        &ctx,
        "SELECT n * 2 AS twice, s FROM (VALUES (1, 'a'), (2, 'b'), (3, NULL)) AS v(n, s) WHERE n > 1",
    )
    .await;
    let expected = collect(reset_plan_states(values.clone()).unwrap(), ctx.task_ctx())
        .await
        .unwrap();
    let page = Arc::new(task_context(Default::default()));
    let decoded = decode_plan(&encode_plan(values, &Default::default()).unwrap(), &page).unwrap();
    operators(&decoded, &mut names);
    let ran = collect(decoded, page).await.unwrap();
    assert_eq!(
        pretty_format_batches(&ran).unwrap().to_string(),
        pretty_format_batches(&expected).unwrap().to_string()
    );

    let expected = [
        "AggregateExec",
        "BoundedWindowAggExec",
        "CoalesceBatchesExec",
        "CoalescePartitionsExec",
        "CooperativeExec",
        "CrossJoinExec",
        "CutInputExec",
        "DataSourceExec",
        "EmptyExec",
        "FilterExec",
        "GlobalLimitExec",
        "HashJoinExec",
        "LocalLimitExec",
        "NestedLoopJoinExec",
        "PlaceholderRowExec",
        "ProjectionExec",
        "ScalarSubqueryExec",
        "SortExec",
        "SortPreservingMergeExec",
        "UnionExec",
        "UnnestExec",
        "WindowAggExec",
    ];
    let missing: Vec<_> = (expected.iter())
        .filter(|name| !names.contains(**name))
        .collect();
    assert!(
        missing.is_empty(),
        "not decoded: {missing:?}, decoded: {names:?}"
    );
}

#[tokio::test]
async fn a_page_names_an_operator_that_it_does_not_run() {
    let ctx = session();
    let scan = plan(&ctx, "SELECT k FROM t").await;
    let (scan, _) = cut(&ctx, scan).await;
    let repartition = RepartitionExec::try_new(scan, Partitioning::RoundRobinBatch(2)).unwrap();
    let bytes = encode_plan(Arc::new(repartition), &Default::default()).unwrap();
    let error = decode_plan(&bytes, &task_context(Default::default()))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("a page does not run the plan node Repartition"),
        "{error}"
    );
}

/// A selection's operator travels unbound, and the decoded plan, bound to a brush, runs to the rows
/// of the native plan bound to it.
#[tokio::test]
async fn a_page_decodes_a_selection_s_operator() {
    use avenger_selection::{Clause, Condition, Resolution, SelectionState};
    use avenger_selection_datafusion::{bind_selections, with_selection_nodes, SelectionFields};
    let ctx = session();
    let sql =
        "SELECT k, sum(d) AS total FROM t WHERE selected($s) GROUP BY k ORDER BY k NULLS LAST";
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let fields = SelectionFields::from([("s".to_string(), Some(vec!["d".to_string()]))]);
    let optimized = (ctx.state())
        .optimize(&with_selection_nodes(plan, &fields).unwrap())
        .unwrap();
    let physical = ctx.state().create_physical_plan(&optimized).await.unwrap();
    let range = Condition::Range {
        column: "d".into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![1.0, 3.0])),
        exclude_end: false,
        cells: None,
    };
    let state = SelectionState::new(
        Resolution::Global,
        vec![Clause::new("scatter").condition(range)],
    );
    let states = std::collections::BTreeMap::from([("s".to_string(), state)]);
    let (cut, results) = cut(&ctx, physical.clone()).await;
    let bound = bind_selections(physical, &states).unwrap();
    let expected = collect(reset_plan_states(bound).unwrap(), ctx.task_ctx())
        .await
        .unwrap();
    let page = Arc::new(task_context(Default::default()));
    let bytes = encode_plan(cut, &Default::default()).unwrap();
    let decoded = decode_plan(&bytes, &page).unwrap();
    let mut names = BTreeSet::new();
    operators(&decoded, &mut names);
    assert!(names.contains("SelectionExec"), "{names:?}");
    let decoded = substitute(decoded, &results, 1).unwrap();
    let bound = reset_plan_states(bind_selections(decoded, &states).unwrap()).unwrap();
    let ran = collect(bound, page).await.unwrap();
    assert_eq!(
        pretty_format_batches(&ran).unwrap().to_string(),
        pretty_format_batches(&expected).unwrap().to_string()
    );
}
