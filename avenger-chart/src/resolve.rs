//! The models of a chart's tree that renderers read: its plots and layers with their identities
//! and partitions, the grouping of their channels into scales, and the grouping of their map plots
//! into projections.
//!
//! The resolve rules of plots, concats, and wraps and the share families of specs decide which
//! channels share a scale, and the specs of each group merge by the agreement rule of `ScaleSpec`.

use std::collections::{BTreeMap, HashMap};

use arrow::{
    array::{ArrayRef, RecordBatch},
    util::display::{ArrayFormatter, FormatOptions},
};

use crate::{
    layer::{CoordKind, LayerRef, MarkMessage},
    merge,
    proto::{self, domain, node::Node, plot::Coord, Channel, Sharing},
    validate::{is_position, value_key},
    Chart, ChartError,
};

type Result<T> = std::result::Result<T, ChartError>;

/// The identity of a plot, a layer, or another node with an id: its id and its partition, the
/// values that apply to every layer and empty cell below it. Two with one identity are a
/// conflict, so a node or a layer of a new chart with the same identity stands for the same part,
/// whatever its place in the tree.
#[derive(Clone, Debug)]
pub struct Identity {
    /// The id.
    pub id: String,
    /// The partition, by column.
    pub partition: BTreeMap<String, ArrayRef>,
}

/// Identities are equal when their ids and the types and values of their partitions are.
impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.partition.len() == other.partition.len()
            && (self.partition.iter()).zip(&other.partition).all(
                |((column, value), (other_column, other_value))| {
                    column == other_column && value.as_ref() == other_value.as_ref()
                },
            )
    }
}

impl Identity {
    /// The name of the identity: its id, with its partition values after it in brackets when it
    /// has any, such as `panel[era=1990s, genre=Drama]`.
    pub fn name(&self) -> String {
        if self.partition.is_empty() {
            return self.id.clone();
        }
        let values: Vec<String> = (self.partition.iter())
            .map(|(column, value)| {
                let value = ArrayFormatter::try_new(value.as_ref(), &FormatOptions::default())
                    .map_or_else(
                        |_| "?".to_string(),
                        |formatter| formatter.value(0).to_string(),
                    );
                format!("{column}={value}")
            })
            .collect();
        format!("{}[{}]", self.id, values.join(", "))
    }

    /// The key of the identity, equal exactly for equal identities.
    fn key(&self) -> Vec<u8> {
        let mut key = self.id.clone().into_bytes();
        for (column, value) in &self.partition {
            key.push(0);
            key.extend_from_slice(column.as_bytes());
            key.push(0);
            key.extend(value_key(value).unwrap_or_default());
        }
        key
    }
}

/// A concat or a wrap above a plot, which shares or separates each channel among its children.
#[derive(Clone)]
pub(crate) struct Ancestor<'a> {
    pub(crate) path: String,
    pub(crate) node: &'a Node,
}

/// A plot of a chart, by its place in the tree.
pub struct PlotRef<'a> {
    /// Its path, such as `root.concat.children[1].plot`.
    pub path: String,
    pub plot: &'a proto::Plot,
    /// The kind of its coordinate system.
    pub kind: CoordKind,
    /// The name of its identity, or its path when it has no id. The plots of a chart have
    /// different names.
    pub name: String,
    /// Its identity, when it has an id.
    pub identity: Option<Identity>,
    /// Its partition: the values that apply to every layer of the plot.
    pub partition: BTreeMap<String, ArrayRef>,
    /// The indices of its layers in the layers of the tree.
    pub layers: std::ops::Range<usize>,
    /// The concats and the wraps above the plot, from the root.
    pub(crate) ancestors: Vec<Ancestor<'a>>,
}

/// A layer of a chart, by its place in the tree.
pub struct Layer<'a> {
    /// Its path, such as `root.plot.cartesian.layers[0]`.
    pub path: String,
    pub layer: LayerRef<'a>,
    pub mark: &'a dyn MarkMessage,
    pub table: &'a RecordBatch,
    /// The index of its plot in the plots of the tree.
    pub plot: usize,
    /// Its position among the layers of its plot, which draw in order.
    pub position: usize,
    /// Its identity, when it has an id.
    pub identity: Option<Identity>,
    /// Its partition: the values that its ancestors, its plot, and the layer declare.
    pub partition: BTreeMap<String, ArrayRef>,
}

