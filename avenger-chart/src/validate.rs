//! The checks of the rules of the format, which decoding and the builder run on every chart that
//! they return. Each error names the path of the part that breaks a rule, such as
//! `root/children[1]/encodings[0]/scale`.
//!
//! The agreement of the specs of shared scales, and the count of the scales of each direction of a
//! plot, depend on how the resolve rules and the share families group the channels of a tree, so
//! the last check groups them as the renderer does.

use std::collections::{HashMap, HashSet};

use arrow::{
    array::{Array, ArrayRef, AsArray, RecordBatch, Scalar},
    compute::kernels::cmp::{lt, not_distinct},
    datatypes::DataType,
    row::{RowConverter, SortField},
};
use avenger_scales::scales::{
    domain_summary::DomainSummary, DomainKind, OptionDefinition, ScaleImpl,
};

use crate::{
    payload,
    proto::{
        self, category_fit, condition, coord, domain, domain_fit, encoding, geo, guide, mark,
        node::Node, predicate, range, select, tool, track, tuple_field, Channel,
    },
    resolve, table, tree, Chart, ChartError,
};

type Result<T> = std::result::Result<T, ChartError>;

fn fail(path: impl Into<String>, reason: impl Into<String>) -> ChartError {
    ChartError::invalid(path, reason)
}

/// Checks `chart` against the rules of the format.
pub(crate) fn check(chart: &Chart) -> Result<()> {
    let message = chart.message();
    if !message.tables.is_empty() {
        return Err(fail(
            "tables",
            "a chart holds its tables decoded, so the message's tables are empty",
        ));
    }
    if message.values.is_some() {
        return Err(fail(
            "values",
            "a chart holds its values decoded, so the message has no values",
        ));
    }
    for (index, value) in chart.values().iter().enumerate() {
        if value.len() != 1 {
            return Err(fail(
                format!("values[{index}]"),
                format!("a value is one element, not {}", value.len()),
            ));
        }
    }
    let root = message
        .root
        .as_ref()
        .ok_or_else(|| fail("root", "a chart has a root node"))?;
    let selections = selection_names(&message.selections)?;
    let mut checker = Checker {
        chart,
        values: Values(chart.values()),
        selections,
        identities: HashMap::new(),
    };
    checker.node(root, "root", &Scope::default())?;
    checker.initial_contributions(&message.selections)?;
    resolve::check(chart)
}

/// The name of `channel` in a message, such as `` `x` ``.
fn channel_name(channel: Channel) -> String {
    let name = channel.as_str_name().trim_start_matches("CHANNEL_");
    format!("`{}`", name.to_ascii_lowercase())
}

/// The kind of a plot's coordinate system, with the facts of the kind that do not depend on a
/// chart. A plot without a `Coord` is Cartesian.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordKind {
    Cartesian,
    Polar,
    Geo,
}

impl CoordKind {
    /// The kind of `coord`, which is Cartesian when it is not set.
    pub fn of(coord: Option<&proto::Coord>) -> Self {
        match coord.and_then(|coord| coord.coord.as_ref()) {
            Some(coord::Coord::Geo(_)) => Self::Geo,
            Some(coord::Coord::Polar(_)) => Self::Polar,
            Some(coord::Coord::Cartesian(_)) | None => Self::Cartesian,
        }
    }

    /// The name of the kind in messages, such as `a polar plot`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cartesian => "Cartesian",
            Self::Geo => "map",
            Self::Polar => "polar",
        }
    }

    /// The position channels: `x` and `y` of a Cartesian plot, `theta` and `radius` of a polar
    /// plot, and `longitude` and `latitude` of a map plot.
    pub fn positions(self) -> &'static [Channel] {
        match self {
            Self::Cartesian => &[Channel::X, Channel::Y],
            Self::Geo => &[Channel::Longitude, Channel::Latitude],
            Self::Polar => &[Channel::Theta, Channel::Radius],
        }
    }

    /// Each position channel with its second channel, such as `x` with `x2`, from which a mark
    /// with a range spans to its second channel.
    pub fn pairs(self) -> &'static [[Channel; 2]] {
        match self {
            Self::Cartesian => &[[Channel::X, Channel::X2], [Channel::Y, Channel::Y2]],
            Self::Geo => &[
                [Channel::Longitude, Channel::Longitude2],
                [Channel::Latitude, Channel::Latitude2],
            ],
            Self::Polar => &[
                [Channel::Theta, Channel::Theta2],
                [Channel::Radius, Channel::Radius2],
            ],
        }
    }

    /// The position channels whose scales run along a side of the plot: `x` and `y` of a
    /// Cartesian plot. The axis of such a scale stands on a side of the plot, its domain pads by
    /// the reach of the marks past their positions, and its range is the plot's length along the
    /// side.
    pub fn edges(self) -> &'static [Channel] {
        match self {
            Self::Cartesian => &[Channel::X, Channel::Y],
            Self::Geo | Self::Polar => &[],
        }
    }

    /// The position channels whose scales can have a view, which a `Navigate` moves: `x` and `y`
    /// of a Cartesian plot. A map has no scales, and the navigation of polar plots is to come.
    pub fn navigated(self) -> &'static [Channel] {
        match self {
            Self::Cartesian => &[Channel::X, Channel::Y],
            Self::Geo | Self::Polar => &[],
        }
    }

    /// Whether a plot of the kind draws `mark`.
    pub fn draws(self, mark: &mark::Mark) -> bool {
        match self {
            Self::Cartesian => !matches!(mark, mark::Mark::Geoshape(_)),
            Self::Geo => !matches!(
                mark,
                mark::Mark::Area(_) | mark::Mark::Bar(_) | mark::Mark::Rect(_)
            ),
            Self::Polar => !matches!(
                mark,
                mark::Mark::Image(_) | mark::Mark::Path(_) | mark::Mark::Geoshape(_)
            ),
        }
    }
}

/// The parent of a node.
#[derive(Clone, Copy, Default)]
enum Parent<'a> {
    #[default]
    None,
    /// An overlay, with the coordinate system of its plot: its own, its parent overlay's, or the
    /// one that its children set, or `None` for a Cartesian plot without options.
    Overlay(Option<&'a proto::Coord>),
    Concat,
    Wrap,
}

/// What a node inherits from its ancestors.
#[derive(Clone, Default)]
struct Scope<'a> {
    /// The partition values of the ancestors.
    partition: Vec<&'a proto::PartitionValue>,
    parent: Parent<'a>,
}

/// A node's id and its partition, with each value as its key, in the order of the columns.
type Identity = (String, Vec<(String, Vec<u8>)>);

/// The values of the chart, which the fields of its message refer to by index.
#[derive(Clone, Copy)]
struct Values<'a>(&'a [ArrayRef]);

impl<'a> Values<'a> {
    /// The value at `index`, an array of one element.
    fn scalar(self, index: u32, path: &str) -> Result<&'a ArrayRef> {
        self.0.get(index as usize).ok_or_else(|| {
            fail(
                path,
                format!(
                    "the field refers to value {index}, and the chart has {} values",
                    self.0.len()
                ),
            )
        })
    }

    /// The elements of the list at `index`.
    fn list(self, index: u32, path: &str) -> Result<ArrayRef> {
        let value = self.scalar(index, path)?;
        match value.as_list_opt::<i32>() {
            Some(list) if list.is_valid(0) => Ok(list.value(0)),
            _ => Err(fail(
                path,
                format!(
                    "the field holds a list that is not null, not a value of type {}",
                    value.data_type()
                ),
            )),
        }
    }

    /// The domain summary at `index`.
    fn summary(self, index: u32, path: &str) -> Result<DomainSummary> {
        DomainSummary::from_arrow(self.scalar(index, path)?.as_ref(), 0)
            .map_err(|error| fail(path, format!("the summary does not decode: {error}")))
    }

    /// The key of the value at `index` in an identity: its type and the row of its value, which
    /// are equal exactly when two values are equal.
    fn key(self, index: u32, path: &str) -> Result<Vec<u8>> {
        let value = self.scalar(index, path)?;
        let converter = RowConverter::new(vec![SortField::new(value.data_type().clone())])
            .map_err(|error| fail(path, error.to_string()))?;
        let rows = converter
            .convert_columns(std::slice::from_ref(value))
            .map_err(|error| fail(path, error.to_string()))?;
        let mut key = format!("{}", value.data_type()).into_bytes();
        key.push(0);
        key.extend_from_slice(rows.row(0).as_ref());
        Ok(key)
    }
}

/// The place of a node with an id in tree order, and whether it is a unit or an overlay, which can
/// own contributions.
#[derive(Clone, Copy)]
struct Identified {
    position: usize,
    plot: bool,
}

struct Checker<'a> {
    chart: &'a Chart,
    values: Values<'a>,
    selections: HashSet<&'a str>,
    identities: HashMap<Identity, Identified>,
}

