//! The grouping of the channels of a chart into scales: each group is one scale, which its
//! encodings, and the second channels of their pairs, go through. The resolve rules of overlays,
//! concats, and wraps and the share families of specs decide which channels share a scale, and
//! the specs of each group merge by the agreement rule.

use std::collections::{BTreeMap, HashMap};

use arrow::{
    array::{ArrayRef, RecordBatch},
    util::display::{ArrayFormatter, FormatOptions},
};

use crate::{
    merge,
    proto::{self, domain, node::Node, Channel, Sharing},
    table,
    validate::{first_coord, is_position},
    Chart, ChartError, Contribution,
};

pub use crate::tree::partition;

type Result<T> = std::result::Result<T, ChartError>;

/// A node above a unit, which shares or separates each channel among its children.
pub(crate) struct Ancestor<'a> {
    path: String,
    node: &'a Node,
}

impl Ancestor<'_> {
    fn sharing(&self, channel: Channel) -> Sharing {
        sharing(self.node, channel)
    }
}

/// The sharing of `channel` among the children of `node`: its rule, or the default of its kind.
/// An overlay shares every channel. A concat or a wrap shares every channel with a facet, and
/// without one, every channel but `x`, `y`, `theta`, and `radius`.
pub(crate) fn sharing(node: &Node, channel: Channel) -> Sharing {
    let (rules, facet) = match node {
        Node::Overlay(overlay) => (&overlay.resolve, true),
        Node::Concat(concat) => (&concat.resolve, concat.facet.is_some()),
        Node::Wrap(wrap) => (&wrap.resolve, wrap.facet.is_some()),
        Node::Unit(_) | Node::Empty(_) => return Sharing::Shared,
    };
    let default = if facet
        || !matches!(
            channel,
            Channel::X | Channel::Y | Channel::Theta | Channel::Radius
        ) {
        Sharing::Shared
    } else {
        Sharing::Independent
    };
    rules
        .iter()
        .find(|rule| rule.channel == channel as i32)
        .and_then(|rule| Sharing::try_from(rule.sharing).ok())
        .unwrap_or(default)
}

/// The sharing of the projections of the map plots below `node` among its children: its rule, or
/// the default of its kind. An overlay shares its projection, because its children draw into one
/// plot, and a concat or a wrap shares it with a facet and keeps one for each child without one.
pub fn projection_sharing(node: &Node) -> Sharing {
    let (rule, facet) = match node {
        Node::Concat(concat) => (concat.projection, concat.facet.is_some()),
        Node::Wrap(wrap) => (wrap.projection, wrap.facet.is_some()),
        Node::Overlay(_) | Node::Unit(_) | Node::Empty(_) => return Sharing::Shared,
    };
    match Sharing::try_from(rule) {
        Ok(sharing @ (Sharing::Shared | Sharing::Independent)) => sharing,
        _ if facet => Sharing::Shared,
        _ => Sharing::Independent,
    }
}

/// A unit of a chart, by its place in the tree.
pub struct UnitRef<'a> {
    /// Its node path, such as `root/children[1]`.
    pub path: String,
    /// The unit.
    pub unit: &'a proto::Unit,
    /// The table of the unit.
    pub table: &'a RecordBatch,
    /// The plot that the unit draws into.
    pub plot: usize,
    /// The position of the unit among the units of its plot, in tree order, which names its
    /// marks.
    pub position: usize,
    /// The nodes above the unit, from the root.
    ancestors: Vec<Ancestor<'a>>,
    /// The partition values of the unit, its own and its ancestors'.
    pub partition: BTreeMap<String, ArrayRef>,
    /// The identities of the unit and the overlays that hold it, those with ids, whose
    /// contributions `exclude_own` skips.
    pub owners: Vec<Identity>,
}

/// A plot: a unit outside an overlay, or an outermost overlay, which draws its units into one
/// rectangle.
pub struct PlotRef<'a> {
    /// Its node path.
    pub path: String,
    /// The name of its identity, or its node path when its node has no id, which names its
    /// marks. The plots of a chart have different names.
    pub name: String,
    /// The identity of its node, when it has an id.
    pub identity: Option<Identity>,
    /// Its coordinate system: its own, or for an overlay, the one that its children set.
    pub coord: Option<&'a proto::Coord>,
    /// The attribution of its node.
    pub attribution: Option<&'a proto::Attribution>,
    /// The tools that its node and the nodes inside it declare, in tree order.
    pub tools: Vec<&'a proto::Tool>,
}

