//! The one-direction solver: the track sizes, gaps and positions of one track owner.
//!
//! An owner's problem lists its tracks, the grids that share them (the owner and its subgrids),
//! and its items: each child's tracks, minimum size, overflow on both sides, and place across
//! the direction. Solving runs in three steps, so that the caller can match gaps across owners
//! in between:
//!
//! 1. [`contributions`]: what each boundary between tracks needs, per lane.
//! 2. [`track_sizes`]: each track's base size, the smallest that holds every item, and its
//!    growth limit, the size its items take with no constraint, given the final gaps.
//! 3. [`allocate`]: the track sizes and starts in the space that the owner gets.

use std::ops::Range;

use crate::node::{Distribute, Tag, TrackSize};

/// A grid whose gap applies to some of the owner's boundaries: the owner, or a subgrid that
/// shares its tracks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GapGrid {
    /// The owner's first track that the grid spans.
    pub start: usize,
    /// One past the grid's last track.
    pub end: usize,
    pub min: f32,
    pub clearance: f32,
}

impl GapGrid {
    /// Whether the boundary after track `boundary` lies inside the grid.
    fn covers(&self, boundary: usize) -> bool {
        self.start <= boundary && boundary + 1 < self.end
    }
}

/// One item of an owner: a child that occupies some of its tracks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrackOwnerItem<'a> {
    pub start: usize,
    /// One past the item's last track.
    pub end: usize,
    /// The item's minimum contribution: the smallest size of its box.
    pub min: f32,
    /// The item's max-content contribution, at least its minimum.
    pub max: f32,
    /// Overflow before the item's first track.
    pub lead: f32,
    /// Overflow after the item's last track.
    pub trail: f32,
    /// The item's tracks across the direction, among the owner's cross tracks.
    pub lane: Range<usize>,
    /// Indices into the problem's grids, from the item's parent up to the owner.
    pub chain: &'a [usize],
}

/// The tracks, grids and items of one owner in one direction.
#[derive(Clone, Debug)]
pub(crate) struct TrackOwnerProblem<'a> {
    pub tracks: Vec<TrackSize>,
    /// The match groups: for each tag, the tracks that stay equal.
    pub groups: &'a [(Tag, Vec<usize>)],
    /// The owner first, then its subgrids.
    pub grids: Vec<GapGrid>,
    pub items: Vec<TrackOwnerItem<'a>>,
}

/// What one lane needs at one boundary.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Contribution {
    pub value: f32,
    /// The grid whose gap applies, by index into the problem's grids.
    pub grid: usize,
    pub term: Term,
}

/// The term that decides a contribution.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Term {
    /// The grid's minimum gap, which the overflow and clearance do not reach.
    Minimum,
    /// The overflow of the items on either side, plus the grid's clearance.
    Overflow {
        trailing: Option<usize>,
        leading: Option<usize>,
    },
}