impl<'a> Checker<'a> {
    fn node(&mut self, node: &'a proto::Node, path: &str, scope: &Scope<'a>) -> Result<()> {
        let kind = node
            .node
            .as_ref()
            .ok_or_else(|| fail(path, "a node has a kind"))?;
        let (id, own, tools) = match kind {
            Node::Unit(unit) => (unit.id.as_str(), &unit.partition, &unit.tools[..]),
            Node::Overlay(overlay) => (overlay.id.as_str(), &overlay.partition, &overlay.tools[..]),
            Node::Concat(concat) => (concat.id.as_str(), &concat.partition, &[][..]),
            Node::Wrap(wrap) => (wrap.id.as_str(), &wrap.partition, &[][..]),
            Node::Empty(empty) => ("", &empty.partition, &[][..]),
        };
        partition_values(self.values, own, &scope.partition, path)?;
        let mut inner = Scope {
            partition: scope.partition.clone(),
            parent: Parent::None,
        };
        inner.partition.extend(own);

        if !id.is_empty() {
            let identity = (
                id.to_string(),
                sorted_partition(self.values, inner.partition.iter().copied(), path)?,
            );
            let identified = Identified {
                position: self.identities.len(),
                plot: matches!(kind, Node::Unit(_) | Node::Overlay(_)),
            };
            if self.identities.insert(identity, identified).is_some() {
                return Err(fail(
                    path,
                    format!("two nodes have the id `{id}` and one partition"),
                ));
            }
        }
        if !tools.is_empty() && id.is_empty() {
            return Err(fail(path, "a node that declares tools has an id"));
        }
        // The plot of a unit or an overlay inside an overlay takes the overlay's coordinate system.
        let coord = match kind {
            Node::Unit(unit) => unit.coord.as_ref(),
            Node::Overlay(overlay) => overlay.coord.as_ref(),
            _ => None,
        };
        let coord = match scope.parent {
            Parent::Overlay(overlay_coord) => coord.or(overlay_coord),
            _ => coord,
        };
        for (index, tool) in tools.iter().enumerate() {
            let path = format!("{path}/tools[{index}]");
            self.tool(tool, node, CoordKind::of(coord), &inner.partition, &path)?;
        }

        if let Parent::Overlay(overlay_coord) = scope.parent {
            let (coord, title, attribution) = match kind {
                Node::Unit(unit) => (unit.coord.as_ref(), &unit.title, &unit.attribution),
                Node::Overlay(child) => (child.coord.as_ref(), &child.title, &child.attribution),
                _ => return Err(fail(path, "an overlay holds units and overlays")),
            };
            if coord.is_some() && coord != overlay_coord {
                return Err(fail(
                    path,
                    "the nodes inside an overlay set the coordinate system of the overlay or none",
                ));
            }
            if title.is_some() || attribution.is_some() {
                return Err(fail(
                    path,
                    "the nodes inside an overlay have no titles and no attributions",
                ));
            }
        }

        match kind {
            Node::Unit(unit) => {
                if let Some(coord) = &unit.coord {
                    coord_options(coord, &format!("{path}/coord"))?;
                }
                let coord = match scope.parent {
                    Parent::Overlay(overlay_coord) => unit.coord.as_ref().or(overlay_coord),
                    _ => unit.coord.as_ref(),
                };
                self.unit(unit, CoordKind::of(coord), path, &inner)
            }
            Node::Overlay(overlay) => {
                if let Some(coord) = &overlay.coord {
                    coord_options(coord, &format!("{path}/coord"))?;
                }
                resolves(&overlay.resolve, path)?;
                let outer = match scope.parent {
                    Parent::Overlay(coord) => coord,
                    _ => None,
                };
                inner.parent = Parent::Overlay(
                    overlay
                        .coord
                        .as_ref()
                        .or(outer)
                        .or_else(|| first_coord(&overlay.children)),
                );
                self.children(&overlay.children, path, &inner)
            }
            Node::Concat(concat) => {
                let direction = proto::Direction::try_from(concat.direction)
                    .ok()
                    .filter(|direction| *direction != proto::Direction::Unspecified)
                    .ok_or_else(|| fail(path, "a concat is a row or a column"))?;
                resolves(&concat.resolve, path)?;
                sharing(concat.projection, &format!("{path}/projection"))?;
                if let Some(layout) = &concat.layout {
                    let path = format!("{path}/layout");
                    if !layout.tracks.is_empty() && layout.tracks.len() != concat.children.len() {
                        return Err(fail(
                            &path,
                            format!(
                                "a concat has one track for each child or none, and it has {} \
                                 tracks for {} children",
                                layout.tracks.len(),
                                concat.children.len()
                            ),
                        ));
                    }
                    for (index, (child, track)) in
                        concat.children.iter().zip(&layout.tracks).enumerate()
                    {
                        let path = format!("{path}/tracks[{index}]");
                        let Some(size) = &track.size else {
                            continue;
                        };
                        let same_direction = matches!(
                            &child.node,
                            Some(Node::Concat(inner)) if inner.direction == concat.direction
                        );
                        if same_direction || table::extent(child)[table::direction(concat)] > 1 {
                            return Err(fail(
                                &path,
                                "a child that is a concat of the same direction, or that spans \
                                 several tracks, takes an empty track, and the concats inside it \
                                 set the sizes of their tracks",
                            ));
                        }
                        track_size(size, &path)?;
                    }
                    if matches!(scope.parent, Parent::Concat)
                        && (layout.horizontal_distribute != 0 || layout.vertical_distribute != 0)
                    {
                        return Err(fail(
                            &path,
                            "only the outermost concat of a table, the root or a child of a \
                             wrap, distributes the length that its tracks leave",
                        ));
                    }
                    guide_rules(&layout.guide_rules, &path)?;
                }
                if let Some(facet) = &concat.facet {
                    self.facet(facet, &concat.children, Some(direction), path)?;
                }
                inner.parent = Parent::Concat;
                self.children(&concat.children, path, &inner)
            }
            Node::Wrap(wrap) => {
                resolves(&wrap.resolve, path)?;
                sharing(wrap.projection, &format!("{path}/projection"))?;
                if let Some(layout) = &wrap.layout {
                    let path = format!("{path}/layout");
                    if layout.max_columns == Some(0) {
                        return Err(fail(&path, "a wrap has one column or more"));
                    }
                    guide_rules(&layout.guide_rules, &path)?;
                }
                if let Some(facet) = &wrap.facet {
                    self.facet(facet, &wrap.children, None, path)?;
                }
                inner.parent = Parent::Wrap;
                self.children(&wrap.children, path, &inner)
            }
            Node::Empty(_) => {
                if !matches!(scope.parent, Parent::Concat | Parent::Wrap) {
                    return Err(fail(
                        path,
                        "an empty child is a child of a concat or a wrap",
                    ));
                }
                Ok(())
            }
        }
    }

    fn children(
        &mut self,
        children: &'a [proto::Node],
        path: &str,
        scope: &Scope<'a>,
    ) -> Result<()> {
        for (index, child) in children.iter().enumerate() {
            self.node(child, &format!("{path}/children[{index}]"), scope)?;
        }
        Ok(())
    }

    fn facet(
        &self,
        facet: &proto::Facet,
        children: &[proto::Node],
        direction: Option<proto::Direction>,
        path: &str,
    ) -> Result<()> {
        agreement(children, path)?;
        let path = format!("{path}/facet");
        if facet.column.is_empty() {
            return Err(fail(&path, "a facet names its partition column"));
        }
        let mut data_type: Option<DataType> = None;
        for (index, child) in children.iter().enumerate() {
            let value = tree::partition(child)
                .iter()
                .find(|value| value.column == facet.column)
                .ok_or_else(|| {
                    fail(
                        &path,
                        format!(
                            "child {index} declares no value of the facet's column `{}`",
                            facet.column
                        ),
                    )
                })?;
            let value = self
                .values
                .scalar(value.value, &format!("{path}/children[{index}]"))?;
            match &data_type {
                Some(data_type) if data_type != value.data_type() => {
                    return Err(fail(
                        &path,
                        format!(
                            "the values of the facet's column have one type, and child {index} \
                             has {} where child 0 has {data_type}",
                            value.data_type()
                        ),
                    ))
                }
                Some(_) => {}
                None => data_type = Some(value.data_type().clone()),
            }
        }
        if let (Some(proto::facet::Header::Shown(header)), Some(direction)) =
            (&facet.header, direction)
        {
            let side = proto::Side::try_from(header.side).unwrap_or(proto::Side::Unspecified);
            let takes = match direction {
                proto::Direction::Row => matches!(
                    side,
                    proto::Side::Unspecified | proto::Side::Top | proto::Side::Bottom
                ),
                _ => matches!(
                    side,
                    proto::Side::Unspecified | proto::Side::Left | proto::Side::Right
                ),
            };
            if !takes {
                return Err(fail(
                    format!("{path}/shown"),
                    "the header of a row stands on the top or the bottom, and the header of a \
                     column on the left or the right",
                ));
            }
        }
        Ok(())
    }