/// The plots and the units of a chart, in tree order.
pub struct Tree<'a> {
    /// The plots, in tree order. A unit refers to its plot by its index.
    pub plots: Vec<PlotRef<'a>>,
    /// The units, in tree order. A member of a scale group refers to its unit by its index.
    pub units: Vec<UnitRef<'a>>,
}

/// The identity of a node with an id: its id and its partition, its own values and those of its
/// ancestors. The nodes of a chart have different identities, so the plots of the facets of one
/// table can share an id, and a node of a new chart with the same identity stands for the same
/// part, whatever its place in the tree.
#[derive(Clone, Debug)]
pub struct Identity {
    /// The id of the node.
    pub id: String,
    /// The partition values of the node, its own and its ancestors', by column.
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
    /// The name of the node: its id, with its partition values after it in brackets when it has
    /// any, such as `panel[era=1990s, genre=Drama]`.
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

    /// Whether the node wrote `contribution`: the contribution names the node's id and its
    /// partition values.
    pub fn wrote(&self, contribution: &Contribution) -> bool {
        self.id == contribution.owner
            && self.partition.len() == contribution.partition.len()
            && self.partition.iter().all(|(column, value)| {
                contribution
                    .partition
                    .get(column)
                    .is_some_and(|other| other.0.as_ref() == value.as_ref())
            })
    }
}

/// The plots and the units of `chart`.
pub fn tree(chart: &Chart) -> Tree<'_> {
    let mut walk = Walk {
        chart,
        ancestors: Vec::new(),
        tree: Tree {
            plots: Vec::new(),
            units: Vec::new(),
        },
    };
    if let Some(root) = &chart.root().node {
        walk.node(root, "root".to_string(), &BTreeMap::new(), &[], None);
    }
    walk.tree
}

/// A walk over the nodes of a chart, which collects its plots and its units.
struct Walk<'a> {
    chart: &'a Chart,
    /// The nodes above the node that the walk visits, from the root.
    ancestors: Vec<Ancestor<'a>>,
    tree: Tree<'a>,
}

impl<'a> Walk<'a> {
    /// Visits `node` at `path`, with the partition values of its ancestors, the owners of
    /// contributions that hold it in its plot, and its plot when an overlay holds it.
    fn node(
        &mut self,
        node: &'a Node,
        path: String,
        partition: &BTreeMap<String, ArrayRef>,
        owners: &[Identity],
        plot: Option<usize>,
    ) {
        let (id, own) = match node {
            Node::Unit(unit) => (&unit.id, &unit.partition),
            Node::Overlay(overlay) => (&overlay.id, &overlay.partition),
            Node::Concat(concat) => (&concat.id, &concat.partition),
            Node::Wrap(wrap) => (&wrap.id, &wrap.partition),
            Node::Empty(_) => return,
        };
        let mut partition = partition.clone();
        for value in own {
            partition.insert(value.column.clone(), self.chart.value(value.value).clone());
        }
        let identity = (!id.is_empty()).then(|| Identity {
            id: id.clone(),
            partition: partition.clone(),
        });
        let mut owners = owners.to_vec();
        if matches!(node, Node::Unit(_) | Node::Overlay(_)) {
            owners.extend(identity.clone());
        }
        let plots = &mut self.tree.plots;
        let mut new_plot = |coord, attribution| {
            plots.push(PlotRef {
                path: path.clone(),
                name: identity
                    .as_ref()
                    .map_or_else(|| path.clone(), Identity::name),
                identity: identity.clone(),
                coord,
                attribution,
                tools: Vec::new(),
            });
            plots.len() - 1
        };
        match node {
            Node::Unit(unit) => {
                let plot = plot
                    .unwrap_or_else(|| new_plot(unit.coord.as_ref(), unit.attribution.as_ref()));
                self.tree.plots[plot].tools.extend(&unit.tools);
                let units = &mut self.tree.units;
                let position = units.iter().filter(|unit| unit.plot == plot).count();
                units.push(UnitRef {
                    path,
                    unit,
                    table: self.chart.table(unit),
                    plot,
                    position,
                    ancestors: self
                        .ancestors
                        .iter()
                        .map(|ancestor| Ancestor {
                            path: ancestor.path.clone(),
                            node: ancestor.node,
                        })
                        .collect(),
                    partition,
                    owners,
                });
            }
            Node::Overlay(overlay) => {
                let plot = plot.unwrap_or_else(|| {
                    new_plot(
                        overlay
                            .coord
                            .as_ref()
                            .or_else(|| first_coord(&overlay.children)),
                        overlay.attribution.as_ref(),
                    )
                });
                self.tree.plots[plot].tools.extend(&overlay.tools);
                self.children(
                    node,
                    &overlay.children,
                    path,
                    &partition,
                    &owners,
                    Some(plot),
                );
            }
            // The plots below a concat or a wrap own their own contributions.
            Node::Concat(concat) => {
                self.children(node, &concat.children, path, &partition, &[], None)
            }
            Node::Wrap(wrap) => self.children(node, &wrap.children, path, &partition, &[], None),
            Node::Empty(_) => {}
        }
    }