/// Whether two items share a lane: whether their tracks across the direction overlap.
fn overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// What each boundary needs, per lane: one list for each boundary between consecutive tracks.
///
/// Facing items in the same lane add their overflow: `trail + lead + clearance`, at least the
/// minimum, of the innermost grid that holds both. An item with nothing facing it in its lane
/// adds its own overflow and the clearance of the innermost grid around the boundary. At a
/// boundary that every item reaching it spans, the gap only spaces those items' insides, so
/// each one adds the minimum and clearance of the innermost grid around the boundary in its
/// chain. A boundary that no item reaches gets the owner's minimum and clearance.
pub(crate) fn contributions(problem: &TrackOwnerProblem) -> Vec<Vec<Contribution>> {
    let boundaries = problem.tracks.len().saturating_sub(1);
    let mut result = vec![Vec::new(); boundaries];
    let push = |list: &mut Vec<Contribution>,
                grid_index: usize,
                overflow: f32,
                trailing: Option<usize>,
                leading: Option<usize>| {
        let grid = &problem.grids[grid_index];
        let needed = overflow + grid.clearance;
        let (value, term) = if grid.min >= needed {
            (grid.min, Term::Minimum)
        } else {
            (needed, Term::Overflow { trailing, leading })
        };
        list.push(Contribution {
            value,
            grid: grid_index,
            term,
        });
    };
    // Every chain ends at the owner, which covers every boundary.
    let innermost = |item: &TrackOwnerItem, boundary: usize| {
        item.chain
            .iter()
            .copied()
            .find(|&grid| problem.grids[grid].covers(boundary))
            .expect("the owner covers every boundary")
    };
    // The items ordered by their end and by their start, ties in item order, so that each
    // boundary finds the items that end and start at it by a binary search. Each item starts
    // at one boundary, so `faced` is never reset.
    let count = problem.items.len();
    let mut orders: Vec<usize> = (0..count).chain(0..count).collect();
    let (by_end, by_start) = orders.split_at_mut(count);
    by_end.sort_by_key(|&index| problem.items[index].end);
    by_start.sort_by_key(|&index| problem.items[index].start);
    let (by_end, by_start) = (&*by_end, &*by_start);
    let mut faced = vec![false; count];
    for (boundary, list) in result.iter_mut().enumerate() {
        let starting = with_key(by_start, &problem.items, |item| item.start, boundary + 1);
        for &a in with_key(by_end, &problem.items, |item| item.end, boundary + 1) {
            let item_a = &problem.items[a];
            let mut partnered = false;
            for &b in starting {
                let item_b = &problem.items[b];
                if !overlap(&item_a.lane, &item_b.lane) {
                    continue;
                }
                partnered = true;
                faced[b] = true;
                let common = item_a
                    .chain
                    .iter()
                    .copied()
                    .find(|grid| item_b.chain.contains(grid))
                    .expect("every chain ends at the owner");
                push(list, common, item_a.trail + item_b.lead, Some(a), Some(b));
            }
            if !partnered {
                push(
                    list,
                    innermost(item_a, boundary),
                    item_a.trail,
                    Some(a),
                    None,
                );
            }
        }
        for &b in starting {
            let item_b = &problem.items[b];
            if !faced[b] {
                push(
                    list,
                    innermost(item_b, boundary),
                    item_b.lead,
                    None,
                    Some(b),
                );
            }
        }
        if list.is_empty() {
            for item in &problem.items {
                if item.start <= boundary && boundary + 1 < item.end {
                    push(list, innermost(item, boundary), 0.0, None, None);
                }
            }
        }
        if list.is_empty() {
            push(list, 0, 0.0, None, None);
        }
    }
    result
}

/// The items of `order`, which is sorted by `key`, whose key is `value`.
fn with_key<'a>(
    order: &'a [usize],
    items: &[TrackOwnerItem],
    key: fn(&TrackOwnerItem) -> usize,
    value: usize,
) -> &'a [usize] {
    let low = order.partition_point(|&index| key(&items[index]) < value);
    let high = order.partition_point(|&index| key(&items[index]) <= value);
    &order[low..high]
}

/// The intrinsic sizes of each track, as the CSS track sizing algorithm names them: the base
/// size, the smallest size that holds every item's minimum contribution, and the growth limit,
/// the size that the track grows toward when there is room.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrackSizes {
    pub base: Vec<f32>,
    pub limits: Vec<f32>,
    /// The item that set each track's base size last, if any.
    pub set_by: Vec<Option<usize>>,
    /// The base size each matched track had before its group raised it, when the group raised
    /// it.
    pub raised_from: Vec<Option<f32>>,
}

/// Whether a track grows toward its growth limit before Fr tracks share the rest.
fn grows(size: TrackSize) -> bool {
    matches!(size, TrackSize::Auto | TrackSize::FitContent(_))
}

/// Whether a track takes its items' sizes: an Auto, fit-content or Fr track.
fn intrinsic(size: TrackSize) -> bool {
    grows(size) || matches!(size, TrackSize::Fr { .. })
}