    /// Checks a tool of the plot `node` in the coordinate system of `plot`, whose partition is
    /// `partition`: the channels of a navigation are channels whose scales the plot navigates by,
    /// the channels of an interval are positional channels of the plot, a selection names a
    /// selection of the chart, its scope columns are partition columns of the plot, its key column
    /// is a column of a unit of the plot or of its partition, and a lasso without a key needs two
    /// continuous positional channels.
    fn tool(
        &self,
        tool: &proto::Tool,
        node: &proto::Node,
        plot: CoordKind,
        partition: &[&proto::PartitionValue],
        path: &str,
    ) -> Result<()> {
        let within = |channels: &[i32], allowed: &[Channel], what: &str| {
            let names: Vec<String> = allowed
                .iter()
                .map(|channel| channel_name(*channel))
                .collect();
            let inside = |channel: &i32| {
                Channel::try_from(*channel).is_ok_and(|channel| allowed.contains(&channel))
            };
            match channels.iter().all(inside) {
                true => Ok(()),
                false if names.is_empty() => Err(fail(
                    path,
                    format!("{what} no channels in a {} plot", plot.name()),
                )),
                false => Err(fail(
                    path,
                    format!("{what} {} in a {} plot", names.join(" or "), plot.name()),
                )),
            }
        };
        let partitioned = |column: &str| partition.iter().any(|value| value.column == column);
        let keyed = |key: &str| {
            if partitioned(key) || self.holds(node, key) {
                Ok(())
            } else {
                Err(fail(
                    path,
                    format!("the key column `{key}` is in no table and no partition of the plot"),
                ))
            }
        };
        match tool.kind.as_ref() {
            None => Err(fail(path, "a tool has a kind")),
            Some(tool::Kind::Navigate(navigate)) => {
                within(&navigate.channels, plot.navigated(), "a navigation moves")
            }
            Some(tool::Kind::Tooltip(_)) => Ok(()),
            Some(tool::Kind::Select(selection)) => {
                if !self.selections.contains(selection.selection.as_str()) {
                    return Err(fail(
                        path,
                        format!("the chart has no selection `{}`", selection.selection),
                    ));
                }
                if let Some(column) = (selection.scope.iter()).find(|column| !partitioned(column)) {
                    return Err(fail(
                        path,
                        format!(
                            "the scope column `{column}` is not a partition column of the plot"
                        ),
                    ));
                }
                match &selection.form {
                    Some(select::Form::Interval(interval)) => within(
                        &interval.channels,
                        plot.positions(),
                        "an interval selects on",
                    ),
                    Some(select::Form::Points(points)) => keyed(&points.key),
                    Some(select::Form::Lasso(lasso)) if !lasso.key.is_empty() => keyed(&lasso.key),
                    Some(select::Form::Lasso(_)) => {
                        let positions = plot.positions();
                        if (positions.iter()).all(|channel| self.continuous(node, *channel)) {
                            Ok(())
                        } else {
                            let names: Vec<String> = positions
                                .iter()
                                .map(|channel| channel_name(*channel))
                                .collect();
                            Err(fail(
                                path,
                                format!(
                                    "a lasso without a key selects on continuous {}",
                                    names.join(" and ")
                                ),
                            ))
                        }
                    }
                    None => Ok(()),
                }
            }
        }
    }

