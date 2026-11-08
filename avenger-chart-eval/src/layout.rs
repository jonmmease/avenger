//! The lowering of a chart to an avenger-layout tree, the `Measure` that answers each slab with the
//! extent of its guide, and the drawing of each guide in the slab that represents it.
//!
//! A concat lowers to a `Row` or a `Column` on the tracks of its table, a wrap to a `Wrap` whose
//! children each start a table of their own, an empty child to a `Leaf` that keeps its track and
//! hosts the shared axes of the plots in its lanes, so they stand on the edge of a grid past it, and
//! a plot to a `Leaf` with a slab for each part of each axis nearest the plot, a stack of legends
//! for each owner of its legends, its attribution, and outermost the facets and the title that it
//! holds. Each slab of a guide of a scale has a tag of its scale and its part, and the owner of the
//! scale declares the rule of the tag from its part rules, so the guides of a shared scale
//! collapse or merge. A concat or a wrap whose rule merges a legend holds it beyond its facets,
//! and the owner of a shared `x` or `y` whose plots stand in several tracks matches the lengths of
//! those tracks.
//!
//! A facet is a slab for its labels and a slab for its title on the node that holds it: the concat
//! with the facet, or each child of a wrap with a facet. The facets of a column collapse below the
//! innermost node that holds them all, which also matches the thickness of their labels, and the
//! titles of a wrap's facets merge on the wrap.
//!
//! A guide's computed `offset` widens its slab on the side of what is inside it, so the guide
//! stands that far from it.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
};

use avenger_chart::{
    proto::{self, node::Node, track, Part, PartRuleKind},
    resolve::Resolved,
    table::Place,
    Chart,
};
use avenger_guides::{axis::AxisPart, facet::FacetGuidePart, GuideSide, PartExtent};
use avenger_layout::{
    Column, Direction, Distribute, Edges, Gap, Layout, Leaf, Measure, Measurement, Row, Side, Size,
    Slab, SlabStack, Solution, SolveOptions, Tag, Track, Wrap,
};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use avenger_style::{Computed, ComputedStyle, ComputedStyles, Property};

use crate::{
    geo::GeoPlots,
    guides::{node_path, Guides, LegendGuide, FACET_PARTS},
    polar::Sector,
    style::{
        number as style_number,
        tree::{owner_path, StyleNodes},
    },
};

/// The parts of an axis, from the plot outward, which stand in a slab each.
pub(crate) const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The parts of the format for the axis parts of [`AXIS_PARTS`], in their order.
const AXIS_RULE_PARTS: [Part; 3] = [Part::AxisTicks, Part::AxisLabels, Part::AxisTitle];

/// The parts of the format for the facet parts of [`FACET_PARTS`], in their order.
const FACET_RULE_PARTS: [Part; 2] = [Part::FacetLabels, Part::FacetTitle];

/// The id of a node of a chart's layout: a plot, a concat, a wrap, an empty cell, or a slab of one
/// of the guides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SlabId {
    /// A plot, by its position in `Tree::plots`.
    Plot(usize),
    /// A part of an axis: what hosts it, the axis in `Guides::axes`, and the part in
    /// [`AXIS_PARTS`].
    Axis {
        host: AxisHost,
        axis: usize,
        part: usize,
    },
    /// A legend or a colorbar of a plot, by its position in `Guides::legends`.
    Legend { plot: usize, legend: usize },
    /// A legend or a colorbar that a concat or a wrap holds for the plots below it, because its
    /// rule merges the legend.
    HeldLegend(usize),
    /// A title, by its position in `Guides::titles`.
    Title(usize),
    /// An attribution, by its position in `Guides::attributions`.
    Attribution(usize),
    /// A part of a facet: the facet in `Guides::facets`, and the part in [`FACET_PARTS`].
    Facet { facet: usize, part: usize },
    /// A concat, a wrap, or an empty cell, by its order in the lowering, whose region places the
    /// labels of a facet.
    Node(usize),
    /// The labels of the graticule of a geo plot on one side: the plot, and the position of the
    /// side among the labeled sides of its `GeoGuide`.
    Graticule { plot: usize, side: usize },
}

/// What hosts the slabs of an axis: a plot that shows it, by its position in `Tree::plots`, or an
/// empty cell of a concat, by its number in `SlabId::Node`, in a lane of plots that show it, so a
/// collapsed axis stands at the edge of the lane past the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AxisHost {
    Plot(usize),
    Empty(usize),
}