/// The base size and growth limit of each track. Px sizes and Aspect sizes from `aspect` are
/// both. For the other tracks, the base size is the largest minimum contribution of an item that
/// spans only that track, no smaller than an Fr track's basis, and the growth limit the largest
/// max-content contribution, no smaller than the base size and no larger than a fit-content
/// track's limit or an Fr track's maximum. Each match group then takes its largest member, and
/// items that span several tracks spread what they still need over the Auto and fit-content
/// tracks they span, or the Fr tracks when there are none, narrowest spans first.
pub(crate) fn track_sizes(
    problem: &TrackOwnerProblem,
    gaps: &[f32],
    aspect: &[Option<f32>],
) -> TrackSizes {
    let count = problem.tracks.len();
    let mut sizes = TrackSizes {
        base: vec![0.0; count],
        limits: vec![0.0; count],
        set_by: vec![None; count],
        raised_from: vec![None; count],
    };
    for (index, &track) in problem.tracks.iter().enumerate() {
        sizes.base[index] = match track {
            TrackSize::Px(pixels) => pixels,
            TrackSize::Aspect { .. } => aspect.get(index).copied().flatten().unwrap_or(0.0),
            TrackSize::Fr { basis, .. } => basis,
            TrackSize::Auto | TrackSize::FitContent(_) => 0.0,
        };
    }
    for (index, item) in problem.items.iter().enumerate() {
        let track = item.start;
        if item.end != track + 1 || !intrinsic(problem.tracks[track]) {
            continue;
        }
        if item.min > sizes.base[track] {
            sizes.base[track] = item.min;
            sizes.set_by[track] = Some(index);
        }
        sizes.limits[track] = sizes.limits[track].max(item.max);
    }
    equalize_groups(problem, &mut sizes.base, Some(&mut sizes.raised_from));
    let spanning = spanning(problem);
    for &index in &spanning {
        let item = &problem.items[index];
        let grown = grow_to(problem, &mut sizes.base, gaps, item, item.min, |_| {
            f32::INFINITY
        });
        if !grown.is_empty() {
            for track in grown {
                sizes.set_by[track] = Some(index);
            }
            equalize_groups(problem, &mut sizes.base, Some(&mut sizes.raised_from));
        }
    }
    let clamp = |sizes: &mut TrackSizes| {
        for (index, &track) in problem.tracks.iter().enumerate() {
            let limit = match track {
                TrackSize::FitContent(cap) => sizes.limits[index].min(cap),
                TrackSize::Auto | TrackSize::Fr { .. } => sizes.limits[index],
                _ => 0.0,
            };
            sizes.limits[index] = limit.max(sizes.base[index]);
        }
    };
    clamp(&mut sizes);
    equalize_groups(problem, &mut sizes.limits, None);
    for &index in &spanning {
        let item = &problem.items[index];
        // A fit-content track's growth limit stops at its limit, or at its base size above it.
        let base = sizes.base.clone();
        let cap = |track: usize| match problem.tracks[track] {
            TrackSize::FitContent(cap) => cap.max(base[track]),
            _ => f32::INFINITY,
        };
        if !grow_to(problem, &mut sizes.limits, gaps, item, item.max, cap).is_empty() {
            clamp(&mut sizes);
            equalize_groups(problem, &mut sizes.limits, None);
        }
    }
    sizes
}

/// Grows the tracks that a spanning item covers in `values` until they hold `need` with the
/// gaps between them, spreading what is missing equally over the tracks that
/// [`spread_over`] picks. A track stops at its `cap`, and the tracks below theirs share what it
/// does not take, as CSS grid distributes space to tracks until each is frozen at its limit.
/// Returns the tracks that grew.
fn grow_to(
    problem: &TrackOwnerProblem,
    values: &mut [f32],
    gaps: &[f32],
    item: &TrackOwnerItem,
    need: f32,
    cap: impl Fn(usize) -> f32,
) -> Vec<usize> {
    let mut growable = spread_over(problem, item.start..item.end);
    let mut grown = Vec::new();
    loop {
        growable.retain(|&track| values[track] < cap(track));
        let current = extent(
            &values[item.start..item.end],
            &gaps[item.start..item.end - 1],
        );
        let deficit = need - current;
        if deficit <= 0.0 || growable.is_empty() {
            return grown;
        }
        let share = deficit / growable.len() as f32;
        // Every track grows alike, until the first one reaches its cap.
        let step = growable
            .iter()
            .map(|&track| cap(track) - values[track])
            .fold(share, f32::min);
        for &track in &growable {
            values[track] += step;
            if !grown.contains(&track) {
                grown.push(track);
            }
        }
        if step >= share {
            return grown;
        }
    }
}

