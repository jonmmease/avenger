//! Solving a lowered tree: the width pass, the height pass, measuring, and the re-solve loop.
//!
//! Collapse and merge are found when the tree is prepared, and again on each variant of the
//! tree at other Wrap arrangements. Each Wrap picks its arrangement in the width pass, and a
//! solve whose Wraps pick other arrangements than its tree lays out runs again on a variant of
//! the tree that lays them out so. The prepared layout keeps the variants of its recent solves,
//! and each solve starts on the most recent one.
//!
//! Each pass runs in one direction over one root:
//!
//! 1. Slab thicknesses, measured and matched.
//! 2. Overflow, from the leaves up: what each node exports into its parent's gaps and what it
//!    keeps inside its box.
//! 3. Gaps per lane in every track owner, then matched gaps.
//! 4. Track base sizes and growth limits, and nodes' minimum and max-content contributions,
//!    from the leaves up.
//! 5. Allocation from the root down, then slab positions.

use std::hash::Hash;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::explain::{
    AspectFit, ExplainedSizes, Explanation, GapExplanation, GapTerm, MatchGroup, MatchedParts,
    RaisedOverhang, RaisedWidth, Representation, TrackExplanation, TrackOwnerExplanation,
};
use crate::geometry::{Direction, Rect, Side, Size};
use crate::lower::{Arrangement, DirPlan, Extras, GridKind, NodeKind, Tree};
use crate::measure::{Answer, AnswerKey, History, Holds, Measure, Measurement};
use crate::node::{
    Distribute, Extent, Gap, GridSlot, Layout, Overflow, Placement, Position, Sizing, SolveOptions,
    Tag, Track, TrackSize,
};
use crate::share;
use crate::snap::Snap;
use crate::solution::{Cell, GridTracks, PathStep, Region, RegionKind, Solution, TrackSpan};
use crate::tracks::{
    self, Allocation, Contribution, GapGrid, Term, TrackOwnerItem, TrackOwnerProblem, TrackSizes,
};
use crate::validate::{LayoutError, validate};

const H: Direction = Direction::Horizontal;
const V: Direction = Direction::Vertical;

/// How much a measured width or overhang must grow to start another solve: a thousandth of a
/// unit, the tolerance that snapping and the warnings use, so that a width that converges
/// settles in a few solves.
const GROWTH: f32 = 1e-3;

/// The most Wrap arrangements whose trees a prepared layout keeps.
const KEPT_VARIANTS: usize = 8;

/// The one column that a Wrap has in the width pass.
static WRAP_COLUMN: [Track; 1] = [Track {
    size: TrackSize::Fr {
        weight: 1.0,
        basis: 0.0,
        max: None,
    },
    tag: None,
}];

/// A validated tree with its plans, and the runs and merges of its collapse and merge rules.
///
/// The prepared tree lays out each Wrap at one column. A solve that finds other counts works on
/// a [`Variant`] at those counts instead, and every variant shares the width plan, since the
/// width pass does not depend on the counts. [`Variants`] keeps the variants of recent solves.
#[derive(Clone, Debug)]
pub(crate) struct Prepared<Id> {
    /// The tree before merges, for finding the runs again at other Wrap counts.
    first: Tree<Id>,
    /// The tree with each Wrap at one column, with its height plan and its runs and merges.
    pub base: Arc<Variant<Id>>,
    pub width_plan: DirPlan,
    /// The Wraps, in tree order.
    pub wraps: Vec<usize>,
    /// For each Wrap's left and right sides, the tags whose slabs collapse along its rows, which
    /// do not widen its column gaps.
    pub wrap_collapses: Vec<[Vec<Tag>; 2]>,
    variants: Variants<Id>,
}

/// The arrangements that a prepared layout's recent solves took, most recent first, each with
/// its tree. A resize that keeps every Wrap's arrangement solves once on a kept tree, and a
/// resize back to an earlier arrangement builds no new tree.
#[derive(Debug)]
struct Variants<Id> {
    recent: Mutex<Vec<Kept<Id>>>,
}

/// A kept tree and the arrangements that it is laid out at.
type Kept<Id> = (Vec<Arrangement>, Arc<Variant<Id>>);

impl<Id> Clone for Variants<Id> {
    fn clone(&self) -> Self {
        Self {
            recent: Mutex::new(self.lock().clone()),
        }
    }
}

impl<Id> Variants<Id> {
    fn lock(&self) -> MutexGuard<'_, Vec<Kept<Id>>> {
        self.recent.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The tree that the last solve ended on.
    fn latest(&self) -> Option<Arc<Variant<Id>>> {
        self.lock().first().map(|(_, variant)| variant.clone())
    }

    /// The kept tree at `arrangements`, made the most recent, if one is kept.
    fn take(&self, arrangements: &[Arrangement]) -> Option<Arc<Variant<Id>>> {
        let mut recent = self.lock();
        let position = recent.iter().position(|(kept, _)| kept == arrangements)?;
        let entry = recent.remove(position);
        let variant = entry.1.clone();
        recent.insert(0, entry);
        Some(variant)
    }

    /// Keeps `variant` as the most recent, dropping the oldest past [`KEPT_VARIANTS`].
    fn keep(&self, arrangements: &[Arrangement], variant: Arc<Variant<Id>>) {
        let mut recent = self.lock();
        recent.retain(|(kept, _)| kept != arrangements);
        recent.insert(0, (arrangements.to_vec(), variant));
        recent.truncate(KEPT_VARIANTS);
    }
}

/// The prepared tree laid out at some Wrap arrangements, with its height plan and the runs and
/// merges at those arrangements.
#[derive(Clone, Debug)]
pub(crate) struct Variant<Id> {
    pub tree: Tree<Id>,
    pub height_plan: DirPlan,
    pub representations: Vec<Representation>,
}

impl<Id: Clone + Eq + Hash> Prepared<Id> {
    pub fn new(layout: &Layout<Id>, options: &SolveOptions) -> Result<Self, LayoutError> {
        let first = Tree::lower(layout, &Extras::default());
        validate(&first, options)?;
        let first_plans = [DirPlan::new(&first, H), DirPlan::new(&first, V)];
        let found = share::find(&first, &first_plans);
        let mut tree = if found.merges.is_empty() {
            first.clone()
        } else {
            Tree::lower(layout, &share::extras(&first, &found))
        };
        let representations = share::mark(&mut tree, &first, &found);
        let width_plan = DirPlan::new(&tree, H);
        let height_plan = DirPlan::new(&tree, V);
        let wraps = tree.wraps();
        let wrap_collapses = wraps
            .iter()
            .map(|&wrap| share::wrap_collapses(&tree, wrap))
            .collect();
        Ok(Self {
            first,
            base: Arc::new(Variant {
                tree,
                height_plan,
                representations,
            }),
            width_plan,
            wraps,
            wrap_collapses,
            variants: Variants {
                recent: Mutex::new(Vec::new()),
            },
        })
    }

    /// The tree at `arrangements`, kept or built, made the most recent.
    fn variant_at(&self, arrangements: &[Arrangement]) -> Arc<Variant<Id>> {
        if let Some(variant) = self.variants.take(arrangements) {
            return variant;
        }
        let variant = if arrangements == self.base.tree.arrangements() {
            self.base.clone()
        } else {
            Arc::new(self.variant(arrangements))
        };
        self.variants.keep(arrangements, variant.clone());
        variant
    }

    /// The tree, height plan and runs with each Wrap at its arrangement in `arrangements`.
    fn variant(&self, arrangements: &[Arrangement]) -> Variant<Id> {
        let mut first = self.first.clone();
        first.lay_out_wraps(arrangements);
        let plans = [DirPlan::new(&first, H), DirPlan::new(&first, V)];
        let found = share::find(&first, &plans);
        let mut tree = self.base.tree.clone();
        tree.lay_out_wraps(arrangements);
        for node in &mut tree.nodes {
            node.represented_by = None;
        }
        let representations = share::mark(&mut tree, &first, &found);
        let height_plan = DirPlan::new(&tree, V);
        Variant {
            tree,
            height_plan,
            representations,
        }
    }
}

/// A stack's tracks along its side: their sizes before allocation, whether the members'
/// overhang decided each gap, and the tracks in the stack's range.
struct StackAllocation {
    sizes: TrackSizes,
    overflowed: Vec<bool>,
    allocation: Allocation,
}

/// Solves a prepared layout: once, or again on another tree when a Wrap takes another count
/// than the tree it was solved on. The solve starts on the tree that the last solve ended on.
/// The width pass does not depend on the counts, so the result does not depend on the tree it
/// starts on, unless a `fit_aspect` grid caps a Wrap's width by heights that depend on its
/// count. The last attempt keeps its tree's counts.
pub(crate) fn solve<Id: Clone + Eq + Hash>(
    prepared: &Prepared<Id>,
    options: SolveOptions,
    measurements: &mut Measurements,
    answers: &mut Answers<'_, Id>,
    minimums: bool,
) -> Result<Solution<Id>, LayoutError> {
    let mut variant = prepared
        .variants
        .latest()
        .unwrap_or_else(|| prepared.base.clone());
    let mut carried = Carried::default();
    // Each restart settles at least one Wrap's count.
    for attempt in 0..=prepared.wraps.len() + 1 {
        let attempt = Attempt {
            minimums,
            recount: attempt <= prepared.wraps.len(),
            carried: std::mem::take(&mut carried),
        };
        let solver = Solver::new(
            prepared,
            &variant,
            options,
            &mut *measurements,
            &mut *answers,
            attempt,
        );
        match solver.run()? {
            Outcome::Solved(solution) => return Ok(*solution),
            Outcome::Recount(chosen, kept) => {
                variant = prepared.variant_at(&chosen);
                carried = kept;
            }
        }
    }
    unreachable!("the last attempt keeps its counts")
}

/// How a solve ended.
enum Outcome<Id> {
    Solved(Box<Solution<Id>>),
    /// A Wrap took another arrangement than the tree lays it out at.
    Recount(Vec<Arrangement>, Carried),
}

/// How one attempt of a solve runs.
struct Attempt {
    /// Whether a direction without a canvas size takes the layout's minimum instead of its
    /// max-content size.
    minimums: bool,
    /// Whether a Wrap that takes another count stops the attempt. The last attempt keeps its
    /// tree's counts.
    recount: bool,
    carried: Carried,
}

/// What an attempt that a recount stopped passes to the next: the solves that it completed and
/// the widths and overhangs that it raised, which the shared measurements keep.
#[derive(Default)]
struct Carried {
    solves: u32,
    raised_widths: Vec<RaisedWidth>,
    raised_overhangs: Vec<RaisedOverhang>,
    stopped_at_limit: bool,
}

/// The answers that a solve used, per node.
#[derive(Clone, Debug)]
pub(crate) struct Measurements {
    /// The height answer of each leaf and top or bottom slab.
    pub height: Vec<Option<Answer>>,
    /// The width answer that set each leaf's or left or right slab's width.
    pub width: Vec<Option<Answer>>,
    /// Each leaf's and slab's max-content answer per direction, by [`Direction::index`].
    pub max_content: Vec<[Option<f32>; 2]>,
    /// The largest overhang that each slab's answers gave in this measure, past the start and
    /// the end of its slot. The passes count it, so it only grows.
    pub overhang: Vec<[f32; 2]>,
    /// The height that each leaf or top or bottom slab needs at its final width, when an aspect
    /// fit changed that width after the height pass asked. The passes count it, so it only
    /// grows.
    pub fitted_height: Vec<f32>,
}