    /// Whether every encoding of `channel` in the units at or below `node` is continuous: a
    /// longitude or a latitude, which are degrees, or a channel of a continuous scale.
    fn continuous(&self, node: &proto::Node, channel: Channel) -> bool {
        match &node.node {
            Some(Node::Unit(unit)) => (unit.encodings.iter())
                .filter(|encoding| encoding.channel == channel as i32)
                .all(|encoding| {
                    matches!(channel, Channel::Longitude | Channel::Latitude)
                        || (encoding.scale.as_ref())
                            .is_some_and(|scale| FitKind::of(&scale.r#type) == FitKind::Interval)
                }),
            Some(Node::Overlay(overlay)) => {
                (overlay.children.iter()).all(|child| self.continuous(child, channel))
            }
            _ => false,
        }
    }

    /// Whether the table or the partition of a unit at or below `node` has `column`.
    fn holds(&self, node: &proto::Node, column: &str) -> bool {
        let own = tree::partition(node)
            .iter()
            .any(|value| value.column == column);
        own || match &node.node {
            Some(Node::Unit(unit)) => self
                .chart
                .tables()
                .get(unit.table as usize)
                .is_some_and(|table| table.column_by_name(column).is_some()),
            Some(Node::Overlay(overlay)) => {
                (overlay.children.iter()).any(|child| self.holds(child, column))
            }
            _ => false,
        }
    }

    fn unit(
        &self,
        unit: &proto::Unit,
        plot: CoordKind,
        path: &str,
        scope: &Scope<'a>,
    ) -> Result<()> {
        let table = self
            .chart
            .tables()
            .get(unit.table as usize)
            .ok_or_else(|| {
                fail(
                    path,
                    format!(
                        "the unit refers to table {}, and the chart has {} tables",
                        unit.table,
                        self.chart.tables().len()
                    ),
                )
            })?;
        let mark = unit
            .mark
            .as_ref()
            .and_then(|mark| mark.mark.as_ref())
            .ok_or_else(|| fail(path, "a unit has a mark"))?;
        let mark_name = mark_name(mark);
        if !plot.draws(mark) {
            return Err(fail(
                path,
                format!("a {} plot does not draw a {mark_name} mark", plot.name()),
            ));
        }
        let channels = mark_channels(mark, plot);
        let encoded = |channel: Channel| {
            unit.encodings
                .iter()
                .any(|encoding| encoding.channel == channel as i32)
        };
        let mut previous: Option<i32> = None;
        for (index, encoding) in unit.encodings.iter().enumerate() {
            let path = format!("{path}/encodings[{index}]");
            if previous.is_some_and(|previous| previous >= encoding.channel) {
                return Err(fail(
                    &path,
                    "a unit has at most one encoding for each channel, in the order of `Channel`",
                ));
            }
            previous = Some(encoding.channel);
            let channel = Channel::try_from(encoding.channel)
                .ok()
                .filter(|channel| *channel != Channel::Unspecified)
                .ok_or_else(|| fail(&path, "an encoding has a channel"))?;
            if !channels.contains(&channel) {
                return Err(fail(
                    &path,
                    format!(
                        "a {mark_name} mark in a {} plot does not take `{}`",
                        plot.name(),
                        channel.as_str_name()
                    ),
                ));
            }
            if let Some(first) = first_of(channel) {
                if !encoded(first) {
                    return Err(fail(
                        &path,
                        format!(
                            "`{}` is the second channel of `{}`, which the unit does not encode",
                            channel.as_str_name(),
                            first.as_str_name()
                        ),
                    ));
                }
            }
            self.encoding(encoding, channel, table, plot, &path)?;
        }
        let required: &[Channel] = match mark {
            mark::Mark::Text(_) => &[Channel::Text],
            mark::Mark::Image(_) => &[Channel::Image],
            mark::Mark::Path(_) => &[Channel::Path],
            mark::Mark::Geoshape(_) => &[Channel::Geometry],
            mark::Mark::Rule(_) if plot == CoordKind::Geo => {
                &[Channel::Longitude, Channel::Latitude]
            }
            _ => &[],
        };
        if let Some(missing) = required.iter().find(|channel| !encoded(**channel)) {
            return Err(fail(
                path,
                format!(
                    "a {mark_name} mark in a {} plot needs `{}`",
                    plot.name(),
                    missing.as_str_name()
                ),
            ));
        }
        match mark {
            mark::Mark::Rect(_) => rect_spans(unit, plot, path)?,
            mark::Mark::Bar(bar) if plot == CoordKind::Polar => polar_bar_spans(unit, bar, path)?,
            mark::Mark::Text(text) if plot == CoordKind::Polar => {
                if text.align != 0 || text.baseline != 0 {
                    return Err(fail(
                        path,
                        "a text in a polar plot lies by `radius_side` and `theta_side`, not \
                         `align` and `baseline`",
                    ));
                }
                if text.orient != 0 && encoded(Channel::Angle) {
                    return Err(fail(path, "a text with an orient does not encode `angle`"));
                }
            }
            mark::Mark::Text(text) => {
                if text.orient != 0 {
                    return Err(fail(path, "only a text in a polar plot takes an orient"));
                }
                if text.radius_side != 0 || text.theta_side != 0 {
                    return Err(fail(
                        path,
                        "only a text in a polar plot takes a radius or theta side",
                    ));
                }
            }
            _ => {}
        }
        partition_in_table(self.values, table, &scope.partition, path)?;
        one_value_per_series(unit, mark, table, path)
    }

    fn encoding(
        &self,
        encoding: &proto::Encoding,
        channel: Channel,
        table: &RecordBatch,
        plot: CoordKind,
        path: &str,
    ) -> Result<()> {
        let value = encoding
            .value
            .as_ref()
            .ok_or_else(|| fail(path, "an encoding has a value"))?;
        let value_type = match value {
            encoding::Value::Column(name) => column(table, name, path)?.data_type().clone(),
            encoding::Value::Constant(index) | encoding::Value::ScaledConstant(index) => {
                self.values.scalar(*index, path)?.data_type().clone()
            }
        };
        if channel == Channel::Geometry {
            let geoarrow = match value {
                encoding::Value::Column(name) => table
                    .schema()
                    .field_with_name(name)
                    .ok()
                    .and_then(|field| field.extension_type_name())
                    .is_some_and(|extension| extension.starts_with("geoarrow.")),
                _ => false,
            };
            if !geoarrow {
                return Err(fail(
                    path,
                    "a `geometry` encoding is a column of GeoArrow geometries, whose field names a \
                     `geoarrow` extension type",
                ));
            }
        }
        if matches!(value, encoding::Value::Constant(_))
            && matches!(channel, Channel::X | Channel::Y | Channel::X2 | Channel::Y2)
        {
            return Err(fail(
                path,
                "a Cartesian position is a column or a scaled constant, which go through the \
                 scale of `x` or `y`",
            ));
        }
        let scaled = matches!(value, encoding::Value::ScaledConstant(_))
            || encoding.conditions.iter().any(|condition| {
                matches!(condition.value, Some(condition::Value::ScaledConstant(_)))
            });
        let constants = matches!(value, encoding::Value::Constant(_))
            && encoding
                .conditions
                .iter()
                .all(|condition| matches!(condition.value, Some(condition::Value::Constant(_))));
        match (&encoding.scale, takes_scale(channel)) {
            (Some(_), false) => {
                return Err(fail(
                    path,
                    format!("`{}` takes no scale", channel.as_str_name()),
                ))
            }
            (Some(_), true) if constants => {
                return Err(fail(
                    path,
                    "an encoding whose values are all constants has no scale",
                ))
            }
            (None, true) if scaled => {
                return Err(fail(
                    path,
                    "a scaled constant goes through the channel's scale",
                ))
            }
            (None, true) if matches!(channel, Channel::X | Channel::Y) => {
                return Err(fail(
                    path,
                    "`x` and `y` have a scale, because the axes and the fit of the plot need one",
                ))
            }
            _ => {}
        }
        match &encoding.scale {
            Some(scale) => {
                self.scale(scale, channel, &value_type, plot, &format!("{path}/scale"))?
            }
            None => {
                if let encoding::Value::Column(_) = value {
                    raw_values(channel, &value_type, path)?;
                }
            }
        }
        if !encoding.conditions.is_empty() && (is_position(channel) || channel == Channel::Detail) {
            return Err(fail(
                path,
                "the position channels and `detail` have no conditions",
            ));
        }
        for (index, condition) in encoding.conditions.iter().enumerate() {
            let path = format!("{path}/conditions[{index}]");
            let predicate = condition
                .predicate
                .as_ref()
                .ok_or_else(|| fail(&path, "a condition has a predicate"))?;
            self.predicate(predicate, &path)?;
            match condition
                .value
                .as_ref()
                .ok_or_else(|| fail(&path, "a condition has a value"))?
            {
                condition::Value::Column(name) => {
                    column(table, name, &path)?;
                }
                condition::Value::Constant(index) | condition::Value::ScaledConstant(index) => {
                    self.values.scalar(*index, &path)?;
                }
            }
        }
        if encoding.title.is_some() && encoding.scale.is_none() {
            return Err(fail(
                path,
                "an encoding without a scale has no guide, so it has no title",
            ));
        }
        if !encoding.selection_field.is_empty() {
            if first_of(channel).is_some() {
                return Err(fail(
                    path,
                    "the second channel of a pair does not set a selection field",
                ));
            }
            column(table, &encoding.selection_field, path)?;
        }
        Ok(())
    }

    fn scale(
        &self,
        spec: &proto::ScaleSpec,
        channel: Channel,
        value_type: &DataType,
        plot: CoordKind,
        path: &str,
    ) -> Result<()> {
        let scale_impl = scale_impl(&spec.r#type, path)?;
        let fit_kind = FitKind::of(&spec.r#type);
        let domain_kind = scale_impl.domain_kind();

        // Only the scales of `x` and `y` have views, which a navigation moves.
        let navigated = matches!(channel, Channel::X | Channel::Y);
        if spec.navigation.is_some() && !(navigated && fit_kind == FitKind::Interval) {
            return Err(fail(
                path,
                "only a continuous scale of `x` or `y` has a navigation",
            ));
        }

        if spec
            .options
            .windows(2)
            .any(|pair| pair[0].name >= pair[1].name)
        {
            return Err(fail(
                path,
                "the options of a scale are in the order of their names, each name at most once",
            ));
        }
        let options =
            payload::options(&spec.options).map_err(|error| fail(path, error.to_string()))?;
        OptionDefinition::validate_all(
            scale_impl.option_definitions(),
            &options.into_iter().collect(),
        )
        .map_err(|error| fail(path, error.to_string()))?;

        if !takes_values(domain_kind, value_type) {
            return Err(fail(
                path,
                format!(
                    "a {} scale does not take values of type {value_type}",
                    spec.r#type
                ),
            ));
        }

        let explicit = self.domain(
            spec.domain.as_ref(),
            &spec.r#type,
            fit_kind,
            domain_kind,
            &format!("{path}/domain"),
        )?;
        let fitted =
            explicit.is_none() && matches!(fit_kind, FitKind::Interval | FitKind::Categories);
        match (&spec.fit, fitted) {
            (None, true) => {
                return Err(fail(
                    path,
                    format!(
                        "a summary or data domain of a {} scale has a fit",
                        spec.r#type
                    ),
                ))
            }
            (Some(_), false) => {
                return Err(fail(
                    path,
                    format!(
                        "only a summary or data domain of an interval or category scale has a \
                         fit, and this {} scale has none",
                        spec.r#type
                    ),
                ))
            }
            (Some(fit), true) => fit_matches(
                self.values,
                fit,
                fit_kind,
                &spec.r#type,
                &format!("{path}/fit"),
            )?,
            (None, false) => {}
        }

        if let Some(range) = &spec.range {
            let path = format!("{path}/range");
            match &range.range {
                Some(_) if is_position(channel) => {
                    return Err(fail(
                        &path,
                        "the plot gives the range of a position channel, which sets only \
                         `reverse`",
                    ))
                }
                Some(range::Range::Values(index)) => {
                    self.values.list(*index, &path)?;
                }
                Some(range::Range::Palette(name)) if name.is_empty() => {
                    return Err(fail(&path, "a palette range names a palette"))
                }
                _ => {}
            }
        }

        if let Some(guide) = spec.guide.as_ref().and_then(|guide| guide.guide.as_ref()) {
            let domain_type = explicit
                .as_ref()
                .map_or(value_type, |array| array.data_type());
            let target = GuideTarget {
                values: self.values,
                channel,
                scale_type: &spec.r#type,
                domain_type,
                plot,
            };
            target.guide(guide, &format!("{path}/guide"))?;
        }
        Ok(())
    }

    /// Checks a domain, and returns its explicit array, or `None` for a summary or data domain,
    /// whose fit the spec states.
    fn domain(
        &self,
        domain: Option<&proto::Domain>,
        scale_type: &str,
        fit_kind: FitKind,
        domain_kind: DomainKind,
        path: &str,
    ) -> Result<Option<ArrayRef>> {
        let domain = domain
            .and_then(|domain| domain.domain.as_ref())
            .ok_or_else(|| fail(path, "a scale has a domain"))?;
        match domain {
            domain::Domain::Explicit(index) => {
                let array = self.values.list(*index, path)?;
                if !takes_values(domain_kind, array.data_type()) {
                    return Err(fail(
                        path,
                        format!(
                            "a {scale_type} scale does not take a domain of type {}",
                            array.data_type()
                        ),
                    ));
                }
                if fit_kind == FitKind::Interval && array.len() != 2 {
                    return Err(fail(
                        path,
                        format!(
                            "the explicit domain of a {scale_type} scale holds its two ends, not \
                             {} values",
                            array.len()
                        ),
                    ));
                }
                if fit_kind == FitKind::Interval || scale_type == "threshold" {
                    ascends(&array, path)?;
                }
                Ok(Some(array))
            }
            domain::Domain::Summary(index) => {
                let summary = self.values.summary(*index, path)?;
                let kind = match summary {
                    DomainSummary::Interval(_) => FitKind::Interval,
                    DomainSummary::Categories(_) => FitKind::Categories,
                    DomainSummary::Sample(_) => FitKind::Sample,
                };
                if kind != fit_kind {
                    return Err(fail(
                        path,
                        format!("the summary is not of the kind that a {scale_type} scale fits"),
                    ));
                }
                Ok(None)
            }
            domain::Domain::Data(_) => {
                if fit_kind == FitKind::Explicit {
                    return Err(fail(
                        path,
                        format!("a {scale_type} scale takes an explicit domain"),
                    ));
                }
                Ok(None)
            }
            domain::Domain::Selection(selection) => {
                if !self.selections.contains(selection.selection.as_str()) {
                    return Err(fail(
                        path,
                        format!("the chart has no selection `{}`", selection.selection),
                    ));
                }
                Channel::try_from(selection.channel)
                    .ok()
                    .filter(|channel| *channel != Channel::Unspecified)
                    .ok_or_else(|| fail(path, "a selection domain names a channel"))?;
                let fallback = selection
                    .fallback
                    .as_deref()
                    .ok_or_else(|| fail(path, "a selection domain has a fallback"))?;
                if matches!(fallback.domain, Some(domain::Domain::Selection(_))) {
                    return Err(fail(
                        path,
                        "the fallback of a selection domain is not a selection domain",
                    ));
                }
                self.domain(
                    Some(fallback),
                    scale_type,
                    fit_kind,
                    domain_kind,
                    &format!("{path}/fallback"),
                )
            }
        }
    }

    fn predicate(&self, predicate: &proto::Predicate, path: &str) -> Result<()> {
        match predicate
            .predicate
            .as_ref()
            .ok_or_else(|| fail(path, "a predicate has a kind"))?
        {
            predicate::Predicate::Selection(test) => {
                if !self.selections.contains(test.selection.as_str()) {
                    return Err(fail(
                        path,
                        format!("the chart has no selection `{}`", test.selection),
                    ));
                }
                Ok(())
            }
            predicate::Predicate::And(list) | predicate::Predicate::Or(list) => {
                if list.predicates.is_empty() {
                    return Err(fail(path, "`and` and `or` combine one predicate or more"));
                }
                for (index, inner) in list.predicates.iter().enumerate() {
                    self.predicate(inner, &format!("{path}/predicates[{index}]"))?;
                }
                Ok(())
            }
            predicate::Predicate::Not(inner) => self.predicate(inner, &format!("{path}/not")),
        }
    }

    fn initial_contributions(&self, selections: &[proto::Selection]) -> Result<()> {
        for (index, selection) in selections.iter().enumerate() {
            let mut previous: Option<usize> = None;
            for (position, contribution) in selection.initial.iter().enumerate() {
                let path = format!("selections[{index}]/initial[{position}]");
                contribution_fields(self.values, contribution, &path)?;
                let identity = (
                    contribution.owner.clone(),
                    sorted_partition(self.values, contribution.partition.iter(), &path)?,
                );
                let owner = self
                    .identities
                    .get(&identity)
                    .filter(|owner| owner.plot)
                    .ok_or_else(|| {
                        fail(
                            &path,
                            format!(
                                "the chart has no unit or overlay with the id `{}` and this \
                                 partition",
                                contribution.owner
                            ),
                        )
                    })?;
                if previous.is_some_and(|previous| previous >= owner.position) {
                    return Err(fail(
                        &path,
                        "the initial contributions are in the tree order of their owners, each \
                         owner at most once",
                    ));
                }
                previous = Some(owner.position);
            }
        }
        Ok(())
    }
}

/// The scale whose guide a guide is.
struct GuideTarget<'a> {
    values: Values<'a>,
    channel: Channel,
    scale_type: &'a str,
    domain_type: &'a DataType,
    plot: CoordKind,
}

impl GuideTarget<'_> {
    fn guide(&self, guide: &guide::Guide, path: &str) -> Result<()> {
        // The continuous scales whose legends and colorbars show ticks, unlike the bins of a
        // quantize, a quantile, or a threshold scale.
        let ticked = matches!(
            self.scale_type,
            "linear" | "log" | "pow" | "symlog" | "time"
        );
        match guide {
            guide::Guide::Axis(axis) => {
                if !matches!(
                    self.channel,
                    Channel::X | Channel::Y | Channel::Theta | Channel::Radius
                ) {
                    return Err(fail(
                        path,
                        "an axis is the guide of `x`, `y`, `theta`, or `radius`",
                    ));
                }
                self.axis(axis, false, path)
            }
            guide::Guide::Legend(legend) => {
                if is_position(self.channel) {
                    return Err(fail(
                        path,
                        "a legend is the guide of a channel other than a position",
                    ));
                }
                if !ticked && (legend.tick_count.is_some() || legend.values.is_some()) {
                    return Err(fail(
                        path,
                        "only the legend of a linear, log, pow, symlog, or time scale takes a \
                         tick count or values",
                    ));
                }
                if legend.tick_count.is_some() && legend.values.is_some() {
                    return Err(fail(
                        path,
                        "a legend with its own values takes no tick count",
                    ));
                }
                if let Some(values) = legend.values {
                    self.domain_values(values, path)?;
                }
                Ok(())
            }
            guide::Guide::Colorbar(colorbar) => {
                if !matches!(
                    self.channel,
                    Channel::Color | Channel::Fill | Channel::Stroke
                ) || !ticked
                {
                    return Err(fail(
                        path,
                        "a colorbar is the guide of a linear, log, pow, symlog, or time scale of \
                         a color channel",
                    ));
                }
                match &colorbar.axis {
                    Some(axis) => self.axis(axis, true, &format!("{path}/axis")),
                    None => Ok(()),
                }
            }
            guide::Guide::None(_) => Ok(()),
        }
    }

    /// Checks an axis of the scale, or the axis of its colorbar.
    fn axis(&self, axis: &proto::Axis, colorbar: bool, path: &str) -> Result<()> {
        let polar = self.plot == CoordKind::Polar && !colorbar;
        if polar && (axis.title.is_some() || axis.side != 0) {
            return Err(fail(path, "a polar axis has no title and no side"));
        }
        if axis.angle.is_some() && !(polar && self.channel == Channel::Radius) {
            return Err(fail(
                path,
                "only an axis of `radius` in a polar plot takes an angle",
            ));
        }
        if axis.route != 0 && !polar {
            return Err(fail(path, "only an axis of a polar plot takes a route"));
        }
        if matches!(
            axis.label_turn,
            Some(proto::axis::LabelTurn::LabelOrient(_))
        ) && !polar
        {
            return Err(fail(
                path,
                "only an axis of a polar plot takes a label orient",
            ));
        }
        if colorbar
            && (axis.grid.is_some() || !axis.grid_color.is_empty() || axis.grid_width.is_some())
        {
            return Err(fail(path, "the axis of a colorbar draws no grid"));
        }
        if axis.ticks.is_some() && axis.tick_count.is_some() {
            return Err(fail(path, "an axis with its own ticks takes no tick count"));
        }
        if let Some(proto::axis::Ticks::TickValues(index)) = &axis.ticks {
            self.domain_values(*index, path)?;
        }
        Ok(())
    }

    /// Checks tick or entry values, which are in the type of the scale's domain.
    fn domain_values(&self, index: u32, path: &str) -> Result<()> {
        let values = self.values.list(index, path)?;
        if !compatible(values.data_type(), self.domain_type) {
            return Err(fail(
                path,
                format!(
                    "the values have type {}, and the scale's domain has type {}",
                    values.data_type(),
                    self.domain_type
                ),
            ));
        }
        Ok(())
    }
}

/// The coordinate system of the first unit or overlay among `children`, and their descendants,
/// that sets one.
pub(crate) fn first_coord(children: &[proto::Node]) -> Option<&proto::Coord> {
    children.iter().find_map(|child| match &child.node {
        Some(Node::Unit(unit)) => unit.coord.as_ref(),
        Some(Node::Overlay(overlay)) => overlay
            .coord
            .as_ref()
            .or_else(|| first_coord(&overlay.children)),
        _ => None,
    })
}

fn coord_options(coord: &proto::Coord, path: &str) -> Result<()> {
    match coord
        .coord
        .as_ref()
        .ok_or_else(|| fail(path, "a coordinate system has a kind"))?
    {
        coord::Coord::Cartesian(cartesian) => positive_aspect(cartesian.aspect_ratio, path),
        coord::Coord::Geo(geo) => {
            let planar = match geo
                .surface
                .as_ref()
                .ok_or_else(|| fail(path, "a map takes spherical or planar coordinates"))?
            {
                geo::Surface::Spherical(spherical) => {
                    if spherical.projection.is_empty() {
                        return Err(fail(path, "a spherical map names its projection"));
                    }
                    false
                }
                geo::Surface::Planar(_) => true,
            };
            if planar && matches!(geo.fit, Some(geo::Fit::FitSphere(_))) {
                return Err(fail(path, "a planar map does not fit the sphere"));
            }
            if planar && (geo.graticule.is_some() || geo.sphere.is_some() || geo.tiles.is_some()) {
                return Err(fail(
                    path,
                    "a planar map has no graticule, no sphere, and no tiles",
                ));
            }
            let sides = geo
                .graticule
                .as_ref()
                .and_then(|graticule| graticule.label_sides.as_ref())
                .map_or(&[][..], |sides| &sides.sides[..]);
            for (index, side) in sides.iter().enumerate() {
                if !matches!(proto::Side::try_from(*side), Ok(side) if side != proto::Side::Unspecified)
                    || sides[..index].contains(side)
                {
                    return Err(fail(
                        path,
                        "the label sides of a graticule are sides, each at most once",
                    ));
                }
            }
            if let Some(geo::Aspect::AspectRatio(ratio)) = geo.aspect {
                positive_aspect(Some(ratio), path)?;
            }
            Ok(())
        }
        coord::Coord::Polar(polar) => {
            let start = polar.start_angle.unwrap_or(0.0);
            let end = polar.end_angle.unwrap_or(start + 360.0);
            if !(start.is_finite() && end.is_finite() && end > start && end - start <= 360.0) {
                return Err(fail(
                    path,
                    "the end angle of a polar plot is more than its start angle and at most 360 \
                     degrees past it",
                ));
            }
            if !(0.0..1.0).contains(&polar.inner_radius.unwrap_or(0.0)) {
                return Err(fail(
                    path,
                    "the inner radius of a polar plot is at least 0 and less than 1",
                ));
            }
            Ok(())
        }
    }
}

fn positive_aspect(ratio: Option<f32>, path: &str) -> Result<()> {
    if ratio.is_some_and(|ratio| !(ratio.is_finite() && ratio > 0.0)) {
        return Err(fail(path, "an aspect ratio is a positive number"));
    }
    Ok(())
}

/// The names of the selections, which are in order, each at most once.
fn selection_names(selections: &[proto::Selection]) -> Result<HashSet<&str>> {
    for (index, selection) in selections.iter().enumerate() {
        let path = format!("selections[{index}]");
        if selection.name.is_empty() {
            return Err(fail(&path, "a selection has a name"));
        }
        if index > 0 && selections[index - 1].name >= selection.name {
            return Err(fail(
                &path,
                "the selections are in the order of their names, each name at most once",
            ));
        }
        if matches!(
            proto::SelectionResolution::try_from(selection.resolution),
            Ok(proto::SelectionResolution::Unspecified) | Err(_)
        ) {
            return Err(fail(&path, "a selection has a resolution"));
        }
    }
    Ok(selections
        .iter()
        .map(|selection| selection.name.as_str())
        .collect())
}

fn contribution_fields(
    values: Values,
    contribution: &proto::Contribution,
    path: &str,
) -> Result<()> {
    if contribution.owner.is_empty() {
        return Err(fail(path, "a contribution names its owner"));
    }
    if contribution.tuples.is_empty() {
        return Err(fail(path, "a contribution has a tuple or more"));
    }
    if contribution
        .partition
        .windows(2)
        .any(|pair| pair[0].column >= pair[1].column)
    {
        return Err(fail(
            path,
            "the owner's partition is in the order of its columns, each column at most once",
        ));
    }
    for (index, tuple) in contribution.tuples.iter().enumerate() {
        let path = format!("{path}/tuples[{index}]");
        if tuple.fields.is_empty() {
            return Err(fail(&path, "a tuple has a field or more"));
        }
        for (field_index, field) in tuple.fields.iter().enumerate() {
            let path = format!("{path}/fields[{field_index}]");
            match field
                .test
                .as_ref()
                .ok_or_else(|| fail(&path, "a tuple field has a test"))?
            {
                tuple_field::Test::Set(set) => {
                    named(&set.column, &path)?;
                    values.list(set.values, &path)?;
                }
                tuple_field::Test::Range(range) => {
                    named(&range.column, &path)?;
                    let ends = values.list(range.ends, &path)?;
                    if ends.len() != 2 {
                        return Err(fail(&path, "a range holds its two ends"));
                    }
                    ascends(&ends, &path)?;
                }
                tuple_field::Test::Polygon(polygon) => {
                    named(&polygon.x_column, &path)?;
                    named(&polygon.y_column, &path)?;
                    let vertices = values.list(polygon.vertices, &path)?;
                    let points = matches!(vertices.data_type(), DataType::Struct(fields)
                        if fields.len() == 2
                            && fields[0].name() == "x"
                            && fields[1].name() == "y");
                    if !points || vertices.len() < 3 {
                        return Err(fail(
                            &path,
                            "the vertices of a polygon are three structs or more of an `x` and a \
                             `y` field",
                        ));
                    }
                }
                tuple_field::Test::CyclicRange(range) => {
                    named(&range.column, &path)?;
                    if !(range.start.is_finite() && range.end.is_finite()) {
                        return Err(fail(&path, "a cyclic range has finite ends"));
                    }
                    if !(range.period.is_finite() && range.period > 0.0) {
                        return Err(fail(&path, "a cyclic range has a positive, finite period"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn named(column: &str, path: &str) -> Result<()> {
    if column.is_empty() {
        return Err(fail(path, "a test names its column"));
    }
    Ok(())
}

/// A partition as the key of an identity, in the order of its columns.
fn sorted_partition<'b>(
    values: Values,
    partition: impl IntoIterator<Item = &'b proto::PartitionValue>,
    path: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut partition: Vec<(String, Vec<u8>)> = partition
        .into_iter()
        .map(|value| Ok((value.column.clone(), values.key(value.value, path)?)))
        .collect::<Result<_>>()?;
    partition.sort();
    Ok(partition)
}

/// Checks the partition values that a node declares: in the order of their columns, each column
/// at most once, no column that an ancestor declares, and each value one value.
fn partition_values(
    chart_values: Values,
    values: &[proto::PartitionValue],
    ancestors: &[&proto::PartitionValue],
    path: &str,
) -> Result<()> {
    for (index, value) in values.iter().enumerate() {
        let path = format!("{path}/partition[{index}]");
        if value.column.is_empty() {
            return Err(fail(&path, "a partition value names its column"));
        }
        if index > 0 && values[index - 1].column >= value.column {
            return Err(fail(
                &path,
                "the partition values are in the order of their columns, each column at most \
                 once",
            ));
        }
        if ancestors
            .iter()
            .any(|ancestor| ancestor.column == value.column)
        {
            return Err(fail(
                &path,
                format!(
                    "an ancestor declares the partition column `{}`",
                    value.column
                ),
            ));
        }
        chart_values.scalar(value.value, &path)?;
    }
    Ok(())
}

/// Checks that each partition column that the unit's table has holds the partition value in
/// every row.
fn partition_in_table(
    values: Values,
    table: &RecordBatch,
    partition: &[&proto::PartitionValue],
    path: &str,
) -> Result<()> {
    for value in partition {
        let Some(column) = table.column_by_name(&value.column) else {
            continue;
        };
        let expected = values.scalar(value.value, path)?.clone();
        if column.data_type() != expected.data_type() {
            return Err(fail(
                path,
                format!(
                    "the column `{}` has type {}, and its partition value has type {}",
                    value.column,
                    column.data_type(),
                    expected.data_type()
                ),
            ));
        }
        let equal = not_distinct(column, &Scalar::new(expected))
            .map_err(|error| fail(path, error.to_string()))?;
        if equal.false_count() > 0 {
            return Err(fail(
                path,
                format!(
                    "the column `{}` holds other values than the partition value",
                    value.column
                ),
            ));
        }
    }
    Ok(())
}

/// Checks the resolve rules of a node: in the order of `Channel`, each channel at most once, on a
/// channel that takes a scale, with a sharing.
fn resolves(resolves: &[proto::Resolve], path: &str) -> Result<()> {
    for (index, resolve) in resolves.iter().enumerate() {
        let path = format!("{path}/resolve[{index}]");
        if index > 0 && resolves[index - 1].channel >= resolve.channel {
            return Err(fail(
                &path,
                "the resolve rules are in the order of `Channel`, each channel at most once",
            ));
        }
        let channel = Channel::try_from(resolve.channel).unwrap_or(Channel::Unspecified);
        if !takes_scale(channel) {
            return Err(fail(
                &path,
                "a resolve rule names a channel that takes a scale",
            ));
        }
        if resolve.sharing == 0 {
            return Err(fail(&path, "a resolve rule is shared or independent"));
        }
        sharing(resolve.sharing, &path)?;
    }
    Ok(())
}

/// Checks that the children of a facet that combine charts agree: the children of a facet show
/// one chart over parts of one table, so overlays share each channel alike, and concats and wraps
/// share each channel alike and facet one column.
fn agreement(children: &[proto::Node], path: &str) -> Result<()> {
    let mut firsts: [Option<(usize, &Node)>; 2] = [None, None];
    for (index, child) in children.iter().enumerate() {
        let kind = match &child.node {
            Some(node @ Node::Overlay(_)) => (0, node),
            Some(node @ (Node::Concat(_) | Node::Wrap(_))) => (1, node),
            _ => continue,
        };
        let Some((first, first_node)) = firsts[kind.0] else {
            firsts[kind.0] = Some((index, kind.1));
            continue;
        };
        let node = kind.1;
        let names = format!("the children {first} and {index} of a facet");
        let column = |node: &Node| match node {
            Node::Concat(concat) => concat.facet.as_ref().map(|facet| facet.column.clone()),
            Node::Wrap(wrap) => wrap.facet.as_ref().map(|facet| facet.column.clone()),
            _ => None,
        };
        let child_path = format!("{path}/children[{index}]");
        if column(first_node) != column(node) {
            let name = |column: Option<String>| {
                column.map_or_else(|| "no column".to_string(), |column| format!("`{column}`"))
            };
            return Err(fail(
                &child_path,
                format!(
                    "{names} facet {} and {}, and the children of a facet show one chart over \
                     parts of one table",
                    name(column(first_node)),
                    name(column(node))
                ),
            ));
        }
        let rules = |node: &Node| match node {
            Node::Overlay(overlay) => overlay.resolve.clone(),
            Node::Concat(concat) => concat.resolve.clone(),
            Node::Wrap(wrap) => wrap.resolve.clone(),
            _ => Vec::new(),
        };
        let mut channels: Vec<i32> = rules(first_node)
            .iter()
            .chain(&rules(node))
            .map(|rule| rule.channel)
            .collect();
        channels.sort_unstable();
        channels.dedup();
        if resolve::projection_sharing(first_node) != resolve::projection_sharing(node) {
            return Err(fail(
                &child_path,
                format!(
                    "{names} share the projection differently, and the children of a facet show \
                     one chart over parts of one table"
                ),
            ));
        }
        for channel in channels {
            let channel = Channel::try_from(channel).unwrap_or(Channel::Unspecified);
            if resolve::sharing(first_node, channel) != resolve::sharing(node, channel) {
                return Err(fail(
                    &child_path,
                    format!(
                        "{names} share `{}` differently, and the children of a facet show one \
                         chart over parts of one table",
                        channel.as_str_name()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Checks the size of a track: a length of 0 pixels or more, or a share of a positive weight
/// with a positive maximum.
fn track_size(size: &track::Size, path: &str) -> Result<()> {
    match size {
        track::Size::Length(length) if !(length.is_finite() && *length >= 0.0) => Err(fail(
            path,
            "the length of a track is a finite count of pixels, 0 or more",
        )),
        track::Size::Share(share) if !(share.weight.is_finite() && share.weight > 0.0) => {
            Err(fail(path, "a share has a positive weight"))
        }
        track::Size::Share(proto::Share { max: Some(max), .. })
            if !(max.is_finite() && *max > 0.0) =>
        {
            Err(fail(path, "the maximum of a share is a positive length"))
        }
        _ => Ok(()),
    }
}

/// Checks a sharing that is not set or is one of the values of `Sharing`.
fn sharing(sharing: i32, path: &str) -> Result<()> {
    if proto::Sharing::try_from(sharing).is_err() {
        return Err(fail(path, format!("{sharing} is not a value of `Sharing`")));
    }
    Ok(())
}

/// Checks the guide rules of a layout: in the order of `GuidePart`, each part at most once, with
/// a rule.
fn guide_rules(rules: &[proto::GuideRule], path: &str) -> Result<()> {
    for (index, rule) in rules.iter().enumerate() {
        let path = format!("{path}/guide_rules[{index}]");
        if index > 0 && rules[index - 1].part >= rule.part {
            return Err(fail(
                &path,
                "the guide rules are in the order of `GuidePart`, each part at most once",
            ));
        }
        if !matches!(
            proto::GuidePart::try_from(rule.part),
            Ok(part) if part != proto::GuidePart::Unspecified
        ) || !matches!(
            proto::GuideRuleKind::try_from(rule.rule),
            Ok(kind) if kind != proto::GuideRuleKind::Unspecified
        ) {
            return Err(fail(&path, "a guide rule names a part and a rule"));
        }
    }
    Ok(())
}

/// Checks that a rect without the second channel of a position spans the band of that position
/// on a band scale.
fn rect_spans(unit: &proto::Unit, plot: CoordKind, path: &str) -> Result<()> {
    let encoding = |channel: Channel| {
        unit.encodings
            .iter()
            .find(|encoding| encoding.channel == channel as i32)
    };
    for [primary, pair] in plot.pairs().iter().copied() {
        if let (Some(position), None) = (encoding(primary), encoding(pair)) {
            let band = position
                .scale
                .as_ref()
                .is_some_and(|scale| scale.r#type == "band");
            if !band {
                return Err(fail(
                    path,
                    format!(
                        "a rect with `{}` and without `{}` spans a band of a band scale",
                        primary.as_str_name(),
                        pair.as_str_name()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Checks that a bar in a polar plot without the second channel of its key, `theta` for a
/// vertical bar and `radius` for a horizontal one, spans a band of its key on a band scale,
/// because the thickness of a bar in pixels is neither an angle nor a radius.
fn polar_bar_spans(unit: &proto::Unit, bar: &proto::BarMark, path: &str) -> Result<()> {
    let encoding = |channel: Channel| {
        unit.encodings
            .iter()
            .find(|encoding| encoding.channel == channel as i32)
    };
    let scale_type = |channel: Channel| {
        encoding(channel)
            .and_then(|encoding| encoding.scale.as_ref())
            .map(|scale| scale.r#type.as_str())
    };
    let continuous =
        |channel| scale_type(channel).is_some_and(|kind| FitKind::of(kind) == FitKind::Interval);
    let horizontal = match proto::Orient::try_from(bar.orient) {
        Ok(proto::Orient::Horizontal) => true,
        Ok(proto::Orient::Vertical) => false,
        _ => continuous(Channel::Theta) && !continuous(Channel::Radius),
    };
    let (key, key2) = if horizontal {
        (Channel::Radius, Channel::Radius2)
    } else {
        (Channel::Theta, Channel::Theta2)
    };
    if encoding(key).is_some() && encoding(key2).is_none() && scale_type(key) != Some("band") {
        return Err(fail(
            path,
            format!(
                "a bar in a polar plot with `{}` and without `{}` spans a band of a band scale",
                key.as_str_name(),
                key2.as_str_name()
            ),
        ));
    }
    Ok(())
}

/// Checks that the channels of a line, a trail, or an area that take one value for each series
/// hold one value in each series that the `detail` and the color channels split.
fn one_value_per_series(
    unit: &proto::Unit,
    mark: &mark::Mark,
    table: &RecordBatch,
    path: &str,
) -> Result<()> {
    let per_series: &[Channel] = match mark {
        mark::Mark::Line(_) => &[
            Channel::Opacity,
            Channel::StrokeOpacity,
            Channel::StrokeWidth,
            Channel::StrokeDash,
        ],
        mark::Mark::Trail(_) => &[Channel::Opacity],
        mark::Mark::Area(_) => &[
            Channel::Opacity,
            Channel::FillOpacity,
            Channel::StrokeOpacity,
            Channel::StrokeWidth,
            Channel::StrokeDash,
            Channel::Pattern,
        ],
        _ => return Ok(()),
    };
    let column_of = |channel: Channel| {
        unit.encodings
            .iter()
            .find_map(|encoding| match &encoding.value {
                Some(encoding::Value::Column(name)) if encoding.channel == channel as i32 => {
                    table.column_by_name(name).cloned()
                }
                _ => None,
            })
    };
    let checked: Vec<(Channel, ArrayRef)> = per_series
        .iter()
        .filter_map(|channel| column_of(*channel).map(|column| (*channel, column)))
        .collect();
    if checked.is_empty() || table.num_rows() == 0 {
        return Ok(());
    }
    let keys: Vec<ArrayRef> = [
        Channel::Color,
        Channel::Fill,
        Channel::Stroke,
        Channel::Detail,
    ]
    .into_iter()
    .filter_map(column_of)
    .collect();
    let rows = |columns: &[ArrayRef]| {
        let converter = RowConverter::new(
            columns
                .iter()
                .map(|column| SortField::new(column.data_type().clone()))
                .collect(),
        )?;
        converter.convert_columns(columns)
    };
    let key_rows = if keys.is_empty() {
        None
    } else {
        Some(rows(&keys).map_err(|error| fail(path, error.to_string()))?)
    };
    for (channel, column) in checked {
        let values =
            rows(std::slice::from_ref(&column)).map_err(|error| fail(path, error.to_string()))?;
        let mut first: HashMap<&[u8], &[u8]> = HashMap::new();
        for row in 0..table.num_rows() {
            let key = key_rows
                .as_ref()
                .map_or(&[][..], |rows| rows.row(row).data());
            let value = values.row(row).data();
            if *first.entry(key).or_insert(value) != value {
                return Err(fail(
                    path,
                    format!(
                        "`{}` takes one value for each series of the {}, and its column changes \
                         within a series",
                        channel.as_str_name(),
                        mark_name(mark)
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Checks a column of raw values, without a scale, against the form of its channel.
fn raw_values(channel: Channel, data_type: &DataType, path: &str) -> Result<()> {
    let ok = match channel {
        Channel::Color
        | Channel::Fill
        | Channel::Stroke
        | Channel::Shape
        | Channel::Pattern
        | Channel::Path => is_string(data_type),
        Channel::Opacity
        | Channel::FillOpacity
        | Channel::StrokeOpacity
        | Channel::StrokeWidth
        | Channel::Size
        | Channel::Angle
        | Channel::Theta
        | Channel::Radius
        | Channel::Longitude
        | Channel::Latitude
        | Channel::Longitude2
        | Channel::Latitude2 => data_type.is_numeric(),
        Channel::StrokeDash => {
            is_string(data_type)
                || matches!(
                    data_type,
                    DataType::List(field) | DataType::LargeList(field) | DataType::FixedSizeList(field, _)
                        if field.data_type().is_numeric()
                )
        }
        Channel::Image => {
            is_string(data_type)
                || matches!(data_type, DataType::Struct(fields)
                    if ["width", "height", "data"].iter().all(|name| fields.find(name).is_some()))
        }
        _ => true,
    };
    if !ok {
        return Err(fail(
            path,
            format!(
                "a `{}` column without a scale cannot hold values of type {data_type}",
                channel.as_str_name()
            ),
        ));
    }
    Ok(())
}

/// The kind of fit of a scale type, as the format names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitKind {
    /// Linear, log, pow, symlog, time, and quantize scales.
    Interval,
    /// Band, point, nested band, and ordinal scales.
    Categories,
    /// Quantile scales, whose summaries are samples and which have no fit.
    Sample,
    /// Threshold scales, which take only an explicit domain.
    Explicit,
}

impl FitKind {
    /// The kind of fit of the scale type `scale_type`, a typetag name of avenger-scales.
    pub fn of(scale_type: &str) -> Self {
        match scale_type {
            "linear" | "log" | "pow" | "symlog" | "time" | "quantize" => Self::Interval,
            "band" | "point" | "nested_band" | "ordinal" => Self::Categories,
            "quantile" => Self::Sample,
            _ => Self::Explicit,
        }
    }
}

fn fit_matches(
    values: Values,
    fit: &proto::DomainFit,
    kind: FitKind,
    scale_type: &str,
    path: &str,
) -> Result<()> {
    match (fit.fit.as_ref(), kind) {
        (Some(domain_fit::Fit::Interval(interval)), FitKind::Interval) => {
            if let Some(empty) = interval.empty {
                values.list(empty, path)?;
            }
            Ok(())
        }
        (Some(domain_fit::Fit::Categories(categories)), FitKind::Categories) => {
            match categories.order.as_ref() {
                Some(category_fit::Order::Sort(sort)) => {
                    match proto::CategorySort::try_from(*sort) {
                        Ok(proto::CategorySort::Ascending | proto::CategorySort::Descending) => {
                            Ok(())
                        }
                        _ => Err(fail(path, "a category sort is ascending or descending")),
                    }
                }
                Some(category_fit::Order::Preferred(index)) => {
                    if scale_type == "nested_band" {
                        return Err(fail(path, "a nested band scale takes no preferred order"));
                    }
                    values.list(*index, path).map(|_| ())
                }
                None => Err(fail(path, "a category fit has an order")),
            }
        }
        _ => Err(fail(
            path,
            format!("the fit is not of the kind that a {scale_type} scale fits"),
        )),
    }
}

/// The scale type of a typetag name of avenger-scales.
fn scale_impl(scale_type: &str, path: &str) -> Result<Box<dyn ScaleImpl>> {
    serde_json::from_value(serde_json::json!({ "type": scale_type, "fields": null })).map_err(
        |_| {
            fail(
                path,
                format!("avenger-scales has no scale type `{scale_type}`"),
            )
        },
    )
}

/// Whether a scale whose domain is of `kind` takes values of `data_type`.
fn takes_values(kind: DomainKind, data_type: &DataType) -> bool {
    let data_type = dictionary_values(data_type);
    match kind {
        DomainKind::Numeric => data_type.is_numeric(),
        DomainKind::Temporal => data_type.is_temporal(),
        DomainKind::Categorical => true,
        DomainKind::NestedCategorical => matches!(data_type, DataType::Struct(_)),
    }
}

/// Whether values of type `values` stand for values of a domain of type `domain`: numbers for
/// numbers, strings for strings, and otherwise one type, with a dictionary as the type of its
/// values.
fn compatible(values: &DataType, domain: &DataType) -> bool {
    let (values, domain) = (dictionary_values(values), dictionary_values(domain));
    (values.is_numeric() && domain.is_numeric())
        || (is_string(values) && is_string(domain))
        || values == domain
}

pub(crate) fn dictionary_values(data_type: &DataType) -> &DataType {
    match data_type {
        DataType::Dictionary(_, values) => values,
        other => other,
    }
}

/// Fails when a value of `array` is less than the one before it.
fn ascends(array: &ArrayRef, path: &str) -> Result<()> {
    if array.len() < 2 {
        return Ok(());
    }
    let next = array.slice(1, array.len() - 1);
    let previous = array.slice(0, array.len() - 1);
    let less = lt(&next, &previous).map_err(|error| fail(path, error.to_string()))?;
    if less.true_count() > 0 {
        return Err(fail(path, "the values are in ascending order"));
    }
    Ok(())
}

fn column<'b>(table: &'b RecordBatch, name: &str, path: &str) -> Result<&'b ArrayRef> {
    table
        .column_by_name(name)
        .ok_or_else(|| fail(path, format!("the unit's table has no column `{name}`")))
}

fn is_string(data_type: &DataType) -> bool {
    matches!(
        dictionary_values(data_type),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    )
}

pub(crate) fn is_position(channel: Channel) -> bool {
    matches!(
        channel,
        Channel::X
            | Channel::Y
            | Channel::X2
            | Channel::Y2
            | Channel::Longitude
            | Channel::Latitude
            | Channel::Longitude2
            | Channel::Latitude2
            | Channel::Geometry
            | Channel::Theta
            | Channel::Radius
            | Channel::Theta2
            | Channel::Radius2
    )
}

/// The first channel of a pair whose second channel is `channel`.
fn first_of(channel: Channel) -> Option<Channel> {
    match channel {
        Channel::X2 => Some(Channel::X),
        Channel::Y2 => Some(Channel::Y),
        Channel::Theta2 => Some(Channel::Theta),
        Channel::Radius2 => Some(Channel::Radius),
        Channel::Longitude2 => Some(Channel::Longitude),
        Channel::Latitude2 => Some(Channel::Latitude),
        _ => None,
    }
}

/// Whether a channel can take a scale: not the second channels of pairs, which follow the first,
/// and not the channels of raw values.
pub(crate) fn takes_scale(channel: Channel) -> bool {
    !matches!(
        channel,
        Channel::Unspecified
            | Channel::X2
            | Channel::Y2
            | Channel::Theta2
            | Channel::Radius2
            | Channel::Longitude
            | Channel::Latitude
            | Channel::Longitude2
            | Channel::Latitude2
            | Channel::Geometry
            | Channel::Text
            | Channel::Image
            | Channel::Path
            | Channel::Detail
    )
}

fn mark_name(mark: &mark::Mark) -> &'static str {
    match mark {
        mark::Mark::Symbol(_) => "symbol",
        mark::Mark::Line(_) => "line",
        mark::Mark::Trail(_) => "trail",
        mark::Mark::Area(_) => "area",
        mark::Mark::Bar(_) => "bar",
        mark::Mark::Rect(_) => "rect",
        mark::Mark::Rule(_) => "rule",
        mark::Mark::Text(_) => "text",
        mark::Mark::Image(_) => "image",
        mark::Mark::Path(_) => "path",
        mark::Mark::Geoshape(_) => "geoshape",
    }
}

/// The channels that `mark` takes in a plot of `plot`, as the comment of each mark lists them.
fn mark_channels(mark: &mark::Mark, plot: CoordKind) -> Vec<Channel> {
    use Channel::*;
    let (pairs, others): (bool, &[Channel]) = match mark {
        mark::Mark::Symbol(_) => (
            false,
            &[
                Color,
                Fill,
                Stroke,
                Opacity,
                FillOpacity,
                StrokeOpacity,
                Size,
                Shape,
                Angle,
                Pattern,
            ],
        ),
        mark::Mark::Line(_) => (
            false,
            &[
                Color,
                Stroke,
                Opacity,
                StrokeOpacity,
                StrokeWidth,
                StrokeDash,
                Detail,
            ],
        ),
        mark::Mark::Trail(_) => (false, &[Color, Stroke, Opacity, Size, Detail]),
        mark::Mark::Area(_) => (
            true,
            &[
                Color,
                Fill,
                Stroke,
                Opacity,
                FillOpacity,
                StrokeOpacity,
                StrokeWidth,
                StrokeDash,
                Pattern,
                Detail,
            ],
        ),
        mark::Mark::Bar(_) | mark::Mark::Rect(_) => (
            true,
            &[
                Color,
                Fill,
                Stroke,
                Opacity,
                FillOpacity,
                StrokeOpacity,
                StrokeWidth,
                Pattern,
            ],
        ),
        mark::Mark::Rule(_) => (true, &[Color, Stroke, Opacity, StrokeWidth, StrokeDash]),
        mark::Mark::Text(_) => (false, &[Text, Color, Fill, Opacity, Size, Angle]),
        mark::Mark::Image(_) => (plot == CoordKind::Cartesian, &[Image]),
        mark::Mark::Path(_) => (
            false,
            &[
                Path,
                Color,
                Fill,
                Stroke,
                Opacity,
                FillOpacity,
                StrokeOpacity,
                Angle,
                Pattern,
            ],
        ),
        mark::Mark::Geoshape(_) => {
            return vec![
                Geometry,
                Color,
                Fill,
                Stroke,
                Opacity,
                FillOpacity,
                StrokeOpacity,
                Size,
                Shape,
                Pattern,
            ]
        }
    };
    let mut channels = plot.positions().to_vec();
    if pairs {
        channels.extend(plot.pairs().iter().map(|[_, second]| *second));
    }
    channels.extend_from_slice(others);
    channels
}