/// The items that span several tracks, narrowest first.
fn spanning(problem: &TrackOwnerProblem) -> Vec<usize> {
    let mut spanning: Vec<usize> = (0..problem.items.len())
        .filter(|&index| problem.items[index].end > problem.items[index].start + 1)
        .collect();
    spanning.sort_by_key(|&index| problem.items[index].end - problem.items[index].start);
    spanning
}

/// The tracks of `tracks` that a spanning item grows: its Auto and fit-content tracks, or its
/// Fr tracks when it has none.
fn spread_over(problem: &TrackOwnerProblem, tracks: Range<usize>) -> Vec<usize> {
    let of_kind = |auto: bool| -> Vec<usize> {
        tracks
            .clone()
            .filter(|&track| {
                let size = problem.tracks[track];
                if auto {
                    grows(size)
                } else {
                    matches!(size, TrackSize::Fr { .. })
                }
            })
            .collect()
    };
    let growable = of_kind(true);
    if growable.is_empty() {
        of_kind(false)
    } else {
        growable
    }
}

/// Raises every track of each match group to the group's largest value in `values`, recording
/// the value it had before in `raised_from` when given.
fn equalize_groups(
    problem: &TrackOwnerProblem,
    values: &mut [f32],
    mut raised_from: Option<&mut Vec<Option<f32>>>,
) {
    for (_, members) in problem.groups {
        let largest = members
            .iter()
            .map(|&track| values[track])
            .fold(0.0f32, f32::max);
        for &track in members {
            if values[track] < largest {
                if let Some(raised_from) = raised_from.as_deref_mut() {
                    raised_from[track].get_or_insert(values[track]);
                }
                values[track] = largest;
            }
        }
    }
}

/// The owner's max-content length, with the gaps: each track at its growth limit, and the Fr
/// tracks at their basis plus the share of the largest fr unit that their items' max-content
/// sizes need, as CSS sizes flexible tracks under a max-content constraint. Allocated at this
/// length, every Fr track reaches its growth limit and the weights still hold.
///
/// Only Fr tracks whose items have max-content sizes above their base sizes set the unit, so a
/// layout without max-content sizes has its minimum as its max-content length.
pub(crate) fn max_content(tracks: &[TrackSize], sizes: &TrackSizes, gaps: &[f32]) -> f32 {
    let unit = tracks
        .iter()
        .enumerate()
        .filter_map(|(index, &track)| {
            let TrackSize::Fr { weight, basis, max } = track else {
                return None;
            };
            // A track cannot grow past its maximum, so a larger limit sets no unit.
            let limit = sizes.limits[index].min(max.unwrap_or(f32::INFINITY));
            (weight > 0.0 && limit > sizes.base[index]).then(|| (limit - basis) / weight)
        })
        .fold(0.0f32, f32::max);
    let lengths: Vec<f32> = tracks
        .iter()
        .enumerate()
        .map(|(index, &track)| match track {
            TrackSize::Fr { weight, basis, max } => (basis + unit * weight.max(0.0))
                .max(sizes.limits[index])
                .min(max.unwrap_or(f32::INFINITY))
                .max(sizes.base[index]),
            _ => sizes.limits[index],
        })
        .collect();
    extent(&lengths, gaps)
}

/// The sum of the track sizes and the gaps between them.
pub(crate) fn extent(sizes: &[f32], gaps: &[f32]) -> f32 {
    sizes.iter().sum::<f32>() + gaps.iter().sum::<f32>()
}

/// The solved tracks of one owner.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Allocation {
    pub sizes: Vec<f32>,
    /// From the start of the owner's content.
    pub starts: Vec<f32>,
    /// The gaps after distribution, which adds to them for [`Distribute::SpaceBetween`].
    pub gaps: Vec<f32>,
}

