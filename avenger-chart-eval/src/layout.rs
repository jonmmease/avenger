//! The lowering of a chart to an avenger-layout tree, the `Measure` that answers each slab with
//! the extent of its guide, and the drawing of each guide in the slab that represents it.
//!
//! A concat lowers to a `Row` or a `Column`, a wrap to a `Wrap` whose children each start a table
//! of their own, an empty child to a `Leaf` that keeps its track, and a plot to a `Leaf` with a
//! slab for each part of each axis, a stack of legends for each owner of its legends, its
//! attribution, and its title. Each slab of a guide of a scale has a tag of its scale and its
//! part, and the owner of the scale declares the rule of the tag from its layout options, so the
//! guides of a shared scale collapse. A concat or a wrap whose rule merges a legend holds it
//! beyond its facet headers, and the owner of a shared `x` or `y` whose plots stand in several
//! tracks matches the lengths of those tracks.
//!
//! A facet header is a slab for its labels and a slab for its title on the node that holds it:
//! the concat with the facet, or each child of a wrap with a facet. The headers of a concat's
//! column collapse below the innermost node that holds them all, which also matches the thickness
//! of their labels, and the titles of a wrap's headers merge on the wrap.

use std::collections::{BTreeMap, HashMap, HashSet};

use avenger_guides::{facet::FacetGuidePart, GuideSide, PartExtent};
use avenger_layout::{
    Column, Direction, Distribute, Gap, Layout, Leaf, Measure, Measurement, Row, Side, Slab,
    SlabStack, Solution, Track, Wrap,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};

use avenger_chart::{
    proto::{self, coord, node::Node, track, GuidePart, GuideRuleKind},
    resolve::{ScaleGroup, Tree},
    table::{self, Matches, Tables},
    Chart,
};

use crate::{
    guides::{Guides, LegendGuide, AXIS_PARTS, HEADER_PARTS},
    polar::Sector,
    Theme,
};

/// The id of a node of a chart's layout: a plot, or a slab of one of its guides.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SlabId {
    Plot(usize),
    /// A part of an axis of a plot: the plot, the axis, and the index of the part in
    /// [`AXIS_PARTS`].
    Axis {
        plot: usize,
        axis: usize,
        part: usize,
    },
    /// A legend or a colorbar of a plot.
    Legend {
        plot: usize,
        legend: usize,
    },
    /// A legend or a colorbar that a concat or a wrap holds for the plots below it, because its
    /// rule merges the legend.
    HeldLegend(usize),
    /// The title of a plot or a concat.
    Title(usize),
    /// The attribution of a plot.
    Attribution(usize),
    /// A part of a facet header: the header, and the index of the part in [`HEADER_PARTS`].
    Header {
        header: usize,
        part: usize,
    },
    /// A concat, a wrap, or an empty child, by the order of its path in the lowering, whose
    /// region places the labels of a facet header.
    Node(usize),
    /// The labels of the graticule of a map plot on one side, which the evaluator draws through
    /// the plot's projector.
    Graticule {
        plot: usize,
        side: GuideSide,
    },
}

fn layout_side(side: GuideSide) -> Side {
    match side {
        GuideSide::Top => Side::Top,
        GuideSide::Bottom => Side::Bottom,
        GuideSide::Left => Side::Left,
        GuideSide::Right => Side::Right,
    }
}

/// The rule of a guide part among the plots of a concat or a wrap: its rule in `rules`, or the
/// default, which collapses the ticks and the labels of an axis and the labels and the title of a
/// facet header, and merges the title of an axis, legends, colorbars, and attributions. A wrap
/// merges the titles of its children's headers by default, because each child has a header of
/// its own.
fn rule(rules: &[proto::GuideRule], part: GuidePart, wrap: bool) -> GuideRuleKind {
    let set = rules
        .iter()
        .find(|rule| rule.part == part as i32)
        .and_then(|rule| GuideRuleKind::try_from(rule.rule).ok());
    match (set, part) {
        (Some(rule), _) => rule,
        (None, GuidePart::HeaderTitle) if wrap => GuideRuleKind::Merge,
        (
            None,
            GuidePart::AxisTicks
            | GuidePart::AxisLabels
            | GuidePart::HeaderLabels
            | GuidePart::HeaderTitle,
        ) => GuideRuleKind::Collapse,
        (None, _) => GuideRuleKind::Merge,
    }
}