    fn children(
        &mut self,
        node: &'a Node,
        children: &'a [proto::Node],
        path: String,
        partition: &BTreeMap<String, ArrayRef>,
        owners: &[Identity],
        plot: Option<usize>,
    ) {
        self.ancestors.push(Ancestor {
            path: path.clone(),
            node,
        });
        for (index, child) in children.iter().enumerate() {
            if let Some(child_node) = &child.node {
                self.node(
                    child_node,
                    format!("{path}/children[{index}]"),
                    partition,
                    owners,
                    plot,
                );
            }
        }
        self.ancestors.pop();
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

/// The path of the node at which `unit` shares `channel`: its outermost ancestor up to which each
/// ancestor shares the channel among its children, or the unit itself. An inner independent rule
/// therefore wins over an outer shared rule.
fn scope<'a>(unit: &'a UnitRef, channel: Channel) -> &'a str {
    let mut scope = unit.path.as_str();
    for ancestor in unit.ancestors.iter().rev() {
        if ancestor.sharing(channel) != Sharing::Shared {
            break;
        }
        scope = &ancestor.path;
    }
    scope
}

/// The innermost node that holds the nodes at `a` and `b`, by their paths.
pub fn common(a: &str, b: &str) -> String {
    let a: Vec<&str> = a.split('/').collect();
    let b: Vec<&str> = b.split('/').collect();
    a.iter()
        .zip(&b)
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| *a)
        .collect::<Vec<_>>()
        .join("/")
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
    /// The path of the node that owns the scale: the innermost node that holds every unit of the
    /// scale, which declares the rules of its guides and holds its legend when the rule merges it.
    pub owner: String,
}

impl ScaleGroup {
    /// Whether the scale is a position scale, which fits at the length of its plot.
    pub fn is_position(&self) -> bool {
        is_position(self.channel)
    }
}

/// An encoding of a scale group: the channel of a unit, and where its part of the domain comes
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    /// The index of the unit in the units of the tree.
    pub unit: usize,
    /// The channel of the unit.
    pub channel: Channel,
    /// The index in `Chart.values` of the producer's summary when the domain of the unit's spec
    /// is a summary, which covers the values of both channels of a pair. An evaluator summarizes
    /// the data of the other members of an inferred domain.
    pub summary: Option<u32>,
}

/// An encoding with a scale, before the grouping.
struct Entry<'a> {
    unit: usize,
    channel: Channel,
    spec: &'a proto::ScaleSpec,
    scope: &'a str,
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