/// Sizes the tracks in `available`, as the CSS track sizing algorithm does. Auto and
/// fit-content tracks first grow from their base sizes toward their growth limits, sharing the
/// space equally. Fr tracks then share what is left by weight, up to their maximum and `cap`.
/// With no Fr tracks, [`Distribute::Stretch`] grows the Auto tracks equally. Space that is still
/// left is placed by `distribute`. When the base sizes do not fit, the tracks keep their base
/// sizes and overflow the end.
pub(crate) fn allocate(
    tracks: &[TrackSize],
    sizes: &TrackSizes,
    gaps: &[f32],
    available: f32,
    distribute: Distribute,
    cap: f32,
) -> Allocation {
    let limits = &sizes.limits;
    let mut sizes = sizes.base.clone();
    let mut leftover = available - extent(&sizes, gaps);
    if leftover > 0.0 {
        leftover = maximize(tracks, &mut sizes, limits, leftover);
    }
    let fr: Vec<usize> = (0..tracks.len())
        .filter(|&track| matches!(tracks[track], TrackSize::Fr { .. }))
        .collect();
    if leftover > 0.0 && !fr.is_empty() {
        share_fr(tracks, &fr, &mut sizes, leftover, cap);
        leftover = available - extent(&sizes, gaps);
    } else if leftover > 0.0 && distribute == Distribute::Stretch {
        let auto: Vec<usize> = (0..tracks.len())
            .filter(|&track| tracks[track] == TrackSize::Auto)
            .collect();
        if !auto.is_empty() {
            let increment = leftover / auto.len() as f32;
            for track in auto {
                sizes[track] += increment;
            }
            leftover = 0.0;
        }
    }
    let (offset, extra) = if leftover > 0.0 {
        distribution(distribute, leftover, sizes.len())
    } else {
        (0.0, 0.0)
    };
    let gaps: Vec<f32> = gaps.iter().map(|gap| gap + extra).collect();
    let mut starts = Vec::with_capacity(sizes.len());
    let mut cursor = offset;
    for (index, size) in sizes.iter().enumerate() {
        if index > 0 {
            cursor += gaps[index - 1];
        }
        starts.push(cursor);
        cursor += size;
    }
    Allocation {
        sizes,
        starts,
        gaps,
    }
}

/// Where `distribute` puts `leftover` space among `count` tracks: the space before the first
/// track and the space that each gap gains. [`Distribute::Stretch`] and [`Distribute::Start`]
/// leave it after the last track.
pub(crate) fn distribution(distribute: Distribute, leftover: f32, count: usize) -> (f32, f32) {
    let count_f = count as f32;
    match distribute {
        Distribute::Stretch | Distribute::Start => (0.0, 0.0),
        Distribute::Center => (leftover / 2.0, 0.0),
        Distribute::End => (leftover, 0.0),
        Distribute::SpaceBetween if count > 1 => (0.0, leftover / (count_f - 1.0)),
        Distribute::SpaceBetween => (0.0, 0.0),
        Distribute::SpaceAround => (leftover / count_f / 2.0, leftover / count_f),
        Distribute::SpaceEvenly => (leftover / (count_f + 1.0), leftover / (count_f + 1.0)),
    }
}

/// Grows the Auto and fit-content tracks toward their growth limits, sharing `leftover`
/// equally and freezing each track that reaches its limit, as CSS's maximize step does. Returns
/// the space that is still left.
fn maximize(tracks: &[TrackSize], sizes: &mut [f32], limits: &[f32], mut leftover: f32) -> f32 {
    let mut growing: Vec<usize> = (0..tracks.len())
        .filter(|&track| grows(tracks[track]) && limits[track] > sizes[track])
        .collect();
    // Each round either spends the whole share or brings at least one track to its limit.
    while leftover > 0.0 && !growing.is_empty() {
        let share = leftover / growing.len() as f32;
        let room = growing
            .iter()
            .map(|&track| limits[track] - sizes[track])
            .fold(f32::INFINITY, f32::min);
        let step = share.min(room);
        for &track in &growing {
            let grown = if limits[track] - sizes[track] <= step {
                limits[track]
            } else {
                sizes[track] + step
            };
            leftover -= grown - sizes[track];
            sizes[track] = grown;
        }
        growing.retain(|&track| limits[track] > sizes[track]);
        if step >= share {
            break;
        }
    }
    leftover.max(0.0)
}