/// The guide parts of the header parts of [`HEADER_PARTS`], in their order.
const HEADER_RULE_PARTS: [GuidePart; 2] = [GuidePart::HeaderLabels, GuidePart::HeaderTitle];

fn header_tag(name: &str, part: usize) -> String {
    format!("facet {name} {part}")
}

/// The guide parts of the axis parts of [`AXIS_PARTS`], in their order.
const AXIS_RULE_PARTS: [GuidePart; 3] = [
    GuidePart::AxisTicks,
    GuidePart::AxisLabels,
    GuidePart::AxisTitle,
];

/// The tag of the stacks of the legends of one owner on one side whose part has one rule, so the
/// owner's rule for each part applies to its stacks.
fn legend_tag(owner: &str, side: GuideSide, part: GuidePart) -> String {
    format!("legends {owner} {side:?} {}", part.as_str_name())
}

const ATTRIBUTION_TAG: &str = "attribution";

/// The tag of the ticks or the labels of the axes of the scale group `group`. They draw its
/// scale, so each part of the axes of one group draws one merged guide.
fn axis_tag(group: usize, part: usize) -> String {
    format!("axis {group} {part}")
}

/// The tag of the titles of the axes at `position` on a side of their plots, from the plot out.
/// A title draws only its text, so every concat and wrap declares a rule for its tag, and equal
/// titles collapse and merge wherever they line up, whatever their scales.
fn title_tag(position: usize) -> String {
    format!("axis title {position}")
}

/// The tag of the tracks whose lengths a shared position scale matches, in a direction.
fn track_tag(direction: usize) -> String {
    format!("plot {direction}")
}

/// What the lowering reads.
pub(crate) struct Lowering<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) tree: &'a Tree<'a>,
    pub(crate) groups: &'a [ScaleGroup],
    pub(crate) guides: &'a Guides,
    pub(crate) tables: &'a Tables,
    pub(crate) matches: &'a Matches,
    pub(crate) theme: &'a Theme,
}

/// A lowered chart: its layout, the ids of the slabs of its guides in tree order, and the id of
/// each node that is not a unit inside an overlay, by its path.
pub(crate) struct Lowered {
    pub(crate) layout: Layout<SlabId>,
    pub(crate) slabs: Vec<SlabId>,
    pub(crate) nodes: HashMap<String, SlabId>,
}

/// The ids that the lowering hands out.
#[derive(Default)]
struct Ids {
    slabs: Vec<SlabId>,
    nodes: HashMap<String, SlabId>,
}

impl Ids {
    fn slab(&mut self, id: SlabId) -> SlabId {
        self.slabs.push(id.clone());
        id
    }

    fn node(&mut self, path: &str) -> SlabId {
        let id = SlabId::Node(self.nodes.len());
        self.nodes.insert(path.to_string(), id.clone());
        id
    }
}

/// The layout of a chart.
pub(crate) fn lower(lowering: &Lowering) -> Lowered {
    let mut ids = Ids::default();
    let layout = lowering.node(lowering.chart.root(), "root", true, &mut ids);
    Lowered {
        layout,
        slabs: ids.slabs,
        nodes: ids.nodes,
    }
}

