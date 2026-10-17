//! Lowering a layout into an indexed tree, and the per-direction plans that the solver runs.
//!
//! The tree lists nodes in drawing order: each node, then its slabs and stacks, each stack
//! followed by its members, then its children, then its overlays. Overlay nodes start their own roots, which are solved after their host.
//!
//! Rows and columns lower to grids. A sequence that joins its parent's table becomes a subgrid
//! in both directions, and the outermost sequence of a table owns the table's tracks, with the
//! sizes that the table's sequences declare.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::geometry::{Direction, Side, Size};
use crate::measure::AnswerKey;
use crate::node::{
    ContentKey, Distribute, Extent, Gap, GridSlot, Layout, Overflow, Placement, Rules, Sequence,
    SideEntry, Slab, SlabStack, Tag, Track, TrackSize,
};
use crate::solution::PathStep;
use crate::validate::LayoutError;

/// One node of the lowered tree.
#[derive(Clone, Debug)]
pub(crate) struct Node<Id> {
    pub id: Option<Id>,
    pub path: Vec<PathStep>,
    /// The container that holds a child, or the host of a slab. `None` for a root.
    pub parent: Option<usize>,
    /// The root of the node's tree: the layout's root, or an overlay's.
    pub root: usize,
    /// The slot of a container's child, in the container's own tracks.
    pub slot: Option<GridSlot>,
    /// The span that the node declares along a parent sequence.
    pub span: Option<usize>,
    pub content_key: Option<ContentKey>,
    pub kind: NodeKind,
    pub min: Size,
    pub max: Size,
    /// The declared max-content size of a leaf or slab.
    pub max_content: Option<Size>,
    /// By [`Side::index`].
    pub overflow: [Overflow; 4],
    /// By [`Side::index`].
    pub padding: [f32; 4],
    /// By [`Direction::index`].
    pub place: [Placement; 2],
    /// Band nodes per side, by [`Side::index`], innermost first: slabs, and stacks, whose
    /// members follow them in the tree.
    pub slabs: [Vec<usize>; 4],
    /// For a slab that another slab draws, the slab that draws it: the kept slab of its run or
    /// a merged slab. Such a slab takes no space, and is measured only on a Wrap's child, which
    /// another count can draw.
    pub represented_by: Option<usize>,
    /// Whether the slab's id moved to the merged slab that draws it, which leaves it without a
    /// region.
    pub moved: bool,
}

/// The nodes of one band: a slab's node, or a stack's node followed by its members' nodes.
pub(crate) type BandNodes<Id> = Vec<Node<Id>>;

/// Merged slabs to insert while lowering, and the declared slabs whose ids move to them.
#[derive(Clone, Debug)]
pub(crate) struct Extras<Id> {
    /// By container path: for each side by [`Side::index`], the merged slabs and stacks
    /// innermost first, as copies of a member's nodes that carry the merged one's ids.
    pub merged: HashMap<Vec<PathStep>, [Vec<BandNodes<Id>>; 4]>,
    pub moved: HashSet<Vec<PathStep>>,
}

