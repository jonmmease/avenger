//! avenger-sql's plans where they run baked: a chart's plan travels through
//! avenger-chart-datafusion's codec with avenger-sql's host, and plans and runs in the session of
//! baked plans as it does in the session that planned it.

mod pass;

use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_datafusion::codec::{Hosts, PlanCodec};
use avenger_datafusion_shared::SharedNode;
use avenger_selection::{Clause, Condition, SelectionState};
use avenger_sql::{
    analysis::{selection_placeholder, Overrides},
    bake::{bake_plan, baked_session},
};
use datafusion::{
    arrow::{
        array::{AsArray, StringArray},
        util::pretty::pretty_format_batches,
    },
    catalog::TableProvider,
    common::tree_node::TreeNodeRecursion,
    dataframe::DataFrame,
    datasource::source_as_provider,
    logical_expr::LogicalPlan,
};

/// The tables that `plan` scans, once each.
fn tables(plan: &LogicalPlan) -> Vec<Arc<dyn TableProvider>> {
    let mut tables: Vec<Arc<dyn TableProvider>> = Vec::new();
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            let table = source_as_provider(&scan.source)?;
            if !tables.iter().any(|known| Arc::ptr_eq(known, &table)) {
                tables.push(table);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .unwrap();
    tables
}

/// The number of the shared nodes of `plan`.
fn shared_nodes(plan: &LogicalPlan) -> usize {
    let mut count = 0;
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::Extension(extension) = node {
            count += usize::from(extension.node.as_any().is::<SharedNode>());
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .unwrap();
    count
}

/// The rows of the table of the chart that `frame` builds, sorted.
async fn bars(frame: DataFrame) -> Vec<String> {
    let batches = frame.collect().await.unwrap();
    let column = batches[0].column(batches[0].num_columns() - 1);
    let chart = Chart::from_bytes(column.as_binary_view().value(0).to_vec()).unwrap();
    let text = pretty_format_batches(chart.tables()).unwrap().to_string();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    lines.sort();
    lines
}

/// A chart whose layers share an input travels with its shared nodes and its brush, and builds
/// the bars of the session that planned it, for an empty brush and for a brush of one species.
#[tokio::test]
async fn a_chart_with_a_shared_input_runs_in_a_baked_session() {
    let text = "DECLARE brush SELECTION;
FROM penguins |> WHERE brush |> AGGREGATE mark_bar(x => species, y => AVG(body_mass_g), color => species) AS chart";
    let session = pass::session();
    let analysis = pass::analysis(text);
    let mut query = (analysis.planned_query(&session, &Overrides::new()).await).unwrap();
    assert!(
        shared_nodes(&query.plan) > 1,
        "{}",
        query.plan.display_indent()
    );

    let sent = bake_plan(query.plan.clone(), &session.state()).unwrap();
    let tables = tables(&sent);
    let hosts = Hosts::of(session.state().config());
    let bytes = PlanCodec::new(hosts, tables.clone()).encode(&sent).unwrap();
    let runner = baked_session().unwrap();
    let hosts = Hosts::of(runner.state().config());
    let decoded = PlanCodec::new(hosts, tables)
        .decode(&bytes, &runner.task_ctx())
        .unwrap();
    assert_eq!(shared_nodes(&decoded), shared_nodes(&query.plan));
    assert_eq!(decoded, sent);

    let adelie = Arc::new(StringArray::from(vec!["Adelie"]));
    let one = Clause::new("bars").condition(Condition::values("species", None, adelie));
    let brushed = SelectionState::new(avenger_selection::Resolution::Global, vec![one]);
    let placeholder = selection_placeholder("brush");
    for state in [SelectionState::default(), brushed] {
        *query.selections.get_mut("brush").unwrap() = state.clone();
        let expected = bars(query.dataframe(&session).await.unwrap()).await;
        let bound = avenger_selection_datafusion::bind_selections(
            decoded.clone(),
            &[(placeholder.as_str(), &state)],
        )
        .unwrap();
        let ran = bars(DataFrame::new(runner.state(), bound)).await;
        assert_eq!(ran, expected);
    }
}