/// A concat or a wrap of a chart with an id.
pub struct NodeRef {
    pub path: String,
    pub identity: Identity,
}

/// The plots, the layers, and the identified concats and wraps of a chart, in tree order.
pub struct Tree<'a> {
    pub plots: Vec<PlotRef<'a>>,
    pub layers: Vec<Layer<'a>>,
    pub nodes: Vec<NodeRef>,
    /// The partition of each node, by its path, such as `root.concat.children[1].plot`: the
    /// values that apply to every layer and empty cell below it.
    pub partitions: HashMap<String, BTreeMap<String, ArrayRef>>,
}

impl<'a> Tree<'a> {
    /// The tree of `chart`, a chart whose messages are each valid. It fails for a layer or an
    /// empty cell that gets two different values of one partition column, and for two plots,
    /// layers, or nodes with one identity.
    pub fn new(chart: &'a Chart) -> Result<Self> {
        let mut walk = Walk {
            chart,
            ancestors: Vec::new(),
            tree: Tree {
                plots: Vec::new(),
                layers: Vec::new(),
                nodes: Vec::new(),
                partitions: HashMap::new(),
            },
        };
        walk.node(chart.root(), "root".to_string(), &BTreeMap::new())?;
        let tree = walk.tree;
        let mut seen: HashMap<Vec<u8>, String> = HashMap::new();
        let identities = (tree.plots.iter())
            .filter_map(|plot| Some((plot.identity.as_ref()?, &plot.path)))
            .chain(
                tree.layers
                    .iter()
                    .filter_map(|layer| Some((layer.identity.as_ref()?, &layer.path))),
            )
            .chain(tree.nodes.iter().map(|node| (&node.identity, &node.path)));
        for (identity, path) in identities {
            if let Some(first) = seen.insert(identity.key(), path.clone()) {
                return Err(ChartError::invalid(
                    path,
                    format!(
                        "`{first}` and `{path}` have one id and one partition, `{}`, which is a conflict",
                        identity.name()
                    ),
                ));
            }
        }
        Ok(tree)
    }
}

/// A walk over the nodes of a chart, which collects its plots, its layers, and its identified
/// nodes, and the partition of each.
struct Walk<'a> {
    chart: &'a Chart,
    ancestors: Vec<Ancestor<'a>>,
    tree: Tree<'a>,
}

impl<'a> Walk<'a> {
    /// `partition` with `values` added, failing for a column that it holds with another value.
    fn add(
        &self,
        partition: &BTreeMap<String, ArrayRef>,
        values: &[proto::PartitionValue],
        path: &str,
    ) -> Result<BTreeMap<String, ArrayRef>> {
        let mut partition = partition.clone();
        for value in values {
            let array = self.chart.partition_value(value).clone();
            match partition.get(&value.column) {
                Some(known) if known.as_ref() != array.as_ref() => {
                    return Err(ChartError::invalid(
                        format!("{path}.partition"),
                        format!(
                            "the column `{}` gets two different values from this node and its ancestors, which is a conflict",
                            value.column
                        ),
                    ))
                }
                _ => {
                    partition.insert(value.column.clone(), array);
                }
            }
        }
        Ok(partition)
    }