impl<Id> Default for Extras<Id> {
    fn default() -> Self {
        Self {
            merged: HashMap::new(),
            moved: HashSet::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum NodeKind {
    Leaf {
        aspect: Option<f32>,
    },
    Grid(Box<GridNode>),
    /// A slab, or a member of the stack `stack`. A member's `extent` is what it declares, which
    /// validation keeps at [`Extent::Content`]; its stack's extent applies.
    Slab {
        host: usize,
        side: Side,
        extent: Extent,
        tag: Option<Tag>,
        stack: Option<usize>,
    },
    Stack(Box<StackNode>),
}

/// A stack of slabs on a side of `host`, which share one band.
#[derive(Clone, Debug)]
pub(crate) struct StackNode {
    pub host: usize,
    pub side: Side,
    pub extent: Extent,
    pub tag: Option<Tag>,
    /// What the stack declares for its members' tracks, by index, and for every other member.
    pub declared: Vec<(usize, Track)>,
    pub all: Option<Track>,
    /// Each member's track size.
    pub tracks: Vec<TrackSize>,
    pub gap: Gap,
    pub distribute: Distribute,
    /// The member slab nodes, from the side's start.
    pub members: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct GridNode {
    /// Columns, then rows. A subgrid holds its owner's tracks over its range.
    pub tracks: [Vec<Track>; 2],
    pub children: Vec<usize>,
    pub gaps: [Option<Gap>; 2],
    pub distribute: [Distribute; 2],
    pub rules: Rules,
    pub kind: GridKind,
}

/// What kind of container a grid node is, with the settings of that kind.
#[derive(Clone, Debug)]
pub(crate) enum GridKind {
    Grid { fit_aspect: bool },
    Sequence(SequenceNode),
    Wrap(WrapNode),
}

/// The settings of a Wrap, and the column count that its tracks and its children's slots are
/// laid out at.
#[derive(Clone, Debug)]
pub(crate) struct WrapNode {
    pub min_column: f32,
    pub max_columns: Option<usize>,
    pub rows: Track,
    pub flow: bool,
    pub arrangement: Arrangement,
}

/// Where a Wrap places its children: in equal columns, or for a flow, each child on its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Arrangement {
    Columns(usize),
    /// The line of each child.
    Lines(Vec<usize>),
}

/// What a Row or Column declares, for building its table and checking it.
#[derive(Clone, Debug)]
pub(crate) struct SequenceNode {
    pub direction: Direction,
    pub share: bool,
    /// Whether the sequence joins its parent's table.
    pub joins: bool,
    pub tracks: Vec<(usize, Track)>,
    pub all: Option<Track>,
    pub distribute: [Option<Distribute>; 2],
}

impl SequenceNode {
    /// The track that the sequence declares at `index` in its direction, if any.
    fn declared(&self, index: usize) -> Option<&Track> {
        self.tracks
            .iter()
            .rev()
            .find(|(declared, _)| *declared == index)
            .map(|(_, track)| track)
            .or(self.all.as_ref())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OverlayNode {
    pub host: usize,
    /// Left, top, right and bottom fractions of the host's content.
    pub rect: [f32; 4],
    pub root: usize,
}

/// A layout as an indexed tree.
#[derive(Clone, Debug)]
pub(crate) struct Tree<Id> {
    pub nodes: Vec<Node<Id>>,
    pub overlays: Vec<OverlayNode>,
    /// The layout's root, then each overlay's root after its host's root.
    pub roots: Vec<usize>,
    /// The first track that two sequences of one table declare differently, found while
    /// lowering.
    pub conflict: Option<LayoutError>,
    /// A sequence whose children need more tracks than a table holds, found before its tracks
    /// are made. Lowering gives no sequence tracks then, so it comes before every other check.
    pub oversized: Option<LayoutError>,
}

/// The most tracks that a sequence's table holds in one direction.
const MAX_TRACKS: usize = 1 << 20;

impl<Id: Clone> Tree<Id> {
    pub fn lower(layout: &Layout<Id>, extras: &Extras<Id>) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            overlays: Vec::new(),
            roots: vec![0],
            conflict: None,
            oversized: None,
        };
        tree.lower_node(layout, extras, None, None, None, Vec::new());
        tree.build_tables();
        tree.lay_out_wraps(&[]);
        tree
    }

    /// Lays out each Wrap at its arrangement in `arrangements`, by Wrap in tree order, or at
    /// the one it was lowered with when `arrangements` leaves it out: its tracks, its children's
    /// slots, and a flushed bottom on each child above an empty cell. A flow's children each
    /// take a column of their own, so that nothing lines up across lines.
    pub fn lay_out_wraps(&mut self, arrangements: &[Arrangement]) {
        for (position, wrap) in self.wraps().into_iter().enumerate() {
            let NodeKind::Grid(grid) = &mut self.nodes[wrap].kind else {
                unreachable!("wraps are grids");
            };
            let GridKind::Wrap(settings) = &mut grid.kind else {
                unreachable!("wraps have settings");
            };
            if let Some(arrangement) = arrangements.get(position) {
                settings.arrangement = arrangement.clone();
            }
            let children = grid.children.clone();
            let arrangement = settings.arrangement.clone();
            match &arrangement {
                &Arrangement::Columns(count) => {
                    let rows = children.len().div_ceil(count).max(1);
                    grid.tracks = [
                        vec![Track::fr(1.0); count],
                        vec![settings.rows.clone(); rows],
                    ];
                    for (index, &child) in children.iter().enumerate() {
                        self.nodes[child].slot = Some(GridSlot::new(index % count, index / count));
                        if index + count >= children.len() && index / count + 1 < rows {
                            self.nodes[child].overflow[Side::Bottom.index()] = Overflow::Flush;
                        }
                    }
                }
                Arrangement::Lines(lines) => {
                    let rows = lines.iter().max().map_or(1, |&last| last + 1);
                    grid.tracks = [
                        vec![Track::fr(1.0); children.len().max(1)],
                        vec![settings.rows.clone(); rows],
                    ];
                    for (index, &child) in children.iter().enumerate() {
                        self.nodes[child].slot = Some(GridSlot::new(index, lines[index]));
                    }
                }
            }
        }
    }

    /// The arrangement that each Wrap was lowered with, by Wrap in tree order.
    pub fn arrangements(&self) -> Vec<Arrangement> {
        self.wraps()
            .into_iter()
            .filter_map(|wrap| self.wrap(wrap).map(|settings| settings.arrangement.clone()))
            .collect()
    }

    fn lower_node(
        &mut self,
        layout: &Layout<Id>,
        extras: &Extras<Id>,
        parent: Option<usize>,
        root: Option<usize>,
        slot: Option<GridSlot>,
        path: Vec<PathStep>,
    ) -> usize {
        let index = self.nodes.len();
        let root = root.unwrap_or(index);
        let common = layout.common();
        let (content_key, max_content) = match layout {
            Layout::Leaf(leaf) => (leaf.content_key.clone(), leaf.max_content),
            _ => (None, None),
        };
        // A Wrap and a Flow start at one column, so that the width pass measures every child
        // as one column, whatever the arrangement that it then takes.
        let wrap_node = |flow, min_column, max_columns, rows: &Track| WrapNode {
            min_column,
            max_columns,
            rows: rows.clone(),
            flow,
            arrangement: Arrangement::Columns(1),
        };
        let kind = match layout {
            Layout::Leaf(leaf) => NodeKind::Leaf {
                aspect: leaf.aspect,
            },
            Layout::Row(row) => sequence_kind(&row.sequence, Direction::Horizontal),
            Layout::Column(column) => sequence_kind(&column.sequence, Direction::Vertical),
            Layout::Grid(grid) => NodeKind::Grid(Box::new(GridNode {
                tracks: grid.tracks.clone(),
                children: Vec::new(),
                gaps: grid.gaps.clone(),
                distribute: grid.distribute,
                rules: grid.rules.clone(),
                kind: GridKind::Grid {
                    fit_aspect: grid.fit_aspect,
                },
            })),
            Layout::Wrap(wrap) => NodeKind::Grid(Box::new(GridNode {
                tracks: [Vec::new(), Vec::new()],
                children: Vec::new(),
                gaps: [wrap.gap.clone(), wrap.gap.clone()],
                distribute: [Distribute::Stretch, wrap.distribute],
                rules: wrap.rules.clone(),
                kind: GridKind::Wrap(wrap_node(
                    false,
                    wrap.min_column,
                    wrap.max_columns,
                    &wrap.rows,
                )),
            })),
            Layout::Flow(flow) => NodeKind::Grid(Box::new(GridNode {
                tracks: [Vec::new(), Vec::new()],
                children: Vec::new(),
                gaps: [flow.gap.clone(), flow.gap.clone()],
                distribute: flow.distribute,
                rules: Rules {
                    matches: flow.matches.clone(),
                    ..Rules::default()
                },
                kind: GridKind::Wrap(wrap_node(true, 0.0, None, &flow.rows)),
            })),
        };
        self.nodes.push(Node {
            id: common.id.clone(),
            path: path.clone(),
            parent,
            root,
            slot,
            span: common.span,
            content_key,
            kind,
            min: common.min,
            max: common.max,
            max_content,
            overflow: common.overflow,
            padding: common.padding,
            place: common.place,
            slabs: Default::default(),
            represented_by: None,
            moved: false,
        });

        for side in Side::ALL {
            let merged = extras
                .merged
                .get(&path)
                .map_or(&[][..], |m| &m[side.index()][..]);
            for (position, template) in merged.iter().enumerate() {
                let band = self.nodes.len();
                let mut band_path = path.clone();
                band_path.push(PathStep::Merged(side, position));
                let (first, members) = template.split_first().expect("a merged band has a node");
                let mut node = first.clone();
                node.path = band_path.clone();
                node.parent = Some(index);
                node.root = root;
                match &mut node.kind {
                    NodeKind::Slab { host, .. } => *host = index,
                    NodeKind::Stack(stack) => {
                        stack.host = index;
                        stack.members = (band + 1..band + 1 + members.len()).collect();
                    }
                    _ => {}
                }
                self.nodes.push(node);
                for (member, template) in members.iter().enumerate() {
                    let mut node = template.clone();
                    node.path = band_path.clone();
                    node.path.push(PathStep::Member(member));
                    node.parent = Some(band);
                    node.root = root;
                    if let NodeKind::Slab { host, stack, .. } = &mut node.kind {
                        *host = index;
                        *stack = Some(band);
                    }
                    self.nodes.push(node);
                }
                self.nodes[index].slabs[side.index()].push(band);
            }
            for (position, entry) in common.slabs[side.index()].iter().enumerate() {
                let mut entry_path = path.clone();
                entry_path.push(PathStep::Slab(side, position));
                let moved = extras.moved.contains(&entry_path);
                let entry_index = self.nodes.len();
                match entry {
                    SideEntry::Slab(slab) => {
                        let node = slab_node(slab, side, index, root, entry_path, moved, None);
                        self.nodes.push(node);
                    }
                    SideEntry::Stack(stack) => {
                        let node = stack_node(stack, side, index, root, entry_path.clone(), moved);
                        self.nodes.push(node);
                        for (member, slab) in stack.slabs.iter().enumerate() {
                            let mut member_path = entry_path.clone();
                            member_path.push(PathStep::Member(member));
                            let member_index = self.nodes.len();
                            let mut node = slab_node(
                                slab,
                                side,
                                index,
                                root,
                                member_path,
                                moved,
                                Some(entry_index),
                            );
                            node.parent = Some(entry_index);
                            self.nodes.push(node);
                            if let NodeKind::Stack(stack) = &mut self.nodes[entry_index].kind {
                                stack.members.push(member_index);
                            }
                        }
                    }
                }
                self.nodes[index].slabs[side.index()].push(entry_index);
            }
        }

        let children: Vec<(Option<GridSlot>, &Layout<Id>)> = match layout {
            Layout::Leaf(_) => Vec::new(),
            Layout::Row(row) => row.sequence.children.iter().map(|c| (None, c)).collect(),
            Layout::Column(column) => column.sequence.children.iter().map(|c| (None, c)).collect(),
            Layout::Grid(grid) => grid.children.iter().map(|(s, c)| (Some(*s), c)).collect(),
            // `lay_out_wraps` gives a Wrap's children their slots.
            Layout::Wrap(wrap) => wrap.children.iter().map(|c| (None, c)).collect(),
            Layout::Flow(flow) => flow.children.iter().map(|c| (None, c)).collect(),
        };
        for (position, (child_slot, child)) in children.into_iter().enumerate() {
            let mut child_path = path.clone();
            child_path.push(PathStep::Child(position));
            let child_index = self.lower_node(
                child,
                extras,
                Some(index),
                Some(root),
                child_slot,
                child_path,
            );
            if let NodeKind::Grid(node) = &mut self.nodes[index].kind {
                node.children.push(child_index);
            }
        }

        for (position, overlay) in common.overlays.iter().enumerate() {
            let overlay_root = self.nodes.len();
            self.roots.push(overlay_root);
            self.overlays.push(OverlayNode {
                host: index,
                rect: overlay.rect,
                root: overlay_root,
            });
            let mut overlay_path = path.clone();
            overlay_path.push(PathStep::Overlay(position));
            self.lower_node(&overlay.node, extras, None, None, None, overlay_path);
        }
        index
    }
}

/// The node of `slab` on `side` of `host`, or of a member of the stack node `stack`. A moved
/// slab's id is with the merged slab that draws it.
fn slab_node<Id: Clone>(
    slab: &Slab<Id>,
    side: Side,
    host: usize,
    root: usize,
    path: Vec<PathStep>,
    moved: bool,
    stack: Option<usize>,
) -> Node<Id> {
    let along = side.direction().cross();
    let mut place = [Placement::default(); 2];
    place[along.index()] = slab.place;
    // A slab's maximum and max-content size run along its side only.
    let along_side = |length: f32, across: f32| match along {
        Direction::Horizontal => Size::new(length, across),
        Direction::Vertical => Size::new(across, length),
    };
    Node {
        id: if moved { None } else { slab.id.clone() },
        path,
        parent: Some(host),
        root,
        slot: None,
        span: None,
        content_key: slab.content_key.clone(),
        kind: NodeKind::Slab {
            host,
            side,
            extent: slab.extent,
            tag: slab.tag.clone(),
            stack,
        },
        min: slab.min,
        max: along_side(slab.max_length, f32::INFINITY),
        max_content: slab.max_content.map(|length| along_side(length, 0.0)),
        overflow: [Overflow::Propagate; 4],
        padding: [0.0; 4],
        place,
        slabs: Default::default(),
        represented_by: None,
        moved,
    }
}

/// The node of `stack` on `side` of `host`, without its members, which follow it. A stack's
/// minimum length holds its members' minimum lengths and its gaps' minimums.
fn stack_node<Id: Clone>(
    stack: &SlabStack<Id>,
    side: Side,
    host: usize,
    root: usize,
    path: Vec<PathStep>,
    moved: bool,
) -> Node<Id> {
    let declared = |member: usize| {
        stack
            .tracks
            .iter()
            .rev()
            .find(|(declared, _)| *declared == member)
            .map(|(_, track)| track)
            .or(stack.all.as_ref())
            .map_or(Track::fr(1.0).size, |track| track.size)
    };
    let gap = stack.gap.clone().unwrap_or_default();
    let along = side.direction().cross();
    let gaps = gap.min * stack.slabs.len().saturating_sub(1) as f32;
    let length = stack
        .slabs
        .iter()
        .map(|slab| slab.min.along(along))
        .sum::<f32>()
        + gaps;
    let min = match along {
        Direction::Horizontal => Size::new(length, 0.0),
        Direction::Vertical => Size::new(0.0, length),
    };
    Node {
        id: if moved { None } else { stack.id.clone() },
        path,
        parent: Some(host),
        root,
        slot: None,
        span: None,
        content_key: stack.content_key.clone(),
        kind: NodeKind::Stack(Box::new(StackNode {
            host,
            side,
            extent: stack.extent,
            tag: stack.tag.clone(),
            declared: stack.tracks.clone(),
            all: stack.all.clone(),
            tracks: (0..stack.slabs.len()).map(declared).collect(),
            gap,
            distribute: stack.distribute,
            members: Vec::new(),
        })),
        min,
        max: Size::new(f32::INFINITY, f32::INFINITY),
        max_content: None,
        overflow: [Overflow::Propagate; 4],
        padding: [0.0; 4],
        place: [Placement::default(); 2],
        slabs: Default::default(),
        represented_by: None,
        moved,
    }
}

fn sequence_kind<Id>(sequence: &Sequence<Id>, direction: Direction) -> NodeKind {
    let mut gaps = [None, None];
    gaps[direction.index()] = sequence.gap.clone();
    NodeKind::Grid(Box::new(GridNode {
        tracks: [Vec::new(), Vec::new()],
        children: Vec::new(),
        gaps,
        // Only a table's outermost sequence allocates, and validation keeps the others at the
        // default.
        distribute: sequence.distribute.map(Option::unwrap_or_default),
        rules: sequence.rules.clone(),
        kind: GridKind::Sequence(SequenceNode {
            direction,
            share: sequence.share,
            joins: false,
            tracks: sequence.tracks.clone(),
            all: sequence.all.clone(),
            distribute: sequence.distribute,
        }),
    }))
}

impl<Id> Tree<Id> {
    /// The Wraps of the tree, in tree order.
    pub fn wraps(&self) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&node| self.wrap(node).is_some())
            .collect()
    }

    /// The settings of a Wrap.
    pub fn wrap(&self, node: usize) -> Option<&WrapNode> {
        match &self.grid(node)?.kind {
            GridKind::Wrap(wrap) => Some(wrap),
            _ => None,
        }
    }

    /// The settings of a Row or Column.
    pub fn sequence(&self, node: usize) -> Option<&SequenceNode> {
        match &self.grid(node)?.kind {
            GridKind::Sequence(sequence) => Some(sequence),
            _ => None,
        }
    }

    /// Whether `node` is a sequence that joins its parent's table, as a subgrid in both
    /// directions.
    pub fn joins(&self, node: usize) -> bool {
        self.sequence(node).is_some_and(|sequence| sequence.joins)
    }

    /// Gives every Row and Column its tracks and its children their slots, by the table rules.
    fn build_tables(&mut self) {
        let count = self.nodes.len();
        for node in 0..count {
            // A sequence joins when its parent is a sequence that shares, and it shares too.
            let joins = self.nodes[node]
                .parent
                .is_some_and(|parent| self.sequence(parent).is_some_and(|parent| parent.share))
                && self.sequence(node).is_some_and(|sequence| sequence.share);
            if let NodeKind::Grid(grid) = &mut self.nodes[node].kind
                && let GridKind::Sequence(sequence) = &mut grid.kind
            {
                sequence.joins = joins;
            }
        }
        // Extents from the leaves up: the children of a node come after it.
        let mut extent = vec![[1usize; 2]; count];
        for node in (0..count).rev() {
            let Some(sequence) = self.sequence(node) else {
                continue;
            };
            let d = sequence.direction.index();
            let mut size = [0usize; 2];
            size[1 - d] = 1;
            for &child in &self.grid(node).expect("sequences are grids").children {
                let (along, across) = self.child_extent(child, d, &extent);
                size[d] = size[d].saturating_add(along);
                size[1 - d] = size[1 - d].max(across);
            }
            size[d] = size[d].max(1);
            // A span on a sequence that joins its parent widens its own tracks along the
            // parent, so that its cells fill them.
            if let (Some(span), Some(parent)) = (self.nodes[node].span, self.nodes[node].parent)
                && let Some(parent) = self.sequence(parent)
                && sequence.joins
            {
                let along = parent.direction.index();
                size[along] = size[along].max(span);
            }
            if size.iter().any(|&tracks| tracks > MAX_TRACKS) {
                self.oversized = Some(LayoutError::InvalidValue {
                    path: self.nodes[node].path.clone(),
                    field: "track count",
                });
                return;
            }
            extent[node] = size;
        }
        // Slots from the root down, and each sequence's first track in its owner's.
        let mut owner = (0..count).collect::<Vec<usize>>();
        let mut offset = vec![[0usize; 2]; count];
        for node in 0..count {
            let Some(sequence) = self.sequence(node) else {
                continue;
            };
            let d = sequence.direction.index();
            let children = self
                .grid(node)
                .expect("sequences are grids")
                .children
                .clone();
            let mut cursor = 0;
            for child in children {
                let (along, _) = self.child_extent(child, d, &extent);
                let across = if self.joins(child) {
                    extent[child][1 - d]
                } else {
                    extent[node][1 - d]
                };
                let mut start = [0; 2];
                let mut span = [0; 2];
                start[d] = cursor;
                span[d] = along;
                span[1 - d] = across;
                self.nodes[child].slot =
                    Some(GridSlot::new(start[0], start[1]).span(span[0], span[1]));
                if self.joins(child) {
                    owner[child] = owner[node];
                    offset[child] = [offset[node][0] + start[0], offset[node][1] + start[1]];
                }
                cursor += along;
            }
        }
        // Each table's tracks from its sequences' declarations.
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); count];
        for node in 0..count {
            if self.sequence(node).is_some() {
                members[owner[node]].push(node);
            }
        }
        let mut conflict = None;
        for (table, members) in members.iter().enumerate() {
            if members.is_empty() {
                continue;
            }
            let mut tracks: [Vec<Track>; 2] = [Vec::new(), Vec::new()];
            for (d, tracks) in tracks.iter_mut().enumerate() {
                let mut declared: Vec<Option<(Track, usize)>> = vec![None; extent[table][d]];
                for &member in members {
                    let sequence = self.sequence(member).expect("table members are sequences");
                    if sequence.direction.index() != d {
                        continue;
                    }
                    for index in 0..extent[member][d] {
                        let Some(track) = sequence.declared(index) else {
                            continue;
                        };
                        let at = offset[member][d] + index;
                        match &declared[at] {
                            None => declared[at] = Some((track.clone(), member)),
                            Some((first, by)) if first != track => {
                                conflict.get_or_insert_with(|| LayoutError::SharedTracksDiffer {
                                    direction: Direction::BOTH[d],
                                    track: at,
                                    first: self.nodes[*by].path.clone(),
                                    second: self.nodes[member].path.clone(),
                                });
                            }
                            Some(_) => {}
                        }
                    }
                }
                *tracks = declared
                    .into_iter()
                    .map(|declared| declared.map_or_else(|| Track::fr(1.0), |(track, _)| track))
                    .collect();
            }
            for &member in members {
                let range = |d: usize| offset[member][d]..offset[member][d] + extent[member][d];
                let own = [tracks[0][range(0)].to_vec(), tracks[1][range(1)].to_vec()];
                if let NodeKind::Grid(grid) = &mut self.nodes[member].kind {
                    grid.tracks = own;
                }
            }
        }
        self.conflict = conflict;
    }

    /// A child's tracks along and across a parent sequence's direction `d`: a sequence that joins
    /// covers its own extent, which its span widens, and a cell covers its span along and one
    /// track across.
    fn child_extent(&self, child: usize, d: usize, extent: &[[usize; 2]]) -> (usize, usize) {
        if self.joins(child) {
            (extent[child][d], extent[child][1 - d])
        } else {
            (self.nodes[child].span.unwrap_or(1), 1)
        }
    }
}