impl Measurements {
    pub fn new(count: usize) -> Self {
        Self {
            height: vec![None; count],
            width: vec![None; count],
            max_content: vec![[None; 2]; count],
            overhang: vec![[0.0; 2]; count],
            fitted_height: vec![0.0; count],
        }
    }
}

/// Where a solve gets its answers: the stored history first, then either the caller's
/// [`Measure`] or, for a solve that must not measure, the answers of the solve that the history
/// came from.
pub(crate) enum Answers<'a, Id> {
    /// Asks `measure` for what the history does not hold, and keeps its answers.
    Live {
        measure: &'a mut dyn Measure<Id>,
        history: &'a mut History<AnswerKey<Id>>,
    },
    /// Takes the answer of `last` where the history holds none, and marks the node stale.
    Stored {
        history: &'a History<AnswerKey<Id>>,
        last: &'a Measurements,
        /// Per node and measured direction, whether the solve took an answer from `last`. Widths
        /// and overhangs only grow within a solve, so one such answer can change the solution
        /// even when the later answers hold.
        stale: Vec<[bool; 2]>,
    },
}

impl<Id: Clone + Eq + Hash> Answers<'_, Id> {
    /// The answer for `node`, which has `id` and stores its answers under `key`, in the
    /// `measured` direction at `value`.
    fn ask(
        &mut self,
        node: usize,
        id: &Id,
        key: &AnswerKey<Id>,
        path: &[PathStep],
        measured: Direction,
        value: f32,
    ) -> Result<Option<Answer>, LayoutError> {
        match self {
            Answers::Live { measure, history } => {
                if let Some(answer) = history.lookup(key, measured, value) {
                    return Ok(Some(answer));
                }
                let measurement = match measured {
                    Direction::Vertical => measure.height(id, value),
                    Direction::Horizontal => measure.width(id, value),
                };
                let Some(measurement) = measurement else {
                    return Ok(None);
                };
                check_measurement(&measurement, value, path)?;
                let answer = Answer::new(&measurement, value);
                history.insert(key, measured, value, answer);
                Ok(Some(answer))
            }
            Answers::Stored {
                history,
                last,
                stale,
            } => {
                let stored = history.find(key, measured, value);
                let fallback = match measured {
                    Direction::Vertical => last.height[node],
                    Direction::Horizontal => last.width[node],
                };
                stale[node][measured.index()] |= stored.is_none() && fallback.is_some();
                Ok(stored.or(fallback))
            }
        }
    }

    /// The max-content size of `node` in `direction`.
    fn max_content(
        &mut self,
        node: usize,
        id: &Id,
        key: &AnswerKey<Id>,
        path: &[PathStep],
        direction: Direction,
    ) -> Result<Option<f32>, LayoutError> {
        match self {
            Answers::Live { measure, history } => {
                if let Some(answer) = history.max_content(key, direction) {
                    return Ok(answer);
                }
                let answer = measure.max_content(id, direction);
                if answer.is_some_and(|size| !size.is_finite() || size < 0.0) {
                    return Err(LayoutError::InvalidMeasurement {
                        path: path.to_vec(),
                    });
                }
                history.insert_max_content(key, direction, answer);
                Ok(answer)
            }
            Answers::Stored {
                history,
                last,
                stale,
            } => {
                let stored = history.find_max_content(key, direction);
                let fallback = last.max_content[node][direction.index()];
                stale[node][direction.index()] |= stored.is_none() && fallback.is_some();
                Ok(stored.unwrap_or(fallback))
            }
        }
    }

    /// The nodes that took an answer from the last solve in either direction.
    fn stale(&self) -> impl Iterator<Item = usize> + '_ {
        let stale: &[[bool; 2]] = match self {
            Answers::Stored { stale, .. } => stale,
            Answers::Live { .. } => &[],
        };
        (0..stale.len()).filter(|&node| stale[node].contains(&true))
    }
}

fn check_measurement(
    measurement: &Measurement,
    value: f32,
    path: &[PathStep],
) -> Result<(), LayoutError> {
    let invalid = || LayoutError::InvalidMeasurement {
        path: path.to_vec(),
    };
    if !measurement.size.is_finite() || measurement.size < 0.0 {
        return Err(invalid());
    }
    if measurement
        .overhang
        .iter()
        .any(|overhang| !overhang.is_finite() || *overhang < 0.0)
    {
        return Err(invalid());
    }
    if let Holds::Over(range) = &measurement.holds {
        let (low, high) = (*range.start(), *range.end());
        if low.is_nan() || high.is_nan() || low > high {
            return Err(invalid());
        }
        if !(low <= value && value <= high) {
            return Err(LayoutError::MeasurementOutsideRange {
                path: path.to_vec(),
                value,
            });
        }
    }
    Ok(())
}

/// The state of one direction: per node, then per track owner.
#[derive(Clone, Debug)]
struct DirState {
    /// The thickness of each slab that is thick in this direction.
    thickness: Vec<f32>,
    /// For each grid, its children's overflow on its leading and trailing edges.
    edge: Vec<[f32; 2]>,
    /// The overflow that each node exports into its parent's gaps, and for each subgrid with
    /// slabs or a flushed side, what its edges export.
    export: Vec<[f32; 2]>,
    /// The overflow that each node keeps inside its box.
    contain: Vec<[f32; 2]>,
    box_min: Vec<f32>,
    /// Each node's max-content box: its fit-content target, and its max-content contribution.
    box_fit: Vec<f32>,
    /// For each Wrap in the width pass, its column minimum and its column gap, which do not
    /// depend on its count.
    column_min: Vec<f32>,
    column_gap: Vec<f32>,
    slot: Vec<(f32, f32)>,
    boxed: Vec<(f32, f32)>,
    content: Vec<(f32, f32)>,
    owners: Vec<TrackOwnerState>,
    /// What the match groups did, kept only when the solve explains.
    provenance: Option<MatchProvenance>,
}

/// What the match groups of one direction did.
#[derive(Clone, Debug)]
struct MatchProvenance {
    /// For each Wrap in the width pass, its column gap before a match group raised it.
    column_gap_raised: Vec<Option<f32>>,
    /// The gap and slab match groups of the last pass over each root, by root.
    groups: Vec<(usize, MatchGroup)>,
}

#[derive(Clone, Debug, Default)]
struct TrackOwnerState {
    contributions: Vec<Vec<Contribution>>,
    gaps: Vec<f32>,
    /// Why each gap has its size, kept only when the solve explains.
    provenance: Option<GapProvenance>,
    sizes: Option<TrackSizes>,
    allocation: Option<Allocation>,
}

/// Why each gap of an owner has its size.
#[derive(Clone, Debug)]
struct GapProvenance {
    /// For each contribution, its value before a match group raised it.
    raised: Vec<Vec<Option<f32>>>,
    /// For each boundary, the contribution that decided it.
    decided_by: Vec<usize>,
}

impl DirState {
    fn new(nodes: usize, owners: usize, explain: bool) -> Self {
        Self {
            thickness: vec![0.0; nodes],
            edge: vec![[0.0; 2]; nodes],
            export: vec![[0.0; 2]; nodes],
            contain: vec![[0.0; 2]; nodes],
            box_min: vec![0.0; nodes],
            box_fit: vec![0.0; nodes],
            column_min: vec![0.0; nodes],
            column_gap: vec![0.0; nodes],
            slot: vec![(0.0, 0.0); nodes],
            boxed: vec![(0.0, 0.0); nodes],
            content: vec![(0.0, 0.0); nodes],
            owners: vec![TrackOwnerState::default(); owners],
            provenance: explain.then(|| MatchProvenance {
                column_gap_raised: vec![None; nodes],
                groups: Vec::new(),
            }),
        }
    }
}

/// Solves every root of a prepared tree.
struct Solver<'a, 's, Id> {
    prepared: &'a Prepared<Id>,
    tree: &'a Tree<Id>,
    plans: [&'a DirPlan; 2],
    options: SolveOptions,
    measurements: &'a mut Measurements,
    answers: &'a mut Answers<'s, Id>,
    states: [DirState; 2],
    /// For each horizontal owner, the cap on its Fr tracks from an aspect fit pass.
    caps: Vec<f32>,
    snap: Option<Snap>,
    explanation: Explanation,
    /// The solves that the attempts before this one completed.
    solves_before: u32,
    /// Whether a direction without a canvas size takes the layout's minimum instead of its
    /// max-content size.
    minimums: bool,
    /// The arrangement that the tree lays out each Wrap at, and the one that each Wrap took.
    arrangements: Vec<Arrangement>,
    chosen: Vec<Arrangement>,
    /// Whether a Wrap that takes another count stops the solve, so that it runs again on
    /// another tree. Otherwise each Wrap keeps the tree's count.
    recount: bool,
    /// Whether a slab's overhang grew in this solve, after the pass that counts it.
    overhang_grew: bool,
}

impl<'a, 's, Id: Clone + Eq + Hash> Solver<'a, 's, Id> {
    fn new(
        prepared: &'a Prepared<Id>,
        variant: &'a Variant<Id>,
        options: SolveOptions,
        measurements: &'a mut Measurements,
        answers: &'a mut Answers<'s, Id>,
        attempt: Attempt,
    ) -> Self {
        let Attempt {
            minimums,
            recount,
            carried,
        } = attempt;
        let tree = &variant.tree;
        let plans = [&prepared.width_plan, &variant.height_plan];
        let count = tree.nodes.len();
        let states = [0, 1].map(|d| DirState::new(count, plans[d].owners.len(), options.explain));
        let arrangements = tree.arrangements();
        Self {
            prepared,
            tree,
            plans,
            options,
            measurements,
            answers,
            states,
            caps: vec![f32::INFINITY; plans[0].owners.len()],
            snap: Snap::new(options.pixel_ratio),
            explanation: Explanation {
                representations: variant.representations.clone(),
                raised_widths: carried.raised_widths,
                raised_overhangs: carried.raised_overhangs,
                stopped_at_limit: carried.stopped_at_limit,
                ..Explanation::default()
            },
            solves_before: carried.solves,
            minimums,
            chosen: arrangements.clone(),
            arrangements,
            recount,
            overhang_grew: false,
        }
    }

    /// Whether a Wrap took another count than the tree lays it out at, which stops the solve.
    fn recounted(&self) -> bool {
        self.recount && self.chosen != self.arrangements
    }

    /// What this attempt passes to the next after a recount.
    fn carried(&self) -> Carried {
        Carried {
            solves: self.explanation.solves,
            raised_widths: self.explanation.raised_widths.clone(),
            raised_overhangs: self.explanation.raised_overhangs.clone(),
            stopped_at_limit: self.explanation.stopped_at_limit,
        }
    }

