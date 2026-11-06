//! The preparation of a chart, which reads it once, and its evaluation at a size.

use std::sync::Arc;

use avenger_chart::{
    proto::{node::Node, plot::Coord},
    Chart,
};
use avenger_style::{Cascade, ComputedStyles, HostInputs, StyleTree};
use avenger_typst_label::LabelEngine;

use crate::{
    style::tree::{self, StyleNodes},
    EvalError, EvalWarning, Registry,
};

/// A chart prepared for evaluation at any size: its resolved models, its style tree, and the
/// computed styles of the last host inputs.
pub struct ChartEvaluator {
    chart: Chart,
    cascade: Arc<Cascade>,
    #[allow(dead_code)]
    engine: LabelEngine,
    #[allow(dead_code)]
    registry: Registry,
    tree: StyleTree,
    #[allow(dead_code)]
    nodes: StyleNodes,
    styles: Option<ComputedStyles>,
    warnings: Vec<EvalWarning>,
}

impl ChartEvaluator {
    /// Prepares `chart` with the cascade of its style table, a label engine, and the custom scales
    /// of `registry`. It fails for a chart whose root is not one Cartesian plot, which this version
    /// of the evaluator does not draw.
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        engine: LabelEngine,
        registry: Registry,
    ) -> Result<Self, EvalError> {
        match chart
            .root()
            .node
            .as_ref()
            .expect("a valid chart has a root")
        {
            Node::Plot(plot) => match plot
                .coord
                .as_ref()
                .expect("a valid plot has a coordinate system")
            {
                Coord::Cartesian(_) => {}
                Coord::Polar(_) => return Err(EvalError::Unsupported("a polar plot".into())),
                Coord::Geo(_) => return Err(EvalError::Unsupported("a map plot".into())),
            },
            Node::Concat(_) => return Err(EvalError::Unsupported("a concat".into())),
            Node::Wrap(_) => return Err(EvalError::Unsupported("a wrap".into())),
            Node::Empty(_) => return Err(EvalError::Unsupported("an empty chart".into())),
        }
        crate::convert::format::check(&chart, &engine)?;
        let (tree, nodes, warnings) = {
            let resolved = chart.resolve();
            debug_assert!(tree::is_cartesian(resolved.tree.plots[0].plot));
            let (tree, nodes) = tree::build(&chart, &resolved);
            let warnings: Vec<EvalWarning> = (resolved.widgets.iter())
                .map(|group| {
                    let member = &group.members[0];
                    let layer = &resolved.tree.layers[member.layer];
                    EvalWarning::NotDrawn {
                        path: format!("{}.widgets[{}]", layer.path, member.index),
                        reason: format!(
                            "the widget `{}` does not draw yet, but its initial clause applies",
                            group.key
                        ),
                    }
                })
                .collect();
            (tree, nodes, warnings)
        };
        Ok(Self {
            chart,
            cascade,
            engine,
            registry,
            tree,
            nodes,
            styles: None,
            warnings,
        })
    }

    /// The chart that the evaluator draws.
    pub fn chart(&self) -> &Chart {
        &self.chart
    }

    /// The style tree of the chart, whose nodes stand for its parts.
    pub fn style_tree(&self) -> &StyleTree {
        &self.tree
    }

    /// The warnings of preparing the chart, such as widgets that do not draw yet.
    pub fn warnings(&self) -> &[EvalWarning] {
        &self.warnings
    }

    /// The computed styles of the chart's style tree for `host`. They are computed again only
    /// when the host's preferences or theme change, or when a resize changes the outcome of a
    /// condition, so a resize within a breakpoint keeps them.
    pub fn styles(&mut self, host: &HostInputs) -> &ComputedStyles {
        let stale = (self.styles.as_ref()).is_none_or(|styles| styles.needs_update(host));
        if stale {
            self.styles = Some(self.cascade.compute(&self.tree, host));
        }
        self.styles.as_ref().expect("the styles are computed")
    }

    /// The warnings of the computed styles of the last host inputs, as warnings of evaluation.
    pub fn style_warnings(&self) -> Vec<EvalWarning> {
        (self.styles.iter())
            .flat_map(|styles| styles.warnings())
            .cloned()
            .map(EvalWarning::Style)
            .collect()
    }
}