impl<Id> Tree<Id> {
    pub fn grid(&self, node: usize) -> Option<&GridNode> {
        match &self.nodes[node].kind {
            NodeKind::Grid(grid) => Some(grid),
            _ => None,
        }
    }

    /// The tracks that a grid child's slot spans in `direction`.
    pub fn slot_range(&self, node: usize, direction: Direction) -> Range<usize> {
        let (start, span) = self.nodes[node]
            .slot
            .map_or((0, 1), |slot| slot.along(direction));
        start..start + span
    }

    /// Whether a sequence in its parent's table needs a length along `direction`: a minimum of
    /// its own, or a slab whose minimum length runs that way.
    /// Whether a node has a measured slab on a side across `direction`, whose overhang can reach
    /// past the node's edges in `direction`.
    fn may_overhang(&self, node: usize, direction: Direction) -> bool {
        direction.cross().sides().into_iter().any(|side| {
            self.nodes[node].slabs[side.index()]
                .iter()
                .any(|&band| match self.stack(band) {
                    Some(stack) => stack
                        .members
                        .iter()
                        .any(|&member| self.nodes[member].id.is_some()),
                    None => self.nodes[band].id.is_some(),
                })
        })
    }

    fn has_minimum(&self, node: usize, direction: Direction) -> bool {
        self.nodes[node].min.along(direction) > 0.0
            || direction.cross().sides().into_iter().any(|side| {
                self.nodes[node].slabs[side.index()].iter().any(|&slab| {
                    self.nodes[slab].represented_by.is_none()
                        && self.nodes[slab].min.along(direction) > 0.0
                })
            })
    }