    /// Visits `node` at `path`, with the partition of its ancestors, and returns the partitions
    /// of the layers and the empty cells below it.
    fn node(
        &mut self,
        node: &'a proto::Node,
        path: String,
        inherited: &BTreeMap<String, ArrayRef>,
    ) -> Result<Vec<BTreeMap<String, ArrayRef>>> {
        match node.node.as_ref().expect("a valid node has a kind") {
            Node::Plot(plot) => {
                let path = format!("{path}.plot");
                let own = self.add(inherited, &plot.partition, &path)?;
                let kind = CoordKind::of(plot).expect("a valid plot has a coordinate system");
                let index = self.tree.plots.len();
                let start = self.tree.layers.len();
                let coord = match kind {
                    CoordKind::Cartesian => "cartesian",
                    CoordKind::Polar => "polar",
                    CoordKind::Geo => "geo",
                };
                let mut leaves = Vec::new();
                for (position, layer) in LayerRef::of(plot).into_iter().enumerate() {
                    let layer_path = format!("{path}.{coord}.layers[{position}]");
                    let partition = self.add(&own, layer.partition(), &layer_path)?;
                    let identity = (!layer.id().is_empty()).then(|| Identity {
                        id: layer.id().to_string(),
                        partition: partition.clone(),
                    });
                    leaves.push(partition.clone());
                    self.tree.layers.push(Layer {
                        path: layer_path,
                        layer,
                        mark: layer.mark().expect("a valid layer has a mark"),
                        table: self
                            .chart
                            .table(layer.table().expect("a valid layer has a table")),
                        plot: index,
                        position,
                        identity,
                        partition,
                    });
                }
                let partition = common(&leaves).unwrap_or(own);
                self.tree.partitions.insert(path.clone(), partition.clone());
                let identity = (!plot.id.is_empty()).then(|| Identity {
                    id: plot.id.clone(),
                    partition: partition.clone(),
                });
                self.tree.plots.push(PlotRef {
                    name: identity
                        .as_ref()
                        .map_or_else(|| path.clone(), Identity::name),
                    path,
                    plot,
                    kind,
                    identity,
                    partition,
                    layers: start..self.tree.layers.len(),
                    ancestors: self.ancestors.clone(),
                });
                Ok(leaves)
            }
            Node::Empty(empty) => {
                let path = format!("{path}.empty");
                let partition = self.add(inherited, &empty.partition, &path)?;
                self.tree.partitions.insert(path, partition.clone());
                Ok(vec![partition])
            }
            Node::Concat(proto::Concat {
                id,
                children,
                partition,
                ..
            })
            | Node::Wrap(proto::Wrap {
                id,
                children,
                partition,
                ..
            }) => {
                let kind = match node.node.as_ref() {
                    Some(Node::Concat(_)) => "concat",
                    _ => "wrap",
                };
                let path = format!("{path}.{kind}");
                let own = self.add(inherited, partition, &path)?;
                self.ancestors.push(Ancestor {
                    path: path.clone(),
                    node: node.node.as_ref().expect("the node has a kind"),
                });
                let mut leaves = Vec::new();
                for (index, child) in children.iter().enumerate() {
                    leaves.extend(self.node(child, format!("{path}.children[{index}]"), &own)?);
                }
                self.ancestors.pop();
                let partition = common(&leaves).unwrap_or(own);
                self.tree.partitions.insert(path.clone(), partition.clone());
                if !id.is_empty() {
                    self.tree.nodes.push(NodeRef {
                        path,
                        identity: Identity {
                            id: id.clone(),
                            partition,
                        },
                    });
                }
                Ok(leaves)
            }
        }
    }
}

/// The values that every one of `partitions` holds, or `None` for no partitions.
fn common(partitions: &[BTreeMap<String, ArrayRef>]) -> Option<BTreeMap<String, ArrayRef>> {
    let (first, rest) = partitions.split_first()?;
    let mut shared = first.clone();
    shared.retain(|column, value| {
        rest.iter().all(|partition| {
            partition
                .get(column)
                .is_some_and(|other| other.as_ref() == value.as_ref())
        })
    });
    Some(shared)
}