    /// Solves the layout's root at the options' canvas, then each overlay inside its host.
    fn run(mut self) -> Result<Outcome<Id>, LayoutError> {
        let tree = self.tree;
        let padding = self.padding();
        let canvas = [self.options.width, self.options.height];
        let available = [0, 1].map(|d| {
            let [leading, trailing] = padding[d];
            canvas[d].map(|canvas| (canvas - leading - trailing).max(0.0))
        });
        let origin = padding.map(|[leading, _]| leading);
        self.solve_root(tree.roots[0], origin, available)?;
        if self.recounted() {
            return Ok(Outcome::Recount(self.chosen.clone(), self.carried()));
        }
        for overlay in &tree.overlays {
            let frame = Rect::from_ranges(
                self.states[0].content[overlay.host],
                self.states[1].content[overlay.host],
            );
            let [left, top, right, bottom] = overlay.rect;
            let origin = [frame.x + left * frame.width, frame.y + top * frame.height];
            let size = [(right - left) * frame.width, (bottom - top) * frame.height];
            self.solve_root(overlay.root, origin, size.map(Some))?;
            if self.recounted() {
                return Ok(Outcome::Recount(self.chosen.clone(), self.carried()));
            }
        }
        self.finish()
            .map(|solution| Outcome::Solved(Box::new(solution)))
    }

    /// The canvas padding before and after the root, per direction.
    fn padding(&self) -> [[f32; 2]; 2] {
        Direction::BOTH.map(|direction| {
            direction
                .sides()
                .map(|side| self.options.padding.side(side))
        })
    }

    /// Runs the passes for one root until no measured width or overhang grows.
    fn solve_root(
        &mut self,
        root: usize,
        origin: [f32; 2],
        available: [Option<f32>; 2],
    ) -> Result<(), LayoutError> {
        let mut solves = 0;
        self.measure_max_content(root)?;
        loop {
            solves += 1;
            self.caps.fill(f32::INFINITY);
            self.overhang_grew = false;
            if !self.passes(root, origin, available)?
                || !self.fit_aspect(root, origin, available)?
            {
                // The solve stopped for a recount, so this one did not complete.
                solves -= 1;
                break;
            }
            self.contain_fit(root);
            let fitted = self.measure_fitted_heights(root)?;
            let grew = self.measure_widths(root)? || fitted || self.overhang_grew;
            if !grew {
                break;
            }
            if solves >= self.options.max_solves {
                self.explanation.stopped_at_limit = true;
                break;
            }
        }
        if root == self.tree.roots[0] {
            self.explanation.solves = self.solves_before + solves;
        }
        Ok(())
    }

    /// The width pass, then the heights and the height pass. Returns `false` without measuring
    /// when a Wrap took another count in the width pass, which stops the solve before anything
    /// is measured on the wrong rows.
    fn passes(
        &mut self,
        root: usize,
        origin: [f32; 2],
        available: [Option<f32>; 2],
    ) -> Result<bool, LayoutError> {
        self.pass(H, root, origin[0], available[0]);
        if self.recounted() {
            return Ok(false);
        }
        self.measure_heights(root)?;
        self.pass(V, root, origin[1], available[1]);
        Ok(true)
    }

    /// One pass in `direction` over `root`.
    fn pass(&mut self, direction: Direction, root: usize, origin: f32, available: Option<f32>) {
        self.measure_up(direction, root);
        self.allocate(direction, root, origin, available);
        self.place_slabs(direction, root);
    }

    fn measure_up(&mut self, direction: Direction, root: usize) {
        let tree = self.tree;
        let plan = self.plans[direction.index()];
        let members: Vec<usize> = tree.members(root).collect();
        {
            let state = &mut self.states[direction.index()];
            if let Some(provenance) = &mut state.provenance {
                provenance
                    .groups
                    .retain(|(group_root, _)| *group_root != root);
            }
            for &node in &members {
                state.edge[node] = [0.0; 2];
                state.export[node] = [0.0; 2];
                state.contain[node] = [0.0; 2];
            }
        }
        for &node in &members {
            if let NodeKind::Slab { side, .. } = tree.nodes[node].kind
                && side.direction() == direction
            {
                // A slab that another slab draws takes no space.
                if tree.nodes[node].represented_by.is_some() {
                    self.states[direction.index()].thickness[node] = 0.0;
                    continue;
                }
                let thickness = self.measured_min(node, direction);
                let thickness = self.snap.map_or(thickness, |snap| snap.up(thickness));
                self.states[direction.index()].thickness[node] = thickness;
            }
        }
        self.match_slabs(direction, &members);
        // A stack's band is as thick as its thickest member, and takes no space when another
        // stack draws it.
        for &node in &members {
            if let Some(stack) = tree.stack(node)
                && stack.side.direction() == direction
            {
                let state = &mut self.states[direction.index()];
                state.thickness[node] = if tree.nodes[node].represented_by.is_some() {
                    0.0
                } else {
                    stack
                        .members
                        .iter()
                        .map(|&member| state.thickness[member])
                        .fold(0.0f32, f32::max)
                };
            }
        }
        for &node in members.iter().rev() {
            match &tree.nodes[node].kind {
                NodeKind::Leaf { .. } => self.finish_overflow(direction, node),
                NodeKind::Grid(_) if plan.owner[node] == node => {
                    let owner = plan.owner_plan[node].expect("owners have plans");
                    self.owner_overflow(direction, owner);
                    self.finish_overflow(direction, node);
                }
                _ => {}
            }
        }
        let owners: Vec<usize> = (0..plan.owners.len())
            .filter(|&owner| tree.nodes[plan.owners[owner].node].root == root)
            .collect();
        self.gaps(direction, &owners, &members);
        for &node in &members {
            if matches!(tree.nodes[node].kind, NodeKind::Leaf { .. }) {
                let content = self
                    .measured_min(node, direction)
                    .max(self.slab_length_min(direction, node));
                let fit = self.fit_content(node, direction, content);
                self.set_box(direction, node, content, fit);
            }
        }
        for &owner in &owners {
            self.owner_track_sizes(direction, owner);
        }
    }

    /// A leaf's or slab's max-content size in `direction`: its measured answer, or else the
    /// size it declares.
    fn max_content_of(&self, node: usize, direction: Direction) -> Option<f32> {
        self.measurements.max_content[node][direction.index()].or(self.tree.nodes[node]
            .max_content
            .map(|size| size.along(direction)))
    }

    /// A leaf's or slab's fit-content target in `direction`: its max-content size within its
    /// minimum `min` and its maximum, or `min` when it has none.
    fn fit_content(&self, node: usize, direction: Direction, min: f32) -> f32 {
        let max = self.tree.nodes[node].max.along(direction);
        self.max_content_of(node, direction)
            .map_or(min, |size| size.min(max).max(min))
    }

    /// A leaf's or slab's minimum size in `direction`, raised by its measurement.
    fn measured_min(&self, node: usize, direction: Direction) -> f32 {
        let declared = self.tree.nodes[node].min.along(direction);
        let (answer, fitted) = match direction {
            Direction::Horizontal => (self.measurements.width[node], 0.0),
            Direction::Vertical => (
                self.measurements.height[node],
                self.measurements.fitted_height[node],
            ),
        };
        answer
            .map_or(declared, |answer| declared.max(answer.size))
            .max(fitted)
    }

    /// Raises each slab that a match group holds to the group's thickest slab.
    fn match_slabs(&mut self, direction: Direction, members: &[usize]) {
        let tree = self.tree;
        let state = &mut self.states[direction.index()];
        for &grid in members {
            let Some(node) = tree.grid(grid) else {
                continue;
            };
            for tag in &node.rules.matches {
                let slabs: Vec<usize> = members
                    .iter()
                    .copied()
                    .filter(|&slab| match &tree.nodes[slab].kind {
                        NodeKind::Slab {
                            side, tag: Some(t), ..
                        } => {
                            t == tag
                                && side.direction() == direction
                                && tree.nodes[slab].represented_by.is_none()
                                && tree.is_within(slab, grid)
                        }
                        _ => false,
                    })
                    .collect();
                let thickest = slabs
                    .iter()
                    .map(|&slab| state.thickness[slab])
                    .fold(0.0f32, f32::max);
                if let Some(provenance) = &mut state.provenance
                    && !slabs.is_empty()
                {
                    provenance.groups.push((
                        tree.nodes[grid].root,
                        MatchGroup {
                            tag: tag.clone(),
                            direction,
                            parts: MatchedParts::Slabs,
                            size: thickest,
                            grid: tree.nodes[grid].path.clone(),
                        },
                    ));
                }
                for slab in slabs {
                    state.thickness[slab] = thickest;
                }
            }
        }
    }

    fn slab_sum(&self, direction: Direction, node: usize, side: Side) -> f32 {
        let state = &self.states[direction.index()];
        self.tree.nodes[node].slabs[side.index()]
            .iter()
            .map(|&slab| state.thickness[slab])
            .sum()
    }

    /// Sets what a leaf or an owner grid exports and contains, from its edge overflow, its
    /// slabs and the overhang of its other slabs, by its overflow policy. A root contains what
    /// it would propagate.
    fn finish_overflow(&mut self, direction: Direction, node: usize) {
        for (k, side) in direction.sides().into_iter().enumerate() {
            let padding = self.tree.nodes[node].padding[side.index()];
            let slabs = self.slab_sum(direction, node, side);
            let base = self.slab_base(direction, node, k);
            let reach = (base + slabs).max(self.overhang_reach(direction, node, k));
            let policy = self.policy(node, side);
            let state = &mut self.states[direction.index()];
            match policy {
                // Padding, then the slabs, then the children's overflow, inside the box.
                Overflow::Contain => state.contain[node][k] = padding + reach,
                // The padding inside the box, and the rest outside it.
                Overflow::Propagate => {
                    state.contain[node][k] = padding;
                    state.export[node][k] = reach - padding;
                }
                Overflow::Flush => state.contain[node][k] = padding,
            }
        }
    }

    /// How far the overhang of a node's slabs on its other sides reaches past its content edge
    /// on side `k` of `direction`: each slab's overhang past its end there, plus the corner that
    /// a full slab spans. In the width pass, a Wrap child counts the slabs that collapse at this
    /// count too, since another count draws them.
    fn overhang_reach(&self, direction: Direction, node: usize, k: usize) -> f32 {
        let tree = self.tree;
        let wrap_child = direction == H
            && tree.nodes[node]
                .parent
                .is_some_and(|parent| tree.wrap(parent).is_some());
        let reach = direction
            .cross()
            .sides()
            .into_iter()
            .flat_map(|side| tree.nodes[node].slabs[side.index()].iter().copied())
            .filter(|&slab| match tree.nodes[slab].represented_by {
                None => true,
                Some(by) => wrap_child && !share::is_merged(tree, by),
            })
            .map(|band| {
                let overhang = self.band_overhang(band, k);
                if tree.band_extent(band) == Extent::Full && overhang > 0.0 {
                    self.corner(direction, node, k) + overhang
                } else {
                    overhang
                }
            })
            .fold(0.0f32, f32::max);
        self.snap.map_or(reach, |snap| snap.up(reach))
    }

    /// How far a band's content reaches past its end `k` along its side: a slab's measured
    /// overhang, or for a stack, its first member's overhang past the start and its last
    /// member's past the end.
    fn band_overhang(&self, band: usize, k: usize) -> f32 {
        let member = match self.tree.stack(band) {
            Some(stack) if k == 0 => stack.members.first(),
            Some(stack) => stack.members.last(),
            None => Some(&band),
        };
        member.map_or(0.0, |&member| self.measurements.overhang[member][k])
    }