    /// The tag of a slab or stack node.
    pub fn slab_tag(&self, node: usize) -> Option<&Tag> {
        match &self.nodes[node].kind {
            NodeKind::Slab { tag, .. } => tag.as_ref(),
            NodeKind::Stack(stack) => stack.tag.as_ref(),
            _ => None,
        }
    }

    /// The host and side of a band: a slab that is not in a stack, or a stack.
    pub fn band_place(&self, node: usize) -> Option<(usize, Side)> {
        match &self.nodes[node].kind {
            NodeKind::Slab {
                host,
                side,
                stack: None,
                ..
            } => Some((*host, *side)),
            NodeKind::Stack(stack) => Some((stack.host, stack.side)),
            _ => None,
        }
    }

    /// The settings of a stack node.
    pub fn stack(&self, node: usize) -> Option<&StackNode> {
        match &self.nodes[node].kind {
            NodeKind::Stack(stack) => Some(stack),
            _ => None,
        }
    }

    /// How far a band runs along its side: a plain slab's extent, or a stack's.
    pub fn band_extent(&self, band: usize) -> Extent {
        match &self.nodes[band].kind {
            NodeKind::Slab { extent, .. } => *extent,
            NodeKind::Stack(stack) => stack.extent,
            _ => Extent::Content,
        }
    }

