//! Selections' operators: a plan with selection nodes, optimized and planned, then bound to a
//! state during planning or after it, returns the rows of the same query with the selection
//! written as a predicate, for each place that a `selected` call can take in a query.

mod common;

use std::{collections::BTreeMap, sync::Arc};

use avenger_selection::{Clause, Condition, Resolution, SelectionState};
use avenger_selection_datafusion::{
    bind_selections, selected, with_selection_nodes, with_selection_states, SelectionFields,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray},
        util::pretty::pretty_format_batches,
    },
    dataframe::{DataFrame, DataFrameWriteOptions},
    datasource::provider_as_source,
    logical_expr::{
        col, expr::Placeholder, Expr, JoinType, LogicalPlan, LogicalPlanBuilder, SortExpr,
    },
    physical_plan::{collect, execution_plan::reset_plan_states},
    prelude::{CsvReadOptions, SessionContext},
};

fn table() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from_iter_values(0..100)) as ArrayRef,
        ),
        (
            "x",
            Arc::new(Float64Array::from_iter_values((0..100).map(f64::from))),
        ),
        (
            "species",
            Arc::new(StringArray::from_iter_values(
                (0..100).map(|i| ["adelie", "gentoo", "chinstrap"][i % 3]),
            )),
        ),
    ])
    .unwrap()
}

fn names() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "code",
            Arc::new(StringArray::from(vec!["adelie", "gentoo"])) as ArrayRef,
        ),
        (
            "name",
            Arc::new(StringArray::from(vec!["Adélie", "Gentoo"])),
        ),
    ])
    .unwrap()
}

/// A session with the table `t` in memory, the table `c` of the same rows in a CSV file, and the
/// table `u` of names.
async fn context() -> SessionContext {
    let ctx = common::session();
    ctx.register_batch("t", table()).unwrap();
    ctx.register_batch("u", names()).unwrap();
    let path = format!("{}/operator-csv", env!("CARGO_TARGET_TMPDIR"));
    let _ = std::fs::remove_dir_all(&path);
    let frame = ctx.table("t").await.unwrap();
    (frame.write_csv(&path, DataFrameWriteOptions::new(), None))
        .await
        .unwrap();
    ctx.register_csv("c", &path, CsvReadOptions::new())
        .await
        .unwrap();
    ctx
}

/// The range of `x` from `low` to `high`.
fn range(low: f64, high: f64) -> SelectionState {
    let range = Condition::Range {
        column: "x".into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end: false,
        cells: None,
    };
    SelectionState::new(
        Resolution::Global,
        vec![Clause::new("scatter").condition(range)],
    )
}

/// The states, each with the predicate that selects its rows.
fn cases() -> Vec<(SelectionState, String)> {
    let mut cases = vec![(SelectionState::default(), "TRUE".to_string())];
    for (low, high) in [(10.0, 40.0), (60.0, 95.0), (0.0, 5.0)] {
        cases.push((range(low, high), format!("(x >= {low} AND x <= {high})")));
    }
    cases
}

/// The queries of each place of a call, with `{sel}` for the call and `{t}` for the table: a
/// filter, a projection, a group key, an aggregate's `FILTER`, an `ORDER BY` key, and a join's
/// condition on one side, which reads the selection's fields.
const QUERIES: [&str; 6] = [
    "SELECT species, count(*) AS n, sum(x) AS total FROM {t} WHERE {sel} GROUP BY species \
     ORDER BY species",
    "SELECT id, {sel} AS chosen FROM {t} ORDER BY id",
    "SELECT {sel} AS chosen, count(*) AS n FROM {t} GROUP BY 1 ORDER BY 1",
    "SELECT count(*) FILTER (WHERE {sel}) AS n FROM {t}",
    "SELECT id, x FROM {t} ORDER BY {sel}, id",
    "SELECT id, name FROM {t} JOIN u ON species = code AND {sel} ORDER BY id",
];

async fn text(frame: DataFrame) -> String {
    let batches = frame.collect().await.unwrap();
    pretty_format_batches(&batches).unwrap().to_string()
}

/// For each query, table, and fields, the plan bound during planning and the plan planned once
/// and bound after it return the rows of the query with the predicate of each state.
#[tokio::test]
async fn a_plan_binds_each_state() {
    let ctx = context().await;
    let field_sets: [SelectionFields; 2] = [
        BTreeMap::new(),
        BTreeMap::from([("s".to_string(), Some(vec!["x".to_string()]))]),
    ];
    for query in QUERIES {
        for table in ["t", "c"] {
            for fields in &field_sets {
                if query.contains("JOIN") && fields.is_empty() {
                    continue;
                }
                let sql = query.replace("{t}", table);
                let selected = sql.replace("{sel}", "selected($s)");
                let plan = ctx.sql(&selected).await.unwrap().into_unoptimized_plan();
                let with_nodes = with_selection_nodes(plan, fields).unwrap();
                let optimized = ctx.state().optimize(&with_nodes).unwrap();
                let planned = ctx.state().create_physical_plan(&optimized).await.unwrap();
                for (state, predicate) in cases() {
                    let expected =
                        text(ctx.sql(&sql.replace("{sel}", &predicate)).await.unwrap()).await;
                    let states = BTreeMap::from([("s".to_string(), state)]);
                    let session = with_selection_states(&ctx.state(), states.clone());
                    let frame = DataFrame::new(session, with_nodes.clone());
                    assert_eq!(text(frame).await, expected, "{selected} as it plans");
                    let bound = bind_selections(planned.clone(), &states).unwrap();
                    let batches = collect(reset_plan_states(bound).unwrap(), ctx.task_ctx())
                        .await
                        .unwrap();
                    let bound = pretty_format_batches(&batches).unwrap().to_string();
                    assert_eq!(bound, expected, "{selected} planned once");
                }
            }
        }
    }
}

