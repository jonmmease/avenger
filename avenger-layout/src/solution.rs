//! A solved layout: one positioned region per node.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

use crate::explain::Explanation;
use crate::geometry::{Rect, Side, Size};
use crate::node::{Extent, GridSlot, Tag};

/// One step from a node to a part of it, in a region's path from the root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathStep {
    /// The container's child at this index, in the order the children were added.
    Child(usize),
    /// The slab or stack at this index on a side, innermost first among the declared slabs and
    /// stacks. The region's kind tells which.
    Slab(Side, usize),
    /// The member of a stack at this position, from the side's start.
    Member(usize),
    /// The slab at this index among those that merges put on a side, innermost first. Merged
    /// slabs lie inside the declared slabs.
    Merged(Side, usize),
    /// The node's overlay at this index.
    Overlay(usize),
}

impl fmt::Display for PathStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathStep::Child(index) => write!(f, "child {index}"),
            PathStep::Slab(side, index) => write!(f, "{side:?} slab {index}"),
            PathStep::Merged(side, index) => write!(f, "{side:?} merged slab {index}"),
            PathStep::Member(index) => write!(f, "member {index}"),
            PathStep::Overlay(index) => write!(f, "overlay {index}"),
        }
    }
}

/// A path from the root as text, such as `root / child 1 / Left slab 0`.
pub(crate) fn path_text(path: &[PathStep]) -> String {
    let mut text = String::from("root");
    for step in path {
        text.push_str(" / ");
        text.push_str(&step.to_string());
    }
    text
}

/// What kind of node a region holds.
#[derive(Clone, Debug, PartialEq)]
pub enum RegionKind {
    Leaf,
    /// A row, with the positions of the tracks it spans in its table.
    Row {
        tracks: GridTracks,
    },
    /// A column, with the positions of the tracks it spans in its table.
    Column {
        tracks: GridTracks,
    },
    /// A grid, with the positions of its tracks.
    Grid {
        tracks: GridTracks,
    },
    /// A wrap, with the positions of its tracks at the count it took, that count, and the
    /// rectangles of the empty cells that its last row leaves.
    Wrap {
        tracks: GridTracks,
        columns: usize,
        empty: Vec<Rect>,
    },
    /// A flow, with one column track per child, at its span on its line, and one row per line.
    Flow {
        tracks: GridTracks,
    },
    /// A stack of slabs on its host's `side`, whose members are slab regions.
    Stack {
        side: Side,
        extent: Extent,
        tag: Option<Tag>,
        /// The stack that is drawn in this stack's place, the kept stack of its run or a merged
        /// stack, or `None` when the stack draws itself. A stack drawn by another takes no space.
        represented_by: Option<Vec<PathStep>>,
    },
    /// A slab on its host's `side`, or a member of a stack there.
    Slab {
        side: Side,
        extent: Extent,
        tag: Option<Tag>,
        /// The slab that is drawn in this slab's place, the kept slab of its run or a merged
        /// slab, or `None` when the slab draws itself. A slab drawn by another takes no space.
        represented_by: Option<Vec<PathStep>>,
    },
}

/// The solved positions of a container's columns and rows. A Row or Column that joins its
/// parent's table reports the table's tracks that it spans.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridTracks {
    pub columns: Vec<TrackSpan>,
    pub rows: Vec<TrackSpan>,
}

/// Where one track starts and how long it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrackSpan {
    pub start: f32,
    pub length: f32,
}

impl RegionKind {
    /// The tracks of a Row, Column, Grid, Wrap or Flow.
    pub fn tracks(&self) -> Option<&GridTracks> {
        match self {
            RegionKind::Row { tracks }
            | RegionKind::Column { tracks }
            | RegionKind::Grid { tracks }
            | RegionKind::Wrap { tracks, .. }
            | RegionKind::Flow { tracks } => Some(tracks),
            RegionKind::Leaf | RegionKind::Slab { .. } | RegionKind::Stack { .. } => None,
        }
    }
}

/// A child's place in the table of tracks that holds it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// The path of the table's owner: the Grid or Wrap, or the outermost Row or Column of the
    /// table.
    pub table: Vec<PathStep>,
    /// The tracks that the child occupies, counted in the table's tracks. For a child of a Grid,
    /// the slot it declared.
    pub slot: GridSlot,
}

/// One solved node.
#[derive(Clone, Debug, PartialEq)]
pub struct Region<Id> {
    pub id: Option<Id>,
    /// The steps from the root to the node.
    pub path: Vec<PathStep>,
    pub kind: RegionKind,
    /// The child's place in its table, so a caller can tell which panel is first in its row or
    /// lowest in its column. `None` for the root, slabs and overlays.
    pub cell: Option<Cell>,
    /// The space that the node's parent gave it. For a slab, the band it occupies along its side.
    pub slot: Rect,
    /// The node's content: its slot without contained slabs, at its placement. Slabs and
    /// propagated overflow lie outside it.
    pub content: Rect,
}

/// A solved layout.
#[derive(Clone, Debug)]
pub struct Solution<Id> {
    pub(crate) size: Size,
    pub(crate) regions: Vec<Region<Id>>,
    pub(crate) index: HashMap<Id, usize>,
    pub(crate) explanation: Explanation,
    pub(crate) stale: Vec<Id>,
}

impl<Id: Clone + Eq + Hash> Solution<Id> {
    pub(crate) fn new(
        size: Size,
        regions: Vec<Region<Id>>,
        explanation: Explanation,
        stale: Vec<Id>,
    ) -> Self {
        let index = regions
            .iter()
            .enumerate()
            .filter_map(|(index, region)| region.id.clone().map(|id| (id, index)))
            .collect();
        Self {
            size,
            regions,
            index,
            explanation,
            stale,
        }
    }

    /// The canvas size: the requested size, or the layout's max-content size in a direction left
    /// open.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Every region, in drawing order: each node before its slabs and children, and its
    /// overlays after all of them.
    pub fn regions(&self) -> impl Iterator<Item = &Region<Id>> {
        self.regions.iter()
    }

    pub fn region(&self, id: &Id) -> Option<&Region<Id>> {
        self.index.get(id).map(|&index| &self.regions[index])
    }

    pub fn at_path(&self, path: &[PathStep]) -> Option<&Region<Id>> {
        self.regions.iter().find(|region| region.path == path)
    }

    /// The region that draws the node with `id`: the slab or stack that represents it when
    /// containers collapse or merge its tag, for a stack's member the member at its position in
    /// the stack that represents its stack, and otherwise its own region. The first slab or
    /// stack that a merge stands for gives its ids to the merged one, so its ids resolve there.
    pub fn representative(&self, id: &Id) -> Option<&Region<Id>> {
        let region = self.region(id)?;
        match &region.kind {
            RegionKind::Slab {
                represented_by: Some(path),
                ..
            }
            | RegionKind::Stack {
                represented_by: Some(path),
                ..
            } => self.at_path(path),
            _ => Some(region),
        }
    }

    /// Why the solution has its sizes. See [`Explanation`] for what every solve records.
    pub fn explanation(&self) -> &Explanation {
        &self.explanation
    }

    /// The measured leaves that took an answer from the last measure because no stored answer
    /// held, in a solution from [`Measured::resize`](crate::Measured::resize) or
    /// [`Measured::min_size`](crate::Measured::min_size). Empty for any other solution, and
    /// empty when the resized solution is the one that measuring gives.
    pub fn stale(&self) -> &[Id] {
        &self.stale
    }
}
