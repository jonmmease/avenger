//! Which slabs and stacks stand in for others: the runs that collapse rules find along each
//! lane, and the slabs and stacks that merge rules put on containers. A stack takes part as one
//! unit, and its members follow it by position. Both follow from the tree's structure, so they are
//! found when the layout is prepared, and again for each Wrap count that a solve takes.

use std::collections::HashMap;
use std::ops::Range;

use crate::explain::{Representation, RepresentationKind};
use crate::geometry::{Direction, Side};
use crate::lower::{DirPlan, Extras, NodeKind, Tree};
use crate::node::{ContentKey, Tag};
use crate::solution::PathStep;

/// A run along one lane: slabs or stacks that show one content key, of which the outermost is
/// drawn.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub tag: Tag,
    pub side: Side,
    /// The outermost slab, which stands in for the others.
    pub kept: usize,
    /// The slabs inward of it, in tree order.
    pub collapsed: Vec<usize>,
}

/// One slab or stack that a merge puts on a container.
#[derive(Clone, Debug)]
pub(crate) struct Merge {
    pub tag: Tag,
    pub side: Side,
    pub container: usize,
    /// The declared slab whose id the merged slab takes: the first slab in tree order that it
    /// stands for, which does not depend on a Wrap's count.
    pub first: usize,
    /// The declared slabs that the merged slab stands in for: the slabs that the runs kept.
    pub members: Vec<usize>,
}

/// The runs and merges of a tree, in nodes of the tree they were found in.
#[derive(Clone, Debug, Default)]
pub(crate) struct Found {
    pub runs: Vec<Run>,
    /// The merges that are drawn, by container in tree order, then by side, then by where their
    /// first slabs sit among their hosts' slabs, which does not depend on a Wrap's count.
    pub merges: Vec<Merge>,
}

/// A slab or merged slab that is still drawn while merges run from the innermost container out.
#[derive(Clone, Debug)]
struct Live {
    host: usize,
    side: Side,
    key: Option<ContentKey>,
    first: usize,
    members: Vec<usize>,
    /// The merge that made this entry, by index into the merges found so far.
    merge: Option<usize>,
}