/// The key of a tag of the layout, which names the slabs that a rule acts on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TagKey {
    /// The ticks or the labels of the axes of a scale group, which draw one scale.
    Axis {
        group: usize,
        part: usize,
    },
    /// The titles of the axes at a position on a side of their plots, from the plot out, which
    /// collapse and merge wherever equal titles line up, whatever their scales.
    AxisTitle {
        position: usize,
    },
    /// The stack of the legends and colorbars of one owner on one side.
    Legends {
        owner: String,
        side: GuideSide,
    },
    /// The stack of the legends of the scales of one plot on one side.
    PlotLegends {
        plot: String,
        side: GuideSide,
    },
    /// A part of the facets of a column, or of a wrap's children.
    Facet {
        name: String,
        part: usize,
    },
    Attribution,
    /// The tracks whose lengths a shared position scale matches, in a direction.
    Track {
        direction: usize,
    },
}

impl fmt::Display for TagKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagKey::Axis { group, part } => write!(f, "axis {group} {part}"),
            TagKey::AxisTitle { position } => write!(f, "axis title {position}"),
            TagKey::Legends { owner, side } => write!(f, "legends {owner} {side:?}"),
            TagKey::PlotLegends { plot, side } => write!(f, "plot legends {plot} {side:?}"),
            TagKey::Facet { name, part } => write!(f, "facet {name} {part}"),
            TagKey::Attribution => f.write_str("attribution"),
            TagKey::Track { direction } => write!(f, "track {direction}"),
        }
    }
}

impl From<TagKey> for Tag {
    fn from(key: TagKey) -> Self {
        Tag::from(key.to_string())
    }
}

fn layout_side(side: GuideSide) -> Side {
    match side {
        GuideSide::Top => Side::Top,
        GuideSide::Bottom => Side::Bottom,
        GuideSide::Left => Side::Left,
        GuideSide::Right => Side::Right,
    }
}

/// The natural length of a discrete position scale whose step is `step` pixels: the step times the
/// length of its range in steps, with its paddings, or `None` for a continuous scale.
pub(crate) fn natural_length(scale: &ConfiguredScale, step: f32) -> Option<f32> {
    use avenger_scales::scales::{band, nested_band::NestedBandLayout, point};
    let config = &scale.config;
    let steps = match scale.scale_impl.scale_type() {
        "band" => band::steps(config),
        "point" => point::steps(config),
        "nested_band" => NestedBandLayout::from_config(config).ok()?.steps(),
        _ => return None,
    };
    (steps > 0.0).then_some(step * steps as f32)
}

/// The rule of a part among the plots of a concat or a wrap: its rule in `rules`, or the
/// default, which collapses the ticks and the labels of an axis and the labels and the title of a
/// facet, and merges the title of an axis, legends, colorbars, and attributions. A wrap merges the
/// titles of its children's facets by default, because each child has a label of its own.
fn rule(rules: &[proto::PartRule], part: Part, wrap: bool) -> PartRuleKind {
    let set = (rules.iter())
        .find(|rule| rule.part == part as i32)
        .and_then(|rule| PartRuleKind::try_from(rule.rule).ok());
    match (set, part) {
        (Some(rule), _) => rule,
        (None, Part::FacetTitle) if wrap => PartRuleKind::Merge,
        (None, Part::AxisTicks | Part::AxisLabels | Part::FacetLabels | Part::FacetTitle) => {
            PartRuleKind::Collapse
        }
        (None, _) => PartRuleKind::Merge,
    }
}

/// What the lowering reads.
pub(crate) struct Lowering<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) guides: &'a Guides,
    pub(crate) geo_plots: &'a GeoPlots,
    pub(crate) nodes: &'a StyleNodes,
    pub(crate) styles: &'a ComputedStyles,
    /// The natural width and height of each plot that the discrete position scales with a `step`
    /// give it, which take the place of its `contain-intrinsic-size` along their directions.
    pub(crate) natural: &'a [[Option<f32>; 2]],
}

/// A lowered chart: its layout, the ids of the slabs of its guides in tree order, and the id of
/// each plot, concat, wrap, and empty cell, by its path.
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
    count: usize,
}

impl Ids {
    fn slab(&mut self, id: SlabId) -> SlabId {
        self.slabs.push(id);
        id
    }

    fn node(&mut self, path: &str) -> SlabId {
        let id = SlabId::Node(self.count);
        self.count += 1;
        self.nodes.insert(path.to_string(), id);
        id
    }
}

/// A slab or a stack that a node hosts beyond its own guides.
enum Hosted {
    Slab(Slab<SlabId>),
    Stack(SlabStack<SlabId>),
}

/// A rule that a concat or a wrap declares for a tag.
enum Rule {
    Kind(PartRuleKind),
    /// A match of the tag.
    Match,
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

/// The node at the path `path` of a plot, a concat, a wrap, or an empty cell, such as
/// `root.concat.children[1].plot`.
fn node_at<'a>(chart: &'a Chart, path: &str) -> Option<&'a proto::Node> {
    let mut node = chart.root();
    let mut steps = path.split('.').skip(1).peekable();
    while let Some(kind) = steps.next() {
        let Some(step) = steps.next() else {
            return Some(node);
        };
        let index: usize = step
            .strip_prefix("children[")?
            .strip_suffix(']')?
            .parse()
            .ok()?;
        node = match (&node.node, kind) {
            (Some(Node::Concat(concat)), "concat") => concat.children.get(index)?,
            (Some(Node::Wrap(wrap)), "wrap") => wrap.children.get(index)?,
            _ => return None,
        };
    }
    Some(node)
}