    /// What the answers for `node`, with `id`, are stored under.
    pub fn answer_key(&self, node: usize, id: &Id) -> AnswerKey<Id>
    where
        Id: Clone,
    {
        if let Some(key) = &self.nodes[node].content_key {
            return AnswerKey::Content(key.clone());
        }
        if let Some((stack, position)) = self.member_of(node)
            && let Some(key) = &self.nodes[stack].content_key
        {
            return AnswerKey::Member(key.clone(), position);
        }
        AnswerKey::Id(id.clone())
    }

    /// The stack of a stack member, and the member's position in it.
    pub fn member_of(&self, node: usize) -> Option<(usize, usize)> {
        let NodeKind::Slab {
            stack: Some(stack), ..
        } = self.nodes[node].kind
        else {
            return None;
        };
        let position = self
            .stack(stack)?
            .members
            .iter()
            .position(|&member| member == node)?;
        Some((stack, position))
    }

    /// Whether `node` is `ancestor` or lies inside it, without crossing into an overlay.
    pub fn is_within(&self, node: usize, ancestor: usize) -> bool {
        let mut current = Some(node);
        while let Some(index) = current {
            if index == ancestor {
                return true;
            }
            current = self.nodes[index].parent;
        }
        false
    }

    /// The nodes of the tree that `root` starts, in index order.
    pub fn members(&self, root: usize) -> impl Iterator<Item = usize> + '_ {
        (root..self.nodes.len()).filter(move |&index| self.nodes[index].root == root)
    }

    /// Whether a subgrid takes in its children's overflow on `side` of a shared direction: when
    /// it has slabs there, which sit outside that overflow, or when it flushes the side.
    pub fn absorbs(&self, node: usize, side: Side) -> bool {
        !self.nodes[node].slabs[side.index()].is_empty()
            || self.nodes[node].overflow[side.index()] == Overflow::Flush
    }
}