impl Lowering<'_> {
    fn node(
        &self,
        node: &proto::Node,
        path: &str,
        table_root: bool,
        ids: &mut Ids,
    ) -> Layout<SlabId> {
        match &node.node {
            Some(Node::Concat(concat)) => self.concat(concat, path, table_root, ids),
            Some(Node::Wrap(wrap)) => self.wrap(wrap, path, ids),
            Some(Node::Unit(_) | Node::Overlay(_)) => {
                match self.tree.plots.iter().position(|plot| plot.path == path) {
                    Some(plot) => self.plot(plot, ids),
                    None => Leaf::new().into(),
                }
            }
            Some(Node::Empty(_)) | None => {
                let mut leaf = Leaf::new().id(ids.node(path));
                for (side, hosted) in self.host_slabs(path, ids) {
                    leaf = hosted.on(leaf, side);
                }
                leaf.into()
            }
        }
    }

    /// The slabs that the node at `path` hosts beyond its own guides: the headers that it holds,
    /// nearest first, and then its title, which runs along the whole side.
    fn host_slabs(&self, path: &str, ids: &mut Ids) -> Vec<(Side, Hosted)> {
        let mut slabs = Vec::new();
        for (index, header) in self.guides.headers.iter().enumerate() {
            if header.host != path {
                continue;
            }
            for (part, header_part) in HEADER_PARTS.into_iter().enumerate() {
                let id = ids.slab(SlabId::Header {
                    header: index,
                    part,
                });
                slabs.push((
                    layout_side(header.side),
                    Hosted::Slab(
                        Slab::new()
                            .id(id)
                            .tag(header_tag(&header.name, part).as_str())
                            .content_key(header.guide.content_key(header_part).as_str()),
                    ),
                ));
            }
        }
        // The legends that the node merges for the plots below it stand beyond its headers, one
        // stack on each side.
        let mut sides: Vec<(GuideSide, Vec<usize>)> = Vec::new();
        for (index, legend) in self.guides.legends.iter().enumerate() {
            if legend.owner != path || !self.held(index) {
                continue;
            }
            match sides.iter_mut().find(|(side, _)| *side == legend.side) {
                Some((_, members)) => members.push(index),
                None => sides.push((legend.side, vec![index])),
            }
        }
        for (side, members) in sides {
            let stack = self.legend_stack(&members, SlabId::HeldLegend, ids);
            slabs.push((layout_side(side), Hosted::Stack(stack)));
        }
        if let Some(index) = self.guides.title_of.get(path) {
            let id = ids.slab(SlabId::Title(*index));
            slabs.push((
                layout_side(self.guides.titles[*index].side),
                Hosted::Slab(Slab::new().id(id).full()),
            ));
        }
        slabs
    }

    /// Whether the concat or the wrap that owns the scales of a legend holds it, because its rule
    /// merges the legend: one legend stands on the node for the plots below it.
    fn held(&self, legend: usize) -> bool {
        let legend = &self.guides.legends[legend];
        match node_rules(self.chart, &legend.owner) {
            Some((rules, wrap)) => rule(rules, legend.part(), wrap) == GuideRuleKind::Merge,
            None => false,
        }
    }

    /// A stack of the legends `members`, with the id of each from `id`.
    fn legend_stack(
        &self,
        members: &[usize],
        id: impl Fn(usize) -> SlabId,
        ids: &mut Ids,
    ) -> SlabStack<SlabId> {
        let mut slabs = Vec::new();
        let mut tracks = Vec::new();
        for legend_index in members {
            let legend = &self.guides.legends[*legend_index];
            let id = ids.slab(id(*legend_index));
            let mut slab = Slab::new()
                .id(id)
                .content_key(format!("legend {legend_index}").as_str());
            if let Some(length) = legend.max_length {
                slab = slab.max_length(length);
            }
            tracks.push(match legend.guide {
                LegendGuide::Legend(_) => Track::auto(),
                LegendGuide::Colorbar(_) => Track::fr(1.0),
            });
            slabs.push(slab);
        }
        let key = members
            .iter()
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let mut stack = SlabStack::new(slabs)
            .gap(Gap::new(self.theme.legend_gap))
            .content_key(format!("legends {key}").as_str());
        for (index, track) in tracks.into_iter().enumerate() {
            stack = stack.track(index, track);
        }
        stack
    }

    /// The track that a size of the format lowers to.
    fn track(size: Option<&track::Size>) -> Track {
        match size {
            Some(track::Size::Length(length)) => Track::px(*length),
            Some(track::Size::Share(proto::Share {
                weight,
                max: Some(max),
            })) => Track::fr_max(*weight, *max),
            Some(track::Size::Share(share)) => Track::fr(share.weight),
            None => Track::fr(1.0),
        }
    }

    fn concat(
        &self,
        concat: &proto::Concat,
        path: &str,
        table_root: bool,
        ids: &mut Ids,
    ) -> Layout<SlabId> {
        let id = ids.node(path);
        let along = table::direction(concat);
        let children: Vec<Layout<SlabId>> = concat
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                self.node(child, &format!("{path}/children[{index}]"), false, ids)
            })
            .collect();
        let layout = concat.layout.as_ref();
        let gap = Gap::new(
            layout
                .and_then(|layout| layout.gap)
                .unwrap_or(self.theme.concat_gap),
        );
        // Each track that the concat spans, with the size that a concat of its table declares
        // and the tag of a match, so every sequence of the table declares it alike.
        let mut tracks = Vec::new();
        if let Some(place) = self.tables.places.get(path) {
            for (offset, index) in place.tracks[along].clone().enumerate() {
                let declared = self.tables.declared(place.table, along, index);
                let matched = self.matches.tracks.contains(&(place.table, along, index));
                if declared.is_none() && !matched {
                    continue;
                }
                let mut track = Self::track(declared);
                if matched {
                    track = track.tag(track_tag(along));
                }
                tracks.push((offset, track));
            }
        }
        let rules = self.rules(
            path,
            layout.map_or(&[][..], |layout| &layout.guide_rules[..]),
            false,
        );
        let matched = self.matched(path);
        let distribute = |value: i32| match proto::Distribute::try_from(value) {
            Ok(proto::Distribute::Start) => Some(Distribute::Start),
            Ok(proto::Distribute::Center) => Some(Distribute::Center),
            Ok(proto::Distribute::End) => Some(Distribute::End),
            Ok(proto::Distribute::SpaceBetween) => Some(Distribute::SpaceBetween),
            Ok(proto::Distribute::SpaceAround) => Some(Distribute::SpaceAround),
            Ok(proto::Distribute::SpaceEvenly) => Some(Distribute::SpaceEvenly),
            _ => None,
        };
        // Only the outermost sequence of a table sizes its tracks, so only it distributes.
        let distributions = match (table_root, layout) {
            (true, Some(layout)) => [
                distribute(layout.horizontal_distribute),
                distribute(layout.vertical_distribute),
            ],
            _ => [None, None],
        };
        let slabs = self.host_slabs(path, ids);
        macro_rules! sequence {
            ($sequence:expr) => {{
                let mut sequence = $sequence.id(id).gap(gap);
                for (index, track) in tracks {
                    sequence = sequence.track(index, track);
                }
                for (tag, rule) in rules {
                    sequence = match rule {
                        Rule::Kind(GuideRuleKind::Collapse) => sequence.collapse_tag(tag.as_str()),
                        Rule::Kind(GuideRuleKind::Merge) => sequence.merge_tag(tag.as_str()),
                        Rule::Match(tag) => sequence.match_tag(tag.as_str()),
                        Rule::Kind(_) => sequence,
                    };
                }
                for tag in matched {
                    sequence = sequence.match_tag(tag.as_str());
                }
                for (direction, distribute) in [Direction::Horizontal, Direction::Vertical]
                    .into_iter()
                    .zip(distributions)
                {
                    if let Some(distribute) = distribute {
                        sequence = sequence.distribute(direction, distribute);
                    }
                }
                for (side, hosted) in slabs {
                    sequence = match hosted {
                        Hosted::Slab(slab) => sequence.slab(side, slab),
                        Hosted::Stack(stack) => sequence.slab_stack(side, stack),
                    };
                }
                sequence.into()
            }};
        }
        if along == 0 {
            sequence!(Row::new(children))
        } else {
            sequence!(Column::new(children))
        }
    }

    /// A wrap, whose children each start a table of their own in one cell.
    fn wrap(&self, wrap: &proto::Wrap, path: &str, ids: &mut Ids) -> Layout<SlabId> {
        let id = ids.node(path);
        let children: Vec<Layout<SlabId>> = wrap
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| self.node(child, &format!("{path}/children[{index}]"), true, ids))
            .collect();
        let layout = wrap.layout.as_ref();
        let mut node = Wrap::new(children)
            .id(id)
            .min_column(
                layout
                    .and_then(|layout| layout.min_column_width)
                    .unwrap_or(self.theme.wrap_min_column),
            )
            .gap(Gap::new(
                layout
                    .and_then(|layout| layout.gap)
                    .unwrap_or(self.theme.concat_gap),
            ));
        if let Some(layout) = layout {
            if let Some(columns) = layout.max_columns {
                node = node.max_columns(columns as usize);
            }
            if let Some(row) = &layout.row {
                node = node.rows(Self::track(row.size.as_ref()));
            }
            let distribute = match proto::Distribute::try_from(layout.distribute) {
                Ok(proto::Distribute::Start) => Some(Distribute::Start),
                Ok(proto::Distribute::Center) => Some(Distribute::Center),
                Ok(proto::Distribute::End) => Some(Distribute::End),
                Ok(proto::Distribute::SpaceBetween) => Some(Distribute::SpaceBetween),
                Ok(proto::Distribute::SpaceAround) => Some(Distribute::SpaceAround),
                Ok(proto::Distribute::SpaceEvenly) => Some(Distribute::SpaceEvenly),
                _ => None,
            };
            if let Some(distribute) = distribute {
                node = node.distribute(distribute);
            }
        }
        let rules = self.rules(
            path,
            layout.map_or(&[][..], |layout| &layout.guide_rules[..]),
            true,
        );
        for (tag, rule) in rules {
            node = match rule {
                Rule::Kind(GuideRuleKind::Collapse) => node.collapse_tag(tag.as_str()),
                Rule::Kind(GuideRuleKind::Merge) => node.merge_tag(tag.as_str()),
                Rule::Match(tag) => node.match_tag(tag.as_str()),
                Rule::Kind(_) => node,
            };
        }
        for tag in self.matched(path) {
            node = node.match_tag(tag.as_str());
        }
        for (side, hosted) in self.host_slabs(path, ids) {
            node = match hosted {
                Hosted::Slab(slab) => node.slab(side, slab),
                Hosted::Stack(stack) => node.slab_stack(side, stack),
            };
        }
        node.into()
    }

    /// The tags of the tracks whose lengths the node at `path` matches.
    fn matched(&self, path: &str) -> Vec<String> {
        self.matches
            .owners
            .iter()
            .filter(|(owner, _)| owner == path)
            .map(|(_, direction)| track_tag(*direction))
            .collect()
    }

    /// The rules that the concat or the wrap at `path` declares: those of the parts of the guides
    /// of the scales that it owns and of the headers that it holds, and of the axis titles and the
    /// attributions of the plots below it. It matches the thickness of the labels of the headers
    /// that it holds, so the titles beyond headers with and without a rule line up.
    fn rules(&self, path: &str, rules: &[proto::GuideRule], wrap: bool) -> BTreeMap<String, Rule> {
        let mut declared = BTreeMap::new();
        for axis in &self.guides.axes {
            if self.groups[axis.group].owner != path {
                continue;
            }
            for (part, kind) in AXIS_RULE_PARTS.into_iter().enumerate() {
                if kind != GuidePart::AxisTitle {
                    declared.insert(
                        axis_tag(axis.group, part),
                        Rule::Kind(rule(rules, kind, wrap)),
                    );
                }
            }
        }
        for position in 0..self.title_positions() {
            declared.insert(
                title_tag(position),
                Rule::Kind(rule(rules, GuidePart::AxisTitle, wrap)),
            );
        }
        for (index, legend) in self.guides.legends.iter().enumerate() {
            if legend.owner == path && !self.held(index) {
                let part = legend.part();
                declared.insert(
                    legend_tag(path, legend.side, part),
                    Rule::Kind(rule(rules, part, wrap)),
                );
            }
        }
        for header in &self.guides.headers {
            if header.owner != path {
                continue;
            }
            for (part, kind) in HEADER_RULE_PARTS.into_iter().enumerate() {
                declared.insert(
                    header_tag(&header.name, part),
                    Rule::Kind(rule(rules, kind, header.wrap)),
                );
            }
            declared.insert(
                format!("{} match", header_tag(&header.name, 0)),
                Rule::Match(header_tag(&header.name, 0)),
            );
        }
        declared.insert(
            ATTRIBUTION_TAG.to_string(),
            Rule::Kind(rule(rules, GuidePart::Attribution, wrap)),
        );
        declared
    }

    /// The most axes on one side of a plot, which is the number of title tags.
    fn title_positions(&self) -> usize {
        self.guides
            .plots
            .iter()
            .flat_map(|plot| {
                plot.axes
                    .iter()
                    .map(|&axis| self.position(&plot.axes, axis) + 1)
            })
            .max()
            .unwrap_or(0)
    }

    /// The position of the axis `axis` among the axes `axes` of a plot on its side, from the plot
    /// out.
    fn position(&self, axes: &[usize], axis: usize) -> usize {
        let side = self.guides.axes[axis].side;
        axes.iter()
            .take_while(|&&other| other != axis)
            .filter(|&&other| self.guides.axes[other].side == side)
            .count()
    }

    /// The leaf of a plot with its guides: the slabs of its axes nearest the plot, then a stack of
    /// the legends and colorbars of each owner and part on each side that has some, then its
    /// attribution, the headers that it holds as a child of a wrap, and outermost its title,
    /// which runs the whole side.
    fn plot(&self, plot: usize, ids: &mut Ids) -> Layout<SlabId> {
        let guides = self.guides;
        let plot_guides = &guides.plots[plot];
        let plot_ref = &self.tree.plots[plot];
        let id = SlabId::Plot(plot);
        ids.nodes.insert(plot_ref.path.clone(), id.clone());
        let mut leaf = Leaf::new().id(id);
        match plot_ref.coord.and_then(|coord| coord.coord.as_ref()) {
            Some(coord::Coord::Cartesian(cartesian)) => {
                if let Some(ratio) = cartesian.aspect_ratio {
                    leaf = leaf.aspect(ratio);
                }
            }
            // A polar plot keeps the shape of the box around its part of the circle.
            Some(coord::Coord::Polar(polar)) => {
                leaf = leaf.aspect(Sector::of(polar).aspect());
            }
            _ => {}
        }
        // A map plot keeps the aspect ratio of what it fits, and its graticule labels stand
        // nearest it.
        if let Some(map) = &plot_guides.map {
            if let Some(aspect) = map.aspect {
                leaf = leaf.aspect(aspect);
            }
            for (side, _) in &map.labels {
                let id = ids.slab(SlabId::Graticule { plot, side: *side });
                leaf = leaf.slab(layout_side(*side), Slab::new().id(id));
            }
        }
        for axis_index in &plot_guides.axes {
            let axis = &guides.axes[*axis_index];
            for (part, kind) in AXIS_RULE_PARTS.into_iter().enumerate() {
                let id = ids.slab(SlabId::Axis {
                    plot,
                    axis: *axis_index,
                    part,
                });
                let (tag, key) = match kind {
                    GuidePart::AxisTitle => (
                        title_tag(self.position(&plot_guides.axes, *axis_index)),
                        axis.title_key.clone(),
                    ),
                    _ => (
                        axis_tag(axis.group, part),
                        format!("axis {axis_index} {part} {}", axis.revision),
                    ),
                };
                leaf = leaf.slab(
                    layout_side(axis.side),
                    Slab::new()
                        .id(id)
                        .tag(tag.as_str())
                        .content_key(key.as_str()),
                );
            }
        }
        // The legends of the scales that the plot owns stand in one stack on each side, and those
        // of each node that owns a scale of the plot and keeps or collapses its legends stand in
        // a stack for each side and part, which follows the node's rule for the part.
        let mut stacks: Vec<(String, GuideSide, Vec<usize>)> = Vec::new();
        for legend_index in &plot_guides.legends {
            let legend = &guides.legends[*legend_index];
            if self.held(*legend_index) {
                continue;
            }
            let path = &plot_ref.path;
            let local = legend.owner == *path
                || legend
                    .owner
                    .strip_prefix(path.as_str())
                    .is_some_and(|rest| rest.starts_with('/'));
            let tag = if local {
                format!("legends {path} {:?}", legend.side)
            } else {
                legend_tag(&legend.owner, legend.side, legend.part())
            };
            match stacks.iter_mut().find(|(known, _, _)| *known == tag) {
                Some((_, _, members)) => members.push(*legend_index),
                None => stacks.push((tag, legend.side, vec![*legend_index])),
            }
        }
        for (tag, side, members) in stacks {
            let stack = self
                .legend_stack(&members, |legend| SlabId::Legend { plot, legend }, ids)
                .tag(tag.as_str());
            leaf = leaf.slab_stack(layout_side(side), stack);
        }
        if let Some(index) = plot_guides.attribution {
            let attribution = &guides.attributions[index];
            let id = ids.slab(SlabId::Attribution(index));
            leaf = leaf.slab(
                layout_side(attribution.side),
                Slab::new()
                    .id(id)
                    .tag(ATTRIBUTION_TAG)
                    .content_key(attribution.attribution.content_key()),
            );
        }
        for (side, hosted) in self.host_slabs(&plot_ref.path, ids) {
            leaf = hosted.on(leaf, side);
        }
        leaf.into()
    }
}

