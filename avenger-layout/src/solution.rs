//! The result of [`Layout::solve`](crate::build::Layout::solve): absolute
//! geometry for every node, queryable by id or structural path.

use crate::build::Spacing;
use crate::geometry::{Edges, Rect, Side, Size};
use crate::grid::GridSlot;
use crate::region::EdgeGrant;

/// The chrome layer a positioned slab belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeLayer {
    Margin,
    Strip,
    Legend,
    Guide,
}

/// One positioned chrome slab of a chromed node, in root coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChromeSlab {
    pub layer: ChromeLayer,
    pub side: Side,
    /// Index among this side's strips (outside-in declaration order); zero
    /// for the other layers.
    pub strip_index: usize,
    pub rect: Rect,
}

/// Solved track geometry of one grid region, **relative to the region's
/// content rectangle** (add `region.content.x/y` for root coordinates).
/// Relative starts are the solver's exact floats, which byte-stable
/// consumers (placement change detection) rely on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SolvedTracks {
    pub column_starts: Vec<f32>,
    pub column_sizes: Vec<f32>,
    pub row_starts: Vec<f32>,
    pub row_sizes: Vec<f32>,
    /// The spacing the solve actually used on each axis: declared values
    /// merged with the grid's share group (outers and `min_gap` by max).
    pub column_spacing: Spacing,
    pub row_spacing: Spacing,
}

impl SolvedTracks {
    /// Content rectangle covered by a grid slot, relative to the containing
    /// region's content rectangle.
    pub fn content_rect_for_slot(&self, slot: GridSlot) -> Option<Rect> {
        if slot.row_span == 0 || slot.column_span == 0 {
            return None;
        }

        let row_end = slot.row_end();
        let column_end = slot.column_end();
        if row_end > self.row_starts.len()
            || row_end > self.row_sizes.len()
            || column_end > self.column_starts.len()
            || column_end > self.column_sizes.len()
        {
            return None;
        }

        let last_row = row_end - 1;
        let last_column = column_end - 1;
        let x = self.column_starts[slot.column];
        let y = self.row_starts[slot.row];
        Some(Rect::new(
            x,
            y,
            self.column_starts[last_column] + self.column_sizes[last_column] - x,
            self.row_starts[last_row] + self.row_sizes[last_row] - y,
        ))
    }

    /// Content origin covered by a grid slot, relative to the containing
    /// region's content rectangle.
    pub fn content_origin_for_slot(&self, slot: GridSlot) -> Option<[f32; 2]> {
        self.content_rect_for_slot(slot)
            .map(|rect| [rect.x, rect.y])
    }

    /// Content size covered by a grid slot.
    pub fn content_size_for_slot(&self, slot: GridSlot) -> Option<Size> {
        self.content_rect_for_slot(slot)
            .map(|rect| Size::new(rect.width, rect.height))
    }
}

/// Kind-specific detail of one solved region.
#[derive(Clone, Debug, PartialEq)]
pub enum RegionDetail {
    Leaf,
    Grid { tracks: SolvedTracks },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tracks() -> SolvedTracks {
        SolvedTracks {
            column_starts: vec![0.0, 50.0, 125.0],
            column_sizes: vec![40.0, 60.0, 30.0],
            row_starts: vec![0.0, 90.0],
            row_sizes: vec![70.0, 20.0],
            column_spacing: Spacing::default(),
            row_spacing: Spacing::default(),
        }
    }

    #[test]
    fn solved_tracks_reports_slot_rects_for_spans_and_holes() {
        let tracks = sample_tracks();
        let slot = GridSlot {
            row: 0,
            column: 1,
            row_span: 2,
            column_span: 2,
        };

        assert_eq!(
            tracks.content_rect_for_slot(slot),
            Some(Rect::new(50.0, 0.0, 105.0, 110.0))
        );
        assert_eq!(tracks.content_origin_for_slot(slot), Some([50.0, 0.0]));
        assert_eq!(
            tracks.content_size_for_slot(slot),
            Some(Size::new(105.0, 110.0))
        );
    }

    #[test]
    fn solved_tracks_rejects_invalid_slots() {
        let tracks = sample_tracks();
        for slot in [
            GridSlot {
                row: 0,
                column: 0,
                row_span: 0,
                column_span: 1,
            },
            GridSlot {
                row: 0,
                column: 0,
                row_span: 1,
                column_span: 0,
            },
            GridSlot {
                row: 2,
                column: 0,
                row_span: 1,
                column_span: 1,
            },
            GridSlot {
                row: 0,
                column: 3,
                row_span: 1,
                column_span: 1,
            },
            GridSlot {
                row: 1,
                column: 1,
                row_span: 2,
                column_span: 1,
            },
            GridSlot {
                row: 1,
                column: 2,
                row_span: 1,
                column_span: 2,
            },
        ] {
            assert_eq!(tracks.content_rect_for_slot(slot), None);
            assert_eq!(tracks.content_origin_for_slot(slot), None);
            assert_eq!(tracks.content_size_for_slot(slot), None);
        }
    }
}