/// One item of an owner's problem.
#[derive(Clone, Debug)]
pub(crate) struct ItemPlan {
    pub node: usize,
    pub start: usize,
    pub end: usize,
    /// The item's tracks across the direction, among the owner's cross tracks.
    pub lane: Range<usize>,
    /// Positions in the owner's grids, from the item's parent up to the owner.
    pub chain: Vec<usize>,
    /// Whether the item stands for a subgrid's edges, its slabs and the overflow of its children
    /// there, rather than for a child.
    pub container: bool,
    /// For the leading and trailing sides, the position in the owner's grids of the subgrid
    /// that takes in the item's overflow on that side.
    pub absorbed_by: [Option<usize>; 2],
}

/// One track owner in one direction: a grid that does not share its parent's tracks there.
#[derive(Clone, Debug)]
pub(crate) struct TrackOwnerPlan {
    pub node: usize,
    /// The owner, then each subgrid that shares its tracks, parents before children.
    pub grids: Vec<usize>,
    /// For each grid in `grids`, its gap: its own, or else its parent's.
    pub gaps: Vec<Gap>,
    pub items: Vec<ItemPlan>,
    /// For each grid in `grids`, the items whose chain holds it.
    pub members: Vec<Vec<Member>>,
    /// The owner's match groups: for each tag it matches, its tracks with that tag.
    pub groups: Vec<(Tag, Vec<usize>)>,
}