/// A slab or a stack that a node hosts beyond its own guides.
enum Hosted {
    Slab(Slab<SlabId>),
    Stack(SlabStack<SlabId>),
}

impl Hosted {
    fn on(self, leaf: Leaf<SlabId>, side: Side) -> Leaf<SlabId> {
        match self {
            Hosted::Slab(slab) => leaf.slab(side, slab),
            Hosted::Stack(stack) => leaf.slab_stack(side, stack),
        }
    }
}

/// The guide rules of the concat or the wrap at `path`, and whether it is a wrap, or `None` for
/// another node.
fn node_rules<'a>(chart: &'a Chart, path: &str) -> Option<(&'a [proto::GuideRule], bool)> {
    let mut node = chart.root();
    for step in path.split('/').skip(1) {
        let index: usize = step
            .strip_prefix("children[")?
            .strip_suffix(']')?
            .parse()
            .ok()?;
        node = match &node.node {
            Some(Node::Concat(concat)) => concat.children.get(index)?,
            Some(Node::Wrap(wrap)) => wrap.children.get(index)?,
            Some(Node::Overlay(overlay)) => overlay.children.get(index)?,
            _ => return None,
        };
    }
    match &node.node {
        Some(Node::Concat(concat)) => Some((
            concat
                .layout
                .as_ref()
                .map_or(&[][..], |layout| &layout.guide_rules[..]),
            false,
        )),
        Some(Node::Wrap(wrap)) => Some((
            wrap.layout
                .as_ref()
                .map_or(&[][..], |layout| &layout.guide_rules[..]),
            true,
        )),
        _ => None,
    }
}