    /// A node's overflow policy on `side`. A root contains what it would propagate.
    fn policy(&self, node: usize, side: Side) -> Overflow {
        let policy = self.tree.nodes[node].overflow[side.index()];
        if self.tree.nodes[node].parent.is_none() && policy == Overflow::Propagate {
            Overflow::Contain
        } else {
            policy
        }
    }

    /// How far from a node's content edge on side `k` its innermost slab starts: past its
    /// children's overflow, and on a side that the node does not contain, past its padding too.
    fn slab_base(&self, direction: Direction, node: usize, k: usize) -> f32 {
        let side = direction.sides()[k];
        let edge = self.states[direction.index()].edge[node][k];
        match self.policy(node, side) {
            Overflow::Contain => edge,
            _ => edge.max(self.tree.nodes[node].padding[side.index()]),
        }
    }

    /// How far a full slab on a node's other sides reaches past its content on side `k` of
    /// `direction`. A top or bottom slab spans every left and right slab, and a left or right
    /// slab stops at the first full top or bottom slab.
    fn corner(&self, direction: Direction, node: usize, k: usize) -> f32 {
        let tree = self.tree;
        let side = direction.sides()[k];
        let state = &self.states[direction.index()];
        let slabs = tree.nodes[node].slabs[side.index()].iter();
        let counted: f32 = match direction {
            Direction::Horizontal => slabs.map(|&slab| state.thickness[slab]).sum(),
            Direction::Vertical => slabs
                .take_while(|&&slab| tree.band_extent(slab) != Extent::Full)
                .map(|&slab| state.thickness[slab])
                .sum(),
        };
        self.slab_base(direction, node, k) + counted
    }

    /// The smallest content length along `direction` that a node's slabs need: each slab's
    /// minimum length along its side, less the corners that a full slab also spans.
    fn slab_length_min(&self, direction: Direction, node: usize) -> f32 {
        let tree = self.tree;
        let corners = self.corner(direction, node, 0) + self.corner(direction, node, 1);
        direction
            .cross()
            .sides()
            .into_iter()
            .flat_map(|side| tree.nodes[node].slabs[side.index()].iter().copied())
            .filter(|&slab| tree.nodes[slab].represented_by.is_none())
            .map(|band| {
                // A stack's minimum length holds its members' minimums and its gaps.
                let length = tree.nodes[band].min.along(direction);
                if tree.band_extent(band) == Extent::Full {
                    length - corners
                } else {
                    length
                }
            })
            .fold(0.0f32, f32::max)
    }

    /// The edge overflow of an owner and of its subgrids, and what each subgrid's edges export,
    /// innermost first.
    fn owner_overflow(&mut self, direction: Direction, owner: usize) {
        let tree = self.tree;
        let plan = self.plans[direction.index()];
        let owner_plan = &plan.owners[owner];
        for position in (0..owner_plan.grids.len()).rev() {
            let grid = owner_plan.grids[position];
            let start = plan.offset[grid];
            let end = start + self.track_count(direction, grid);
            let mut edge = [0.0f32; 2];
            for member in &owner_plan.members[position] {
                let item = &owner_plan.items[member.item];
                for (k, value) in edge.iter_mut().enumerate() {
                    let at_edge = if k == 0 {
                        item.start == start
                    } else {
                        item.end == end
                    };
                    if at_edge && !member.inner[k] {
                        *value = value.max(self.states[direction.index()].export[item.node][k]);
                    }
                }
            }
            self.states[direction.index()].edge[grid] = edge;
            // The owner's export is set by its overflow policy, in `finish_overflow`.
            if position == 0 {
                continue;
            }
            for (k, side) in direction.sides().into_iter().enumerate() {
                // The children's overflow goes past the subgrid only when the subgrid takes it
                // in, and the overhang of its slabs on its other sides always does.
                let reach = self.overhang_reach(direction, grid, k);
                let export = if tree.nodes[grid].overflow[side.index()] == Overflow::Flush {
                    0.0
                } else if tree.absorbs(grid, side) {
                    (edge[k] + self.slab_sum(direction, grid, side)).max(reach)
                } else {
                    reach
                };
                self.states[direction.index()].export[grid][k] = export;
            }
        }
    }

    /// A grid's tracks in `direction`'s problem. In the width pass, a Wrap has one column
    /// whatever its count, since its width does not depend on the count.
    fn declared_tracks(&self, direction: Direction, grid: usize) -> &'a [Track] {
        let tree = self.tree;
        if direction == H && tree.wrap(grid).is_some() {
            &WRAP_COLUMN
        } else {
            &tree
                .grid(grid)
                .expect("owners and subgrids are grids")
                .tracks[direction.index()]
        }
    }

    /// The number of tracks that a grid has in `direction`'s problem.
    fn track_count(&self, direction: Direction, grid: usize) -> usize {
        self.declared_tracks(direction, grid).len()
    }

    /// The thickness of a Wrap child's left and right slabs that its box contains but that
    /// another child's slab draws at the current count. A merged slab draws its members at every
    /// count, so they do not count.
    fn contained_collapses(&self, child: usize) -> f32 {
        let tree = self.tree;
        H.sides()
            .into_iter()
            .filter(|&side| self.policy(child, side) == Overflow::Contain)
            .flat_map(|side| tree.nodes[child].slabs[side.index()].iter().copied())
            .filter(|&slab| {
                tree.nodes[slab]
                    .represented_by
                    .is_some_and(|by| !share::is_merged(tree, by))
            })
            .map(|band| {
                let thickness = match tree.stack(band) {
                    Some(stack) => stack
                        .members
                        .iter()
                        .map(|&member| self.measured_min(member, H))
                        .fold(0.0f32, f32::max),
                    None => self.measured_min(band, H),
                };
                self.snap.map_or(thickness, |snap| snap.up(thickness))
            })
            .sum()
    }

