//! The lowering of a chart to an avenger-layout tree, the `Measure` that answers each slab with
//! the extent of its guide, and the drawing of each guide in the slab that represents it.
//!
//! A concat lowers to a `Row` or a `Column`, and a plot to a `Leaf` with a slab for each part of
//! each axis, a stack of legends for each owner of its legends, its attribution, and its title.
//! Each slab of a guide of a scale has a tag of its scale and its part, and the owner of the
//! scale declares the rule of the tag from its layout options, so the guides of a shared scale
//! collapse or merge. The owner of a shared `x` or `y` whose plots stand in several tracks also
//! matches the lengths of those tracks.

use std::collections::{BTreeMap, HashSet};

use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Column, Direction, Distribute, Gap, Layout, Leaf, Measure, Measurement, Row, Side, Slab,
    SlabStack, Solution, Track,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};

use avenger_chart::{
    proto::{self, node::Node, track, GuidePart, GuideRuleKind},
    resolve::{ScaleGroup, Tree},
    table::{self, Matches, Tables},
    Chart,
};

use crate::{
    guides::{Guides, LegendGuide, AXIS_PARTS},
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
    /// The title of a plot or a concat.
    Title(usize),
    /// The attribution of a plot.
    Attribution(usize),
}

fn layout_side(side: GuideSide) -> Side {
    match side {
        GuideSide::Top => Side::Top,
        GuideSide::Bottom => Side::Bottom,
        GuideSide::Left => Side::Left,
        GuideSide::Right => Side::Right,
    }
}