/// A rule that a concat or a wrap declares for a tag.
enum Rule {
    Kind(GuideRuleKind),
    /// A match of the tag, which differs from the key of the rule.
    Match(String),
}

/// The answers of a chart's guides to the layout.
pub(crate) struct Measurer<'a> {
    pub(crate) guides: &'a Guides,
}

impl Measurer<'_> {
    fn extent(&self, id: &SlabId, length: f32) -> Option<PartExtent> {
        let guides = self.guides;
        match id {
            SlabId::Axis { axis, part, .. } => {
                Some(guides.axes[*axis].axis.extent(AXIS_PARTS[*part], length))
            }
            SlabId::Legend { legend, .. } | SlabId::HeldLegend(legend) => {
                Some(guides.legends[*legend].extent(length))
            }
            SlabId::Title(index) => Some(guides.titles[*index].extent(length)),
            SlabId::Attribution(index) => {
                Some(guides.attributions[*index].attribution.extent(length))
            }
            SlabId::Header { header, part } => Some(
                guides.headers[*header]
                    .guide
                    .extent(HEADER_PARTS[*part], length),
            ),
            SlabId::Graticule { plot, side } => guides.plots[*plot]
                .map
                .as_ref()?
                .labels
                .iter()
                .find(|(labeled, _)| labeled == side)
                .map(|(_, extent)| extent.clone()),
            SlabId::Plot(_) | SlabId::Node(_) => None,
        }
    }
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<SlabId> for Measurer<'_> {
    fn height(&mut self, id: &SlabId, width: f32) -> Option<Measurement> {
        self.extent(id, width).map(answer)
    }

    fn width(&mut self, id: &SlabId, height: f32) -> Option<Measurement> {
        self.extent(id, height).map(answer)
    }

    /// A legend's length with its entries on as few lines as its wrap allows, along its side.
    fn max_content(&mut self, id: &SlabId, direction: Direction) -> Option<f32> {
        let (SlabId::Legend { legend, .. } | SlabId::HeldLegend(legend)) = id else {
            return None;
        };
        let slot = &self.guides.legends[*legend];
        let along = if slot.side.is_vertical() {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };
        (direction == along).then(|| slot.max_content()).flatten()
    }
}