/// The sharing of `channel` among the children of `node`, a concat or a wrap: its rule, or the
/// default of its kind. A concat or a wrap shares every channel with a facet, and without one,
/// every channel but `x`, `y`, `theta`, and `radius`.
pub(crate) fn sharing(node: &Node, channel: Channel) -> Sharing {
    let (rules, facet) = match node {
        Node::Concat(concat) => (&concat.resolve, concat.facet.is_some()),
        Node::Wrap(wrap) => (&wrap.resolve, wrap.facet.is_some()),
        Node::Plot(plot) => return rule(&plot.resolve, channel).unwrap_or(Sharing::Shared),
        Node::Empty(_) => return Sharing::Shared,
    };
    let default = match facet
        || !matches!(
            channel,
            Channel::X | Channel::Y | Channel::Theta | Channel::Radius
        ) {
        true => Sharing::Shared,
        false => Sharing::Independent,
    };
    rule(rules, channel).unwrap_or(default)
}

fn rule(rules: &[proto::Resolve], channel: Channel) -> Option<Sharing> {
    (rules.iter())
        .find(|rule| rule.channel == channel as i32)
        .and_then(|rule| Sharing::try_from(rule.sharing).ok())
}

/// The sharing of the projections of the map plots below `node`, a concat or a wrap, among its
/// children: its rule, or with a facet one projection and without one, one for each child.
pub fn projection_sharing(node: &Node) -> Sharing {
    let (rule, facet) = match node {
        Node::Concat(concat) => (concat.projection, concat.facet.is_some()),
        Node::Wrap(wrap) => (wrap.projection, wrap.facet.is_some()),
        Node::Plot(_) | Node::Empty(_) => return Sharing::Shared,
    };
    match Sharing::try_from(rule) {
        Ok(sharing @ (Sharing::Shared | Sharing::Independent)) => sharing,
        _ if facet => Sharing::Shared,
        _ => Sharing::Independent,
    }
}

/// The first channel of the pair whose second channel is `channel`, or `channel` itself: the
/// channel whose scale `channel` goes through.
pub fn scale_channel(channel: Channel) -> Channel {
    match channel {
        Channel::X2 => Channel::X,
        Channel::Y2 => Channel::Y,
        Channel::Theta2 => Channel::Theta,
        Channel::Radius2 => Channel::Radius,
        Channel::Longitude2 => Channel::Longitude,
        Channel::Latitude2 => Channel::Latitude,
        other => other,
    }
}

/// The path of the node at which the layer shares `channel`: its plot when the plot shares the
/// channel among its layers, then its outermost ancestor up to which each ancestor shares the
/// channel among its children. An inner independent rule therefore wins over an outer shared rule.
fn scope<'a>(tree: &'a Tree, layer: &'a Layer, channel: Channel) -> &'a str {
    let plot = &tree.plots[layer.plot];
    if rule(&plot.plot.resolve, channel).unwrap_or(Sharing::Shared) != Sharing::Shared {
        return &layer.path;
    }
    let mut scope = plot.path.as_str();
    for ancestor in plot.ancestors.iter().rev() {
        if sharing(ancestor.node, channel) != Sharing::Shared {
            break;
        }
        scope = &ancestor.path;
    }
    scope
}

/// The innermost node that holds the nodes at `a` and `b`, by their paths.
pub fn common_path(a: &str, b: &str) -> String {
    let a: Vec<&str> = a.split('.').collect();
    let b: Vec<&str> = b.split('.').collect();
    (a.iter())
        .zip(&b)
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| *a)
        .collect::<Vec<_>>()
        .join(".")
}

/// One scale: the encodings that go through it, with the second channels of their pairs, and its
/// spec, which merges the specs of its encodings.
pub struct ScaleGroup {
    /// The channel of the scale, the first channel of a pair.
    pub channel: Channel,
    /// The spec of the scale, which merges the specs of its encodings.
    pub spec: proto::ScaleSpec,
    /// The encodings of the scale, in tree order, with the second channel of a pair after its
    /// first.
    pub members: Vec<Member>,
    /// The path of the node that owns the scale: the innermost node that holds every layer of
    /// the scale, which declares the rules of its guides.
    pub owner: String,
    /// The kind of the coordinate system of the plots of a position scale.
    pub kind: Option<CoordKind>,
}

impl ScaleGroup {
    /// Whether the scale is a position scale, which fits at the length of its plot.
    pub fn is_position(&self) -> bool {
        is_position(self.channel)
    }