/// An item inside one of an owner's grids.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Member {
    /// The item's position in the owner's items.
    pub item: usize,
    /// For the leading and trailing sides, whether a grid inside this one takes in the item's
    /// overflow there, so that the grid's edge does not see it.
    pub inner: [bool; 2],
}

/// The track owners of one direction.
#[derive(Clone, Debug)]
pub(crate) struct DirPlan {
    pub direction: Direction,
    /// For each grid, the grid that owns its tracks in this direction.
    pub owner: Vec<usize>,
    /// For each grid, its first track among its owner's tracks.
    pub offset: Vec<usize>,
    /// The owners of every root, roots in order, each root's deepest owners first.
    pub owners: Vec<TrackOwnerPlan>,
    /// For each node that owns tracks, its position in `owners`.
    pub owner_plan: Vec<Option<usize>>,
}

impl DirPlan {
    /// Builds the plan of `direction` for a validated tree.
    pub fn new<Id>(tree: &Tree<Id>, direction: Direction) -> Self {
        let count = tree.nodes.len();
        let mut plan = Self {
            direction,
            owner: vec![usize::MAX; count],
            offset: vec![0; count],
            owners: Vec::new(),
            owner_plan: vec![None; count],
        };
        for node in 0..count {
            if tree.grid(node).is_some() {
                match tree.nodes[node].parent {
                    Some(parent) if tree.joins(node) => {
                        plan.owner[node] = plan.owner[parent];
                        plan.offset[node] =
                            plan.offset[parent] + tree.slot_range(node, direction).start;
                    }
                    _ => plan.owner[node] = node,
                }
            }
        }
        for &root in &tree.roots {
            plan.visit(tree, root);
        }
        plan
    }

    fn visit<Id>(&mut self, tree: &Tree<Id>, node: usize) {
        let Some(grid) = tree.grid(node) else {
            return;
        };
        for &child in &grid.children {
            self.visit(tree, child);
        }
        if self.owner[node] == node {
            let owner = self.owner_problem(tree, node);
            self.owner_plan[node] = Some(self.owners.len());
            self.owners.push(owner);
        }
    }