/// The optimizer keeps a filter on an operator's column above the operator, moves a filter on
/// another column below it, and pushes the projection of the columns that the plan and the test
/// read down to the scan.
#[tokio::test]
async fn the_optimizer_moves_the_plan_around_the_operator() {
    let ctx = context().await;
    let sql = "SELECT id FROM t WHERE selected($s) AND species = 'adelie'";
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let fields = BTreeMap::from([("s".to_string(), Some(vec!["x".to_string()]))]);
    let optimized = ctx
        .state()
        .optimize(&with_selection_nodes(plan, &fields).unwrap())
        .unwrap();
    let text = optimized.display_indent().to_string();
    let line = |needle: &str| {
        (text.lines())
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} in:\n{text}"))
    };
    let selection = line("Selection: $s AS __selected_");
    assert!(line("Filter: __selected_") < selection, "{text}");
    assert!(line("species = Utf8(\"adelie\")") > selection, "{text}");
    assert!(
        line("TableScan: t projection=[id, x, species]") > selection,
        "{text}"
    );
    let narrow = "SELECT id FROM t WHERE selected($s)";
    let plan = ctx.sql(narrow).await.unwrap().into_unoptimized_plan();
    let optimized = ctx
        .state()
        .optimize(&with_selection_nodes(plan, &fields).unwrap())
        .unwrap();
    let text = optimized.display_indent().to_string();
    assert!(text.contains("TableScan: t projection=[id, x]"), "{text}");
}

/// The plan of a scan of the table `name`.
async fn scan(ctx: &SessionContext, name: &str) -> LogicalPlanBuilder {
    let source = provider_as_source(ctx.table_provider(name).await.unwrap());
    LogicalPlanBuilder::scan(name, source, None).unwrap()
}

/// A plan with selection nodes has the schema of the original plan, also where the operator that
/// contains the call passes its input's columns through.
#[tokio::test]
async fn a_plan_with_selection_nodes_keeps_its_schema() {
    let ctx = context().await;
    let call = || {
        let placeholder = Placeholder::new_with_field("$s".to_string(), None);
        selected().call(vec![Expr::Placeholder(placeholder)])
    };
    let filter = scan(&ctx, "t")
        .await
        .filter(call())
        .unwrap()
        .build()
        .unwrap();
    let sort = (scan(&ctx, "t").await)
        .sort(vec![SortExpr::new(call(), true, false)])
        .unwrap()
        .build()
        .unwrap();
    let on = col("t.species").eq(col("u.code")).and(call());
    let join = (scan(&ctx, "t").await)
        .join_on(
            scan(&ctx, "u").await.build().unwrap(),
            JoinType::Inner,
            vec![on],
        )
        .unwrap()
        .build()
        .unwrap();
    let fields = BTreeMap::from([("s".to_string(), Some(vec!["x".to_string()]))]);
    for plan in [filter, sort, join] {
        let with_nodes = with_selection_nodes(plan.clone(), &fields).unwrap();
        assert_eq!(
            with_nodes.schema(),
            plan.schema(),
            "{}",
            with_nodes.display_indent()
        );
    }
}

/// Two copies of one subplan stay equal with their selection nodes, and two equal calls on one
/// input read one node.
#[tokio::test]
async fn equal_calls_get_equal_nodes() {
    let ctx = context().await;
    let sql = "SELECT x FROM t WHERE selected($s) UNION ALL SELECT x FROM t WHERE selected($s)";
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let with_nodes = with_selection_nodes(plan, &BTreeMap::new()).unwrap();
    let LogicalPlan::Union(union) = &with_nodes else {
        panic!("a union: {}", with_nodes.display_indent());
    };
    assert_eq!(union.inputs[0], union.inputs[1]);
    let sql = "SELECT id, selected($s) AS a, selected($s) AS b FROM t";
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let with_nodes = with_selection_nodes(plan, &BTreeMap::new()).unwrap();
    let text = with_nodes.display_indent().to_string();
    assert_eq!(text.matches("Selection: $s").count(), 1, "{text}");
}

/// A call in a join's condition that reads the columns of both sides fails.
#[tokio::test]
async fn a_join_condition_over_both_sides_fails() {
    let ctx = context().await;
    let sql = "SELECT id FROM t JOIN u ON species = code AND selected($s)";
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let fields = BTreeMap::from([(
        "s".to_string(),
        Some(vec!["x".to_string(), "name".to_string()]),
    )]);
    let error = with_selection_nodes(plan, &fields).unwrap_err().to_string();
    assert!(error.contains("reads the columns of both sides"), "{error}");
}

/// A state that tests a column that the selection's fields omit fails to bind, with the column's
/// name, and a plan without a state fails when it runs, with the selection's name.
#[tokio::test]
async fn binding_names_what_it_misses() {
    let ctx = context().await;
    let plan = (ctx.sql("SELECT id FROM t WHERE selected($s)").await)
        .unwrap()
        .into_unoptimized_plan();
    let fields = BTreeMap::from([("s".to_string(), Some(vec!["species".to_string()]))]);
    let with_nodes = with_selection_nodes(plan, &fields).unwrap();
    let optimized = ctx.state().optimize(&with_nodes).unwrap();
    let planned = ctx.state().create_physical_plan(&optimized).await.unwrap();
    let states = BTreeMap::from([("s".to_string(), range(10.0, 40.0))]);
    let error = bind_selections(planned.clone(), &states)
        .unwrap_err()
        .to_string();
    assert!(error.contains("tests the column `x`"), "{error}");
    let error = collect(planned, ctx.task_ctx())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("selection $s is not bound to a state"),
        "{error}"
    );
}