    /// Whether the scale runs along a side of its plots, such as `x` of a Cartesian plot.
    pub fn is_edge(&self) -> bool {
        (self.kind).is_some_and(|kind| kind.edges().contains(&self.channel))
    }

    /// Whether a navigation moves the scale, such as `x` of a Cartesian plot.
    pub fn is_navigated(&self) -> bool {
        (self.kind).is_some_and(|kind| kind.navigated().contains(&self.channel))
    }
}

/// An encoding of a scale group: the channel of a layer, and where its part of the domain comes
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    /// The index of the layer in the layers of the tree.
    pub layer: usize,
    pub channel: Channel,
    /// The producer's summary when the domain of the layer's spec is a summary, which covers the
    /// values of both channels of a pair.
    pub summary: Option<proto::ValueRef>,
}

fn find(parents: &mut [usize], entry: usize) -> usize {
    let mut root = entry;
    while parents[root] != root {
        root = parents[root];
    }
    let mut entry = entry;
    while parents[entry] != root {
        let next = parents[entry];
        parents[entry] = root;
        entry = next;
    }
    root
}

/// Joins the entries with equal keys, by union-find.
fn join<K: std::hash::Hash + Eq>(keys: &[Vec<K>]) -> Vec<usize> {
    let mut parents: Vec<usize> = (0..keys.len()).collect();
    let mut first: HashMap<&K, usize> = HashMap::new();
    for (index, keys) in keys.iter().enumerate() {
        for key in keys {
            match first.get(key) {
                Some(&other) => {
                    let (a, b) = (find(&mut parents, other), find(&mut parents, index));
                    parents[a.max(b)] = a.min(b);
                }
                None => {
                    first.insert(key, index);
                }
            }
        }
    }
    (0..keys.len())
        .map(|index| find(&mut parents, index))
        .collect()
}

/// The scale groups of the layers of `tree`. The encodings of one channel whose layers share it
/// at one node, or whose specs name one share family, are one group, which the second channels of
/// their pairs join. The specs of each group must agree, and an error names the channel and the
/// two layers whose specs disagree.
pub fn groups(chart: &Chart, tree: &Tree) -> Result<Vec<ScaleGroup>> {
    struct Entry<'a> {
        layer: usize,
        channel: Channel,
        spec: &'a proto::ScaleSpec,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut keys: Vec<Vec<(Channel, bool, &str)>> = Vec::new();
    for (index, layer) in tree.layers.iter().enumerate() {
        for (channel, encoding) in layer.mark.encodings() {
            if let Some(spec) = &encoding.scale {
                let mut entry_keys = vec![(channel, false, scope(tree, layer, channel))];
                if !spec.family.is_empty() {
                    entry_keys.push((channel, true, spec.family.as_str()));
                }
                keys.push(entry_keys);
                entries.push(Entry {
                    layer: index,
                    channel,
                    spec,
                });
            }
        }
    }
    let roots = join(&keys);
    let same = |a: proto::ValueRef, b: proto::ValueRef| {
        chart.value(&a).as_ref() == chart.value(&b).as_ref()
    };
    let mut groups: Vec<ScaleGroup> = Vec::new();
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    let mut entries_of: Vec<Vec<usize>> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let layer = &tree.layers[entry.layer];
        let summary = match entry
            .spec
            .domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref())
        {
            Some(domain::Domain::Summary(value)) => Some(*value),
            _ => None,
        };
        let member = Member {
            layer: entry.layer,
            channel: entry.channel,
            summary,
        };
        let group = match group_of.get(&roots[index]) {
            Some(&group) => {
                if let Err(reason) = merge::merge(&mut groups[group].spec, entry.spec, &same) {
                    // The earliest member whose own spec disagrees with this one names the pair.
                    let earlier = (entries_of[group].iter())
                        .map(|&earlier| &entries[earlier])
                        .find_map(|earlier| {
                            let mut spec = earlier.spec.clone();
                            merge::merge(&mut spec, entry.spec, &same)
                                .err()
                                .map(|reason| (earlier, reason))
                        });
                    let (earlier, reason) =
                        earlier.unwrap_or((&entries[entries_of[group][0]], reason));
                    return Err(ChartError::invalid(
                        layer.path.clone(),
                        format!(
                            "the `{}` specs of `{}` and `{}` share a scale and {reason}",
                            entry.channel.field_name(),
                            tree.layers[earlier.layer].path,
                            layer.path,
                        ),
                    ));
                }
                let merged = &mut groups[group];
                merged.owner = common_path(&merged.owner, &layer.path);
                merged.members.push(member);
                entries_of[group].push(index);
                group
            }
            None => {
                groups.push(ScaleGroup {
                    channel: entry.channel,
                    spec: entry.spec.clone(),
                    members: vec![member],
                    owner: layer.path.clone(),
                    kind: is_position(entry.channel).then(|| tree.plots[layer.plot].kind),
                });
                entries_of.push(vec![index]);
                group_of.insert(roots[index], groups.len() - 1);
                groups.len() - 1
            }
        };
        // The second channel of the pair follows its first.
        for (channel, _) in layer.mark.encodings() {
            if channel != entry.channel && scale_channel(channel) == entry.channel {
                groups[group].members.push(Member {
                    layer: entry.layer,
                    channel,
                    summary,
                });
            }
        }
    }
    Ok(groups)
}