/// Shares `leftover` among the Fr tracks: each takes `basis + weight × unit`, where the unit
/// fills the space, but no less than its base size and no more than its maximum and `cap`.
///
/// As in CSS flexbox, each round clamps every track, then freezes the tracks that the larger
/// total violation came from: those below their base size when clamping added space, those above
/// their limit when it removed space. The unit is then found again for the rest.
fn share_fr(tracks: &[TrackSize], fr: &[usize], sizes: &mut [f32], leftover: f32, cap: f32) {
    let base: Vec<f32> = fr.iter().map(|&track| sizes[track]).collect();
    // Each Fr track's weight, basis and largest size.
    let params: Vec<(f32, f32, f32)> = fr
        .iter()
        .zip(&base)
        .map(|(&track, &base)| {
            let TrackSize::Fr { weight, basis, max } = tracks[track] else {
                unreachable!("only Fr tracks share")
            };
            let limit = max.unwrap_or(f32::INFINITY).min(cap).max(base);
            (weight.max(0.0), basis, limit)
        })
        .collect();
    let space = leftover + base.iter().sum::<f32>();
    let mut frozen: Vec<Option<f32>> = vec![None; fr.len()];
    loop {
        let free = || {
            params
                .iter()
                .zip(&frozen)
                .filter(|(_, frozen)| frozen.is_none())
                .map(|(params, _)| params)
        };
        let free_weight: f32 = free().map(|&(weight, _, _)| weight).sum();
        let free_basis: f32 = free().map(|&(_, basis, _)| basis).sum();
        let frozen_space: f32 = frozen.iter().flatten().sum();
        let unit = if free_weight > 0.0 {
            ((space - frozen_space - free_basis) / free_weight).max(0.0)
        } else {
            0.0
        };
        let share = |slot: usize| params[slot].1 + params[slot].0 * unit;
        let mut violation = 0.0;
        let mut clamped = vec![0.0; fr.len()];
        for slot in 0..fr.len() {
            if let Some(size) = frozen[slot] {
                clamped[slot] = size;
                continue;
            }
            let share = share(slot);
            clamped[slot] = share.clamp(base[slot], params[slot].2);
            violation += clamped[slot] - share;
        }
        let mut changed = false;
        for slot in 0..fr.len() {
            if frozen[slot].is_some() {
                continue;
            }
            let share = share(slot);
            let below = clamped[slot] > share;
            let above = clamped[slot] < share;
            if (violation > 0.0 && below) || (violation < 0.0 && above) || free_weight == 0.0 {
                frozen[slot] = Some(clamped[slot]);
                changed = true;
            }
        }
        if !changed {
            for (slot, &track) in fr.iter().enumerate() {
                sizes[track] = clamped[slot];
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Track;

    const OWNER: &[usize] = &[0];

    fn fr(weight: f32) -> TrackSize {
        Track::fr(weight).size
    }

    fn tracks(sizes: &[TrackSize]) -> Vec<TrackSize> {
        sizes.to_vec()
    }

    fn owner(count: usize, min: f32, clearance: f32) -> GapGrid {
        GapGrid {
            start: 0,
            end: count,
            min,
            clearance,
        }
    }

    fn item(start: usize, end: usize, min: f32) -> TrackOwnerItem<'static> {
        TrackOwnerItem {
            start,
            end,
            min,
            max: min,
            lead: 0.0,
            trail: 0.0,
            lane: 0..1,
            chain: OWNER,
        }
    }

    /// Track sizes whose growth limits are their base sizes.
    fn fixed(base: &[f32]) -> TrackSizes {
        TrackSizes {
            base: base.to_vec(),
            limits: base.to_vec(),
            set_by: vec![None; base.len()],
            raised_from: vec![None; base.len()],
        }
    }

    fn gaps(problem: &TrackOwnerProblem) -> Vec<f32> {
        contributions(problem)
            .iter()
            .map(|list| list.iter().map(|c| c.value).fold(0.0, f32::max))
            .collect()
    }

    #[test]
    fn facing_overflow_adds_clearance() {
        let mut a = item(0, 1, 5.0);
        a.trail = 8.0;
        let mut b = item(1, 2, 5.0);
        b.lead = 20.0;
        let problem = TrackOwnerProblem {
            groups: &[],
            tracks: tracks(&[TrackSize::Auto; 2]),
            grids: vec![owner(2, 10.0, 4.0)],
            items: vec![a, b],
        };
        assert_eq!(gaps(&problem), [32.0]);
        let list = &contributions(&problem)[0];
        assert_eq!(
            list[0].term,
            Term::Overflow {
                trailing: Some(0),
                leading: Some(1)
            }
        );
    }

    #[test]
    fn overflow_adds_up_only_within_a_lane() {
        // Two rows: the trailing axis of row 0 faces nothing in row 0, and the leading axis
        // of row 1 faces nothing in row 1, so the gap is the larger of the two, not the sum.
        let mut a = item(0, 1, 5.0);
        a.trail = 25.0;
        let b = item(1, 2, 5.0);
        let mut c = item(0, 1, 5.0);
        c.lane = 1..2;
        let mut d = item(1, 2, 5.0);
        d.lead = 30.0;
        d.lane = 1..2;
        let problem = TrackOwnerProblem {
            groups: &[],
            tracks: tracks(&[TrackSize::Auto; 2]),
            grids: vec![owner(2, 10.0, 0.0)],
            items: vec![a, b, c, d],
        };
        assert_eq!(gaps(&problem), [30.0]);
    }

    #[test]
    fn auto_tracks_take_their_largest_item() {
        let problem = TrackOwnerProblem {
            groups: &[],
            tracks: tracks(&[TrackSize::Auto, TrackSize::Px(30.0)]),
            grids: vec![owner(2, 0.0, 0.0)],
            items: vec![item(0, 1, 40.0), item(0, 1, 70.0), item(1, 2, 90.0)],
        };
        let base = track_sizes(&problem, &[0.0], &[]);
        assert_eq!(base.base, [70.0, 30.0]);
        assert_eq!(base.set_by[0], Some(1));
    }

    #[test]
    fn spans_grow_auto_tracks_before_fr_and_never_px() {
        let fr = fr(1.0);
        let problem = TrackOwnerProblem {
            groups: &[],
            tracks: tracks(&[TrackSize::Px(10.0), fr, TrackSize::Auto]),
            grids: vec![owner(3, 0.0, 0.0)],
            items: vec![item(0, 3, 100.0), item(0, 2, 50.0)],
        };
        let base = track_sizes(&problem, &[5.0, 5.0], &[]);
        // The narrower span goes first: 50 - 10 - 5 = 35 goes to the Fr track, the only one
        // it can grow. Then 100 - (10 + 35 + 0) - 10 = 45 goes to the Auto track.
        assert_eq!(base.base, [10.0, 35.0, 45.0]);
    }

    #[test]
    fn matched_tracks_rise_together() {
        let groups = [(Tag::from("m"), vec![0, 1])];
        let problem = TrackOwnerProblem {
            groups: &groups,
            tracks: tracks(&[TrackSize::Auto; 3]),
            grids: vec![owner(3, 0.0, 0.0)],
            items: vec![item(0, 1, 30.0), item(1, 2, 10.0), item(2, 3, 50.0)],
        };
        let base = track_sizes(&problem, &[0.0, 0.0], &[]);
        assert_eq!(base.base, [30.0, 30.0, 50.0]);
        assert_eq!(base.raised_from[1], Some(10.0));
        assert_eq!(base.raised_from[0], None);
    }

    #[test]
    fn fr_tracks_share_space_by_weight() {
        let tracks = tracks(&[fr(1.0), fr(1.0), fr(2.0)]);
        let allocation = allocate(
            &tracks,
            &fixed(&[100.0, 20.0, 0.0]),
            &[10.0, 10.0],
            420.0,
            Distribute::Stretch,
            f32::INFINITY,
        );
        // 400 to share: one unit is 100, so the base sizes of 100 and 20 hold.
        assert_eq!(allocation.sizes, [100.0, 100.0, 200.0]);
        assert_eq!(allocation.starts, [0.0, 110.0, 220.0]);
    }

    #[test]
    fn a_base_size_above_its_share_drops_out() {
        let fr = fr(1.0);
        let tracks = tracks(&[fr; 3]);
        let allocation = allocate(
            &tracks,
            &fixed(&[200.0, 0.0, 0.0]),
            &[0.0, 0.0],
            300.0,
            Distribute::Stretch,
            f32::INFINITY,
        );
        assert_eq!(allocation.sizes, [200.0, 50.0, 50.0]);
    }

    #[test]
    fn fr_maximums_and_caps_leave_slack() {
        let tracks = tracks(&[Track::fr_max(1.0, 60.0).size, fr(1.0)]);
        let allocation = allocate(
            &tracks,
            &fixed(&[0.0, 0.0]),
            &[0.0],
            200.0,
            Distribute::Center,
            90.0,
        );
        assert_eq!(allocation.sizes, [60.0, 90.0]);
        assert_eq!(allocation.starts, [25.0, 85.0]);
    }

    #[test]
    fn an_fr_basis_is_a_floor_that_items_can_raise() {
        let basis = Track::fr_basis(1.0, 16.0).size;
        let problem = TrackOwnerProblem {
            groups: &[],
            tracks: tracks(&[basis, basis]),
            grids: vec![owner(2, 0.0, 0.0)],
            items: vec![item(0, 1, 10.0), item(1, 2, 20.0)],
        };
        assert_eq!(track_sizes(&problem, &[0.0], &[]).base, [16.0, 20.0]);
    }

    #[test]
    fn the_max_content_unit_counts_from_the_basis() {
        let tracks = tracks(&[
            Track::fr_basis(1.0, 10.0).size,
            Track::fr_basis(2.0, 10.0).size,
        ]);
        let sizes = TrackSizes {
            limits: vec![60.0, 50.0],
            ..fixed(&[10.0, 10.0])
        };
        // The units are 50 and 20, so the tracks are 10 + 50 and 10 + 100.
        assert_eq!(max_content(&tracks, &sizes, &[0.0]), 170.0);
    }

    #[test]
    fn stretch_grows_auto_tracks_without_fr() {
        let tracks = tracks(&[TrackSize::Auto, TrackSize::Px(20.0), TrackSize::Auto]);
        let allocation = allocate(
            &tracks,
            &fixed(&[10.0, 20.0, 30.0]),
            &[0.0, 0.0],
            100.0,
            Distribute::Stretch,
            f32::INFINITY,
        );
        assert_eq!(allocation.sizes, [30.0, 20.0, 50.0]);
    }

    #[test]
    fn distribution_places_leftover_space() {
        let tracks = tracks(&[TrackSize::Auto; 3]);
        let place = |distribute| {
            allocate(
                &tracks,
                &fixed(&[10.0; 3]),
                &[5.0, 5.0],
                100.0,
                distribute,
                f32::INFINITY,
            )
        };
        assert_eq!(place(Distribute::Start).starts, [0.0, 15.0, 30.0]);
        assert_eq!(place(Distribute::Center).starts, [30.0, 45.0, 60.0]);
        assert_eq!(place(Distribute::End).starts, [60.0, 75.0, 90.0]);
        let between = place(Distribute::SpaceBetween);
        assert_eq!(between.starts, [0.0, 45.0, 90.0]);
        assert_eq!(between.gaps, [35.0, 35.0]);
        // 60 left over: 20 around each track, so 10 before the first.
        let around = place(Distribute::SpaceAround);
        assert_eq!(around.starts, [10.0, 45.0, 80.0]);
        assert_eq!(around.gaps, [25.0, 25.0]);
        // 15 before, between and after.
        let evenly = place(Distribute::SpaceEvenly);
        assert_eq!(evenly.starts, [15.0, 45.0, 75.0]);
        assert_eq!(evenly.gaps, [20.0, 20.0]);
    }

    #[test]
    fn base_sizes_that_do_not_fit_overflow() {
        let tracks = tracks(&[TrackSize::Auto; 2]);
        let allocation = allocate(
            &tracks,
            &fixed(&[60.0, 60.0]),
            &[10.0],
            100.0,
            Distribute::Center,
            f32::INFINITY,
        );
        assert_eq!(allocation.sizes, [60.0, 60.0]);
        assert_eq!(allocation.starts, [0.0, 70.0]);
    }
}
