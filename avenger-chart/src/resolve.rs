//! The models of a chart's tree that renderers read: its plots and layers with their identities
//! and partitions, the grouping of their channels into scales, and the grouping of their map plots
//! into projections.
//!
//! The resolve rules of plots, concats, and wraps and the share families of specs decide which
//! channels share a scale, and the specs of each group merge by the agreement rule of `Scale`.

use std::collections::{BTreeMap, HashMap};

use arrow::{
    array::{ArrayRef, RecordBatch},
    util::display::{ArrayFormatter, FormatOptions},
};

use crate::{
    layer::{CoordKind, LayerRef, MarkMessage},
    merge,
    proto::{self, domain, node::Node, plot::Coord, Channel, Sharing},
    table::{self, Matches, Tables},
    validate::{is_position, value_key},
    Chart, ChartError,
};

type Result<T> = std::result::Result<T, ChartError>;

/// The identity of a plot, a layer, a concat, a wrap, a guide, or a facet header with an id: its
/// id and its partition, the values that apply to every layer and empty cell below it. All of
/// them share one namespace, and two with one identity are a conflict, so an identity names one
/// part of a chart, and a part of a new chart with the same identity stands for the same part,
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
        self.key() == other.key()
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

    /// The name of the owner of a clause in a selection state, which a renderer and a query give
    /// avenger-selection: the id of the plot or the layer of a tool's clause, and for the clause of
    /// the widget `widget`, a name that begins with a NUL character and joins the key with the id
    /// of the widget's anchor, empty for a widget that the whole chart shares. No id holds a NUL
    /// character, so the clauses of tools and widgets never share an owner. The partition of either
    /// clause stays apart from its name.
    pub fn owner_name(widget: &str, owner: &str) -> String {
        match widget {
            "" => owner.to_string(),
            widget => format!("\0{widget}\0{owner}"),
        }
    }

    /// The key of the identity, equal exactly for equal identities.
    pub(crate) fn key(&self) -> Vec<u8> {
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

/// A concat, a wrap, or a facet header of a chart with an id.
pub struct NodeRef {
    pub path: String,
    pub identity: Identity,
}

/// The plots, the layers, and the identified concats, wraps, and facet headers of a chart, in
/// tree order.
pub struct Tree<'a> {
    pub plots: Vec<PlotRef<'a>>,
    pub layers: Vec<Layer<'a>>,
    pub nodes: Vec<NodeRef>,
    /// The facet headers with ids, each with the partition of its concat or wrap.
    pub headers: Vec<NodeRef>,
    /// The partition of each node, by its path, such as `root.concat.children[1].plot`: the
    /// values that apply to every layer and empty cell below it.
    pub partitions: HashMap<String, BTreeMap<String, ArrayRef>>,
}

impl<'a> Tree<'a> {
    /// The tree of `chart`, a chart whose messages are each valid. It fails for a layer or an
    /// empty cell that gets two different values of one partition column, and for two plots,
    /// layers, concats, wraps, or facet headers with one identity.
    pub(crate) fn new(chart: &'a Chart) -> Result<Self> {
        let mut walk = Walk {
            chart,
            ancestors: Vec::new(),
            tree: Tree {
                plots: Vec::new(),
                layers: Vec::new(),
                nodes: Vec::new(),
                headers: Vec::new(),
                partitions: HashMap::new(),
            },
        };
        walk.node(chart.root(), "root".to_string(), &BTreeMap::new())?;
        let tree = walk.tree;
        unique(tree.identities())?;
        Ok(tree)
    }

    /// The identities of the plots, the layers, the concats, the wraps, and the facet headers with
    /// ids, with their paths.
    pub fn identities(&self) -> impl Iterator<Item = (&Identity, &str)> {
        (self.plots.iter())
            .filter_map(|plot| Some((plot.identity.as_ref()?, plot.path.as_str())))
            .chain(
                (self.layers.iter())
                    .filter_map(|layer| Some((layer.identity.as_ref()?, layer.path.as_str()))),
            )
            .chain(
                (self.nodes.iter().chain(&self.headers))
                    .map(|node| (&node.identity, node.path.as_str())),
            )
    }
}

