//! The rules that span the messages of a chart: the identities and partitions of its nodes and
//! layers, its facets, its selections and their clauses, its shared scales and projections, and
//! its tables of tracks.

use std::collections::{BTreeMap, HashMap};

use arrow::array::ArrayRef;

use super::{fail, Checker, Result};
use crate::{
    proto::{self, layer_tool, node::Node, tool},
    resolve::{self, Identity, Tree},
};

impl Checker<'_> {
    /// Checks the rules that span the messages of the chart.
    pub(super) fn shared(&self) -> Result<()> {
        let tree = Tree::new(self.chart)?;
        self.facets(self.chart.root(), "root", &tree)?;
        self.joined_selections()?;
        let resolved = resolve::resolve_tree(self.chart, tree)?;
        self.clauses(&resolved.tree, &resolved.widgets)?;
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
        if let (Some(labels), Some(data_type)) = (&facet.labels, &data_type) {
            self.pattern(
                &labels.format,
                data_type,
                &format!("{path}.facet.labels.format"),
            )?;
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
    /// `Select` of its selection, or by its widget key with the anchor's identity, a widget of its
    /// selection, with one clause for each owner.
    fn clauses(&self, tree: &Tree, widgets: &[resolve::WidgetGroup]) -> Result<()> {
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
        // The clauses of the owners of each selection, of every entry with its name, which join,
        // by the key of the widget that wrote them, or empty for a tool's, and the owner.
        let mut owners_of: HashMap<&str, Vec<(Owner, &proto::Clause)>> = HashMap::new();
        for (index, selection) in self.chart.message.selections.iter().enumerate() {
            let owners = owners_of.entry(selection.name.as_str()).or_default();
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
                if clause.widget.is_empty() {
                    let writer = writers.iter().find(|(identity, _, _)| **identity == owner);
                    match writer {
                        Some((_, selections, _))
                            if selections.contains(&selection.name.as_str()) => {}
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
                } else {
                    self.widget_clause(clause, &owner, selection, widgets, &path)?;
                }
                let owner = (clause.widget.as_str(), owner);
                if let Some((_, first)) = owners.iter().find(|(known, _)| *known == owner) {
                    if !self.same_clause(first, clause) {
                        let of = match owner.0 {
                            "" => String::new(),
                            widget => format!(" of the widget `{widget}`"),
                        };
                        return Err(fail(
                            &path,
                            format!(
                                "two initial clauses{of} of the owner `{}` differ, which is a \
                                 conflict",
                                owner.1.name()
                            ),
                        ));
                    }
                }
                owners.push((owner, clause));
            }
        }
        Ok(())
    }

    /// Checks that the initial clause of a widget names a widget of its selection, by its key and
    /// its anchor's identity, which is an empty owner without a partition for a widget that the
    /// whole chart shares.
    fn widget_clause(
        &self,
        clause: &proto::Clause,
        owner: &Identity,
        selection: &proto::Selection,
        widgets: &[resolve::WidgetGroup],
        path: &str,
    ) -> Result<()> {
        let group = widgets.iter().find(|group| {
            group.key == clause.widget
                && match &group.anchor {
                    Some(anchor) => anchor == owner,
                    None => owner.id.is_empty() && owner.partition.is_empty(),
                }
        });
        fn writes(group: &resolve::WidgetGroup) -> &str {
            (group.widget.target()).map_or("", |target| target.selection.as_str())
        }
        match group {
            Some(group) if writes(group) == selection.name => Ok(()),
            Some(group) => Err(fail(
                path,
                format!(
                    "the widget `{}` writes the selection `{}`, not `{}`",
                    clause.widget,
                    writes(group),
                    selection.name
                ),
            )),
            None => Err(fail(
                path,
                format!(
                    "the chart has no widget `{}` with the owner `{}`",
                    clause.widget,
                    owner.name()
                ),
            )),
        }
    }

    /// Whether two clauses of one owner hold equal conditions.
    fn same_clause(&self, a: &proto::Clause, b: &proto::Clause) -> bool {
        let mut a = a.conditions.clone();
        let mut b = b.conditions.clone();
        let values = |conditions: &mut Vec<proto::Condition>| {
            let mut keys = Vec::new();
            for condition in conditions {
                crate::walk::VisitValues::visit_values_mut(condition, &mut |value| {
                    keys.push(super::value_key(self.chart.value(value)).unwrap_or_default());
                    value.index = 0;
                });
            }
            keys
        };
        values(&mut a) == values(&mut b) && a == b
    }
}

/// The owner of a clause: the key of the widget that wrote it, or empty for a tool's clause, and
/// the identity of its owner.
type Owner<'a> = (&'a str, Identity);

/// How a child of a facet shares its channels and widgets: its resolve rules, and the column of its
/// own facet.
type Sharing<'a> = (
    Vec<(Option<crate::resolve::Shares<'a>>, i32)>,
    Option<&'a str>,
);

/// The resolve rules as a sorted list of what they share and their sharings.
fn rules(resolve: &[proto::Resolve]) -> Vec<(Option<crate::resolve::Shares<'_>>, i32)> {
    let mut rules: Vec<(Option<crate::resolve::Shares>, i32)> = resolve
        .iter()
        .map(|rule| (crate::resolve::Shares::of(rule), rule.sharing))
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