/// One projection: the map plots that share it, which fit one view at one size, and its options.
pub struct ProjectionGroup<'a> {
    /// The options of the first plot, whose surface, aspect, and navigation every plot of the
    /// group has, or `None` for the defaults.
    pub spec: Option<&'a proto::Geo>,
    /// The map plots, by their index in the plots of the tree, in tree order.
    pub plots: Vec<usize>,
    /// The path of the innermost node that holds every plot of the group.
    pub owner: String,
}

/// The options of a map plot.
fn geo_options<'a>(plot: &PlotRef<'a>) -> Option<&'a proto::Geo> {
    match plot.plot.coord.as_ref()? {
        Coord::Geo(geo) => geo.options.as_ref(),
        _ => None,
    }
}

/// The projections of the map plots of `tree`. The map plots that share their projection at one
/// node, or whose options name one share family, share one projection, and their surfaces, their
/// aspects, and their navigation must be equal, because they show one view.
pub fn projections<'a>(tree: &'a Tree) -> Result<Vec<ProjectionGroup<'a>>> {
    let maps: Vec<usize> = (0..tree.plots.len())
        .filter(|plot| tree.plots[*plot].kind == CoordKind::Geo)
        .collect();
    let keys: Vec<Vec<(bool, &str)>> = (maps.iter())
        .map(|plot| {
            let plot = &tree.plots[*plot];
            let mut scope = plot.path.as_str();
            for ancestor in plot.ancestors.iter().rev() {
                if projection_sharing(ancestor.node) != Sharing::Shared {
                    break;
                }
                scope = &ancestor.path;
            }
            let mut keys = vec![(false, scope)];
            if let Some(family) = geo_options(plot)
                .map(|geo| geo.family.as_str())
                .filter(|family| !family.is_empty())
            {
                keys.push((true, family));
            }
            keys
        })
        .collect();
    let roots = join(&keys);
    let mut groups: Vec<ProjectionGroup> = Vec::new();
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    for (index, plot) in maps.iter().enumerate() {
        let path = &tree.plots[*plot].path;
        let geo = geo_options(&tree.plots[*plot]);
        match group_of.get(&roots[index]) {
            Some(&group) => {
                let group = &mut groups[group];
                let default = proto::Geo::default();
                let (a, b) = (group.spec.unwrap_or(&default), geo.unwrap_or(&default));
                let differs = [
                    (a.surface != b.surface, "surfaces"),
                    (a.aspect != b.aspect, "aspects"),
                    (a.navigation != b.navigation, "navigations"),
                ]
                .into_iter()
                .find_map(|(differs, what)| differs.then_some(what));
                if let Some(what) = differs {
                    return Err(ChartError::invalid(
                        path.clone(),
                        format!(
                            "the map plots `{}` and `{path}` share a projection, so they show one view, and their {what} differ",
                            tree.plots[group.plots[0]].path
                        ),
                    ));
                }
                group.owner = common_path(&group.owner, path);
                group.plots.push(*plot);
            }
            None => {
                groups.push(ProjectionGroup {
                    spec: geo,
                    plots: vec![*plot],
                    owner: path.clone(),
                });
                group_of.insert(roots[index], groups.len() - 1);
            }
        }
    }
    Ok(groups)
}

