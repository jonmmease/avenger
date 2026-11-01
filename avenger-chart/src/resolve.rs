//! The grouping of the channels of a chart into scales: each group is one scale, which its
//! encodings, and the second channels of their pairs, go through.

use arrow::array::RecordBatch;

use crate::{
    proto::{self, Channel},
    validate::is_position,
    Chart, CoordKind,
};

/// A unit of a chart, by its place in the tree.
pub struct UnitRef<'a> {
    /// Its node path, such as `root/children[1]`.
    pub path: String,
    /// Its id, or its node path when it has none, which names its marks.
    pub name: String,
    /// The unit.
    pub unit: &'a proto::Unit,
    /// The table of the unit.
    pub table: &'a RecordBatch,
    /// The plot that the unit draws into.
    pub plot: usize,
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

/// One scale: the encodings that go through it, with the second channels of their pairs, and its
/// spec.
pub struct ScaleGroup {
    /// The channel of the scale, the first channel of a pair.
    pub channel: Channel,
    /// The spec of the scale, from the encoding of its channel.
    pub spec: proto::ScaleSpec,
    /// The encodings of the scale, in tree order and in the order of their channels.
    pub members: Vec<Member>,
    /// The plot of a position scale.
    pub plot: Option<usize>,
    /// The kind of the coordinate system of the plots of a position scale. Its channel is a
    /// position channel of the kind.
    pub kind: Option<CoordKind>,
}

impl ScaleGroup {
    /// Whether the scale is a position scale that runs along a side of its plots: its channel is
    /// an edge channel of their kind, such as `x` of a Cartesian plot.
    pub fn is_edge(&self) -> bool {
        (self.kind).is_some_and(|kind| kind.edges().contains(&self.channel))
    }
}

/// An encoding of a scale group: the channel of a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    /// The index of the unit in the units of the chart.
    pub unit: usize,
    /// The channel of the unit.
    pub channel: Channel,
}

/// The scale groups of `units`. Each channel of a unit that has a scale is one group, which the
/// second channel of its pair joins. A channel without a scale joins no group, so it has no
/// guide.
pub fn groups(units: &[UnitRef]) -> Vec<ScaleGroup> {
    let mut groups: Vec<ScaleGroup> = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        let first = groups.len();
        for encoding in &unit.unit.encodings {
            let channel = Channel::try_from(encoding.channel).unwrap_or(Channel::Unspecified);
            if let Some(spec) = &encoding.scale {
                groups.push(ScaleGroup {
                    channel,
                    spec: spec.clone(),
                    members: vec![Member {
                        unit: index,
                        channel,
                    }],
                    plot: is_position(channel).then_some(unit.plot),
                    kind: is_position(channel).then(|| CoordKind::of(unit.unit.coord.as_ref())),
                });
            }
        }
        for encoding in &unit.unit.encodings {
            let channel = Channel::try_from(encoding.channel).unwrap_or(Channel::Unspecified);
            let first_channel = scale_channel(channel);
            if first_channel == channel || encoding.scale.is_some() {
                continue;
            }
            if let Some(group) = groups[first..]
                .iter_mut()
                .find(|group| group.channel == first_channel)
            {
                group.members.push(Member {
                    unit: index,
                    channel,
                });
            }
        }
    }
    groups
}

/// The units of `chart` in tree order. A chart that an evaluator draws has a unit at its root.
pub fn units(chart: &Chart) -> Vec<UnitRef<'_>> {
    match &chart.root().node {
        Some(proto::node::Node::Unit(unit)) => vec![UnitRef {
            path: "root".to_string(),
            name: if unit.id.is_empty() {
                "root".to_string()
            } else {
                unit.id.clone()
            },
            unit,
            table: chart.table(unit),
            plot: 0,
        }],
        _ => Vec::new(),
    }
}