/// The rule of a guide part among the plots of a concat: its rule in `layout`, or the default,
/// which collapses the ticks and the labels of an axis, and merges the title of an axis, legends,
/// colorbars, and attributions.
fn rule(layout: Option<&proto::ConcatLayout>, part: GuidePart) -> GuideRuleKind {
    let set = layout
        .and_then(|layout| {
            layout
                .guide_rules
                .iter()
                .find(|rule| rule.part == part as i32)
        })
        .and_then(|rule| GuideRuleKind::try_from(rule.rule).ok());
    match (set, part) {
        (Some(rule), _) => rule,
        (None, GuidePart::AxisTicks | GuidePart::AxisLabels) => GuideRuleKind::Collapse,
        (None, _) => GuideRuleKind::Merge,
    }
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
/// A title draws only its text, so every concat declares a rule for its tag, and equal titles
/// collapse and merge wherever they line up, whatever their scales.
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

/// The layout of a chart, and the ids of the slabs of its guides in tree order.
pub(crate) fn lower(lowering: &Lowering) -> (Layout<SlabId>, Vec<SlabId>) {
    let mut ids = Vec::new();
    let layout = lowering.node(lowering.chart.root(), "root", true, &mut ids);
    (layout, ids)
}

impl Lowering<'_> {
    fn node(
        &self,
        node: &proto::Node,
        path: &str,
        table_root: bool,
        ids: &mut Vec<SlabId>,
    ) -> Layout<SlabId> {
        match &node.node {
            Some(Node::Concat(concat)) => self.concat(concat, path, table_root, ids),
            Some(Node::Unit(_) | Node::Overlay(_)) => {
                match self.tree.plots.iter().position(|plot| plot.path == path) {
                    Some(plot) => self.plot(plot, ids),
                    None => Leaf::new().into(),
                }
            }
            Some(Node::Wrap(_) | Node::Empty(_)) | None => Leaf::new().into(),
        }
    }

    fn concat(
        &self,
        concat: &proto::Concat,
        path: &str,
        table_root: bool,
        ids: &mut Vec<SlabId>,
    ) -> Layout<SlabId> {
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
                let mut track = match declared {
                    Some(track::Size::Length(length)) => Track::px(*length),
                    Some(track::Size::Share(proto::Share {
                        weight,
                        max: Some(max),
                    })) => Track::fr_max(*weight, *max),
                    Some(track::Size::Share(share)) => Track::fr(share.weight),
                    None => Track::fr(1.0),
                };
                if matched {
                    track = track.tag(track_tag(along));
                }
                tracks.push((offset, track));
            }
        }
        let rules = self.rules(path, layout);
        let matched: Vec<String> = self
            .matches
            .owners
            .iter()
            .filter(|(owner, _)| owner == path)
            .map(|(_, direction)| track_tag(*direction))
            .collect();
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
        let title = self.title_slab(path, ids);
        macro_rules! sequence {
            ($sequence:expr) => {{
                let mut sequence = $sequence.gap(gap);
                for (index, track) in tracks {
                    sequence = sequence.track(index, track);
                }
                for (tag, rule) in rules {
                    sequence = match rule {
                        GuideRuleKind::Collapse => sequence.collapse_tag(tag.as_str()),
                        GuideRuleKind::Merge => sequence.merge_tag(tag.as_str()),
                        _ => sequence,
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
                if let Some((side, slab)) = title {
                    sequence = sequence.slab(side, slab);
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

    /// The rules that the concat at `path` declares: those of the parts of the guides of the
    /// scales that it owns, and of the axis titles and the attributions of the plots below it.
    fn rules(
        &self,
        path: &str,
        layout: Option<&proto::ConcatLayout>,
    ) -> BTreeMap<String, GuideRuleKind> {
        let mut rules = BTreeMap::new();
        for axis in &self.guides.axes {
            if self.groups[axis.group].owner != path {
                continue;
            }
            for (part, kind) in AXIS_RULE_PARTS.into_iter().enumerate() {
                if kind != GuidePart::AxisTitle {
                    rules.insert(axis_tag(axis.group, part), rule(layout, kind));
                }
            }
        }
        for position in 0..self.title_positions() {
            rules.insert(title_tag(position), rule(layout, GuidePart::AxisTitle));
        }
        for legend in &self.guides.legends {
            if legend.owner == path {
                let part = legend.part();
                rules.insert(legend_tag(path, legend.side, part), rule(layout, part));
            }
        }
        rules.insert(
            ATTRIBUTION_TAG.to_string(),
            rule(layout, GuidePart::Attribution),
        );
        rules
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

    /// The slab of the title of the node at `path`, which runs along the whole side of the node.
    fn title_slab(&self, path: &str, ids: &mut Vec<SlabId>) -> Option<(Side, Slab<SlabId>)> {
        let index = *self.guides.title_of.get(path)?;
        let id = SlabId::Title(index);
        ids.push(id.clone());
        Some((
            layout_side(self.guides.titles[index].side),
            Slab::new().id(id).full(),
        ))
    }

    /// The leaf of a plot with its guides: the slabs of its axes nearest the plot, then a stack of
    /// the legends and colorbars of each owner and part on each side that has some, then its
    /// attribution, and outermost its title, which runs the whole side.
    fn plot(&self, plot: usize, ids: &mut Vec<SlabId>) -> Layout<SlabId> {
        let guides = self.guides;
        let plot_guides = &guides.plots[plot];
        let plot_ref = &self.tree.plots[plot];
        let mut leaf = Leaf::new().id(SlabId::Plot(plot));
        if let Some(ratio) = plot_guides.projection.aspect {
            leaf = leaf.aspect(ratio);
        }
        for axis_index in &plot_guides.axes {
            let axis = &guides.axes[*axis_index];
            for (part, kind) in AXIS_RULE_PARTS.into_iter().enumerate() {
                let id = SlabId::Axis {
                    plot,
                    axis: *axis_index,
                    part,
                };
                ids.push(id.clone());
                let (tag, key) = match kind {
                    GuidePart::AxisTitle => (
                        title_tag(self.position(&plot_guides.axes, *axis_index)),
                        axis.title_key.clone(),
                    ),
                    _ => (
                        axis_tag(axis.group, part),
                        format!("axis {axis_index} {part}"),
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
        // of each concat that owns a scale of the plot stand in a stack for each side and part,
        // which follows the concat's rule for the part.
        let mut stacks: Vec<(String, GuideSide, Vec<usize>)> = Vec::new();
        for legend_index in &plot_guides.legends {
            let legend = &guides.legends[*legend_index];
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
            let mut slabs = Vec::new();
            let mut tracks = Vec::new();
            for legend_index in &members {
                let legend = &guides.legends[*legend_index];
                let id = SlabId::Legend {
                    plot,
                    legend: *legend_index,
                };
                ids.push(id.clone());
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
                .tag(tag.as_str())
                .content_key(format!("legends {key}").as_str());
            for (index, track) in tracks.into_iter().enumerate() {
                stack = stack.track(index, track);
            }
            leaf = leaf.slab_stack(layout_side(side), stack);
        }
        if let Some(index) = plot_guides.attribution {
            let attribution = &guides.attributions[index];
            let id = SlabId::Attribution(index);
            ids.push(id.clone());
            leaf = leaf.slab(
                layout_side(attribution.side),
                Slab::new()
                    .id(id)
                    .tag(ATTRIBUTION_TAG)
                    .content_key(attribution.attribution.content_key()),
            );
        }
        if let Some((side, slab)) = self.title_slab(&plot_ref.path, ids) {
            leaf = leaf.slab(side, slab);
        }
        leaf.into()
    }
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
            SlabId::Legend { legend, .. } => Some(guides.legends[*legend].extent(length)),
            SlabId::Title(index) => Some(guides.titles[*index].extent(length)),
            SlabId::Attribution(index) => {
                Some(guides.attributions[*index].attribution.extent(length))
            }
            SlabId::Plot(_) => None,
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
        let SlabId::Legend { legend, .. } = id else {
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
pub(crate) fn draw(guides: &Guides, ids: &[SlabId], solution: &Solution<SlabId>) -> Vec<SceneMark> {
    let mut drawn = HashSet::new();
    let mut marks = Vec::new();
    for id in ids {
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
            SlabId::Legend { legend, .. } => guides.legends[*legend].side,
            SlabId::Title(index) => guides.titles[*index].side,
            SlabId::Attribution(index) => guides.attributions[*index].side,
            SlabId::Plot(_) => continue,
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
            SlabId::Legend { legend, .. } => {
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
            SlabId::Plot(_) => None,
        };
        if let Some(group) = group {
            marks.push(group.into());
        }
    }
    marks
}