/// The part rules of the concat or the wrap at `path`, and whether it is a wrap, or `None` for
/// another node.
fn node_rules<'a>(chart: &'a Chart, path: &str) -> Option<(&'a [proto::PartRule], bool)> {
    match &node_at(chart, path)?.node {
        Some(Node::Concat(concat)) => Some((
            (concat.layout.as_ref()).map_or(&[][..], |layout| &layout.part_rules[..]),
            false,
        )),
        Some(Node::Wrap(wrap)) => Some((
            (wrap.layout.as_ref()).map_or(&[][..], |layout| &layout.part_rules[..]),
            true,
        )),
        _ => None,
    }
}

/// The track that a size of the format lowers to.
fn track(size: Option<&track::Size>) -> Track {
    match size {
        Some(track::Size::Length(length)) => Track::px(*length),
        Some(track::Size::Share(proto::Share {
            weight,
            max: Some(max),
        })) => Track::fr_max(weight.unwrap_or(1.0), *max),
        Some(track::Size::Share(share)) => Track::fr(share.weight.unwrap_or(1.0)),
        None => Track::fr(1.0),
    }
}

fn distribute(value: i32) -> Option<Distribute> {
    match proto::Distribute::try_from(value) {
        Ok(proto::Distribute::Start) => Some(Distribute::Start),
        Ok(proto::Distribute::Center) => Some(Distribute::Center),
        Ok(proto::Distribute::End) => Some(Distribute::End),
        Ok(proto::Distribute::SpaceBetween) => Some(Distribute::SpaceBetween),
        Ok(proto::Distribute::SpaceAround) => Some(Distribute::SpaceAround),
        Ok(proto::Distribute::SpaceEvenly) => Some(Distribute::SpaceEvenly),
        _ => None,
    }
}