/// The guides of a chart, each drawn one time in the slab that represents its slabs.
pub(crate) fn draw(
    guides: &Guides,
    lowered: &Lowered,
    solution: &Solution<SlabId>,
) -> Vec<SceneMark> {
    let mut drawn = HashSet::new();
    let mut marks = Vec::new();
    for id in &lowered.slabs {
        let Some(region) = solution.representative(id) else {
            continue;
        };
        if !drawn.insert(region.path.clone()) {
            continue;
        }
        let rect = region.content;
        let slot = [rect.x, rect.y, rect.width, rect.height];
        let side = match id {
            SlabId::Axis { axis, .. } => guides.axes[*axis].side,
            SlabId::Legend { legend, .. } | SlabId::HeldLegend(legend) => {
                guides.legends[*legend].side
            }
            SlabId::Title(index) => guides.titles[*index].side,
            SlabId::Attribution(index) => guides.attributions[*index].side,
            SlabId::Header { header, .. } => guides.headers[*header].side,
            SlabId::Plot(_) | SlabId::Node(_) | SlabId::Graticule { .. } => continue,
        };
        let length = if side.is_vertical() {
            rect.height
        } else {
            rect.width
        };
        let group = match id {
            SlabId::Axis { axis, part, .. } => {
                let axis = &guides.axes[*axis].axis;
                axis.part(AXIS_PARTS[*part], length)
                    .map(|group| SceneGroup {
                        origin: axis.origin(slot),
                        ..group
                    })
            }
            SlabId::Legend { legend, .. } | SlabId::HeldLegend(legend) => {
                let legend = &guides.legends[*legend];
                Some(SceneGroup {
                    origin: legend.origin(slot),
                    ..legend.group(length)
                })
            }
            SlabId::Title(index) => {
                let title = &guides.titles[*index];
                Some(SceneGroup {
                    origin: title.origin(slot),
                    ..title.part(length)
                })
            }
            SlabId::Attribution(index) => {
                let attribution = &guides.attributions[*index].attribution;
                attribution.part(length).map(|group| SceneGroup {
                    origin: attribution.origin(slot),
                    ..group
                })
            }
            SlabId::Header { header, part } => {
                // The slab that draws the labels labels the children of its own header, which
                // line up with those of the headers that it stands for.
                let header = match &region.id {
                    Some(SlabId::Header { header, .. }) => *header,
                    _ => *header,
                };
                let header_guide = &guides.headers[header];
                let guide = &header_guide.guide;
                let group = match HEADER_PARTS[*part] {
                    FacetGuidePart::Labels => {
                        let bands: Option<Vec<[f32; 2]>> = header_guide
                            .children
                            .iter()
                            .map(|child| {
                                let child = solution.region(lowered.nodes.get(child)?)?.content;
                                Some(if side.is_vertical() {
                                    [child.y - rect.y, child.y + child.height - rect.y]
                                } else {
                                    [child.x - rect.x, child.x + child.width - rect.x]
                                })
                            })
                            .collect();
                        bands.and_then(|bands| guide.labels(&bands))
                    }
                    FacetGuidePart::Title => guide.title(length),
                };
                group.map(|group| SceneGroup {
                    origin: guide.origin(slot),
                    ..group
                })
            }
            SlabId::Plot(_) | SlabId::Node(_) | SlabId::Graticule { .. } => None,
        };
        if let Some(group) = group {
            marks.push(group.into());
        }
    }
    marks
}