/// One solved node, in root coordinates (the root envelope's top-left corner
/// is the origin).
///
/// Two rectangles, deliberately distinct:
///
/// - [`slot`](Region::slot) is the **allotment** — the space the parent
///   granted this node (a track-span rectangle for grid children; the
///   solved content area at the root). This is what grows under stretch and
///   coordination, and what a convergence loop adopts for re-measurement.
/// - [`content`](Region::content) is the node's own content rectangle — a
///   leaf's measured size, a grid's solved track extent, or a contained
///   (`SolveFor::Content`/`Margins`) node's derived content — positioned
///   within the slot by the node's `CellAlign`. Content never lies: the
///   solver does not falsify a measurement to fill space; slack shows as
///   `slot` exceeding `content`.
#[derive(Clone, Debug, PartialEq)]
pub struct Region<Id = usize> {
    pub id: Option<Id>,
    /// Index route from the root (empty for the root region itself).
    pub path: Vec<usize>,
    pub depth: usize,
    pub slot: Rect,
    pub content: Rect,
    /// Declared chrome, positioned. Empty when the node declares none.
    pub slabs: Vec<ChromeSlab>,
    /// The overflow this node asked for (measured demands plus lifted
    /// chrome), from the natural pass-1 measurement — pre-merge.
    pub requested: Edges<EdgeGrant>,
    /// This node's own ask after share coordination: the pass-2 demands,
    /// with share-group floors patched in. Equals `requested` when no
    /// share group touches this node (an unpatched node re-measures
    /// identically; with no share patches at all, pass 2 is skipped and
    /// the two fields are copies of the same measurement). Distinct from
    /// `granted`: a share-group member's `coordinated` edge holds the
    /// group's merged layers, while `granted` also folds in unrelated
    /// siblings sharing the parent's tracks.
    pub coordinated: Edges<EdgeGrant>,
    /// The overflow space granted around this node's slot (per-track merged
    /// demand within its parent; equals `requested` at the root).
    pub granted: Edges<EdgeGrant>,
    /// Geometric view of this node's own measured overflow: raw per-side
    /// maxima without the `guide + legend` lift (the node-level
    /// analogue of [`Envelope::geometric_total`]).
    pub geometric_total: Edges<f32>,
    pub detail: RegionDetail,
}

/// What the whole solved layout looks like from outside: its content extent
/// plus per-side overflow, in both edge laws (coordinated grants and raw
/// geometric totals).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Envelope {
    pub content_size: Size,
    /// Coordinated view: per-side grants whose totals are lifted to
    /// `>= guide + legend` (the coordination law), so independently
    /// coordinated strata stay representable — the space the layout
    /// reserves once cousins align, which can exceed any single rendered
    /// total. Carries the strata (`guide`/`legend`), not just the total.
    pub coordinated: Edges<EdgeGrant>,
    /// Geometric view: raw per-side maxima without the lift — the honestly
    /// rendered envelope (no coordination reservation).
    pub geometric_total: Edges<f32>,
}

/// Non-fatal observations from a solve (populated by share-key
/// coordination).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    /// Share groups skipped because members had incompatible shapes.
    pub skipped_groups: Vec<SkippedShare>,
    /// Axes where declared track sizes overrode a `uniform_*` flag.
    pub uniform_conflicts: usize,
}

/// One skipped share group.
#[derive(Clone, Debug, PartialEq)]
pub struct SkippedShare {
    /// Paths of the member grids.
    pub member_paths: Vec<Vec<usize>>,
    pub reason: SkippedShareReason,
}

/// Why a share group could not merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkippedShareReason {
    /// Non-uniform members with different grid shapes.
    ShapeMismatch,
    /// Members declared conflicting `TrackSize` vectors.
    TrackSizeMismatch,
}

/// Solved geometry for one [`Layout`](crate::build::Layout).
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSolution<Id = usize> {
    /// Total solved envelope (the canvas).
    pub size: Size,
    pub(crate) regions: Vec<Region<Id>>,
    pub(crate) envelope: Envelope,
    pub(crate) diagnostics: Diagnostics,
}

impl<Id> LayoutSolution<Id> {
    /// All regions in document order (root first, then children depth-first
    /// in declaration order).
    pub fn regions(&self) -> impl Iterator<Item = &Region<Id>> {
        self.regions.iter()
    }

    /// Leaf regions only.
    pub fn leaves(&self) -> impl Iterator<Item = &Region<Id>> {
        self.regions
            .iter()
            .filter(|region| matches!(region.detail, RegionDetail::Leaf))
    }

    /// The region at a structural index path (empty path = root).
    pub fn at_path(&self, path: &[usize]) -> Option<&Region<Id>> {
        self.regions.iter().find(|region| region.path == path)
    }

    /// The whole layout's content extent plus per-side overflow.
    pub fn envelope(&self) -> &Envelope {
        &self.envelope
    }

    /// Non-fatal observations from the solve.
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
}

impl<Id: PartialEq> LayoutSolution<Id> {
    /// The region carrying a caller id.
    pub fn region(&self, id: &Id) -> Option<&Region<Id>> {
        self.regions
            .iter()
            .find(|region| region.id.as_ref() == Some(id))
    }
}

impl<Id> LayoutSolution<Id> {
    /// Maximum absolute difference in leaf **allotment** (slot) sizes
    /// against another solution, matched by structural path; infinity when
    /// the leaf structures differ.
    ///
    /// This is the convergence-loop driver: the allotment is what a caller
    /// adopts as the next measurement operating point, so a delta below
    /// epsilon means re-measuring would change nothing.
    pub fn content_delta(&self, other: &LayoutSolution<Id>) -> f32 {
        let mut mine = self.leaves();
        let mut theirs = other.leaves();
        let mut delta = 0.0f32;
        loop {
            match (mine.next(), theirs.next()) {
                (None, None) => return delta,
                (Some(a), Some(b)) if a.path == b.path => {
                    delta = delta
                        .max((a.slot.width - b.slot.width).abs())
                        .max((a.slot.height - b.slot.height).abs());
                }
                _ => return f32::INFINITY,
            }
        }
    }
}
