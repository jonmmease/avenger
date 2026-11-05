//! The rules that span the messages of a chart: the identities and partitions of its nodes and
//! layers, its facets, its selections and their clauses, its shared scales and projections, and
//! its tables of tracks.

use std::collections::BTreeMap;

use arrow::array::ArrayRef;

use super::{fail, Checker, Result};
use crate::{
    proto::{self, layer_tool, node::Node, tool},
    resolve::{self, Identity, Tree},
    table,
};

impl Checker<'_> {
    /// Checks the rules that span the messages of the chart.
    pub(super) fn shared(&self) -> Result<()> {
        let tree = Tree::new(self.chart)?;
        self.facets(self.chart.root(), "root", &tree)?;
        self.joined_selections()?;
        self.clauses(&tree)?;
        let groups = resolve::groups(self.chart, &tree)?;
        resolve::check_groups(&tree, &groups)?;
        let projections = resolve::projections(&tree)?;
        let tables = table::tables(self.chart)?;
        table::matches(&tree, &groups, &projections, &tables)?;
        Ok(())
    }

    /// Checks the facets of the concats and the wraps at and below `node`: every child's partition
    /// has a value of the facet's column, of one Arrow type, and the children share each channel
    /// alike, so the panels of a facet grid cannot disagree.
    fn facets(&self, node: &proto::Node, path: &str, tree: &Tree) -> Result<()> {
        let (kind, children, facet) = match node.node.as_ref() {
            Some(Node::Concat(concat)) => ("concat", &concat.children, concat.facet.as_ref()),
            Some(Node::Wrap(wrap)) => ("wrap", &wrap.children, wrap.facet.as_ref()),
            _ => return Ok(()),
        };
        let path = format!("{path}.{kind}");
        for (index, child) in children.iter().enumerate() {
            self.facets(child, &format!("{path}.children[{index}]"), tree)?;
        }
        let Some(facet) = facet else { return Ok(()) };
        let mut data_type = None;
        let mut sharing: Option<Sharing> = None;
        for (index, child) in children.iter().enumerate() {
            let child_path = format!("{path}.children[{index}]");
            let node_path = match child.node.as_ref() {
                Some(Node::Plot(_)) => format!("{child_path}.plot"),
                Some(Node::Concat(_)) => format!("{child_path}.concat"),
                Some(Node::Wrap(_)) => format!("{child_path}.wrap"),
                _ => format!("{child_path}.empty"),
            };
            let value = tree
                .partitions
                .get(&node_path)
                .and_then(|partition| partition.get(&facet.column));
            let Some(value) = value else {
                return Err(fail(
                    &child_path,
                    format!(
                        "the partition of a child of a facet has the facet's column `{}`",
                        facet.column
                    ),
                ));
            };
            match &data_type {
                None => data_type = Some(value.data_type().clone()),
                Some(known) if known != value.data_type() => {
                    return Err(fail(
                        &child_path,
                        "the values of the column of a facet have one Arrow type",
                    ));
                }
                Some(_) => {}
            }
            let child_sharing = match child.node.as_ref() {
                Some(Node::Plot(plot)) => Some((rules(&plot.resolve), None)),
                Some(Node::Concat(concat)) => Some((
                    rules(&concat.resolve),
                    concat.facet.as_ref().map(|facet| facet.column.as_str()),
                )),
                Some(Node::Wrap(wrap)) => Some((
                    rules(&wrap.resolve),
                    wrap.facet.as_ref().map(|facet| facet.column.as_str()),
                )),
                _ => None,
            };
            if let Some(child_sharing) = child_sharing {
                match &sharing {
                    None => sharing = Some(child_sharing),
                    Some(known) if *known != child_sharing => {
                        return Err(fail(
                            &child_path,
                            "the children of a facet share each channel alike and facet one column, so the panels of a facet grid cannot disagree",
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
        if let (Some(proto::facet::Header::Shown(header)), Some(data_type)) =
            (&facet.header, &data_type)
        {
            if let Some(labels) = &header.labels {
                self.pattern(
                    &labels.format,
                    data_type,
                    &format!("{path}.facet.shown.labels.format"),
                )?;
            }
        }
        Ok(())
    }

    /// Checks that the selections with one name, which join, have equal resolutions.
    fn joined_selections(&self) -> Result<()> {
        let selections = &self.chart.message.selections;
        for (index, selection) in selections.iter().enumerate() {
            if let Some(first) = selections[..index]
                .iter()
                .find(|first| first.name == selection.name)
            {
                if first.resolution != selection.resolution {
                    return Err(fail(
                        format!("selections[{index}].resolution"),
                        format!("the selections named `{}` join, and their resolutions differ, which is a conflict", selection.name),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Checks that a plot or a layer declares at most one `Select` of each selection, and that each
    /// initial clause names, by its owner and its partition, a plot or a layer that declares a
    /// `Select` of its selection, with one clause for each owner.
    fn clauses(&self, tree: &Tree) -> Result<()> {
        // The selections that each plot and each layer write, with their identities.
        let mut writers: Vec<(&Identity, Vec<&str>, &str)> = Vec::new();
        for plot in &tree.plots {
            let selections: Vec<&str> = (plot.plot.tools.iter())
                .filter_map(|tool| match &tool.kind {
                    Some(tool::Kind::Select(select)) => Some(select.selection.as_str()),
                    _ => None,
                })
                .collect();
            once_each(&selections, &plot.path)?;
            if let Some(identity) = &plot.identity {
                writers.push((identity, selections, &plot.path));
            }
        }
        for layer in &tree.layers {
            let selections: Vec<&str> = (layer.layer.tools().iter())
                .filter_map(|tool| match &tool.kind {
                    Some(layer_tool::Kind::Select(select)) => Some(select.selection.as_str()),
                    _ => None,
                })
                .collect();
            once_each(&selections, &layer.path)?;
            if let Some(identity) = &layer.identity {
                writers.push((identity, selections, &layer.path));
            }
        }
        for (index, selection) in self.chart.message.selections.iter().enumerate() {
            let mut owners: Vec<(Identity, &proto::Clause)> = Vec::new();
            for (position, clause) in selection.initial.iter().enumerate() {
                let path = format!("selections[{index}].initial[{position}]");
                let partition: BTreeMap<String, ArrayRef> = (clause.partition.iter())
                    .map(|value| {
                        (
                            value.column.clone(),
                            self.chart.partition_value(value).clone(),
                        )
                    })
                    .collect();
                let owner = Identity {
                    id: clause.owner.clone(),
                    partition,
                };
                let writer = writers.iter().find(|(identity, _, _)| **identity == owner);
                match writer {
                    Some((_, selections, _)) if selections.contains(&selection.name.as_str()) => {}
                    Some((_, _, path_of)) => {
                        return Err(fail(
                            &path,
                            format!("`{path_of}` declares no `Select` of the selection `{}`, so it writes none of its clauses", selection.name),
                        ));
                    }
                    None => {
                        return Err(fail(
                            &path,
                            format!(
                                "the chart has no plot or layer with the identity `{}`",
                                owner.name()
                            ),
                        ));
                    }
                }
                if let Some((_, first)) = owners.iter().find(|(known, _)| *known == owner) {
                    if !self.same_clause(first, clause) {
                        return Err(fail(
                            &path,
                            format!(
                                "two initial clauses of the owner `{}` differ, which is a conflict",
                                owner.name()
                            ),
                        ));
                    }
                }
                owners.push((owner, clause));
            }
        }
        Ok(())
    }

    /// Whether two clauses hold equal conditions.
    fn same_clause(&self, a: &proto::Clause, b: &proto::Clause) -> bool {
        let mut a = a.clone();
        let mut b = b.clone();
        let values = |clause: &mut proto::Clause| {
            let mut keys = Vec::new();
            crate::walk::VisitValues::visit_values_mut(clause, &mut |value| {
                keys.push(super::value_key(self.chart.value(value)).unwrap_or_default());
                value.index = 0;
            });
            keys
        };
        values(&mut a) == values(&mut b) && a == b
    }
}

/// How a child of a facet shares its channels: its resolve rules, and the column of its own facet.
type Sharing<'a> = (Vec<(i32, i32)>, Option<&'a str>);

/// The resolve rules as a sorted list of their channels and sharings.
fn rules(resolve: &[proto::Resolve]) -> Vec<(i32, i32)> {
    let mut rules: Vec<(i32, i32)> = resolve
        .iter()
        .map(|rule| (rule.channel, rule.sharing))
        .collect();
    rules.sort_unstable();
    rules.dedup();
    rules
}

/// Fails when `selections` names one selection twice: a plot or a layer owns one clause of a
/// selection.
fn once_each(selections: &[&str], path: &str) -> Result<()> {
    for (index, selection) in selections.iter().enumerate() {
        if selections[..index].contains(selection) {
            return Err(fail(
                path,
                format!("two `Select` tools of one owner write the selection `{selection}`, which is a conflict"),
            ));
        }
    }
    Ok(())
}