/// The scale groups of `units`. The encodings of one channel whose units share it at one node,
/// or whose specs name one share family, are one group, which the second channels of their pairs
/// join. A channel without a scale joins no group, so it has no guide. The specs of each group
/// must agree, and an error names the channel and the two units whose specs disagree.
pub fn groups(chart: &Chart, units: &[UnitRef]) -> Result<Vec<ScaleGroup>> {
    let mut entries: Vec<Entry> = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        for encoding in &unit.unit.encodings {
            let channel = Channel::try_from(encoding.channel).unwrap_or(Channel::Unspecified);
            if let Some(spec) = &encoding.scale {
                entries.push(Entry {
                    unit: index,
                    channel,
                    spec,
                    scope: scope(unit, channel),
                });
            }
        }
    }

    // Join the entries of one channel that share it at one node or name one family.
    let mut parents: Vec<usize> = (0..entries.len()).collect();
    let mut first: HashMap<(Channel, bool, &str), usize> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let keys = [
            Some((entry.channel, false, entry.scope)),
            (!entry.spec.family.is_empty()).then_some((entry.channel, true, &*entry.spec.family)),
        ];
        for key in keys.into_iter().flatten() {
            match first.get(&key) {
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

    let same = |a: u32, b: u32| chart.value(a).as_ref() == chart.value(b).as_ref();
    let mut groups: Vec<ScaleGroup> = Vec::new();
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    let mut entries_of: Vec<Vec<usize>> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let root = find(&mut parents, index);
        let summary = match entry
            .spec
            .domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref())
        {
            Some(domain::Domain::Summary(index)) => Some(*index),
            _ => None,
        };
        let member = Member {
            unit: entry.unit,
            channel: entry.channel,
            summary,
        };
        let group = match group_of.get(&root) {
            Some(&group) => {
                let group_entries = &entries_of[group];
                let merged = &mut groups[group];
                if let Err(reason) = merge::merge(&mut merged.spec, entry.spec, &same) {
                    // The earliest member whose own spec disagrees with this one names the pair.
                    let earlier = group_entries
                        .iter()
                        .map(|&earlier| &entries[earlier])
                        .find_map(|earlier| {
                            let mut spec = earlier.spec.clone();
                            merge::merge(&mut spec, entry.spec, &same)
                                .err()
                                .map(|reason| (earlier, reason))
                        });
                    let (earlier, reason) = earlier.unwrap_or((&entries[group_entries[0]], reason));
                    return Err(ChartError::invalid(
                        units[entry.unit].path.clone(),
                        format!(
                            "the `{}` specs of `{}` and `{}` share a scale and {reason}",
                            entry.channel.as_str_name(),
                            units[earlier.unit].path,
                            units[entry.unit].path,
                        ),
                    ));
                }
                merged.owner = common(&merged.owner, &units[entry.unit].path);
                merged.members.push(member);
                entries_of[group].push(index);
                group
            }
            None => {
                groups.push(ScaleGroup {
                    channel: entry.channel,
                    spec: entry.spec.clone(),
                    members: vec![member],
                    owner: units[entry.unit].path.clone(),
                });
                entries_of.push(vec![index]);
                group_of.insert(root, groups.len() - 1);
                groups.len() - 1
            }
        };
        // The second channel of the pair follows its first.
        let unit = units[entry.unit].unit;
        for encoding in &unit.encodings {
            let channel = Channel::try_from(encoding.channel).unwrap_or(Channel::Unspecified);
            if channel != entry.channel && scale_channel(channel) == entry.channel {
                groups[group].members.push(Member {
                    unit: entry.unit,
                    channel,
                    summary,
                });
            }
        }
    }
    Ok(groups)
}

/// One projection: the map plots that share it, which fit one view at one size, and its spec.
pub struct ProjectionGroup<'a> {
    /// The coordinate system of the first plot, whose surface, fit, and aspect every plot of the
    /// group has.
    pub spec: &'a proto::Geo,
    /// The map plots, by their index in the plots of the tree, in tree order.
    pub plots: Vec<usize>,
    /// The path of the innermost node that holds every plot of the group, which declares the
    /// match of their lengths.
    pub owner: String,
}

/// The path of the node at which the map plot `plot` shares its projection: its outermost
/// ancestor up to which each ancestor shares the projection among its children, or the plot
/// itself.
fn projection_scope<'a>(tree: &'a Tree, plot: usize) -> &'a str {
    let path = tree.plots[plot].path.as_str();
    let Some(unit) = tree.units.iter().find(|unit| unit.plot == plot) else {
        return path;
    };
    let mut scope = path;
    for ancestor in unit.ancestors.iter().rev() {
        // The overlays inside the plot share their projection.
        if ancestor.path.len() >= path.len() {
            continue;
        }
        if projection_sharing(ancestor.node) != Sharing::Shared {
            break;
        }
        scope = &ancestor.path;
    }
    scope
}