/// Fails for two of `identities` that are equal, naming both by their paths.
fn unique<'a>(identities: impl IntoIterator<Item = (&'a Identity, &'a str)>) -> Result<()> {
    let mut seen: HashMap<Vec<u8>, &str> = HashMap::new();
    for (identity, path) in identities {
        if identity.id.contains('\0') {
            return Err(ChartError::invalid(
                path,
                "an id holds no NUL character, which the owners of widgets' clauses begin with",
            ));
        }
        if let Some(first) = seen.insert(identity.key(), path) {
            return Err(ChartError::invalid(
                path,
                format!(
                    "`{first}` and `{path}` have one id and one partition, `{}`, which is a conflict",
                    identity.name()
                ),
            ));
        }
    }
    Ok(())
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
                let coord = kind.field();
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
                facet,
                ..
            })
            | Node::Wrap(proto::Wrap {
                id,
                children,
                partition,
                facet,
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
                // A concat or a wrap without children holds its place as an empty cell does.
                if children.is_empty() {
                    leaves.push(own.clone());
                }
                let partition = common(&leaves).expect("the node has a leaf");
                self.tree.partitions.insert(path.clone(), partition.clone());
                if let Some(proto::Facet {
                    header: Some(proto::facet::Header::Shown(header)),
                    ..
                }) = facet
                {
                    if !header.id.is_empty() {
                        self.tree.headers.push(NodeRef {
                            path: format!("{path}.facet.shown"),
                            identity: Identity {
                                id: header.id.clone(),
                                partition: partition.clone(),
                            },
                        });
                    }
                }
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

/// What a resolve rule shares: the scale of a channel, or the copies of a widget's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Shares<'a> {
    Channel(Channel),
    Widget(&'a str),
}

impl<'a> Shares<'a> {
    /// What `rule` shares, or `None` for a rule without a key.
    pub(crate) fn of(rule: &'a proto::Resolve) -> Option<Self> {
        match rule.key.as_ref()? {
            proto::resolve::Key::Channel(channel) => Some(Shares::Channel(
                Channel::try_from(*channel).unwrap_or_default(),
            )),
            proto::resolve::Key::Widget(key) => Some(Shares::Widget(key)),
        }
    }
}

/// The sharing of `key` among the children of `node`, a concat or a wrap: its rule, or the default
/// of its kind. A concat or a wrap shares every channel with a facet, and without one, every channel
/// but `x`, `y`, `theta`, and `radius`. It shares the copies of every widget, with or without a
/// facet.
pub(crate) fn sharing(node: &Node, key: Shares) -> Sharing {
    let (rules, facet) = match node {
        Node::Concat(concat) => (&concat.resolve, concat.facet.is_some()),
        Node::Wrap(wrap) => (&wrap.resolve, wrap.facet.is_some()),
        Node::Plot(plot) => return rule(&plot.resolve, key).unwrap_or(Sharing::Shared),
        Node::Empty(_) => return Sharing::Shared,
    };
    let default = match key {
        Shares::Channel(Channel::X | Channel::Y | Channel::Theta | Channel::Radius) if !facet => {
            Sharing::Independent
        }
        _ => Sharing::Shared,
    };
    rule(rules, key).unwrap_or(default)
}

fn rule(rules: &[proto::Resolve], key: Shares) -> Option<Sharing> {
    (rules.iter())
        .find(|rule| Shares::of(rule) == Some(key))
        .and_then(|rule| Sharing::try_from(rule.sharing).ok())
}

/// The sharing of the projections of the map plots below `node`, a concat or a wrap, among its
/// children: its rule, or with a facet one projection and without one, one for each child.
pub(crate) fn projection_sharing(node: &Node) -> Sharing {
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

/// The path of the node at which the layer shares `key`: its plot when the plot shares it among its
/// layers, then its outermost ancestor up to which each ancestor shares it among its children. An
/// inner independent rule therefore wins over an outer shared rule.
fn scope<'a>(tree: &'a Tree, layer: &'a Layer, key: Shares) -> &'a str {
    let plot = &tree.plots[layer.plot];
    if rule(&plot.plot.resolve, key).unwrap_or(Sharing::Shared) != Sharing::Shared {
        return &layer.path;
    }
    let mut scope = plot.path.as_str();
    for ancestor in plot.ancestors.iter().rev() {
        if sharing(ancestor.node, key) != Sharing::Shared {
            break;
        }
        scope = &ancestor.path;
    }
    scope
}

/// The innermost node that holds the nodes at `a` and `b`, by their paths.
pub(crate) fn common_path(a: &str, b: &str) -> String {
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
    /// The spec of the scale, which merges the specs of its encodings. Its domain is the domain of
    /// the first encoding, which for summary and data domains holds the merged fit, and each member
    /// holds its own summary.
    pub spec: proto::Scale,
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
    /// The identity of the scale's axis or legend, when it has an id: the id, with the partition
    /// values that every layer of the scale shares.
    pub fn axis_or_legend_identity(&self, tree: &Tree) -> Option<Identity> {
        let (id, _) = axis_or_legend_id(&self.spec)?;
        let partitions: Vec<BTreeMap<String, ArrayRef>> = (self.members.iter())
            .map(|member| tree.layers[member.layer].partition.clone())
            .collect();
        Some(Identity {
            id: id.to_string(),
            partition: common(&partitions).unwrap_or_default(),
        })
    }

    /// The path of the axis or the legend of the first encoding whose spec gives it its id, such
    /// as `root.plot.cartesian.layers[0].bar.x.scale.axis`.
    fn axis_or_legend_path(&self, tree: &Tree, id: &str) -> String {
        (self.members.iter())
            .find_map(|member| {
                let layer = &tree.layers[member.layer];
                let spec = layer.mark.encoding(member.channel)?.scale.as_ref()?;
                let (found, field) = axis_or_legend_id(spec)?;
                (found == id).then(|| {
                    format!(
                        "{}.{}.{}.scale.{field}",
                        layer.path,
                        layer.mark.name(),
                        member.channel.field_name(),
                    )
                })
            })
            .unwrap_or_else(|| self.owner.clone())
    }

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

/// The id of a scale's axis or legend, when it has one, with the name of the field that holds it.
fn axis_or_legend_id(spec: &proto::Scale) -> Option<(&str, &'static str)> {
    let (id, field) = match (&spec.axis, &spec.legend) {
        (Some(axis), _) => (&axis.id, "axis"),
        (None, Some(legend)) => (&legend.id, "legend"),
        (None, None) => return None,
    };
    (!id.is_empty()).then_some((id.as_str(), field))
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
pub(crate) fn groups(chart: &Chart, tree: &Tree) -> Result<Vec<ScaleGroup>> {
    struct Entry<'a> {
        layer: usize,
        channel: Channel,
        spec: &'a proto::Scale,
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut keys: Vec<Vec<(Channel, bool, &str)>> = Vec::new();
    for (index, layer) in tree.layers.iter().enumerate() {
        for (channel, encoding) in layer.mark.encodings() {
            if let Some(spec) = &encoding.scale {
                let mut entry_keys =
                    vec![(channel, false, scope(tree, layer, Shares::Channel(channel)))];
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
            Some(domain::Domain::Summary(summary)) => summary.value,
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
    /// group has.
    pub spec: &'a proto::Geo,
    /// The map plots, by their index in the plots of the tree, in tree order.
    pub plots: Vec<usize>,
    /// The path of the innermost node that holds every plot of the group.
    pub owner: String,
}

/// The options of a map plot.
fn geo_options<'a>(plot: &PlotRef<'a>) -> &'a proto::Geo {
    match plot.plot.coord.as_ref() {
        Some(Coord::Geo(geo)) => (geo.options.as_ref()).expect("a valid map plot has options"),
        _ => panic!("the plot is a map plot"),
    }
}

/// The projections of the map plots of `tree`. The map plots that share their projection at one
/// node, or whose options name one share family, share one projection, and their surfaces, their
/// aspects, and their navigation must be equal, because they show one view.
pub(crate) fn projections<'a>(tree: &Tree<'a>) -> Result<Vec<ProjectionGroup<'a>>> {
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
            let family = geo_options(plot).family.as_str();
            if !family.is_empty() {
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
                let (a, b) = (group.spec, geo);
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
    match plot.coord.as_ref()? {
        Coord::Polar(polar) => Some(polar_circle(&polar.options.unwrap_or_default())),
        _ => None,
    }
}

/// The start angle, the end angle, and the inner radius of polar options, with their defaults: a
/// whole circle from the top, without a hole.
pub(crate) fn polar_circle(polar: &proto::Polar) -> [f32; 3] {
    let start = polar.start_angle.unwrap_or(0.0);
    [
        start,
        polar.end_angle.unwrap_or(start + 360.0),
        polar.inner_radius.unwrap_or(0.0),
    ]
}

/// The models of a chart that a renderer reads: the tree of its plots and layers, its scales, its
/// projections, and the tables of tracks of its concats and wraps.
pub struct Resolved<'a> {
    /// The plots, the layers, and the identified nodes, in tree order.
    pub tree: Tree<'a>,
    /// The scales: the encodings that share each scale, with the spec that merges their specs.
    pub scales: Vec<ScaleGroup>,
    /// The map plots that share each projection.
    pub projections: Vec<ProjectionGroup<'a>>,
    /// The place of each node in its table of tracks, and the size of each track.
    pub tables: Tables,
    /// The tracks whose lengths the plots of shared position scales and projections match.
    pub matches: Matches,
    /// The widgets: the copies of each key that share one widget, with the declaration that
    /// merges theirs.
    pub widgets: Vec<WidgetGroup>,
}

/// The models of `chart`, whose messages are each valid, from its tree. It fails for a chart whose
/// shared scales, projections, or tracks conflict, and for a guide whose identity is another
/// part's.
pub(crate) fn resolve_tree<'a>(chart: &'a Chart, tree: Tree<'a>) -> Result<Resolved<'a>> {
    let scales = groups(chart, &tree)?;
    check_groups(&tree, &scales)?;
    let guides: Vec<(Identity, String)> = (scales.iter())
        .filter_map(|group| {
            let identity = group.axis_or_legend_identity(&tree)?;
            let path = group.axis_or_legend_path(&tree, &identity.id);
            Some((identity, path))
        })
        .collect();
    unique(
        tree.identities().chain(
            guides
                .iter()
                .map(|(identity, path)| (identity, path.as_str())),
        ),
    )?;
    let projections = projections(&tree)?;
    let widgets = widgets(chart, &tree)?;
    let tables = table::tables(chart)?;
    let matches = table::matches(&tree, &scales, &projections, &tables)?;
    Ok(Resolved {
        tree,
        scales,
        projections,
        tables,
        matches,
        widgets,
    })
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

/// One widget: the copies of one key that share it, as `Widget` describes, and the declaration
/// that merges theirs.
pub struct WidgetGroup {
    /// The key of the copies.
    pub key: String,
    /// The declaration that merges the copies'. Its column is the first copy's, and its domain is
    /// the first copy's, which for summary and data domains holds the merged fit, and each member
    /// holds its own summary.
    pub widget: proto::Widget,
    /// The copies, in tree order.
    pub members: Vec<WidgetMember>,
    /// The identity of the anchor of the group, or `None` for a group that anchors at the root. A
    /// clause of the widget names the anchor as its owner and its partition.
    pub anchor: Option<Identity>,
    /// The path of the innermost node that holds every layer of the group, whose part rules decide
    /// how many times the widget draws.
    pub owner: String,
}

/// A copy of a widget: the layer that declares it, the column of the layer that the widget's
/// clause checks, and where its part of the domain comes from.
pub struct WidgetMember {
    /// The position of the layer in `Tree::layers`.
    pub layer: usize,
    /// The position of the copy in the layer's widgets.
    pub index: usize,
    /// The column of the layer's table, or a partition column of the layer.
    pub column: String,
    /// The copy's summary, for a summary domain.
    pub summary: Option<proto::ValueRef>,
}

/// The type of `column` of `layer`: of its table, or of its partition value, without a
/// dictionary.
fn widget_column_type(layer: &Layer, column: &str) -> Option<arrow::datatypes::DataType> {
    let data_type = match layer.table.column_by_name(column) {
        Some(values) => values.data_type().clone(),
        None => layer.partition.get(column)?.data_type().clone(),
    };
    Some(crate::validate::dictionary_values(&data_type).clone())
}

/// The widgets of the layers of `tree`. The copies of one key whose layers share it at one node
/// are one group. The copies of each group must agree, the columns of their layers have one type,
/// and the anchor of each group other than the root has an identity.
pub(crate) fn widgets(chart: &Chart, tree: &Tree) -> Result<Vec<WidgetGroup>> {
    let same = |a: proto::ValueRef, b: proto::ValueRef| {
        chart.value(&a).as_ref() == chart.value(&b).as_ref()
    };
    let mut groups: Vec<WidgetGroup> = Vec::new();
    let mut group_of: HashMap<(&str, &str), usize> = HashMap::new();
    for (position, layer) in tree.layers.iter().enumerate() {
        for (index, declared) in layer.layer.widgets().iter().enumerate() {
            let path = format!("{}.widgets[{index}]", layer.path);
            let (anchor, identity) = anchor(tree, layer, &declared.key);
            let summary = match declared.domain() {
                Some(proto::widget_domain::Domain::Summary(summary)) => summary.values,
                _ => None,
            };
            let member = WidgetMember {
                layer: position,
                index,
                column: declared.column().to_string(),
                summary,
            };
            match group_of.get(&(declared.key.as_str(), anchor)) {
                Some(&group) => {
                    let merged = &mut groups[group];
                    let first = &merged.members[0];
                    let first_path =
                        format!("{}.widgets[{}]", tree.layers[first.layer].path, first.index);
                    let types = (
                        widget_column_type(&tree.layers[first.layer], &first.column),
                        widget_column_type(layer, declared.column()),
                    );
                    if types.0 != types.1 {
                        return Err(ChartError::invalid(
                            path,
                            format!(
                                "the copies of the widget `{}` at `{first_path}` and here share \
                                 one widget, and their columns hold different types",
                                declared.key
                            ),
                        ));
                    }
                    if let Err(reason) = merge::merge_widget(&mut merged.widget, declared, &same) {
                        return Err(ChartError::invalid(
                            path,
                            format!(
                                "the copies of the widget `{}` at `{first_path}` and here share \
                                 one widget and {reason}",
                                declared.key
                            ),
                        ));
                    }
                    merged.owner = common_path(&merged.owner, &layer.path);
                    merged.members.push(member);
                }
                None => {
                    let identity = identity.ok_or_else(|| {
                        ChartError::invalid(
                            anchor,
                            format!(
                                "a resolve rule makes the widget `{}` independent above this \
                                 node, so the node anchors one of its widgets and has an id",
                                declared.key
                            ),
                        )
                    })?;
                    group_of.insert((declared.key.as_str(), anchor), groups.len());
                    groups.push(WidgetGroup {
                        key: declared.key.clone(),
                        widget: declared.clone(),
                        members: vec![member],
                        anchor: identity,
                        owner: layer.path.clone(),
                    });
                }
            }
        }
    }
    Ok(groups)
}

/// The anchor of the group of the widget `key` that `layer` declares: its path, and its identity,
/// `Some(None)` for a group that anchors at the root, the node at a path of one step, and `None`
/// for an anchor without an id.
fn anchor<'t>(tree: &'t Tree, layer: &'t Layer, key: &str) -> (&'t str, Option<Option<Identity>>) {
    let path = scope(tree, layer, Shares::Widget(key));
    if path.matches('.').count() == 1 {
        return (path, Some(None));
    }
    let identity = (tree.plots.iter())
        .find(|plot| plot.path == path)
        .and_then(|plot| plot.identity.as_ref())
        .or_else(|| {
            (tree.layers.iter())
                .find(|layer| layer.path == path)
                .and_then(|layer| layer.identity.as_ref())
        })
        .or_else(|| {
            (tree.nodes.iter())
                .find(|node| node.path == path)
                .map(|node| &node.identity)
        });
    (path, identity.map(|identity| Some(identity.clone())))
}

/// The owner of the clause of the widget `key` that `layer` declares: its anchor's identity, or an
/// identity with an empty id and no partition for a group that anchors at the root. `None` for an
/// anchor without an id, which a valid chart does not have.
pub(crate) fn widget_owner(tree: &Tree, layer: &Layer, key: &str) -> Option<Identity> {
    let (_, identity) = anchor(tree, layer, key);
    Some(identity?.unwrap_or(Identity {
        id: String::new(),
        partition: BTreeMap::new(),
    }))
}

impl proto::Widget {
    /// The selection and the column that the widget's kind writes.
    pub fn target(&self) -> Option<&proto::WidgetTarget> {
        use proto::widget::Kind;
        match self.kind.as_ref()? {
            Kind::Slider(slider) => slider.target.as_ref(),
            Kind::RangeSlider(slider) => slider.target.as_ref(),
            Kind::Radio(radio) => radio.target.as_ref(),
            Kind::Checkboxes(checkboxes) => checkboxes.target.as_ref(),
            Kind::Checkbox(checkbox) => checkbox.target.as_ref(),
        }
    }

    /// The domain of the widget's kind, or `None` for a check box, which has none.
    pub fn domain(&self) -> Option<&proto::widget_domain::Domain> {
        use proto::widget::Kind;
        let domain = match self.kind.as_ref()? {
            Kind::Slider(slider) => slider.domain.as_ref(),
            Kind::RangeSlider(slider) => slider.domain.as_ref(),
            Kind::Radio(radio) => radio.domain.as_ref(),
            Kind::Checkboxes(checkboxes) => checkboxes.domain.as_ref(),
            Kind::Checkbox(_) => None,
        };
        domain?.domain.as_ref()
    }

    /// The column of the widget's target, or an empty name for a widget without one.
    pub fn column(&self) -> &str {
        self.target().map_or("", |target| target.column.as_str())
    }
}