    fn owner_problem<Id>(&self, tree: &Tree<Id>, owner: usize) -> TrackOwnerPlan {
        let direction = self.direction;
        let mut plan = TrackOwnerPlan {
            node: owner,
            grids: vec![owner],
            gaps: vec![
                tree.grid(owner).expect("owners are grids").gaps[direction.index()]
                    .clone()
                    .unwrap_or_default(),
            ],
            items: Vec::new(),
            members: Vec::new(),
            groups: Vec::new(),
        };
        self.collect(tree, 0, &[0], &mut plan);
        for item in 0..plan.items.len() {
            plan.items[item].absorbed_by = [0, 1].map(|k| self.absorber(tree, &plan, item, k));
        }
        plan.members = vec![Vec::new(); plan.grids.len()];
        for (index, item) in plan.items.iter().enumerate() {
            let absorbed = item
                .absorbed_by
                .map(|absorber| absorber.and_then(|a| item.chain.iter().position(|&p| p == a)));
            for (depth, &position) in item.chain.iter().enumerate() {
                plan.members[position].push(Member {
                    item: index,
                    inner: absorbed.map(|absorbed| absorbed.is_some_and(|a| a < depth)),
                });
            }
        }
        // Each grid of the table matches the tracks with the tag that it spans. Groups that
        // share a track become one group.
        for &grid in &plan.grids {
            let node = tree.grid(grid).expect("owners and subgrids are grids");
            let start = self.offset[grid];
            for tag in &node.rules.matches {
                let mut tracks: Vec<usize> = node.tracks[direction.index()]
                    .iter()
                    .enumerate()
                    .filter(|(_, track)| track.tag.as_ref() == Some(tag))
                    .map(|(index, _)| start + index)
                    .collect();
                if tracks.is_empty() {
                    continue;
                }
                plan.groups.retain(|(existing, members)| {
                    let shared = existing == tag && members.iter().any(|m| tracks.contains(m));
                    if shared {
                        let added: Vec<usize> = members
                            .iter()
                            .copied()
                            .filter(|m| !tracks.contains(m))
                            .collect();
                        tracks.extend(added);
                    }
                    !shared
                });
                tracks.sort_unstable();
                plan.groups.push((tag.clone(), tracks));
            }
        }
        plan
    }

    /// Adds the children of the grid at `position` in the owner's grids to the plan, descending
    /// into subgrids that share the owner's tracks. `chain` holds the positions from that grid
    /// up to the owner.
    fn collect<Id>(
        &self,
        tree: &Tree<Id>,
        position: usize,
        chain: &[usize],
        plan: &mut TrackOwnerPlan,
    ) {
        let direction = self.direction;
        let d = direction.index();
        let grid = plan.grids[position];
        let children = &tree
            .grid(grid)
            .expect("owners and subgrids are grids")
            .children;
        for &child in children {
            let range = tree.slot_range(child, direction);
            let shared = tree.joins(child);
            let item = ItemPlan {
                node: child,
                start: self.offset[grid] + range.start,
                end: self.offset[grid] + range.end,
                lane: self.lane(tree, child, grid, plan.node),
                chain: chain.to_vec(),
                container: shared,
                absorbed_by: [None; 2],
            };
            if shared {
                plan.grids.push(child);
                let gap = tree.grid(child).expect("subgrids are grids").gaps[d]
                    .clone()
                    .unwrap_or_else(|| plan.gaps[position].clone());
                plan.gaps.push(gap);
                let child_position = plan.grids.len() - 1;
                let [leading, trailing] = direction.sides();
                if tree.absorbs(child, leading)
                    || tree.absorbs(child, trailing)
                    || tree.has_minimum(child, direction)
                    || tree.may_overhang(child, direction)
                {
                    plan.items.push(item);
                }
                let mut inner = vec![child_position];
                inner.extend_from_slice(chain);
                self.collect(tree, child_position, &inner, plan);
            } else {
                plan.items.push(item);
            }
        }
    }

    /// The item's tracks across the direction, among the owner's cross tracks. Every grid
    /// between the item and its owner joins the owner's table, so it shares the cross tracks
    /// too.
    fn lane<Id>(&self, tree: &Tree<Id>, child: usize, parent: usize, owner: usize) -> Range<usize> {
        // A flow's lines are separate rows, each with columns of its own, so in the height pass a
        // child spans the flow's width: what a line puts below it faces all of the next line.
        if self.direction == Direction::Vertical && tree.wrap(parent).is_some_and(|wrap| wrap.flow)
        {
            let columns = tree.grid(parent).map_or(1, |grid| grid.tracks[0].len());
            return 0..columns.max(1);
        }
        let cross = self.direction.cross();
        let mut range = tree.slot_range(child, cross);
        let mut grid = parent;
        while grid != owner {
            let start = tree.slot_range(grid, cross).start;
            range = range.start + start..range.end + start;
            grid = tree.nodes[grid].parent.expect("subgrids have parents");
        }
        range
    }

    /// The subgrid that takes in an item's overflow on side `k` (0 for leading, 1 for
    /// trailing): the innermost subgrid around the item, at whose edge the item lies, that
    /// absorbs the side.
    fn absorber<Id>(
        &self,
        tree: &Tree<Id>,
        plan: &TrackOwnerPlan,
        item: usize,
        k: usize,
    ) -> Option<usize> {
        let item = &plan.items[item];
        let side = self.direction.sides()[k];
        for &position in &item.chain {
            if position == 0 {
                return None;
            }
            let grid = plan.grids[position];
            let start = self.offset[grid];
            let end = start + tree.slot_range(grid, self.direction).len();
            let at_edge = if k == 0 {
                item.start == start
            } else {
                item.end == end
            };
            if !at_edge {
                return None;
            }
            if tree.absorbs(grid, side) {
                return Some(position);
            }
        }
        None
    }
}