/// The projections of the map plots of `tree`. The map plots that share their projection at one
/// node, or whose specs name one share family, share one projection, and their surfaces, their
/// fits, and their aspects must be equal, because they fit one view.
pub fn projections<'a>(tree: &'a Tree) -> Result<Vec<ProjectionGroup<'a>>> {
    let maps: Vec<(usize, &proto::Geo)> = tree
        .plots
        .iter()
        .enumerate()
        .filter_map(|(index, plot)| match plot.coord?.coord.as_ref()? {
            proto::coord::Coord::Geo(geo) => Some((index, geo)),
            _ => None,
        })
        .collect();
    let mut parents: Vec<usize> = (0..maps.len()).collect();
    let mut first: HashMap<(bool, &str), usize> = HashMap::new();
    for (index, (plot, geo)) in maps.iter().enumerate() {
        let keys = [
            Some((false, projection_scope(tree, *plot))),
            (!geo.family.is_empty()).then_some((true, geo.family.as_str())),
        ];
        for key in keys.into_iter().flatten() {
            match first.get(&key) {
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

    let mut groups: Vec<ProjectionGroup> = Vec::new();
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    for (index, (plot, geo)) in maps.iter().enumerate() {
        let root = find(&mut parents, index);
        let path = &tree.plots[*plot].path;
        match group_of.get(&root) {
            Some(&group) => {
                let group = &mut groups[group];
                let differs = [
                    (group.spec.surface != geo.surface, "surfaces"),
                    (group.spec.fit != geo.fit, "fits"),
                    (group.spec.aspect != geo.aspect, "aspects"),
                ]
                .into_iter()
                .find_map(|(differs, what)| differs.then_some(what));
                if let Some(what) = differs {
                    return Err(ChartError::invalid(
                        path.clone(),
                        format!(
                            "the map plots `{}` and `{path}` share a projection, so they fit one \
                             view, and their {what} differ",
                            tree.plots[group.plots[0]].path
                        ),
                    ));
                }
                group.owner = common(&group.owner, path);
                group.plots.push(*plot);
            }
            None => {
                groups.push(ProjectionGroup {
                    spec: geo,
                    plots: vec![*plot],
                    owner: path.clone(),
                });
                group_of.insert(root, groups.len() - 1);
            }
        }
    }
    Ok(groups)
}

/// The polar coordinate system of `plot`, when it is a polar plot.
fn polar<'a>(plot: &PlotRef<'a>) -> Option<&'a proto::Polar> {
    match plot.coord?.coord.as_ref()? {
        proto::coord::Coord::Polar(polar) => Some(polar),
        _ => None,
    }
}

/// The angles and the inner radius of a polar plot, with their defaults.
fn circle(polar: &proto::Polar) -> [f32; 3] {
    let start = polar.start_angle.unwrap_or(0.0);
    [
        start,
        polar.end_angle.unwrap_or(start + 360.0),
        polar.inner_radius.unwrap_or(0.0),
    ]
}

/// Checks that the specs of the scales of `chart` agree, that each Cartesian plot has at most two
/// scales of `x` and of `y`, one for the axis on each side, and each polar plot one scale of
/// `theta` and one of `radius`, that the polar plots of a shared `theta` or `radius` draw one
/// part of a circle, that the map plots of each projection agree, that the concats of a table
/// declare one size for each track, and that the plots of a shared `x`, `y`, or projection can
/// take one length.
pub(crate) fn check(chart: &Chart) -> Result<()> {
    let tree = tree(chart);
    let groups = groups(chart, &tree.units)?;
    let projections = projections(&tree)?;
    let tables = table::tables(chart)?;
    table::matches(&tree, &groups, &projections, &tables)?;
    for (index, plot) in tree.plots.iter().enumerate() {
        let limits = match polar(plot) {
            Some(_) => [(Channel::Theta, 1), (Channel::Radius, 1)],
            None => [(Channel::X, 2), (Channel::Y, 2)],
        };
        for (channel, limit) in limits {
            let count = groups
                .iter()
                .filter(|group| {
                    group.channel == channel
                        && group
                            .members
                            .iter()
                            .any(|member| tree.units[member.unit].plot == index)
                })
                .count();
            if count > limit {
                let reason = if limit == 1 {
                    format!(
                        "a polar plot has one scale of `{}`, and it has {count}",
                        channel.as_str_name()
                    )
                } else {
                    format!(
                        "a plot has at most two scales of `{}`, one on each side, and it has \
                         {count}",
                        channel.as_str_name()
                    )
                };
                return Err(ChartError::invalid(plot.path.clone(), reason));
            }
        }
    }
    // The polar plots of a shared scale take one outer radius and one range of the scale.
    for group in &groups {
        if !matches!(group.channel, Channel::Theta | Channel::Radius) {
            continue;
        }
        let mut first: Option<(&str, [f32; 3])> = None;
        for member in &group.members {
            let plot = &tree.plots[tree.units[member.unit].plot];
            let Some(spec) = polar(plot) else {
                continue;
            };
            match first {
                None => first = Some((&plot.path, circle(spec))),
                Some((path, known)) if known != circle(spec) => {
                    return Err(ChartError::invalid(
                        plot.path.clone(),
                        format!(
                            "the polar plots `{path}` and `{}` share `{}`, so they draw one part \
                             of a circle, and their angles or inner radii differ",
                            plot.path,
                            group.channel.as_str_name()
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}
