//! Why each track and gap has its size: the record that a solve keeps when asked to explain.

use crate::geometry::{Direction, Side};
use crate::node::{ContentKey, Tag, TrackSize};
use crate::solution::PathStep;

/// Why a solved layout has its sizes.
///
/// Every solution keeps the solve count, whether measured widths or overhangs were still
/// growing, the raised widths and overhangs, the aspect fits, and the runs and merges of the
/// collapse and merge rules. The
/// tracks, gaps and match groups are kept only when
/// [`SolveOptions::explain`](crate::SolveOptions::explain) is set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Explanation {
    /// How many solves ran, counting those that growing measured widths and overhangs started.
    pub solves: u32,
    /// Whether measured widths or overhangs were still growing when the solves ran out.
    pub stopped_at_limit: bool,
    /// The leaves whose measured width started another solve.
    pub raised_widths: Vec<RaisedWidth>,
    /// The slabs whose measured overhang started another solve.
    pub raised_overhangs: Vec<RaisedOverhang>,
    /// The grids whose Fr columns an extra width pass capped to fit their Aspect rows.
    pub aspect_fits: Vec<AspectFit>,
    /// Each run and merge of the containers' collapse and merge rules.
    pub representations: Vec<Representation>,
    /// The tracks, gaps and match groups, or `None` when the solve was not asked to explain.
    pub sizes: Option<ExplainedSizes>,
}

/// Why each track and gap has its size: what a solve records when asked to explain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExplainedSizes {
    /// The tracks and gaps of each track owner, per direction, for every root.
    pub owners: Vec<TrackOwnerExplanation>,
    /// The match groups that held tracks, gaps or slabs.
    pub match_groups: Vec<MatchGroup>,
}

/// One slab that stands in for others with the same tag and content key on one side.
#[derive(Clone, Debug, PartialEq)]
pub struct Representation {
    pub kind: RepresentationKind,
    pub tag: Tag,
    pub content_key: ContentKey,
    pub side: Side,
    /// The slab that stands in: the outermost slab of a run, or a merged slab.
    pub representative: Vec<PathStep>,
    /// The slabs that it stands in for, in tree order. A merge's members are the slabs that its
    /// runs kept.
    pub members: Vec<Vec<PathStep>>,
}

/// Which rule made a [`Representation`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepresentationKind {
    /// A run along one lane, from a collapse or merge rule.
    Run,
    /// A slab that a merge rule put on a container.
    Merge,
}

/// One match group: the parts with a tag that a grid matches, in one direction.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchGroup {
    pub tag: Tag,
    pub direction: Direction,
    pub parts: MatchedParts,
    /// The size that every part of the group took.
    pub size: f32,
    /// The grid that declared the match.
    pub grid: Vec<PathStep>,
}

/// What kind of parts a match group holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MatchedParts {
    Tracks,
    Gaps,
    Slabs,
}

/// The tracks and gaps of one track owner in one direction.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackOwnerExplanation {
    pub path: Vec<PathStep>,
    pub direction: Direction,
    pub tracks: Vec<TrackExplanation>,
    /// One per boundary, the gap after track `i` at index `i`.
    pub gaps: Vec<GapExplanation>,
}

/// Why one track has its size.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackExplanation {
    pub size: TrackSize,
    pub tag: Option<Tag>,
    pub start: f32,
    pub length: f32,
    /// The track's base size: the smallest size that holds its items' minimum contributions,
    /// before leftover space.
    pub base_size: f32,
    /// The size the track grows toward before Fr tracks share what is left: the largest
    /// max-content contribution of its items, at least its base size.
    pub growth_limit: f32,
    /// The item whose minimum contribution set the base size last, if any.
    pub set_by: Option<Vec<PathStep>>,
    /// The base size before the match group of the track's tag raised it, if it did.
    pub raised_from: Option<f32>,
}

/// Why one gap has its size.
#[derive(Clone, Debug, PartialEq)]
pub struct GapExplanation {
    pub start: f32,
    pub length: f32,
    /// The term of the lane that decided the gap.
    pub term: GapTerm,
    /// The grid whose gap applied in that lane.
    pub grid: Vec<PathStep>,
    /// The tag of that grid's gap.
    pub tag: Option<Tag>,
    /// The size before the match group of the gap's tag raised it, if it did.
    pub raised_from: Option<f32>,
}

/// The term that decided a gap.
#[derive(Clone, Debug, PartialEq)]
pub enum GapTerm {
    /// The grid's minimum gap.
    Minimum,
    /// The overflow of the items before and after the gap in one lane, plus the grid's
    /// clearance.
    Overflow {
        trailing: Option<Vec<PathStep>>,
        leading: Option<Vec<PathStep>>,
    },
}

/// A measured width that started another solve.
#[derive(Clone, Debug, PartialEq)]
pub struct RaisedWidth {
    pub path: Vec<PathStep>,
    pub from: f32,
    pub to: f32,
}

/// A slab's measured overhang, past the start and the end of its slot, that started another
/// solve.
#[derive(Clone, Debug, PartialEq)]
pub struct RaisedOverhang {
    pub path: Vec<PathStep>,
    pub from: [f32; 2],
    pub to: [f32; 2],
}

/// An extra width pass that capped a grid's Fr columns to fit its Aspect rows.
#[derive(Clone, Debug, PartialEq)]
pub struct AspectFit {
    pub path: Vec<PathStep>,
    /// The width that the grid's Fr columns were capped at.
    pub cap: f32,
    /// How much taller than the canvas the layout still was after the pass.
    pub remaining: f32,
}
