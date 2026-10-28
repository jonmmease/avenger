//! The checks of the rules of the format, which decoding and the builder run on every chart that
//! they return.

use crate::{proto::node::Node, tree, Chart, ChartError};

/// Checks `chart` against the rules of the format.
pub(crate) fn check(chart: &Chart) -> Result<(), ChartError> {
    let message = chart.message();
    if !message.tables.is_empty() {
        return Err(ChartError::invalid(
            "tables",
            "a chart holds its tables decoded, so the message's tables are empty",
        ));
    }
    let Some(root) = &message.root else {
        return Err(ChartError::invalid("root", "a chart has a root node"));
    };
    let mut result = Ok(());
    tree::visit(root, "root", &mut |node, path| {
        if result.is_err() {
            return;
        }
        result = check_node(chart, node, path);
    });
    result
}

fn check_node(chart: &Chart, node: &crate::proto::Node, path: &str) -> Result<(), ChartError> {
    match &node.node {
        None => Err(ChartError::invalid(path, "a node has a kind")),
        Some(Node::Unit(unit)) => {
            if unit.table as usize >= chart.tables().len() {
                return Err(ChartError::invalid(
                    path,
                    format!(
                        "the unit refers to table {}, and the chart has {} tables",
                        unit.table,
                        chart.tables().len()
                    ),
                ));
            }
            Ok(())
        }
        Some(_) => Ok(()),
    }
}