    /// The gap that a Wrap declares between its columns and between its rows.
    fn wrap_gap(&self, node: usize) -> &'a Gap {
        let plan = self.plans[0];
        &plan.owners[plan.owner_plan[node].expect("Wraps own their tracks")].gaps[0]
    }

    /// The problem of a track owner, with the current sizes and overflow of its items.
    fn problem(&self, direction: Direction, owner: usize) -> TrackOwnerProblem<'a> {
        let tree = self.tree;
        let plan = self.plans[direction.index()];
        let owner_plan = &plan.owners[owner];
        let state = &self.states[direction.index()];
        // A Wrap's width does not depend on its count, so in the width pass each child counts
        // the collapsing slabs that it contains as drawn, though another child draws them at
        // this count.
        let wrap_width = direction == H && tree.wrap(owner_plan.node).is_some();
        let tracks = self
            .declared_tracks(direction, owner_plan.node)
            .iter()
            .map(|track| track.size)
            .collect();
        let grids = owner_plan
            .grids
            .iter()
            .zip(&owner_plan.gaps)
            .map(|(&grid, gap)| {
                let start = plan.offset[grid];
                let count = self.track_count(direction, grid);
                GapGrid {
                    start,
                    end: start + count,
                    min: gap.min,
                    clearance: gap.clearance,
                }
            })
            .collect();
        let items = owner_plan
            .items
            .iter()
            .map(|item| {
                let effective = |k: usize| {
                    if item.absorbed_by[k].is_some() {
                        0.0
                    } else {
                        state.export[item.node][k]
                    }
                };
                let hidden = if wrap_width {
                    self.contained_collapses(item.node)
                } else {
                    0.0
                };
                let min = if item.container {
                    tree.nodes[item.node]
                        .min
                        .along(direction)
                        .max(self.slab_length_min(direction, item.node))
                } else {
                    state.box_min[item.node] + hidden
                };
                let max = if item.container {
                    min
                } else {
                    (state.box_fit[item.node] + hidden).max(min)
                };
                TrackOwnerItem {
                    start: item.start,
                    end: item.end,
                    min,
                    max,
                    lead: effective(0),
                    trail: effective(1),
                    lane: item.lane.clone(),
                    chain: &item.chain,
                }
            })
            .collect();
        TrackOwnerProblem {
            tracks,
            groups: &owner_plan.groups,
            grids,
            items,
        }
    }

    /// The gaps of every owner of a root, with matched gaps raised to their group's largest.
    fn gaps(&mut self, direction: Direction, owners: &[usize], members: &[usize]) {
        let tree = self.tree;
        let plan = self.plans[direction.index()];
        let explain = self.options.explain;
        for &owner in owners {
            let contributions = tracks::contributions(&self.problem(direction, owner));
            let provenance = explain.then(|| GapProvenance {
                raised: contributions
                    .iter()
                    .map(|list| vec![None; list.len()])
                    .collect(),
                decided_by: Vec::new(),
            });
            let state = &mut self.states[direction.index()].owners[owner];
            state.contributions = contributions;
            state.provenance = provenance;
        }
        // A Wrap's columns are not tracks of its owner in the width pass, so its column gap joins
        // the match groups on its own.
        let wraps: Vec<usize> = if direction == H {
            members
                .iter()
                .copied()
                .filter(|&node| tree.wrap(node).is_some())
                .collect()
        } else {
            Vec::new()
        };
        for &wrap in &wraps {
            let gap = self.wrap_column_gap(wrap);
            let state = &mut self.states[0];
            state.column_gap[wrap] = gap;
            if let Some(provenance) = &mut state.provenance {
                provenance.column_gap_raised[wrap] = None;
            }
        }
        let gap_tag = |owner: usize, grid: usize| {
            let owner_plan = &plan.owners[owner];
            let node = owner_plan.grids[grid];
            owner_plan.gaps[grid].tag.clone().map(|tag| (node, tag))
        };
        for &matcher in members {
            let Some(grid) = tree.grid(matcher) else {
                continue;
            };
            for tag in &grid.rules.matches {
                let matched_wraps: Vec<usize> = wraps
                    .iter()
                    .copied()
                    .filter(|&wrap| {
                        self.wrap_gap(wrap).tag.as_ref() == Some(tag)
                            && tree.is_within(wrap, matcher)
                    })
                    .collect();
                let mut largest = matched_wraps
                    .iter()
                    .map(|&wrap| self.states[0].column_gap[wrap])
                    .fold(0.0f32, f32::max);
                let mut members_of_group = Vec::new();
                for &owner in owners {
                    let state = &self.states[direction.index()].owners[owner];
                    for (boundary, list) in state.contributions.iter().enumerate() {
                        for (index, contribution) in list.iter().enumerate() {
                            if let Some((node, t)) = gap_tag(owner, contribution.grid)
                                && &t == tag
                                && tree.is_within(node, matcher)
                            {
                                largest = largest.max(contribution.value);
                                members_of_group.push((owner, boundary, index));
                            }
                        }
                    }
                }
                for &wrap in &matched_wraps {
                    let state = &mut self.states[0];
                    if state.column_gap[wrap] < largest {
                        if let Some(provenance) = &mut state.provenance {
                            provenance.column_gap_raised[wrap]
                                .get_or_insert(state.column_gap[wrap]);
                        }
                        state.column_gap[wrap] = largest;
                    }
                }
                if let Some(provenance) = &mut self.states[direction.index()].provenance
                    && (!members_of_group.is_empty() || !matched_wraps.is_empty())
                {
                    provenance.groups.push((
                        tree.nodes[matcher].root,
                        MatchGroup {
                            tag: tag.clone(),
                            direction,
                            parts: MatchedParts::Gaps,
                            size: largest,
                            grid: tree.nodes[matcher].path.clone(),
                        },
                    ));
                }
                for (owner, boundary, index) in members_of_group {
                    let state = &mut self.states[direction.index()].owners[owner];
                    let contribution = &mut state.contributions[boundary][index];
                    if contribution.value < largest {
                        if let Some(provenance) = &mut state.provenance {
                            provenance.raised[boundary][index].get_or_insert(contribution.value);
                        }
                        contribution.value = largest;
                    }
                }
            }
        }
        for &owner in owners {
            let state = &mut self.states[direction.index()].owners[owner];
            state.gaps.clear();
            for list in &state.contributions {
                let (index, value) = list
                    .iter()
                    .enumerate()
                    .map(|(index, c)| (index, c.value))
                    .fold(
                        (0, 0.0f32),
                        |best, next| if next.1 > best.1 { next } else { best },
                    );
                state.gaps.push(value);
                if let Some(provenance) = &mut state.provenance {
                    provenance.decided_by.push(index);
                }
            }
        }
    }

    fn owner_track_sizes(&mut self, direction: Direction, owner: usize) {
        let tree = self.tree;
        let node = self.plans[direction.index()].owners[owner].node;
        let problem = self.problem(direction, owner);
        let aspect: Vec<Option<f32>> = match direction {
            Direction::Horizontal => Vec::new(),
            Direction::Vertical => tree.grid(node).expect("owners are grids").tracks[1]
                .iter()
                .map(|track| match track.size {
                    TrackSize::Aspect { column, ratio } => {
                        Some(self.column_width(node, column) / ratio)
                    }
                    _ => None,
                })
                .collect(),
        };
        let gaps = self.states[direction.index()].owners[owner].gaps.clone();
        let sizes = tracks::track_sizes(&problem, &gaps, &aspect, self.options.explain);
        let (content, fit) = if direction == H && tree.wrap(node).is_some() {
            self.wrap_lengths(node, &sizes)
        } else {
            (
                tracks::extent(&sizes.base, &gaps),
                tracks::max_content(&problem.tracks, &sizes, &gaps),
            )
        };
        let content = content
            .max(tree.nodes[node].min.along(direction))
            .max(self.slab_length_min(direction, node));
        let fit = fit.min(tree.nodes[node].max.along(direction)).max(content);
        self.set_box(direction, node, content, fit);
        self.states[direction.index()].owners[owner].sizes = Some(sizes);
    }

    /// Sets a node's minimum and max-content boxes from its content's: the content with what
    /// the node keeps inside on both sides.
    fn set_box(&mut self, direction: Direction, node: usize, content: f32, fit: f32) {
        let state = &mut self.states[direction.index()];
        let contained = state.contain[node][0] + state.contain[node][1];
        state.box_min[node] = content + contained;
        state.box_fit[node] = fit + contained;
    }

    /// A Wrap's minimum and max-content widths, which do not depend on its count: one column at
    /// its column minimum, and every child on one row in equal columns. Records the column
    /// minimum and the column gap, the largest gap that any two children could need. A child's
    /// left and right slabs that collapse along the rows count only where they are drawn: on
    /// the first or last child of a row, which faces the Wrap's edge rather than a gap.
    /// A Wrap's column gap in the width pass, before matching: the largest gap that any two of
    /// its children could need side by side, at any count.
    fn wrap_column_gap(&self, node: usize) -> f32 {
        let tree = self.tree;
        let children = &tree.grid(node).expect("wraps are grids").children;
        let collapses = &self.prepared.wrap_collapses[self.wrap_index(node)];
        let state = &self.states[0];
        // A child's overflow on side `k` when its collapsing slabs there are not drawn.
        let facing = |child: usize, k: usize| {
            let side = H.sides()[k];
            let collapsing: f32 = tree.nodes[child].slabs[side.index()]
                .iter()
                .filter(|&&slab| {
                    !share::is_merged(tree, slab)
                        && tree
                            .slab_tag(slab)
                            .is_some_and(|tag| collapses[k].contains(tag))
                })
                .map(|&slab| state.thickness[slab])
                .sum();
            (state.export[child][k] - collapsing).max(0.0)
        };
        let count = children.len();
        if count < 2 {
            return 0.0;
        }
        let declared = self.wrap_gap(node);
        let trailing = children[..count - 1]
            .iter()
            .map(|&child| facing(child, 1))
            .fold(0.0f32, f32::max);
        let leading = children[1..]
            .iter()
            .map(|&child| facing(child, 0))
            .fold(0.0f32, f32::max);
        declared.min.max(trailing + leading + declared.clearance)
    }

    /// A Wrap's column minimum and max-content width in the width pass, from its column gap,
    /// which [`gaps`](Self::gaps) found and matched.
    fn wrap_lengths(&mut self, node: usize, sizes: &TrackSizes) -> (f32, f32) {
        let tree = self.tree;
        let settings = tree.wrap(node).expect("wraps have settings");
        let children = &tree.grid(node).expect("wraps are grids").children;
        let count = children.len();
        let state = &self.states[0];
        let gap = state.column_gap[node];
        let (minimum, fit) = if settings.flow {
            let line: f32 = children.iter().map(|&child| state.box_fit[child]).sum();
            (sizes.base[0], line + count.saturating_sub(1) as f32 * gap)
        } else {
            let minimum = settings.min_column.max(sizes.base[0]);
            let columns = self.most_columns(node);
            let widest = minimum.max(sizes.limits[0]);
            (
                minimum,
                columns as f32 * widest + (columns - 1) as f32 * gap,
            )
        };
        self.states[0].column_min[node] = minimum;
        (minimum, fit)
    }

    /// The solved width of a grid's column, from the width pass.
    fn column_width(&self, grid: usize, column: usize) -> f32 {
        let plan = self.plans[0];
        let owner = plan.owner[grid];
        let Some(owner_plan) = plan.owner_plan[owner] else {
            return 0.0;
        };
        let track = plan.offset[grid] + column;
        self.states[0].owners[owner_plan]
            .allocation
            .as_ref()
            .and_then(|allocation| allocation.sizes.get(track).copied())
            .unwrap_or(0.0)
    }

    fn allocate(&mut self, direction: Direction, root: usize, origin: f32, available: Option<f32>) {
        let state = &mut self.states[direction.index()];
        let open = if self.minimums {
            state.box_min[root]
        } else {
            state.box_fit[root]
        };
        let length = available.unwrap_or(open);
        let range = self.snap.map_or((origin, origin + length), |snap| {
            snap.range((origin, origin + length))
        });
        state.slot[root] = range;
        // A root smaller than the canvas is placed in it, and one too large for it keeps its
        // minimum and overflows it from the start.
        let boxed = if length < state.box_min[root] {
            let boxed = (range.0, range.0 + state.box_min[root]);
            self.snap.map_or(boxed, |snap| snap.range(boxed))
        } else {
            let boxed = place(
                range,
                state.box_min[root],
                state.box_fit[root],
                box_max(self.tree, state, root, direction),
                self.tree.nodes[root].place[direction.index()],
            );
            self.snap.map_or(boxed, |snap| snap.range(boxed))
        };
        state.boxed[root] = boxed;
        let content = (
            boxed.0 + state.contain[root][0],
            boxed.1 - state.contain[root][1],
        );
        state.content[root] = content;
        if let Some(owner) = self.plans[direction.index()].owner_plan[root] {
            self.allocate_owner(direction, owner, content);
        }
    }

    fn allocate_owner(&mut self, direction: Direction, owner: usize, range: (f32, f32)) {
        let tree = self.tree;
        let plan = self.plans[direction.index()];
        let owner_plan = &plan.owners[owner];
        let node = owner_plan.node;
        let settings = (direction == H).then(|| tree.wrap(node)).flatten();
        let mut allocation = match settings {
            Some(settings) if settings.flow => self.flow_lines(node, range.1 - range.0),
            Some(_) => self.wrap_columns(node, range.1 - range.0),
            None => self.owner_allocation(direction, owner, range.1 - range.0),
        };
        let flow = settings.is_some_and(|settings| settings.flow);
        let wrap = settings.is_some();
        for start in &mut allocation.starts {
            *start += range.0;
        }
        let span = |first: usize, last: usize| {
            (
                allocation.starts[first],
                allocation.starts[last] + allocation.sizes[last],
            )
        };
        let mut nested = Vec::new();
        {
            let state = &mut self.states[direction.index()];
            for &subgrid in &owner_plan.grids[1..] {
                let first = plan.offset[subgrid];
                let last = first + tree.slot_range(subgrid, direction).len() - 1;
                let range = span(first, last);
                state.slot[subgrid] = range;
                state.boxed[subgrid] = range;
                state.content[subgrid] = range;
            }
            let items = owner_plan.items.iter().filter(|item| !item.container);
            for (index, item) in items.enumerate() {
                let child = item.node;
                // A Wrap's children take its columns in order, row by row, and a flow's each
                // take the span that their line gives them.
                let slot = if flow {
                    span(index, index)
                } else if wrap {
                    let column = index % allocation.sizes.len();
                    span(column, column)
                } else {
                    span(item.start, item.end - 1)
                };
                let boxed = place(
                    slot,
                    state.box_min[child],
                    state.box_fit[child],
                    box_max(tree, state, child, direction),
                    tree.nodes[child].place[direction.index()],
                );
                let boxed = self.snap.map_or(boxed, |snap| snap.range(boxed));
                state.slot[child] = slot;
                state.boxed[child] = boxed;
                let content = (
                    boxed.0 + state.contain[child][0],
                    boxed.1 - state.contain[child][1],
                );
                state.content[child] = content;
                if let Some(child_owner) = plan.owner_plan[child] {
                    nested.push((child_owner, content));
                }
            }
            state.owners[owner].allocation = Some(allocation);
        }
        for (child_owner, content) in nested {
            self.allocate_owner(direction, child_owner, content);
        }
    }

    /// An owner's tracks in `length`, from its track sizes and gaps.
    fn owner_allocation(&self, direction: Direction, owner: usize, length: f32) -> Allocation {
        let node = self.plans[direction.index()].owners[owner].node;
        let grid = self.tree.grid(node).expect("owners are grids");
        let tracks: Vec<TrackSize> = grid.tracks[direction.index()]
            .iter()
            .map(|track| track.size)
            .collect();
        let cap = match direction {
            Direction::Horizontal => self.caps[owner],
            Direction::Vertical => f32::INFINITY,
        };
        let state = &self.states[direction.index()];
        let sizes = state.owners[owner]
            .sizes
            .as_ref()
            .expect("track sizes come before allocation");
        let mut allocation = tracks::allocate(
            &tracks,
            sizes,
            &state.owners[owner].gaps,
            length,
            grid.distribute[direction.index()],
            cap,
        );
        if let Some(snap) = self.snap {
            snap.tracks(&mut allocation, &sizes.base, &state.owners[owner].gaps);
        }
        allocation
    }

    /// A Wrap's columns in `width`: as many as fit at its column minimum and gap, at least one
    /// and at most its children and its maximum, sharing the width equally. Records the count.
    fn wrap_columns(&mut self, node: usize, width: f32) -> Allocation {
        let state = &self.states[0];
        let (minimum, gap) = (state.column_min[node], state.column_gap[node]);
        let position = self.wrap_index(node);
        let most = self.most_columns(node);
        // Snapping rounds each column and gap up, so the count fits the rounded sizes.
        let (rounded_minimum, rounded_gap) = self
            .snap
            .map_or((minimum, gap), |snap| (snap.up(minimum), snap.up(gap)));
        let fit = (width + rounded_gap) / (rounded_minimum + rounded_gap) + 1e-4;
        let count = match self.arrangements[position] {
            // The last attempt keeps its tree's count, so that the rows match it.
            Arrangement::Columns(count) if !self.recount => count,
            _ if fit.is_finite() => (fit.floor().max(1.0) as usize).min(most),
            _ => most,
        };
        let size = ((width - (count - 1) as f32 * gap) / count as f32).max(minimum);
        let mut allocation = Allocation {
            sizes: vec![size; count],
            starts: (0..count)
                .map(|column| column as f32 * (size + gap))
                .collect(),
            gaps: vec![gap; count - 1],
        };
        if let Some(snap) = self.snap {
            snap.tracks(
                &mut allocation,
                &vec![minimum; count],
                &vec![gap; count - 1],
            );
        }
        self.chosen[position] = Arrangement::Columns(count);
        allocation
    }

    /// A flow's lines in `width`: each child at its fit-content width, a line breaking before a
    /// child that would pass the width, and each line's leftover space placed by the flow's
    /// distribution. Records the lines, and returns one span per child.
    fn flow_lines(&mut self, node: usize, width: f32) -> Allocation {
        let tree = self.tree;
        let state = &self.states[0];
        let gap = state.column_gap[node];
        let children = &tree.grid(node).expect("flows are grids").children;
        let distribute = tree.grid(node).expect("flows are grids").distribute[0];
        let mut sizes: Vec<f32> = children
            .iter()
            .map(|&child| state.box_fit[child].min(width).max(state.box_min[child]))
            .collect();
        let position = self.wrap_index(node);
        let lines = match &self.arrangements[position] {
            // The last attempt keeps its tree's lines, so that the rows match them.
            Arrangement::Lines(lines) if !self.recount => lines.clone(),
            _ => break_lines(&sizes, gap, width),
        };
        let mut starts = vec![0.0f32; children.len()];
        for line in 0..=lines.iter().copied().max().unwrap_or(0) {
            let members: Vec<usize> = (0..children.len()).filter(|&i| lines[i] == line).collect();
            let used: f32 = members.iter().map(|&i| sizes[i]).sum::<f32>()
                + gap * members.len().saturating_sub(1) as f32;
            let leftover = (width - used).max(0.0);
            if distribute == Distribute::Stretch {
                for &i in &members {
                    sizes[i] += leftover / members.len() as f32;
                }
            }
            let (mut offset, extra_gap) = tracks::distribution(distribute, leftover, members.len());
            for &i in &members {
                starts[i] = offset;
                offset += sizes[i] + gap + extra_gap;
            }
        }
        let mut allocation = Allocation {
            sizes,
            starts,
            gaps: Vec::new(),
        };
        if let Some(snap) = self.snap {
            for (size, start) in allocation.sizes.iter_mut().zip(&mut allocation.starts) {
                (*start, *size) = {
                    let (a, b) = snap.range((*start, *start + *size));
                    (a, b - a)
                };
            }
        }
        self.chosen[position] = Arrangement::Lines(lines);
        allocation
    }

    /// A Wrap's index in the prepared Wraps, which every variant lists in the same order.
    fn wrap_index(&self, node: usize) -> usize {
        self.prepared
            .wraps
            .iter()
            .position(|&wrap| wrap == node)
            .expect("every Wrap is prepared")
    }

    /// The most columns a Wrap can take: its children, and at most its maximum.
    fn most_columns(&self, node: usize) -> usize {
        let children = self.tree.grid(node).map_or(0, |grid| grid.children.len());
        let maximum = self.tree.wrap(node).and_then(|wrap| wrap.max_columns);
        children.min(maximum.unwrap_or(usize::MAX)).max(1)
    }

    /// Places each slab of a root: across its side in the slab's thickness direction, and
    /// along its side otherwise.
    fn place_slabs(&mut self, direction: Direction, root: usize) {
        let tree = self.tree;
        let members: Vec<usize> = tree.members(root).collect();
        for band in members {
            let (host, side, extent) = match &tree.nodes[band].kind {
                NodeKind::Slab {
                    host,
                    side,
                    extent,
                    stack: None,
                    ..
                } => (*host, *side, *extent),
                NodeKind::Stack(stack) => (stack.host, stack.side, stack.extent),
                // A stack places its members.
                _ => continue,
            };
            let content = self.states[direction.index()].content[host];
            let across = side.direction() == direction;
            let range = if across {
                let k = usize::from(!side.is_leading());
                let before: f32 = tree.nodes[host].slabs[side.index()]
                    .iter()
                    .take_while(|&&other| other != band)
                    .map(|&other| self.states[direction.index()].thickness[other])
                    .sum();
                let offset = self.slab_base(direction, host, k) + before;
                let thickness = self.states[direction.index()].thickness[band];
                if side.is_leading() {
                    (content.0 - offset - thickness, content.0 - offset)
                } else {
                    (content.1 + offset, content.1 + offset + thickness)
                }
            } else {
                match extent {
                    Extent::Content => content,
                    Extent::Full => (
                        content.0 - self.corner(direction, host, 0),
                        content.1 + self.corner(direction, host, 1),
                    ),
                }
            };
            let Some(stack) = tree.stack(band) else {
                self.place_slab(band, direction, range, across);
                continue;
            };
            let state = &mut self.states[direction.index()];
            state.slot[band] = range;
            state.boxed[band] = range;
            state.content[band] = range;
            // Every member spans the band across the side, and takes its segment along it.
            let segments = if across {
                vec![range; stack.members.len()]
            } else {
                self.stack_segments(band, direction, range)
            };
            for (&member, segment) in stack.members.iter().zip(segments) {
                self.place_slab(member, direction, segment, across);
            }
        }
    }

    /// Places a slab or a stack's member in `range`. Across its side, its content is the range.
    /// Along it, its content takes its place in the range by its maximum and placement.
    fn place_slab(&mut self, slab: usize, direction: Direction, range: (f32, f32), across: bool) {
        let node = &self.tree.nodes[slab];
        let content = if across {
            range
        } else {
            let min = node.min.along(direction);
            let fit = self.fit_content(slab, direction, min);
            let placed = place(
                range,
                min,
                fit,
                node.max.along(direction),
                node.place[direction.index()],
            );
            self.snap.map_or(placed, |snap| snap.range(placed))
        };
        let state = &mut self.states[direction.index()];
        state.slot[slab] = range;
        state.boxed[slab] = range;
        state.content[slab] = content;
    }

    /// A stack's tracks in `range` along its side, by the container track rules: each track
    /// starts at its base size, `auto` and fit-content tracks grow toward their member's
    /// max-content length, `fr` tracks share the rest, and the stack's distribution places what
    /// is left. Each gap holds the overhang of the members on either side of it.
    fn stack_allocation(
        &self,
        band: usize,
        direction: Direction,
        range: (f32, f32),
    ) -> StackAllocation {
        let tree = self.tree;
        let stack = tree.stack(band).expect("a stack band");
        let (base, limits): (Vec<f32>, Vec<f32>) = stack
            .members
            .iter()
            .zip(&stack.tracks)
            .map(|(&member, &track)| {
                let min = tree.nodes[member].min.along(direction);
                let fit = self.fit_content(member, direction, min);
                match track {
                    TrackSize::Px(length) => (length, length),
                    TrackSize::Fr { basis, .. } => {
                        let base = basis.max(min);
                        (base, fit.max(base))
                    }
                    TrackSize::FitContent(limit) => (min, fit.min(limit).max(min)),
                    // Validation rejects Aspect tracks in a stack.
                    TrackSize::Auto | TrackSize::Aspect { .. } => (min, fit.max(min)),
                }
            })
            .unzip();
        let (gaps, overflowed): (Vec<f32>, Vec<bool>) = stack
            .members
            .windows(2)
            .map(|pair| {
                let overhang =
                    self.measurements.overhang[pair[0]][1] + self.measurements.overhang[pair[1]][0];
                let needed = overhang + stack.gap.clearance;
                if stack.gap.min >= needed {
                    (stack.gap.min, false)
                } else {
                    (needed, true)
                }
            })
            .unzip();
        let sizes = TrackSizes {
            base,
            limits,
            provenance: None,
        };
        let mut allocation = tracks::allocate(
            &stack.tracks,
            &sizes,
            &gaps,
            range.1 - range.0,
            stack.distribute,
            f32::INFINITY,
        );
        if let Some(snap) = self.snap {
            snap.tracks(&mut allocation, &sizes.base, &gaps);
        }
        for start in &mut allocation.starts {
            *start += range.0;
        }
        StackAllocation {
            sizes,
            overflowed,
            allocation,
        }
    }

    /// Each member's segment of a stack's `range` along its side.
    fn stack_segments(
        &self,
        band: usize,
        direction: Direction,
        range: (f32, f32),
    ) -> Vec<(f32, f32)> {
        let allocation = self.stack_allocation(band, direction, range).allocation;
        allocation
            .starts
            .iter()
            .zip(&allocation.sizes)
            .map(|(&start, &size)| (start, start + size))
            .collect()
    }

    /// Why a stack's members have their lengths: its tracks and gaps along its side.
    fn explain_stack(&self, band: usize) -> TrackOwnerExplanation {
        let tree = self.tree;
        let stack = tree.stack(band).expect("a stack band");
        let direction = stack.side.direction().cross();
        let range = self.states[direction.index()].slot[band];
        let StackAllocation {
            sizes,
            overflowed,
            allocation,
        } = self.stack_allocation(band, direction, range);
        let path = |member: usize| tree.nodes[stack.members[member]].path.clone();
        let tracks = stack
            .tracks
            .iter()
            .enumerate()
            .map(|(index, &size)| {
                let min = tree.nodes[stack.members[index]].min.along(direction);
                // The member's minimum length sets the base size when it is larger than what the
                // track starts at.
                let set_by = match size {
                    TrackSize::Px(_) => false,
                    TrackSize::Fr { basis, .. } => min > basis,
                    _ => min > 0.0,
                };
                TrackExplanation {
                    size,
                    tag: None,
                    start: allocation.starts[index],
                    length: allocation.sizes[index],
                    base_size: sizes.base[index],
                    growth_limit: sizes.limits[index],
                    set_by: set_by.then(|| path(index)),
                    raised_from: None,
                }
            })
            .collect();
        let gaps = overflowed
            .iter()
            .enumerate()
            .map(|(boundary, &overflowed)| GapExplanation {
                start: allocation.starts[boundary] + allocation.sizes[boundary],
                length: allocation.gaps[boundary],
                term: if overflowed {
                    GapTerm::Overflow {
                        trailing: Some(path(boundary)),
                        leading: Some(path(boundary + 1)),
                    }
                } else {
                    GapTerm::Minimum
                },
                grid: tree.nodes[band].path.clone(),
                tag: stack.gap.tag.clone(),
                raised_from: None,
            })
            .collect();
        TrackOwnerExplanation {
            path: tree.nodes[band].path.clone(),
            direction,
            tracks,
            gaps,
        }
    }

    /// Asks the max-content size of each measured leaf in both directions, and of each measured
    /// slab along its side. A max-content size depends on no other size, so it is asked once.
    fn measure_max_content(&mut self, root: usize) -> Result<(), LayoutError> {
        let tree = self.tree;
        for node in tree.members(root) {
            let Some(id) = &tree.nodes[node].id else {
                continue;
            };
            if !asked(tree, node) {
                continue;
            }
            let directions: &[Direction] = match tree.nodes[node].kind {
                NodeKind::Leaf { .. } => &Direction::BOTH,
                NodeKind::Slab { side, .. } if side.direction() == V => &[H],
                NodeKind::Slab { .. } => &[V],
                NodeKind::Grid(_) | NodeKind::Stack(_) => &[],
            };
            let key = tree.answer_key(node, id);
            for &direction in directions {
                self.measurements.max_content[node][direction.index()] =
                    self.answers
                        .max_content(node, id, &key, &tree.nodes[node].path, direction)?;
            }
        }
        Ok(())
    }

    /// The nodes of `root` that are asked about in the `measured` direction, with their ids and
    /// the value they are asked at: a leaf's content length across it, or a slab's content
    /// length along its side.
    fn asked_at(&self, root: usize, measured: Direction) -> Vec<(usize, &'a Id, f32)> {
        let tree = self.tree;
        let across = &self.states[measured.cross().index()];
        tree.members(root)
            .filter(|&node| asked(tree, node))
            .filter_map(|node| {
                let id = tree.nodes[node].id.as_ref()?;
                let measured_here = match tree.nodes[node].kind {
                    NodeKind::Leaf { .. } => true,
                    NodeKind::Slab { side, .. } => side.direction() == measured,
                    NodeKind::Grid(_) | NodeKind::Stack(_) => false,
                };
                measured_here.then(|| (node, id, length(across.content[node])))
            })
            .collect()
    }

    /// Asks the height of each measured leaf and top or bottom slab at its solved width, and
    /// keeps the overhangs that grew.
    fn measure_heights(&mut self, root: usize) -> Result<(), LayoutError> {
        let tree = self.tree;
        for (node, id, value) in self.asked_at(root, V) {
            let key = tree.answer_key(node, id);
            let answer = self
                .answers
                .ask(node, id, &key, &tree.nodes[node].path, V, value)?;
            if let Some(answer) = &answer {
                self.overhang_grew |= self.take_overhang(node, answer)?;
            }
            self.measurements.height[node] = answer;
        }
        Ok(())
    }

    /// Asks again the height of each measured leaf and top or bottom slab whose width an aspect
    /// fit changed after the height pass, so that its answer no longer holds, at its final width.
    /// A larger height or overhang becomes a minimum for the next solve. Returns whether any
    /// grew.
    fn measure_fitted_heights(&mut self, root: usize) -> Result<bool, LayoutError> {
        let tree = self.tree;
        let mut grew = false;
        for (node, id, value) in self.asked_at(root, V) {
            let Some(asked) = self.measurements.height[node] else {
                continue;
            };
            if asked.holds_at(value) {
                continue;
            }
            let key = tree.answer_key(node, id);
            let Some(answer) =
                self.answers
                    .ask(node, id, &key, &tree.nodes[node].path, V, value)?
            else {
                continue;
            };
            grew |= self.take_overhang(node, &answer)?;
            if answer.size > self.measured_min(node, V) + GROWTH {
                self.measurements.fitted_height[node] = answer.size;
                grew = true;
            }
        }
        Ok(grew)
    }

    /// Keeps the overhang of a slab's `answer` where it is larger than the one that the passes
    /// counted, and returns whether it grew. An overhang belongs only to slabs.
    fn take_overhang(&mut self, node: usize, answer: &Answer) -> Result<bool, LayoutError> {
        let tree = self.tree;
        if !matches!(tree.nodes[node].kind, NodeKind::Slab { .. }) {
            if answer.overhang == [0.0; 2] {
                return Ok(false);
            }
            return Err(LayoutError::InvalidMeasurement {
                path: tree.nodes[node].path.clone(),
            });
        }
        let counted = self.measurements.overhang[node];
        if (0..2).all(|k| answer.overhang[k] <= counted[k] + GROWTH) {
            return Ok(false);
        }
        let grown = [0, 1].map(|k| counted[k].max(answer.overhang[k]));
        self.explanation.raised_overhangs.push(RaisedOverhang {
            path: tree.nodes[node].path.clone(),
            from: counted,
            to: grown,
        });
        self.measurements.overhang[node] = grown;
        Ok(true)
    }

    /// Asks the width of each measured leaf and left or right slab at its solved height, and
    /// raises the widths and overhangs that grew. Returns whether any grew.
    fn measure_widths(&mut self, root: usize) -> Result<bool, LayoutError> {
        let tree = self.tree;
        let mut grew = false;
        for (node, id, value) in self.asked_at(root, H) {
            let key = tree.answer_key(node, id);
            let Some(answer) =
                self.answers
                    .ask(node, id, &key, &tree.nodes[node].path, H, value)?
            else {
                continue;
            };
            grew |= self.take_overhang(node, &answer)?;
            let declared = tree.nodes[node].min.width;
            let current =
                self.measurements.width[node].map_or(declared, |stored| declared.max(stored.size));
            if answer.size > current + GROWTH {
                grew = true;
                self.explanation.raised_widths.push(RaisedWidth {
                    path: tree.nodes[node].path.clone(),
                    from: current,
                    to: answer.size,
                });
                self.measurements.width[node] = Some(answer);
            } else if self.measurements.width[node].is_none() {
                self.measurements.width[node] = Some(answer);
            }
        }
        Ok(grew)
    }

    /// When the canvas height is fixed and the root is taller, caps the Fr columns of each grid
    /// that fits its Aspect rows, and runs the passes once more. Returns `false` when a Wrap
    /// took another count in that width pass.
    fn fit_aspect(
        &mut self,
        root: usize,
        origin: [f32; 2],
        available: [Option<f32>; 2],
    ) -> Result<bool, LayoutError> {
        let tree = self.tree;
        let Some(height) = available[1] else {
            return Ok(true);
        };
        let deficit = self.states[1].box_min[root] - height;
        if deficit <= GROWTH {
            return Ok(true);
        }
        // Only a Grid fits its Aspect rows, and a Grid owns its tracks.
        let fitting: Vec<usize> = tree
            .members(root)
            .filter(|&node| {
                tree.grid(node)
                    .is_some_and(|grid| matches!(grid.kind, GridKind::Grid { fit_aspect: true }))
            })
            .collect();
        let mut caps = Vec::new();
        for &grid in &fitting {
            let Some(cap) = self.aspect_cap(grid, deficit) else {
                continue;
            };
            let owner = self.plans[0].owner_plan[grid].expect("grids own their tracks");
            self.caps[owner] = self.caps[owner].min(cap);
            caps.push((grid, cap));
        }
        if caps.is_empty() {
            return Ok(true);
        }
        if !self.passes(root, origin, available)? {
            return Ok(false);
        }
        let remaining = (self.states[1].box_min[root] - height).max(0.0);
        for (grid, cap) in caps {
            let fit = AspectFit {
                path: tree.nodes[grid].path.clone(),
                cap,
                remaining,
            };
            // A later solve of the root replaces the fit that an earlier one recorded.
            self.explanation
                .aspect_fits
                .retain(|recorded| recorded.path != fit.path);
            self.explanation.aspect_fits.push(fit);
        }
        Ok(true)
    }

    /// The Fr column width at which a grid's Aspect rows shrink by `deficit` in total, or `None`
    /// when no Aspect row follows an Fr column.
    fn aspect_cap(&self, grid: usize, deficit: f32) -> Option<f32> {
        let node = self.tree.grid(grid).expect("fitting grids are grids");
        let mut fixed = 0.0;
        let mut flexible = Vec::new();
        for track in &node.tracks[1] {
            if let TrackSize::Aspect { column, ratio } = track.size {
                let width = self.column_width(grid, column);
                if matches!(node.tracks[0][column].size, TrackSize::Fr { .. }) {
                    flexible.push((width, 1.0 / ratio));
                } else {
                    fixed += width / ratio;
                }
            }
        }
        if flexible.is_empty() {
            return None;
        }
        let total = fixed + flexible.iter().map(|(w, inverse)| w * inverse).sum::<f32>();
        let need = total - deficit - fixed;
        if need <= 0.0 {
            return Some(0.0);
        }
        flexible.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut done = 0.0;
        let mut rest: f32 = flexible.iter().map(|(_, inverse)| inverse).sum();
        for (width, inverse) in flexible.iter().copied() {
            let cap = (need - done) / rest;
            if cap <= width {
                return Some(cap.max(0.0));
            }
            done += width * inverse;
            rest -= inverse;
        }
        flexible.last().map(|(width, _)| *width)
    }

    /// Fits the content of each leaf with an aspect ratio inside its space, then places the
    /// slabs again around the fitted content.
    fn contain_fit(&mut self, root: usize) {
        let tree = self.tree;
        let mut changed = false;
        for node in tree.members(root) {
            let NodeKind::Leaf {
                aspect: Some(ratio),
            } = tree.nodes[node].kind
            else {
                continue;
            };
            let (x, y) = (self.states[0].content[node], self.states[1].content[node]);
            let (width, height) = (length(x), length(y));
            if width <= 0.0 || height <= 0.0 {
                continue;
            }
            let (direction, range, fitted) = if width / height > ratio {
                (H, x, height * ratio)
            } else {
                (V, y, width / ratio)
            };
            let position = tree.nodes[node].place[direction.index()].position;
            let fitted = place(range, fitted, fitted, fitted, Placement::fit(position));
            let fitted = self.snap.map_or(fitted, |snap| snap.range(fitted));
            self.states[direction.index()].content[node] = fitted;
            changed = true;
        }
        if changed {
            self.place_slabs(H, root);
            self.place_slabs(V, root);
        }
    }

    /// Builds the solution's regions, explanation and stale list.
    fn finish(self) -> Result<Solution<Id>, LayoutError> {
        let tree = self.tree;
        let mut regions = Vec::with_capacity(tree.nodes.len());
        for (index, node) in tree.nodes.iter().enumerate() {
            if node.moved {
                continue;
            }
            let rect = |pick: fn(&DirState) -> &Vec<(f32, f32)>| {
                Rect::from_ranges(pick(&self.states[0])[index], pick(&self.states[1])[index])
            };
            let kind = match &node.kind {
                NodeKind::Leaf { .. } => RegionKind::Leaf,
                NodeKind::Slab {
                    side,
                    extent,
                    tag,
                    stack,
                    ..
                } => RegionKind::Slab {
                    side: *side,
                    // A member runs as far as its stack does.
                    extent: stack.map_or(*extent, |stack| tree.band_extent(stack)),
                    tag: tag.clone(),
                    represented_by: node
                        .represented_by
                        .map(|representative| tree.nodes[representative].path.clone()),
                },
                NodeKind::Stack(stack) => RegionKind::Stack {
                    side: stack.side,
                    extent: stack.extent,
                    tag: stack.tag.clone(),
                    represented_by: node
                        .represented_by
                        .map(|representative| tree.nodes[representative].path.clone()),
                },
                NodeKind::Grid(grid) => {
                    let tracks = GridTracks {
                        columns: self.track_spans(H, index),
                        rows: self.track_spans(V, index),
                    };
                    match &grid.kind {
                        GridKind::Wrap(_) => self.wrap_region(index, tracks),
                        GridKind::Grid { .. } => RegionKind::Grid { tracks },
                        GridKind::Sequence(sequence) => match sequence.direction {
                            Direction::Horizontal => RegionKind::Row { tracks },
                            Direction::Vertical => RegionKind::Column { tracks },
                        },
                    }
                }
            };
            regions.push(Region {
                id: node.id.clone(),
                path: node.path.clone(),
                kind,
                cell: self.cell(index),
                slot: rect(|state| &state.slot),
                content: rect(|state| &state.content),
            });
        }
        let finite = |rect: &Rect| {
            [
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                rect.right(),
                rect.bottom(),
            ]
            .into_iter()
            .all(f32::is_finite)
        };
        let root = tree.roots[0];
        let [horizontal, vertical] = self.padding();
        let size = Size::new(
            self.states[0].slot[root].1 + horizontal[1],
            self.states[1].slot[root].1 + vertical[1],
        );
        if !regions
            .iter()
            .all(|region| finite(&region.slot) && finite(&region.content))
            || !size.width.is_finite()
            || !size.height.is_finite()
        {
            return Err(LayoutError::CoordinateOverflow);
        }
        let stale = self
            .answers
            .stale()
            .filter_map(|node| tree.nodes[node].id.clone())
            .collect();
        let explanation = self.explain();
        Ok(Solution::new(size, regions, explanation, stale))
    }

    /// A Wrap's region: its tracks at its count, and the cells that its last row leaves empty.
    /// A flow's region has its children's spans as its column tracks.
    fn wrap_region(&self, node: usize, tracks: GridTracks) -> RegionKind {
        let tree = self.tree;
        if tree.wrap(node).is_some_and(|wrap| wrap.flow) {
            return RegionKind::Flow { tracks };
        }
        let children = tree.grid(node).map_or(0, |grid| grid.children.len());
        let columns = tracks.columns.len().max(1);
        let rows = &tracks.rows;
        let empty = (children..rows.len() * columns)
            .filter_map(|cell| {
                let (column, row) = (
                    tracks.columns.get(cell % columns)?,
                    rows.get(cell / columns)?,
                );
                Some(Rect::new(
                    column.start,
                    row.start,
                    column.length,
                    row.length,
                ))
            })
            .collect();
        RegionKind::Wrap {
            tracks,
            columns,
            empty,
        }
    }

    /// A child's slot in its table's tracks: its slot in its parent, moved by the parent's first
    /// track among the table's.
    fn cell(&self, node: usize) -> Option<Cell> {
        let tree = self.tree;
        let slot = tree.nodes[node].slot?;
        let parent = tree.nodes[node].parent?;
        let [columns, rows] = [0, 1].map(|d| self.plans[d].offset[parent]);
        Some(Cell {
            table: tree.nodes[self.plans[0].owner[parent]].path.clone(),
            slot: GridSlot {
                column: slot.column + columns,
                row: slot.row + rows,
                ..slot
            },
        })
    }

    fn track_spans(&self, direction: Direction, grid: usize) -> Vec<TrackSpan> {
        let plan = self.plans[direction.index()];
        let owner_plan = plan.owner_plan[plan.owner[grid]].expect("owners have plans");
        let allocation = self.states[direction.index()].owners[owner_plan]
            .allocation
            .as_ref()
            .expect("a solved owner is allocated");
        let count = if direction == H && self.tree.wrap(grid).is_some() {
            allocation.sizes.len()
        } else {
            self.tree
                .grid(grid)
                .map_or(0, |g| g.tracks[direction.index()].len())
        };
        let offset = plan.offset[grid];
        (offset..offset + count)
            .map(|track| TrackSpan {
                start: allocation.starts[track],
                length: allocation.sizes[track],
            })
            .collect()
    }

    /// A Wrap's columns at its count: each at the column minimum as its base size, and each gap
    /// at the column gap, the largest gap that any two children could need.
    fn explain_wrap(&self, node: usize, allocation: &Allocation) -> TrackOwnerExplanation {
        let tree = self.tree;
        let state = &self.states[0];
        let path = tree.nodes[node].path.clone();
        let gap = self.wrap_gap(node);
        let tracks = (0..allocation.sizes.len())
            .map(|index| TrackExplanation {
                size: WRAP_COLUMN[0].size,
                tag: None,
                start: allocation.starts[index],
                length: allocation.sizes[index],
                base_size: state.column_min[node],
                growth_limit: state.column_min[node],
                set_by: None,
                raised_from: None,
            })
            .collect();
        let gaps = (0..allocation.gaps.len())
            .map(|boundary| GapExplanation {
                start: allocation.starts[boundary] + allocation.sizes[boundary],
                length: allocation.gaps[boundary],
                term: if gap.min >= state.column_gap[node] {
                    GapTerm::Minimum
                } else {
                    GapTerm::Overflow {
                        trailing: None,
                        leading: None,
                    }
                },
                grid: path.clone(),
                tag: gap.tag.clone(),
                raised_from: state
                    .provenance
                    .as_ref()
                    .and_then(|provenance| provenance.column_gap_raised[node]),
            })
            .collect();
        TrackOwnerExplanation {
            path,
            direction: H,
            tracks,
            gaps,
        }
    }

    fn explain(&self) -> Explanation {
        let tree = self.tree;
        let mut explanation = self.explanation.clone();
        if !self.options.explain {
            return explanation;
        }
        let mut owners = Vec::new();
        let mut match_groups = Vec::new();
        for direction in Direction::BOTH {
            let plan = self.plans[direction.index()];
            for (owner, owner_plan) in plan.owners.iter().enumerate() {
                let state = &self.states[direction.index()].owners[owner];
                let (Some(sizes), Some(allocation)) = (&state.sizes, &state.allocation) else {
                    continue;
                };
                let grid = tree.grid(owner_plan.node).expect("owners are grids");
                if direction == H && matches!(grid.kind, GridKind::Wrap(_)) {
                    owners.push(self.explain_wrap(owner_plan.node, allocation));
                    continue;
                }
                let item_path = |item: usize| tree.nodes[owner_plan.items[item].node].path.clone();
                let (Some(track_provenance), Some(gap_provenance)) =
                    (&sizes.provenance, &state.provenance)
                else {
                    continue;
                };
                let tracks = grid.tracks[direction.index()]
                    .iter()
                    .enumerate()
                    .map(|(index, track)| TrackExplanation {
                        size: track.size,
                        tag: track.tag.clone(),
                        start: allocation.starts[index],
                        length: allocation.sizes[index],
                        base_size: sizes.base[index],
                        growth_limit: sizes.limits[index],
                        set_by: track_provenance.set_by[index].map(item_path),
                        raised_from: track_provenance.raised_from[index],
                    })
                    .collect();
                let gaps = (0..state.gaps.len())
                    .map(|boundary| {
                        let index = gap_provenance.decided_by[boundary];
                        let contribution = &state.contributions[boundary][index];
                        let term = match contribution.term {
                            Term::Minimum => GapTerm::Minimum,
                            Term::Overflow { trailing, leading } => GapTerm::Overflow {
                                trailing: trailing.map(item_path),
                                leading: leading.map(item_path),
                            },
                        };
                        GapExplanation {
                            start: allocation.starts[boundary] + allocation.sizes[boundary],
                            length: allocation.gaps[boundary],
                            term,
                            grid: tree.nodes[owner_plan.grids[contribution.grid]].path.clone(),
                            tag: owner_plan.gaps[contribution.grid].tag.clone(),
                            raised_from: gap_provenance.raised[boundary][index],
                        }
                    })
                    .collect();
                for (tag, members) in &owner_plan.groups {
                    match_groups.push(MatchGroup {
                        tag: tag.clone(),
                        direction,
                        parts: MatchedParts::Tracks,
                        size: members
                            .first()
                            .map_or(0.0, |&track| allocation.sizes[track]),
                        grid: tree.nodes[owner_plan.node].path.clone(),
                    });
                }
                owners.push(TrackOwnerExplanation {
                    path: tree.nodes[owner_plan.node].path.clone(),
                    direction,
                    tracks,
                    gaps,
                });
            }
        }
        // Each drawn stack owns its members' tracks along its side.
        for band in 0..tree.nodes.len() {
            if tree.stack(band).is_some()
                && !tree.nodes[band].moved
                && tree.nodes[band].represented_by.is_none()
            {
                owners.push(self.explain_stack(band));
            }
        }
        for provenance in self
            .states
            .iter()
            .filter_map(|state| state.provenance.as_ref())
        {
            match_groups.extend(provenance.groups.iter().map(|(_, group)| group.clone()));
        }
        explanation.sizes = Some(ExplainedSizes {
            owners,
            match_groups,
        });
        explanation
    }
}

