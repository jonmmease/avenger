//! The grouping of the channels of a chart into scales: each group is one scale, which its
//! encodings, and the second channels of their pairs, go through. The resolve rules of overlays,
//! concats, and wraps and the share families of specs decide which channels share a scale, and
//! the specs of each group merge by the agreement rule.

use std::collections::{BTreeMap, HashMap};

use arrow::array::{ArrayRef, RecordBatch};

use crate::{
    merge,
    proto::{self, domain, node::Node, Channel, Sharing},
    validate::{first_coord, is_position},
    Chart, ChartError, Contribution,
};

type Result<T> = std::result::Result<T, ChartError>;

/// A node above a unit, which shares or separates each channel among its children.
pub(crate) struct Ancestor<'a> {
    path: String,
    node: &'a Node,
}

impl Ancestor<'_> {
    /// The sharing of `channel` among the children of the node: its rule, or the default of its
    /// kind. An overlay shares every channel. A concat or a wrap shares every channel with a
    /// facet, and without one, every channel but `x`, `y`, `theta`, and `radius`.
    fn sharing(&self, channel: Channel) -> Sharing {
        let (rules, facet) = match self.node {
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
    /// The unit and the overlays that hold it, those with ids, whose contributions
    /// `exclude_own` skips.
    pub owners: Vec<Owner>,
}

/// A plot: a unit outside an overlay, or an outermost overlay, which draws its units into one
/// rectangle.
pub struct PlotRef<'a> {
    /// Its node path.
    pub path: String,
    /// The id of its node, or its node path when it has none, which names its marks.
    pub name: String,
    /// Its coordinate system: its own, or for an overlay, the one that its children set.
    pub coord: Option<&'a proto::Coord>,
    /// The title of its node.
    pub title: Option<&'a proto::Title>,
    /// The attribution of its node.
    pub attribution: Option<&'a proto::Attribution>,
}

/// The plots and the units of a chart, in tree order.
pub struct Tree<'a> {
    /// The plots, in tree order. A unit refers to its plot by its index.
    pub plots: Vec<PlotRef<'a>>,
    /// The units, in tree order. A member of a scale group refers to its unit by its index.
    pub units: Vec<UnitRef<'a>>,
}

/// A node that can own contributions: its id and its partition, its own values and those of its
/// ancestors.
#[derive(Clone, Debug)]
pub struct Owner {
    /// The id of the node.
    pub id: String,
    /// The partition values of the node, its own and its ancestors', by column.
    pub partition: BTreeMap<String, ArrayRef>,
}

impl Owner {
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
        owners: &[Owner],
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
        let mut owners = owners.to_vec();
        if !id.is_empty() && matches!(node, Node::Unit(_) | Node::Overlay(_)) {
            owners.push(Owner {
                id: id.clone(),
                partition: partition.clone(),
            });
        }
        let plots = &mut self.tree.plots;
        let mut new_plot = |coord, title, attribution| {
            plots.push(PlotRef {
                path: path.clone(),
                name: if id.is_empty() {
                    path.clone()
                } else {
                    id.clone()
                },
                coord,
                title,
                attribution,
            });
            plots.len() - 1
        };
        match node {
            Node::Unit(unit) => {
                let plot = plot.unwrap_or_else(|| {
                    new_plot(
                        unit.coord.as_ref(),
                        unit.title.as_ref(),
                        unit.attribution.as_ref(),
                    )
                });
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
                        overlay.title.as_ref(),
                        overlay.attribution.as_ref(),
                    )
                });
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
        owners: &[Owner],
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
fn common(a: &str, b: &str) -> String {
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
    /// The path of the node that owns the scale: the innermost node at which every member shares
    /// it, so the discrete legends of one owner can merge.
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
                merged.owner = common(&merged.owner, entry.scope);
                merged.members.push(member);
                entries_of[group].push(index);
                group
            }
            None => {
                groups.push(ScaleGroup {
                    channel: entry.channel,
                    spec: entry.spec.clone(),
                    members: vec![member],
                    owner: entry.scope.to_string(),
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

/// Checks that the specs of the scales of `chart` agree, and that each plot has at most two
/// scales of `x` and of `y`, one for the axis on each side.
pub(crate) fn check(chart: &Chart) -> Result<()> {
    let tree = tree(chart);
    let groups = groups(chart, &tree.units)?;
    for (index, plot) in tree.plots.iter().enumerate() {
        for channel in [Channel::X, Channel::Y] {
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
            if count > 2 {
                return Err(ChartError::invalid(
                    plot.path.clone(),
                    format!(
                        "a plot has at most two scales of `{}`, one on each side, and it has \
                         {count}",
                        channel.as_str_name()
                    ),
                ));
            }
        }
    }
    Ok(())
}