/// The angles and the inner radius of a polar plot, with their defaults.
fn circle(plot: &proto::Plot) -> Option<[f32; 3]> {
    let polar = match plot.coord.as_ref()? {
        Coord::Polar(polar) => polar.options.unwrap_or_default(),
        _ => return None,
    };
    let start = polar.start_angle.unwrap_or(0.0);
    Some([
        start,
        polar.end_angle.unwrap_or(start + 360.0),
        polar.inner_radius.unwrap_or(0.0),
    ])
}

/// Checks the rules of shared scales: each Cartesian plot has at most two scales of `x` and of `y`,
/// one for the axis on each side, and each polar plot one scale of `theta` and one of `radius`; the
/// polar plots of a shared `theta` or `radius` draw one part of a circle; and the plots that share
/// `x` or `y` and fix their lengths along it fix equal lengths.
pub(crate) fn check_groups(tree: &Tree, groups: &[ScaleGroup]) -> Result<()> {
    for (index, plot) in tree.plots.iter().enumerate() {
        let limits = match plot.kind {
            CoordKind::Polar => [(Channel::Theta, 1), (Channel::Radius, 1)],
            _ => [(Channel::X, 2), (Channel::Y, 2)],
        };
        for (channel, limit) in limits {
            let count = (groups.iter())
                .filter(|group| {
                    group.channel == channel
                        && group
                            .members
                            .iter()
                            .any(|member| tree.layers[member.layer].plot == index)
                })
                .count();
            if count > limit {
                let reason = match limit {
                    1 => format!("a polar plot has one scale of `{}`, and it has {count}", channel.field_name()),
                    _ => format!(
                        "a plot has at most two scales of `{}`, one on each side, and it has {count}",
                        channel.field_name()
                    ),
                };
                return Err(ChartError::invalid(plot.path.clone(), reason));
            }
        }
    }
    for group in groups {
        let plots: Vec<&PlotRef> = {
            let mut plots: Vec<usize> = group
                .members
                .iter()
                .map(|member| tree.layers[member.layer].plot)
                .collect();
            plots.dedup();
            plots.into_iter().map(|plot| &tree.plots[plot]).collect()
        };
        if matches!(group.channel, Channel::Theta | Channel::Radius) {
            let mut first: Option<(&str, [f32; 3])> = None;
            for plot in &plots {
                let Some(circle) = circle(plot.plot) else {
                    continue;
                };
                match first {
                    None => first = Some((&plot.path, circle)),
                    Some((path, known)) if known != circle => {
                        return Err(ChartError::invalid(
                            plot.path.clone(),
                            format!(
                                "the polar plots `{path}` and `{}` share `{}`, so they draw one part of a circle, and their angles or inner radii differ",
                                plot.path,
                                group.channel.field_name()
                            ),
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
        let length = |plot: &PlotRef| match group.channel {
            Channel::X => plot.plot.width,
            Channel::Y => plot.plot.height,
            _ => None,
        };
        let mut fixed: Option<(&str, f32)> = None;
        for plot in &plots {
            match (fixed, length(plot)) {
                (None, Some(length)) => fixed = Some((&plot.path, length)),
                (Some((path, known)), Some(length)) if known != length => {
                    return Err(ChartError::invalid(
                        plot.path.clone(),
                        format!(
                            "the plots `{path}` and `{}` share `{}` and fix different lengths along it, which is a conflict",
                            plot.path,
                            group.channel.field_name()
                        ),
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(())
}