impl Lowering<'_> {
    /// The path of the node that owns the scale group `group`.
    fn owner(&self, group: usize) -> &str {
        owner_path(&self.nodes.nodes, &self.resolved.scales[group].owner)
    }

    /// The computed style of the `plot` of the node at `path`.
    fn plot_style(&self, path: &str) -> &ComputedStyle {
        &self.styles[self.nodes.nodes[path].plot]
    }

    /// The node in the slot `slot`, which is the root of its table when `table_root` holds.
    fn node(
        &self,
        node: &proto::Node,
        slot: &str,
        table_root: bool,
        ids: &mut Ids,
    ) -> Layout<SlabId> {
        let path = node_path(node, slot);
        match node.node.as_ref().expect("a valid node has a kind") {
            Node::Concat(concat) => self.concat(concat, slot, &path, table_root, ids),
            Node::Wrap(wrap) => self.wrap(wrap, &path, ids),
            Node::Plot(_) => {
                let plot = (self.resolved.tree.plots.iter())
                    .position(|plot| plot.path == path)
                    .expect("the tree has every plot");
                self.plot(plot, ids)
            }
            Node::Empty(_) => {
                let id = ids.node(&path);
                let SlabId::Node(cell) = id else {
                    unreachable!("a node's id is `SlabId::Node`");
                };
                let mut leaf = Leaf::new().id(id);
                let axes = self.empty_axes(slot, &path);
                for (side, slab) in self.axis_slabs(AxisHost::Empty(cell), &axes, ids) {
                    leaf = leaf.slab(side, slab);
                }
                for (side, hosted) in self.host_slabs(&path, ids) {
                    leaf = match hosted {
                        Hosted::Slab(slab) => leaf.slab(side, slab),
                        Hosted::Stack(stack) => leaf.slab_stack(side, stack),
                    };
                }
                leaf.into()
            }
        }
    }

    /// The slabs that the node at `path` hosts beyond its own guides: the facets that it holds,
    /// nearest first, the legends that it merges for the plots below it, and then its title, which
    /// runs along the whole side.
    fn host_slabs(&self, path: &str, ids: &mut Ids) -> Vec<(Side, Hosted)> {
        let mut slabs = Vec::new();
        for (index, facet) in self.guides.facets.iter().enumerate() {
            if facet.host != path {
                continue;
            }
            for (part, facet_part) in FACET_PARTS.into_iter().enumerate() {
                let id = ids.slab(SlabId::Facet { facet: index, part });
                slabs.push((
                    layout_side(facet.side),
                    Hosted::Slab(
                        Slab::new()
                            .id(id)
                            .tag(TagKey::Facet {
                                name: facet.name.clone(),
                                part,
                            })
                            .content_key(facet.guide.content_key(facet_part)),
                    ),
                ));
            }
        }
        let mut sides: Vec<(GuideSide, Vec<usize>)> = Vec::new();
        for (index, legend) in self.guides.legends.iter().enumerate() {
            if self.owner(legend.group) != path || !self.held(index) {
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
        match node_rules(self.chart, self.owner(legend.group)) {
            Some((rules, wrap)) => rule(rules, Part::Legend, wrap) == PartRuleKind::Merge,
            None => false,
        }
    }

    /// A stack of the legends `members` along their side, apart by the largest offset among them,
    /// with the id of each from `id`. A legend takes the length that its entries need, and a
    /// colorbar shares the rest, up to its longest length.
    fn legend_stack(
        &self,
        members: &[usize],
        id: impl Fn(usize) -> SlabId,
        ids: &mut Ids,
    ) -> SlabStack<SlabId> {
        let mut slabs = Vec::new();
        let mut tracks = Vec::new();
        let mut gap: f32 = 0.0;
        for index in members {
            let legend = &self.guides.legends[*index];
            let id = ids.slab(id(*index));
            let mut slab = Slab::new().id(id).content_key(format!("legend {index}"));
            if let Some(length) = legend.max_length {
                slab = slab.max_length(length);
            }
            tracks.push(match legend.guide {
                LegendGuide::Legend(_) => Track::auto(),
                LegendGuide::Colorbar(_) => Track::fr(1.0),
            });
            slabs.push(slab);
            gap = gap.max(legend.offset);
        }
        let key = (members.iter())
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let mut stack = SlabStack::new(slabs)
            .gap(Gap::new(gap))
            .content_key(format!("legends {key}"));
        for (index, track) in tracks.into_iter().enumerate() {
            stack = stack.track(index, track);
        }
        stack
    }

    /// The gap between the children of the composition at `path`.
    fn gap(&self, path: &str) -> Gap {
        let style = self.plot_style(path);
        let row = style_number(style, Property::RowGap);
        let column = style_number(style, Property::ColumnGap);
        Gap::new(row.max(column))
    }

    fn concat(
        &self,
        concat: &proto::Concat,
        slot: &str,
        path: &str,
        table_root: bool,
        ids: &mut Ids,
    ) -> Layout<SlabId> {
        let id = ids.node(path);
        let along = match proto::Direction::try_from(concat.direction) {
            Ok(proto::Direction::Column) => 1,
            _ => 0,
        };
        let children: Vec<Layout<SlabId>> = (concat.children.iter().enumerate())
            .map(|(index, child)| {
                self.node(child, &format!("{path}.children[{index}]"), false, ids)
            })
            .collect();
        let gap = self.gap(path);
        // Each track that the concat spans, with the size that the table gives it and the tag of
        // a match, so every sequence of the table declares it alike.
        let mut tracks = Vec::new();
        if let Some(place) = self.resolved.tables.place(slot) {
            for (offset, index) in place.tracks[along].clone().enumerate() {
                let size = self.resolved.tables.size(place.table, along, index);
                let matched = (self.resolved.matches.tracks).contains(&(place.table, along, index));
                if size.is_none() && !matched {
                    continue;
                }
                let mut track = track(size);
                if matched {
                    track = track.tag(TagKey::Track { direction: along });
                }
                tracks.push((offset, track));
            }
        }
        let layout = concat.layout.as_ref();
        let rules = self.rules(
            path,
            layout.map_or(&[][..], |layout| &layout.part_rules[..]),
            false,
        );
        let matched = self.matched(path);
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
                        Rule::Kind(PartRuleKind::Collapse) => sequence.collapse_tag(tag),
                        Rule::Kind(PartRuleKind::Merge) => sequence.merge_tag(tag),
                        Rule::Match => sequence.match_tag(tag),
                        Rule::Kind(_) => sequence,
                    };
                }
                for tag in matched {
                    sequence = sequence.match_tag(tag);
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
        match along {
            0 => sequence!(Row::new(children)),
            _ => sequence!(Column::new(children)),
        }
    }

    /// A wrap, whose children each start a table of their own in one cell.
    fn wrap(&self, wrap: &proto::Wrap, path: &str, ids: &mut Ids) -> Layout<SlabId> {
        let id = ids.node(path);
        let children: Vec<Layout<SlabId>> = (wrap.children.iter().enumerate())
            .map(|(index, child)| self.node(child, &format!("{path}.children[{index}]"), true, ids))
            .collect();
        let style = self.plot_style(path);
        let mut node = Wrap::new(children)
            .id(id)
            .min_column(style.number(Property::MinColumnWidth).unwrap_or(0.0))
            .gap(self.gap(path));
        if let Computed::Number(columns) = style.get(Property::MaxColumns) {
            node = node.max_columns(*columns as usize);
        }
        let layout = wrap.layout.as_ref();
        if let Some(layout) = layout {
            if let Some(row) = &layout.row {
                node = node.rows(track(row.size.as_ref()));
            }
            if let Some(distribute) = distribute(layout.distribute) {
                node = node.distribute(distribute);
            }
        }
        let rules = self.rules(
            path,
            layout.map_or(&[][..], |layout| &layout.part_rules[..]),
            true,
        );
        for (tag, rule) in rules {
            node = match rule {
                Rule::Kind(PartRuleKind::Collapse) => node.collapse_tag(tag),
                Rule::Kind(PartRuleKind::Merge) => node.merge_tag(tag),
                Rule::Match => node.match_tag(tag),
                Rule::Kind(_) => node,
            };
        }
        for tag in self.matched(path) {
            node = node.match_tag(tag);
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
    fn matched(&self, path: &str) -> Vec<TagKey> {
        (self.resolved.matches.owners.iter())
            .filter(|(owner, _)| owner_path(&self.nodes.nodes, owner) == path)
            .map(|(_, direction)| TagKey::Track {
                direction: *direction,
            })
            .collect()
    }

    /// The rules that the concat or the wrap at `path` declares: those of the parts of the guides
    /// of the scales that it owns and of the facets that it owns, and of the axis titles and the
    /// attributions of the plots below it. It matches the thickness of the labels of the facets
    /// that it owns, so the titles beyond facets with and without a rule line up.
    fn rules(&self, path: &str, rules: &[proto::PartRule], wrap: bool) -> Vec<(TagKey, Rule)> {
        // Each tag with at most one rule of a kind and one match, in the order of their names.
        let mut declared: BTreeMap<(String, bool), (TagKey, Rule)> = BTreeMap::new();
        let mut declare = |tag: TagKey, matched: bool, rule: Rule| {
            declared.insert((tag.to_string(), matched), (tag, rule));
        };
        for axis in &self.guides.axes {
            if self.owner(axis.group) != path {
                continue;
            }
            for (part, kind) in AXIS_RULE_PARTS.into_iter().enumerate() {
                if kind != Part::AxisTitle {
                    declare(
                        TagKey::Axis {
                            group: axis.group,
                            part,
                        },
                        false,
                        Rule::Kind(rule(rules, kind, wrap)),
                    );
                }
            }
        }
        for position in 0..self.title_positions() {
            declare(
                TagKey::AxisTitle { position },
                false,
                Rule::Kind(rule(rules, Part::AxisTitle, wrap)),
            );
        }
        for (index, legend) in self.guides.legends.iter().enumerate() {
            if self.owner(legend.group) == path && !self.held(index) {
                declare(
                    TagKey::Legends {
                        owner: path.to_string(),
                        side: legend.side,
                    },
                    false,
                    Rule::Kind(rule(rules, Part::Legend, wrap)),
                );
            }
        }
        for facet in &self.guides.facets {
            if facet.owner != path {
                continue;
            }
            for (part, kind) in FACET_RULE_PARTS.into_iter().enumerate() {
                let tag = TagKey::Facet {
                    name: facet.name.clone(),
                    part,
                };
                declare(tag, false, Rule::Kind(rule(rules, kind, facet.wrap)));
            }
            let labels = TagKey::Facet {
                name: facet.name.clone(),
                part: 0,
            };
            declare(labels, true, Rule::Match);
        }
        declare(
            TagKey::Attribution,
            false,
            Rule::Kind(rule(rules, Part::Attribution, wrap)),
        );
        declared.into_values().collect()
    }

    /// The most axes on one side of a plot, which is the number of axis title tags.
    fn title_positions(&self) -> usize {
        (self.guides.plots.iter())
            .flat_map(|plot| (plot.axes.iter()).map(|&axis| self.position(&plot.axes, axis) + 1))
            .max()
            .unwrap_or(0)
    }

    /// The position of the axis `axis` among the axes `axes` of a plot on its side, from the plot
    /// out.
    fn position(&self, axes: &[usize], axis: usize) -> usize {
        let side = self.guides.axes[axis].side;
        (axes.iter())
            .take_while(|&&other| other != axis)
            .filter(|&&other| self.guides.axes[other].side == side)
            .count()
    }

    /// The slabs of the parts of the axes `axes` on `host`, with the tags that their owners' rules
    /// act on. An empty cell hosts only the parts that its owner's rule collapses or merges, so a
    /// collapsed part can stand at the edge of its lane past the cell, and a merged part keeps
    /// one run.
    fn axis_slabs(
        &self,
        host: AxisHost,
        axes: &[usize],
        ids: &mut Ids,
    ) -> Vec<(Side, Slab<SlabId>)> {
        let mut slabs = Vec::new();
        for &axis_index in axes {
            let axis = &self.guides.axes[axis_index];
            let owner_rules = node_rules(self.chart, self.owner(axis.group));
            for (part, kind) in AXIS_RULE_PARTS.into_iter().enumerate() {
                if let AxisHost::Empty(_) = host {
                    let shared = owner_rules
                        .is_some_and(|(rules, wrap)| rule(rules, kind, wrap) != PartRuleKind::Keep);
                    if !shared {
                        continue;
                    }
                }
                let id = ids.slab(SlabId::Axis {
                    host,
                    axis: axis_index,
                    part,
                });
                let (tag, key) = match kind {
                    Part::AxisTitle => (
                        TagKey::AxisTitle {
                            position: self.position(axes, axis_index),
                        },
                        format!("axis title {}", axis.title_key),
                    ),
                    _ => (
                        TagKey::Axis {
                            group: axis.group,
                            part,
                        },
                        format!("axis {axis_index} {part} {}", axis.revision),
                    ),
                };
                slabs.push((
                    layout_side(axis.side),
                    Slab::new().id(id).tag(tag).content_key(key),
                ));
            }
        }
        slabs
    }

    /// The axes whose slabs the empty cell at `path`, in the slot `slot`, hosts: the axes of the
    /// scales that a concat or a wrap holding the cell owns, which a plot in the cell's lane across
    /// the axis's side shows: its column of its table for an axis at the top or the bottom, and
    /// its row for an axis at the left or the right.
    fn empty_axes(&self, slot: &str, path: &str) -> Vec<usize> {
        let Some(place) = self.empty_place(slot) else {
            return Vec::new();
        };
        let plots = &self.resolved.tree.plots;
        (0..self.guides.axes.len())
            .filter(|&index| {
                let axis = &self.guides.axes[index];
                if !path.starts_with(&format!("{}.", self.owner(axis.group))) {
                    return false;
                }
                let lane = usize::from(axis.side.is_vertical());
                (plots.iter().enumerate()).any(|(plot, plot_ref)| {
                    self.guides.plots[plot].axes.contains(&index)
                        && (plot_ref.path.strip_suffix(".plot"))
                            .and_then(|slot| self.resolved.tables.place(slot))
                            .is_some_and(|other| {
                                other.table == place.table
                                    && other.tracks[lane] == place.tracks[lane]
                            })
                })
            })
            .collect()
    }

    /// The place in its table of the empty cell in the slot `slot` of a concat, which the tables
    /// leave out: the track after its siblings before it along the concat, and the first track of
    /// the concat across it. `None` for an empty cell of a wrap, whose children each start a table.
    fn empty_place(&self, slot: &str) -> Option<Place> {
        let (parent, index) = slot.rsplit_once(".children[")?;
        let index: usize = index.strip_suffix(']')?.parse().ok()?;
        let Some(Node::Concat(concat)) = &node_at(self.chart, parent)?.node else {
            return None;
        };
        let concat_place = self
            .resolved
            .tables
            .place(parent.strip_suffix(".concat")?)?;
        let along = match proto::Direction::try_from(concat.direction) {
            Ok(proto::Direction::Column) => 1,
            _ => 0,
        };
        let mut start = concat_place.tracks[along].start;
        for sibling in 0..index {
            start = match self
                .resolved
                .tables
                .place(&format!("{parent}.children[{sibling}]"))
            {
                Some(place) => place.tracks[along].end,
                None => start + 1,
            };
        }
        let mut tracks = concat_place.tracks.clone();
        tracks[along] = start..start + 1;
        tracks[1 - along] = tracks[1 - along].start..tracks[1 - along].start + 1;
        Some(Place {
            table: concat_place.table,
            tracks,
        })
    }

    /// The leaf of a plot with its guides: the slabs of its axes nearest the plot, then a stack of
    /// the legends and colorbars of each owner and part on each side that has some, then its
    /// attribution, and the facets and the title that it holds, outermost.
    fn plot(&self, plot: usize, ids: &mut Ids) -> Layout<SlabId> {
        let guides = self.guides;
        let plot_guides = &guides.plots[plot];
        let plot_ref = &self.resolved.tree.plots[plot];
        let id = SlabId::Plot(plot);
        ids.nodes.insert(plot_ref.path.clone(), id);
        let style = self.plot_style(&plot_ref.path);
        let mut leaf = Leaf::new().id(id);
        if let Some(width) = style.number(Property::Width) {
            leaf = leaf.min_width(width).max_width(width);
        }
        if let Some(height) = style.number(Property::Height) {
            leaf = leaf.min_height(height).max_height(height);
        }
        // A polar plot keeps the shape of the box around its part of the circle, and a geo plot
        // the aspect of its projection or none.
        let geo = self.geo_plots.guide(plot);
        match (
            Sector::of_plot(plot_ref.plot),
            geo,
            style.get(Property::AspectRatio),
        ) {
            (Some(sector), _, _) => leaf = leaf.aspect(sector.aspect()),
            (None, Some(geo), _) => {
                if let Some(aspect) = geo.aspect {
                    leaf = leaf.aspect(aspect);
                }
            }
            (None, None, Computed::Ratio(ratio)) => leaf = leaf.aspect(*ratio),
            (None, None, _) => {}
        }
        for (side, (guide_side, _)) in geo.into_iter().flat_map(|geo| &geo.labels).enumerate() {
            let id = ids.slab(SlabId::Graticule { plot, side });
            leaf = leaf.slab(layout_side(*guide_side), Slab::new().id(id));
        }
        // The computed `contain-intrinsic-size` is the size that the plot counts at in a chart that
        // takes its natural size, which a host's size or the chart's size stretches.
        let natural = self.natural[plot];
        let intrinsic = [
            natural[0].or(style.number(Property::ContainIntrinsicWidth)),
            natural[1].or(style.number(Property::ContainIntrinsicHeight)),
        ];
        if intrinsic.iter().any(Option::is_some) {
            let [width, height] = intrinsic.map(|size| size.unwrap_or(0.0));
            leaf = leaf.max_content(Size::new(width, height));
        }
        for (side, slab) in self.axis_slabs(AxisHost::Plot(plot), &plot_guides.axes, ids) {
            leaf = leaf.slab(side, slab);
        }
        // The legends of the scales that the plot owns stand in one stack on each side, and those
        // of each node that owns a scale of the plot and keeps or collapses its legends stand in a
        // stack for each side and part, which follows the node's rule for the part.
        let mut stacks: Vec<(TagKey, GuideSide, Vec<usize>)> = Vec::new();
        for &legend_index in &plot_guides.legends {
            if self.held(legend_index) {
                continue;
            }
            let legend = &guides.legends[legend_index];
            let owner = self.owner(legend.group);
            let tag = match owner == plot_ref.path {
                true => TagKey::PlotLegends {
                    plot: plot_ref.path.clone(),
                    side: legend.side,
                },
                false => TagKey::Legends {
                    owner: owner.to_string(),
                    side: legend.side,
                },
            };
            match stacks.iter_mut().find(|(known, _, _)| *known == tag) {
                Some((_, _, members)) => members.push(legend_index),
                None => stacks.push((tag, legend.side, vec![legend_index])),
            }
        }
        // The legends run along the space of the plot, so beside a plot that keeps its aspect they
        // may pass its short side rather than squeeze and widen as the plot shrinks.
        for (tag, side, members) in stacks {
            let stack = self
                .legend_stack(&members, |legend| SlabId::Legend { plot, legend }, ids)
                .tag(tag)
                .space();
            leaf = leaf.slab_stack(layout_side(side), stack);
        }
        if let Some(index) = plot_guides.attribution {
            let attribution = &guides.attributions[index];
            let id = ids.slab(SlabId::Attribution(index));
            leaf = leaf.slab(
                layout_side(attribution.side),
                Slab::new()
                    .id(id)
                    .tag(TagKey::Attribution)
                    .content_key(attribution.attribution.content_key()),
            );
        }
        for (side, hosted) in self.host_slabs(&plot_ref.path, ids) {
            leaf = match hosted {
                Hosted::Slab(slab) => leaf.slab(side, slab),
                Hosted::Stack(stack) => leaf.slab_stack(side, stack),
            };
        }
        leaf.into()
    }
}

/// The options of the solve: the size of each dimension that the host or the chart gives, and the
/// max-content size of the others, inside the chart's computed padding.
pub(crate) fn solve_options(size: [Option<f32>; 2], chart: &ComputedStyle) -> SolveOptions {
    let padding = |property| style_number(chart, property);
    SolveOptions {
        width: size[0],
        height: size[1],
        padding: Edges {
            top: padding(Property::PaddingTop),
            right: padding(Property::PaddingRight),
            bottom: padding(Property::PaddingBottom),
            left: padding(Property::PaddingLeft),
        },
        ..SolveOptions::default()
    }
}

/// The side of the guide of a slab, and its offset from what stands inside it.
fn placement(guides: &Guides, id: &SlabId) -> Option<(GuideSide, f32)> {
    Some(match id {
        SlabId::Axis { axis, part, .. } => {
            let axis = &guides.axes[*axis];
            (axis.side, if *part == 0 { axis.offset } else { 0.0 })
        }
        SlabId::Legend { legend, .. } | SlabId::HeldLegend(legend) => {
            let legend = &guides.legends[*legend];
            (legend.side, legend.offset)
        }
        SlabId::Title(index) => {
            let title = &guides.titles[*index];
            (title.side, title.offset)
        }
        SlabId::Attribution(index) => {
            let attribution = &guides.attributions[*index];
            (attribution.side, attribution.offset)
        }
        SlabId::Facet { facet, .. } => (guides.facets[*facet].side, 0.0),
        SlabId::Plot(_) | SlabId::Node(_) | SlabId::Graticule { .. } => return None,
    })
}

/// The answers of a chart's guides to the layout.
pub(crate) struct Measurer<'a> {
    pub(crate) guides: &'a Guides,
    pub(crate) geo_plots: &'a GeoPlots,
}

impl Measurer<'_> {
    fn extent(&self, id: &SlabId, length: f32) -> Option<PartExtent> {
        let guides = self.guides;
        // The labels of a graticule take the thickness that the graticule measures at any length.
        if let SlabId::Graticule { plot, side } = id {
            return Some(self.geo_plots.guide(*plot)?.labels[*side].1.clone());
        }
        let extent = match id {
            SlabId::Axis { axis, part, .. } => {
                guides.axes[*axis].axis.extent(AXIS_PARTS[*part], length)
            }
            SlabId::Legend { legend, .. } | SlabId::HeldLegend(legend) => {
                match &guides.legends[*legend].guide {
                    LegendGuide::Legend(legend) => legend.extent(length),
                    LegendGuide::Colorbar(colorbar) => colorbar.extent(length),
                }
            }
            SlabId::Title(index) => guides.titles[*index].title.extent(length),
            SlabId::Attribution(index) => guides.attributions[*index].attribution.extent(length),
            SlabId::Facet { facet, part } => guides.facets[*facet]
                .guide
                .extent(FACET_PARTS[*part], length),
            SlabId::Plot(_) | SlabId::Node(_) | SlabId::Graticule { .. } => return None,
        };
        let (_, offset) = placement(guides, id)?;
        Some(PartExtent {
            thickness: extent.thickness + offset,
            ..extent
        })
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
        let LegendGuide::Legend(legend) = &slot.guide else {
            return None;
        };
        let along = match slot.side.is_vertical() {
            true => Direction::Vertical,
            false => Direction::Horizontal,
        };
        (direction == along).then(|| legend.max_content())
    }
}