/// The line of each child of a flow whose children have `sizes`: a line breaks before a child
/// that would pass `width`.
fn break_lines(sizes: &[f32], gap: f32, width: f32) -> Vec<usize> {
    let mut lines = vec![0usize; sizes.len()];
    let mut cursor = 0.0f32;
    let mut line = 0;
    for (index, &size) in sizes.iter().enumerate() {
        if index > 0 && cursor + gap + size > width + 1e-4 {
            line += 1;
            cursor = 0.0;
        } else if index > 0 {
            cursor += gap;
        }
        lines[index] = line;
        cursor += size;
    }
    lines
}

/// Whether `Measure` is asked about a node with an id: always, except for a slab that another
/// slab draws. The slabs of a Wrap's children are measured too, since another count can draw
/// them.
fn asked<Id>(tree: &Tree<Id>, node: usize) -> bool {
    tree.nodes[node].represented_by.is_none()
        || match tree.nodes[node].kind {
            NodeKind::Slab { host, .. } => tree.nodes[host]
                .parent
                .is_some_and(|parent| tree.wrap(parent).is_some()),
            _ => false,
        }
}

/// The largest box a node fills its slot to in `direction`.
fn box_max<Id>(tree: &Tree<Id>, state: &DirState, node: usize, direction: Direction) -> f32 {
    tree.nodes[node].max.along(direction) + state.contain[node][0] + state.contain[node][1]
}

/// A node's range in `slot`, at its position: its fit-content size `fit` within the slot, or
/// the slot, each up to its maximum and at least its minimum.
fn place(slot: (f32, f32), min: f32, fit: f32, max: f32, placement: Placement) -> (f32, f32) {
    let available = slot.1 - slot.0;
    let size = match placement.size {
        Sizing::Fit => fit.min(available).min(max).max(min),
        Sizing::Stretch => available.min(max).max(min),
    };
    let leftover = available - size;
    let start = slot.0
        + match placement.position {
            Position::Start => 0.0,
            Position::Center => leftover / 2.0,
            Position::End => leftover,
        };
    (start, start + size)
}

fn length(range: (f32, f32)) -> f32 {
    range.1 - range.0
}