/// Finds the runs and merges of a validated tree with its plans.
pub(crate) fn find<Id>(tree: &Tree<Id>, plans: &[DirPlan; 2]) -> Found {
    let mut tags: Vec<&Tag> = Vec::new();
    for node in 0..tree.nodes.len() {
        if let Some(grid) = tree.grid(node) {
            for tag in grid.rules.collapses.iter().chain(&grid.rules.merges) {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
    }
    let mut found = Found::default();
    let mut merges: Vec<(Merge, bool)> = Vec::new();
    for tag in tags {
        let declares = |node: usize, merge_only: bool| {
            tree.grid(node).is_some_and(|grid| {
                grid.rules.merges.contains(tag)
                    || (!merge_only && grid.rules.collapses.contains(tag))
            })
        };
        let declaring: Vec<usize> = (0..tree.nodes.len())
            .filter(|&node| declares(node, false))
            .collect();
        let outermost: Vec<usize> = declaring
            .iter()
            .copied()
            .filter(|&node| {
                !declaring
                    .iter()
                    .any(|&other| other != node && tree.is_within(node, other))
            })
            .collect();
        let mut collapsed = vec![false; tree.nodes.len()];
        // The first slab in tree order that each kept slab stands for.
        let mut first_of: Vec<usize> = (0..tree.nodes.len()).collect();
        for &container in &outermost {
            for side in Side::ALL {
                for run in runs(tree, plans, tag, side, container) {
                    for &slab in &run.collapsed {
                        collapsed[slab] = true;
                        first_of[run.kept] = first_of[run.kept].min(slab);
                    }
                    found.runs.push(run);
                }
            }
        }
        // Merges from the innermost container out: a descendant comes after its ancestors.
        let mut live: Vec<Live> = Vec::new();
        for (slab, &collapsed) in collapsed.iter().enumerate() {
            if let Some((host, side)) = tree.band_place(slab)
                && tree.slab_tag(slab) == Some(tag)
                && !collapsed
                && !in_flow(tree, host)
                && outermost
                    .iter()
                    .any(|&o| host != o && tree.is_within(host, o))
            {
                live.push(Live {
                    host,
                    side,
                    key: tree.nodes[slab].content_key.clone(),
                    first: first_of[slab],
                    members: vec![slab],
                    merge: None,
                });
            }
        }
        for container in (0..tree.nodes.len()).rev() {
            if !declares(container, true) {
                continue;
            }
            for side in Side::ALL {
                let inside: Vec<usize> = (0..live.len())
                    .filter(|&entry| {
                        let host = live[entry].host;
                        live[entry].side == side
                            && host != container
                            && tree.is_within(host, container)
                    })
                    .collect();
                let Some(&first_entry) = inside.first() else {
                    continue;
                };
                let key = live[first_entry].key.clone();
                if key.is_none()
                    || inside.iter().any(|&entry| live[entry].key != key)
                    || !covers(tree, plans, container, tag, side, &inside, &live)
                    || !inside
                        .iter()
                        .all(|&entry| at_side(tree, plans, live[entry].host, container, tag, side))
                {
                    continue;
                }
                let mut members: Vec<usize> = inside
                    .iter()
                    .flat_map(|&entry| live[entry].members.iter().copied())
                    .collect();
                members.sort_unstable();
                let first = inside
                    .iter()
                    .map(|&entry| live[entry].first)
                    .min()
                    .expect("a merge has members");
                for &entry in &inside {
                    if let Some(merge) = live[entry].merge {
                        merges[merge].1 = false;
                    }
                }
                merges.push((
                    Merge {
                        tag: tag.clone(),
                        side,
                        container,
                        first,
                        members: members.clone(),
                    },
                    true,
                ));
                let mut index = 0;
                live.retain(|_| {
                    let keep = !inside.contains(&index);
                    index += 1;
                    keep
                });
                live.push(Live {
                    host: container,
                    side,
                    key,
                    first,
                    members,
                    merge: Some(merges.len() - 1),
                });
            }
        }
    }
    found.merges = merges
        .into_iter()
        .filter_map(|(merge, drawn)| drawn.then_some(merge))
        .collect();
    let stack_position = |slab: usize| {
        let (host, side) = tree
            .band_place(slab)
            .expect("merges stand for slabs and stacks");
        tree.nodes[host].slabs[side.index()]
            .iter()
            .position(|&s| s == slab)
            .expect("a slab is on its host's side")
    };
    found.merges.sort_by_key(|merge| {
        (
            merge.container,
            merge.side.index(),
            stack_position(merge.first),
            merge.first,
        )
    });
    found
}

/// Where a host lies in its table in one direction: the table and the host's tracks there.
/// `None` for a root, which is in no table.
fn place<Id>(
    tree: &Tree<Id>,
    plans: &[DirPlan; 2],
    host: usize,
    direction: Direction,
) -> Option<(usize, Range<usize>)> {
    let parent = tree.nodes[host].parent?;
    let plan = &plans[direction.index()];
    let range = tree.slot_range(host, direction);
    let offset = plan.offset[parent];
    Some((plan.owner[parent], range.start + offset..range.end + offset))
}

/// Whether a node takes part in the runs of `tag` on `side`: a cell of its table, or a host
/// with a slab of that tag there. A sequence that joins its table without such a slab is only
/// the container of other hosts.
fn takes_part<Id>(tree: &Tree<Id>, node: usize, tag: &Tag, side: Side) -> bool {
    tagged_slab(tree, node, tag, side).is_some()
        || match &tree.nodes[node].kind {
            NodeKind::Leaf { .. } => true,
            NodeKind::Grid(_) => !tree.joins(node),
            NodeKind::Slab { .. } | NodeKind::Stack(_) => false,
        }
}

/// For a Wrap's left and right sides, the tags whose slabs collapse along its rows: those that
/// the Wrap or a container around it collapses or merges, when [`wrap_lanes_collapse`] allows.
pub(crate) fn wrap_collapses<Id>(tree: &Tree<Id>, wrap: usize) -> [Vec<Tag>; 2] {
    let mut tags: Vec<&Tag> = Vec::new();
    let mut current = Some(wrap);
    while let Some(node) = current {
        if let Some(grid) = tree.grid(node) {
            for tag in grid.rules.collapses.iter().chain(&grid.rules.merges) {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
        current = tree.nodes[node].parent;
    }
    Direction::Horizontal.sides().map(|side| {
        tags.iter()
            .filter(|tag| wrap_lanes_collapse(tree, wrap, tag, side))
            .map(|&tag| tag.clone())
            .collect()
    })
}

/// Whether slabs with `tag` on `side` of a Wrap's children can collapse along the Wrap's lanes.
/// Which child is first or last in its row depends on the count, so left and right slabs
/// collapse only when every child shows one content key, and a flow's children never collapse.
fn wrap_lanes_collapse<Id>(tree: &Tree<Id>, wrap: usize, tag: &Tag, side: Side) -> bool {
    let flow = tree.wrap(wrap).is_some_and(|wrap| wrap.flow);
    !flow && (side.direction() == Direction::Vertical || uniform(tree, wrap, tag, side))
}

/// Whether a host is a child of a flow, whose slabs never collapse or merge.
fn in_flow<Id>(tree: &Tree<Id>, host: usize) -> bool {
    tree.nodes[host]
        .parent
        .and_then(|parent| tree.wrap(parent))
        .is_some_and(|wrap| wrap.flow)
}

/// Whether every child of a Wrap has a slab with `tag` on `side` that shows one content key.
fn uniform<Id>(tree: &Tree<Id>, wrap: usize, tag: &Tag, side: Side) -> bool {
    let children = tree.grid(wrap).map_or(&[][..], |grid| &grid.children[..]);
    let mut keys = children.iter().map(|&child| {
        tagged_slab(tree, child, tag, side).and_then(|slab| tree.nodes[slab].content_key.as_ref())
    });
    match keys.next() {
        Some(Some(first)) => keys.all(|key| key == Some(first)),
        _ => false,
    }
}

/// A host's declared slab with `tag` on `side`. A merged slab is left out: it never collapses,
/// and only counts in the merges of the containers around it.
fn tagged_slab<Id>(tree: &Tree<Id>, host: usize, tag: &Tag, side: Side) -> Option<usize> {
    tree.nodes[host].slabs[side.index()]
        .iter()
        .copied()
        .find(|&slab| tree.slab_tag(slab) == Some(tag) && !is_merged(tree, slab))
}

/// Whether a slab is one that a merge put on its host.
pub(crate) fn is_merged<Id>(tree: &Tree<Id>, slab: usize) -> bool {
    matches!(tree.nodes[slab].path.last(), Some(PathStep::Merged(..)))
}

/// The runs of `tag` on `side` among the hosts inside `container`.
fn runs<Id>(
    tree: &Tree<Id>,
    plans: &[DirPlan; 2],
    tag: &Tag,
    side: Side,
    container: usize,
) -> Vec<Run> {
    let along = side.direction();
    let across = along.cross();
    let trailing = !side.is_leading();
    struct Part {
        node: usize,
        table: usize,
        lane: Range<usize>,
        position: Range<usize>,
        slab: Option<usize>,
    }
    let parts: Vec<Part> = (0..tree.nodes.len())
        .filter(|&node| {
            node != container
                && tree.is_within(node, container)
                && takes_part(tree, node, tag, side)
        })
        .filter_map(|node| {
            let (table, lane) = place(tree, plans, node, across)?;
            let (_, position) = place(tree, plans, node, along)?;
            Some(Part {
                node,
                table,
                lane,
                position,
                slab: tagged_slab(tree, node, tag, side),
            })
        })
        .collect();
    // The position of a part's slab along the side, outward from the side's far edge.
    let edge = |part: &Part| {
        if trailing {
            part.position.end as isize
        } else {
            -(part.position.start as isize)
        }
    };
    let mut lanes: Vec<(usize, Range<usize>)> = Vec::new();
    for part in parts.iter().filter(|part| part.slab.is_some()) {
        let lane = (part.table, part.lane.clone());
        if !lanes.contains(&lane) {
            lanes.push(lane);
        }
    }
    let mut result = Vec::new();
    for (table, lane) in lanes {
        if tree.wrap(table).is_some() && !wrap_lanes_collapse(tree, table, tag, side) {
            continue;
        }
        let mut hosts: Vec<&Part> = parts
            .iter()
            .filter(|part| part.slab.is_some() && part.table == table && part.lane == lane)
            .collect();
        hosts.sort_by_key(|part| (edge(part), part.node));
        let breakers: Vec<&Part> = parts
            .iter()
            .filter(|part| {
                part.table == table
                    && part.lane != lane
                    && part.lane.start < lane.end
                    && lane.start < part.lane.end
            })
            .collect();
        let key = |part: &Part| {
            part.slab
                .and_then(|slab| tree.nodes[slab].content_key.as_ref())
        };
        let mut chain: Vec<usize> = Vec::new();
        let flush = |chain: &mut Vec<usize>, result: &mut Vec<Run>| {
            if chain.len() > 1 {
                let kept = chain.pop().expect("a run has slabs");
                let mut collapsed = std::mem::take(chain);
                collapsed.sort_unstable();
                result.push(Run {
                    tag: tag.clone(),
                    side,
                    kept,
                    collapsed,
                });
            }
            chain.clear();
        };
        for pair in hosts.windows(2) {
            let (inner, outer) = (pair[0], pair[1]);
            let ordered = if trailing {
                inner.position.end <= outer.position.start
            } else {
                outer.position.end <= inner.position.start
            };
            // A host that is a container, such as a row of panels under one header, holds cells
            // in other lanes, which do not stand between its slab and the next.
            let blocked = breakers.iter().any(|breaker| {
                edge(inner) <= edge(breaker)
                    && edge(breaker) < edge(outer)
                    && !tree.is_within(breaker.node, inner.node)
                    && !tree.is_within(breaker.node, outer.node)
            });
            let stands_in = ordered && !blocked && key(inner).is_some() && key(inner) == key(outer);
            if stands_in {
                if chain.is_empty() {
                    chain.push(inner.slab.expect("hosts have slabs"));
                }
                chain.push(outer.slab.expect("hosts have slabs"));
            } else {
                flush(&mut chain, &mut result);
            }
        }
        flush(&mut chain, &mut result);
    }
    result
}

/// Whether the hosts of `entries` cover every track of `container` along `side`.
///
/// Which track a Wrap's child takes depends on the count, so a Wrap is covered at every count
/// only when every child holds a host of a slab with `tag` on `side`.
fn covers<Id>(
    tree: &Tree<Id>,
    plans: &[DirPlan; 2],
    container: usize,
    tag: &Tag,
    side: Side,
    entries: &[usize],
    live: &[Live],
) -> bool {
    if tree.wrap(container).is_some() {
        let children = tree
            .grid(container)
            .map_or(&[][..], |grid| &grid.children[..]);
        return !children.is_empty()
            && children.iter().all(|&child| {
                (0..tree.nodes.len()).any(|host| {
                    tree.is_within(host, child) && tagged_slab(tree, host, tag, side).is_some()
                })
            });
    }
    let across = side.direction().cross();
    let plan = &plans[across.index()];
    let table = plan.owner[container];
    let count = tree
        .grid(container)
        .map_or(0, |grid| grid.tracks[across.index()].len());
    let start = plan.offset[container];
    let mut covered = vec![false; count];
    for &entry in entries {
        // The host's ancestor that is a cell or sequence of the container's table.
        let mut node = live[entry].host;
        while let Some(parent) = tree.nodes[node].parent {
            if plan.owner[parent] == table {
                break;
            }
            node = parent;
        }
        let Some((_, range)) = place(tree, plans, node, across) else {
            continue;
        };
        for track in range {
            if let Some(slot) = track.checked_sub(start).and_then(|t| covered.get_mut(t)) {
                *slot = true;
            }
        }
    }
    count > 0 && covered.into_iter().all(|covered| covered)
}

/// The path of each merge's slab, in the order of `found.merges`: its container's path and its
/// place among the merged slabs on its side.
fn merged_paths<Id>(first: &Tree<Id>, found: &Found) -> Vec<Vec<PathStep>> {
    let mut paths: Vec<Vec<PathStep>> = Vec::with_capacity(found.merges.len());
    let mut position = 0;
    for (index, merge) in found.merges.iter().enumerate() {
        let same_side = index > 0 && {
            let previous = &found.merges[index - 1];
            previous.container == merge.container && previous.side == merge.side
        };
        position = if same_side { position + 1 } else { 0 };
        let mut path = first.nodes[merge.container].path.clone();
        path.push(PathStep::Merged(merge.side, position));
        paths.push(path);
    }
    paths
}

/// The merged slabs and stacks of `found`, for lowering: per container side, innermost first, as
/// copies of the first slab or stack that each merge stands for, with its ids.
pub(crate) fn extras<Id: Clone>(first: &Tree<Id>, found: &Found) -> Extras<Id> {
    let mut extras = Extras::default();
    for merge in &found.merges {
        // A stack's members follow its node.
        let mut template = vec![first.nodes[merge.first].clone()];
        if let Some(stack) = first.stack(merge.first) {
            template.extend(
                stack
                    .members
                    .iter()
                    .map(|&member| first.nodes[member].clone()),
            );
        }
        let container = first.nodes[merge.container].path.clone();
        extras
            .merged
            .entry(container)
            .or_insert_with(Default::default)[merge.side.index()]
        .push(template);
        extras.moved.insert(first.nodes[merge.first].path.clone());
    }
    extras
}

/// Marks every slab, stack and stack member of `tree` that another one draws, from the runs and
/// merges that `found` lists for `first`, a tree with the same declared slabs, and lists the
/// runs and merges.
pub(crate) fn mark<Id>(
    tree: &mut Tree<Id>,
    first: &Tree<Id>,
    found: &Found,
) -> Vec<Representation> {
    let merged = merged_paths(first, found);
    let mut kept_by: HashMap<usize, usize> = HashMap::new();
    for run in &found.runs {
        for &slab in &run.collapsed {
            kept_by.insert(slab, run.kept);
        }
    }
    let mut drawn_by: HashMap<usize, &Vec<PathStep>> = HashMap::new();
    for (merge, path) in found.merges.iter().zip(&merged) {
        for &member in &merge.members {
            drawn_by.insert(member, path);
        }
    }
    let index: HashMap<&[PathStep], usize> = tree
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (&node.path[..], index))
        .collect();
    // A slab or stack is drawn by its run's kept one, or by the merge that took that one.
    let representative = |band: usize| {
        let mut representative = None;
        let mut current = band;
        loop {
            if let Some(path) = drawn_by.get(&current) {
                representative = Some(path.to_vec());
                break;
            }
            match kept_by.get(&current) {
                Some(&kept) => {
                    representative = Some(first.nodes[kept].path.clone());
                    current = kept;
                }
                None => break,
            }
        }
        representative
    };
    let mut marks = Vec::new();
    for node in 0..first.nodes.len() {
        // A stack's member is drawn by the member at its position in the stack that draws its
        // stack.
        let path = match first.member_of(node) {
            Some((stack, position)) => representative(stack).map(|mut path| {
                path.push(PathStep::Member(position));
                path
            }),
            None => representative(node),
        };
        if let Some(path) = path
            && let (Some(&node), Some(&by)) =
                (index.get(&first.nodes[node].path[..]), index.get(&path[..]))
        {
            marks.push((node, by));
        }
    }
    for (node, by) in marks {
        tree.nodes[node].represented_by = Some(by);
    }
    let key = |slab: usize| {
        first.nodes[slab]
            .content_key
            .clone()
            .expect("runs and merges have content keys")
    };
    let paths = |slabs: &[usize]| -> Vec<Vec<PathStep>> {
        slabs
            .iter()
            .map(|&slab| first.nodes[slab].path.clone())
            .collect()
    };
    let mut representations: Vec<Representation> = found
        .runs
        .iter()
        .map(|run| Representation {
            kind: RepresentationKind::Run,
            tag: run.tag.clone(),
            content_key: key(run.kept),
            side: run.side,
            representative: first.nodes[run.kept].path.clone(),
            members: paths(&run.collapsed),
        })
        .collect();
    representations.extend(
        found
            .merges
            .iter()
            .zip(&merged)
            .map(|(merge, path)| Representation {
                kind: RepresentationKind::Merge,
                tag: merge.tag.clone(),
                content_key: key(merge.first),
                side: merge.side,
                representative: path.clone(),
                members: paths(&merge.members),
            }),
    );
    representations
}

/// Whether a host inside `container` stands at its `side`: whether no other cell inside the
/// container lies between the host and that side in the host's lane, in each table from the
/// host's out, so that a slab merged onto the container passes no cell. Which cells of a Wrap
/// share a lane depends on the count, so a Wrap's child stands at each side when every cell of
/// the Wrap shows a slab with `tag` on `side`, which the merge stands for too.
fn at_side<Id>(
    tree: &Tree<Id>,
    plans: &[DirPlan; 2],
    host: usize,
    container: usize,
    tag: &Tag,
    side: Side,
) -> bool {
    let along = side.direction();
    let across = along.cross();
    let beyond = |position: &Range<usize>, other: &Range<usize>| {
        if side.is_leading() {
            other.end <= position.start
        } else {
            position.end <= other.start
        }
    };
    let mut node = host;
    while node != container {
        let Some(parent) = tree.nodes[node].parent else {
            return false;
        };
        if tree.wrap(parent).is_some() {
            if !wrap_shows(tree, parent, tag, side) {
                return false;
            }
            node = parent;
            continue;
        }
        let (Some((table, lane)), Some((_, position))) = (
            place(tree, plans, node, across),
            place(tree, plans, node, along),
        ) else {
            return false;
        };
        let blocked = (0..tree.nodes.len()).any(|other| {
            other != container
                && tree.is_within(other, container)
                && is_cell(tree, other)
                && !tree.is_within(other, node)
                && !tree.is_within(node, other)
                && place(tree, plans, other, across).is_some_and(|(other_table, other_lane)| {
                    other_table == table
                        && other_lane.start < lane.end
                        && lane.start < other_lane.end
                })
                && place(tree, plans, other, along)
                    .is_some_and(|(_, other_position)| beyond(&position, &other_position))
        });
        if blocked {
            return false;
        }
        // The table's owner, or the container when the container lies in the table.
        node = if tree.is_within(container, table) {
            container
        } else {
            table
        };
    }
    true
}

/// Whether a node is a cell of its table: a leaf, or a container that starts a table of its own.
fn is_cell<Id>(tree: &Tree<Id>, node: usize) -> bool {
    match &tree.nodes[node].kind {
        NodeKind::Leaf { .. } => true,
        NodeKind::Grid(_) => !tree.joins(node),
        NodeKind::Slab { .. } | NodeKind::Stack(_) => false,
    }
}

/// Whether every leaf of a Wrap shows a slab with `tag` on `side`: whether the leaf, or a
/// container around it inside the Wrap, is a host of one.
fn wrap_shows<Id>(tree: &Tree<Id>, wrap: usize, tag: &Tag, side: Side) -> bool {
    let mut leaves = (0..tree.nodes.len())
        .filter(|&node| {
            node != wrap
                && tree.is_within(node, wrap)
                && matches!(tree.nodes[node].kind, NodeKind::Leaf { .. })
        })
        .peekable();
    leaves.peek().is_some()
        && leaves.all(|leaf| {
            let mut node = leaf;
            while node != wrap {
                if tagged_slab(tree, node, tag, side).is_some() {
                    return true;
                }
                node = tree.nodes[node]
                    .parent
                    .expect("a leaf of a Wrap is inside it");
            }
            false
        })
}