/// `slot` without the offset on its side nearest what stands inside it.
fn past_offset(slot: [f32; 4], side: GuideSide, offset: f32) -> [f32; 4] {
    let [x, y, width, height] = slot;
    match side {
        GuideSide::Right => [x + offset, y, width - offset, height],
        GuideSide::Left => [x, y, width - offset, height],
        GuideSide::Bottom => [x, y + offset, width, height - offset],
        GuideSide::Top => [x, y, width, height - offset],
    }
}

/// The guides of a chart, each drawn once, in the slab that represents its slabs.
pub(crate) fn draw(
    guides: &Guides,
    lowered: &Lowered,
    solution: &Solution<SlabId>,
) -> Vec<SceneMark> {
    let mut drawn = HashSet::new();
    let mut marks = Vec::new();
    for id in &lowered.slabs {
        let (Some(region), Some((side, offset))) =
            (solution.representative(id), placement(guides, id))
        else {
            continue;
        };
        if !drawn.insert(region.path.clone()) {
            continue;
        }
        let rect = region.content;
        let slot = past_offset([rect.x, rect.y, rect.width, rect.height], side, offset);
        let length = match side.is_vertical() {
            true => slot[3],
            false => slot[2],
        };
        let group = match id {
            SlabId::Axis { axis, part, .. } => {
                let axis = &guides.axes[*axis].axis;
                (axis.part(AXIS_PARTS[*part], length)).map(|group| SceneGroup {
                    origin: axis.origin(slot),
                    ..group
                })
            }
            SlabId::Legend { legend, .. } | SlabId::HeldLegend(legend) => {
                Some(match &guides.legends[*legend].guide {
                    LegendGuide::Legend(legend) => SceneGroup {
                        origin: legend.origin(slot),
                        ..legend.marks(length).group
                    },
                    LegendGuide::Colorbar(colorbar) => SceneGroup {
                        origin: colorbar.origin(slot),
                        ..colorbar.marks(length).group
                    },
                })
            }
            SlabId::Title(index) => {
                let title = &guides.titles[*index].title;
                (title.part(length)).map(|group| SceneGroup {
                    origin: title.origin(slot),
                    ..group
                })
            }
            SlabId::Attribution(index) => {
                let attribution = &guides.attributions[*index].attribution;
                (attribution.part(length)).map(|group| SceneGroup {
                    origin: attribution.origin(slot),
                    ..group
                })
            }
            SlabId::Facet { facet, part } => {
                // The slab that draws the labels labels the children of its own facet, which line
                // up with those of the facets that it stands for.
                let facet = match &region.id {
                    Some(SlabId::Facet { facet, .. }) => *facet,
                    _ => *facet,
                };
                let facet_slot = &guides.facets[facet];
                let guide = &facet_slot.guide;
                let group = match FACET_PARTS[*part] {
                    FacetGuidePart::Labels => {
                        let bands: Option<Vec<[f32; 2]>> = (facet_slot.children.iter())
                            .map(|child| {
                                let child = solution.region(lowered.nodes.get(child)?)?.content;
                                Some(match side.is_vertical() {
                                    true => [child.y - rect.y, child.y + child.height - rect.y],
                                    false => [child.x - rect.x, child.x + child.width - rect.x],
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
        marks.extend(group.map(SceneMark::from));
    }
    marks
}
