//! Where the graph cuts plans.

mod common;

use avenger_query_graph::{QueryNode, SourceOptions};
use common::{context, plan, store};

/// The nodes of the one plan `sql` compiles to.
async fn nodes(sql: &str, materialize: bool) -> Vec<QueryNode> {
    let ctx = context();
    let store = store();
    store.set_source_options(
        "flights",
        SourceOptions {
            materialize,
            ttl: None,
        },
    );
    let graph = store
        .compile(&ctx.state(), vec![plan(&ctx, sql).await])
        .unwrap();
    graph.nodes(None).unwrap().to_vec()
}

/// The parameters that each node's own operators read, with the selections as `$s`.
fn reads(nodes: &[QueryNode]) -> Vec<Vec<String>> {
    (nodes.iter())
        .map(|node| {
            let mut reads: Vec<String> = node.params().to_vec();
            reads.extend(node.selections().iter().map(|read| read.parameter.clone()));
            reads
        })
        .collect()
}

#[tokio::test]
async fn the_example_cuts_below_each_new_input() {
    let sql = "SELECT carrier, count(*) AS n FROM flights \
               WHERE year = $year AND selected($brush) GROUP BY carrier";
    // Without materialized scans, the year filter scans the table directly.
    let direct = nodes(sql, false).await;
    assert_eq!(reads(&direct), [vec!["$year"], vec!["$brush"]]);
    assert_eq!(direct[1].inputs(), [0]);
    assert_eq!(direct[1].top_of(), [0]);
    assert_eq!(direct[0].tables().len(), 1);
    assert!(direct[1].tables().is_empty());
    // With materialized scans, the scan is a node of its own.
    let materialized = nodes(sql, true).await;
    assert_eq!(
        reads(&materialized),
        [
            vec![],
            vec!["$year".to_string()],
            vec!["$brush".to_string()]
        ]
    );
    assert_eq!(materialized[1].inputs(), [0]);
    assert_eq!(materialized[2].inputs(), [1]);
    assert!(materialized.iter().all(QueryNode::kept));
}

#[tokio::test]
async fn a_join_cuts_the_input_that_reads_less() {
    let sql = "SELECT f.carrier, f.n, a.mean FROM \
               (SELECT carrier, count(*) AS n FROM flights WHERE year = $year GROUP BY carrier) f \
               JOIN (SELECT carrier, avg(distance) AS mean FROM flights GROUP BY carrier) a \
               ON f.carrier = a.carrier";
    let nodes = nodes(sql, false).await;
    // The right input reads no input, so it serves every year.
    assert_eq!(nodes.len(), 2);
    assert!(reads(&nodes)[0].is_empty());
    assert_eq!(reads(&nodes)[1], ["$year"]);
    assert_eq!(nodes[1].inputs(), [0]);
}

#[tokio::test]
async fn independent_branches_of_a_union_are_nodes() {
    let sql = "SELECT carrier, count(*) AS n FROM flights WHERE year = $a GROUP BY carrier \
               UNION ALL \
               SELECT carrier, count(*) AS n FROM flights WHERE delay > $b GROUP BY carrier";
    let nodes = nodes(sql, false).await;
    assert_eq!(nodes.len(), 3);
    assert_eq!(reads(&nodes), [vec!["$a"], vec!["$b"], vec![]]);
    assert_eq!(nodes[2].inputs(), [0, 1]);
}

#[tokio::test]
async fn branches_that_read_the_same_input_stay_with_the_union() {
    let sql = "SELECT carrier FROM flights WHERE year = $year \
               UNION ALL SELECT carrier FROM flights WHERE year = $year + 1";
    let nodes = nodes(sql, false).await;
    assert_eq!(nodes.len(), 1);
    assert_eq!(reads(&nodes), [vec!["$year"]]);
}

#[tokio::test]
async fn a_cte_with_two_references_is_one_node() {
    let sql = "WITH t AS (SELECT carrier, count(*) AS n FROM flights GROUP BY carrier) \
               SELECT * FROM t WHERE n > $low UNION ALL SELECT * FROM t WHERE n < $high";
    let nodes = nodes(sql, false).await;
    assert_eq!(nodes.len(), 4);
    let shared: Vec<usize> = (nodes.iter())
        .flat_map(|node| node.inputs().iter().copied())
        .filter(|input| *input == 0)
        .collect();
    assert_eq!(shared.len(), 2, "both branches read the CTE's node");
}

#[tokio::test]
async fn a_correlated_subquery_is_not_cut() {
    let sql = "SELECT carrier, delay FROM flights f \
               WHERE year = $year AND distance > \
               (SELECT avg(distance) FROM flights g WHERE g.carrier = f.carrier)";
    let nodes = nodes(sql, false).await;
    // The subquery's filter reads no input and is a node, and nothing inside the subquery is.
    assert_eq!(nodes.len(), 2);
    assert_eq!(reads(&nodes), [vec![], vec!["$year"]]);
}

#[tokio::test]
async fn a_volatile_branch_stays_with_its_reader() {
    let sql = "SELECT carrier FROM flights WHERE random() < 0.5 \
               UNION ALL SELECT carrier FROM flights WHERE year = $year";
    let nodes = nodes(sql, false).await;
    assert_eq!(nodes.len(), 2);
    assert!(nodes[0].kept());
    assert_eq!(reads(&nodes)[0], ["$year"]);
    assert!(!nodes[1].kept());
}

#[tokio::test]
async fn a_filter_splits_by_the_inputs_of_its_conjuncts() {
    let sql = "SELECT carrier FROM flights \
               WHERE delay > $a + $b AND carrier <> 'WN' AND year = $a";
    let nodes = nodes(sql, false).await;
    assert_eq!(reads(&nodes), [vec![], vec!["$a"], vec!["$a", "$b"]]);
}

#[tokio::test]
async fn equal_plans_share_their_top_node() {
    let ctx = context();
    let sql = "SELECT carrier, count(*) AS n FROM flights WHERE year = $year GROUP BY carrier";
    let graph = store()
        .compile(
            &ctx.state(),
            vec![plan(&ctx, sql).await, plan(&ctx, sql).await],
        )
        .unwrap();
    let nodes = graph.nodes(None).unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].top_of(), [0, 1]);
}
