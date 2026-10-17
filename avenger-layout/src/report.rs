//! Reading a solution: its warnings, a plain-text report, and what lies under a point.

use std::fmt::Write;
use std::hash::Hash;

use crate::explain::{
    AspectFit, GapExplanation, GapTerm, RepresentationKind, TrackExplanation, TrackOwnerExplanation,
};
use crate::geometry::{Direction, Rect};
use crate::node::TrackSize;
use crate::solution::{PathStep, Region, RegionKind, Solution, path_text};

/// How far, in layout units, a rectangle may reach before it counts as overlapping or past the
/// canvas.
const TOLERANCE: f32 = 1e-3;

/// Something in a solution that the layout probably did not mean.
#[derive(Clone, Debug, PartialEq)]
pub enum Warning {
    /// A slab overlaps another leaf or slab, as a flushed overhang does when it reaches past the
    /// space next to it.
    Overlap {
        slab: Vec<PathStep>,
        other: Vec<PathStep>,
    },
    /// A leaf or slab reaches past the canvas.
    PastCanvas { path: Vec<PathStep> },
    /// An extra width pass capped a grid's Fr columns to fit its Aspect rows.
    AspectFit(AspectFit),
    /// Measured widths were still growing when the solves ran out.
    StillGrowing,
    /// A measured leaf took an answer from the last measure that does not hold at its size, in
    /// a solution from [`Measured::resize`](crate::Measured::resize).
    Stale { path: Vec<PathStep> },
}

impl Warning {
    /// The warning as text, with each region named by `name`.
    pub(crate) fn describe(&self, name: impl Fn(&[PathStep]) -> String) -> String {
        match self {
            Warning::Overlap { slab, other } => format!("{} overlaps {}", name(slab), name(other)),
            Warning::PastCanvas { path } => format!("{} reaches past the canvas", name(path)),
            Warning::AspectFit(fit) => {
                let mut text = format!(
                    "{} capped its Fr columns at {} to fit its Aspect rows",
                    name(&fit.path),
                    number(fit.cap)
                );
                if fit.remaining > 0.0 {
                    text.push_str(&format!(", {} still too tall", number(fit.remaining)));
                }
                text
            }
            Warning::StillGrowing => {
                "measured widths were still growing at the last solve".to_string()
            }
            Warning::Stale { path } => format!(
                "{} took a measurement that does not hold at its size",
                name(path)
            ),
        }
    }
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe(path_text))
    }
}

/// What lies under one point of a solution.
#[derive(Clone, Debug)]
pub struct PointExplanation<'a, Id> {
    /// The regions whose slot holds the point, in drawing order.
    pub regions: Vec<&'a Region<Id>>,
    /// The tracks that hold the point, in either direction, when the solve explained.
    pub tracks: Vec<TrackAt<'a>>,
    /// The gaps that hold the point, in either direction, when the solve explained.
    pub gaps: Vec<GapAt<'a>>,
}

/// A track under a point.
#[derive(Clone, Copy, Debug)]
pub struct TrackAt<'a> {
    pub owner: &'a TrackOwnerExplanation,
    pub index: usize,
    pub track: &'a TrackExplanation,
}

/// A gap under a point: the gap after track `index`.
#[derive(Clone, Copy, Debug)]
pub struct GapAt<'a> {
    pub owner: &'a TrackOwnerExplanation,
    pub index: usize,
    pub gap: &'a GapExplanation,
}

impl<Id: Clone + Eq + Hash> Solution<Id> {
    /// The parts of the solution that the layout probably did not mean: overlapping slabs,
    /// rectangles past the canvas, aspect fits, widths that were still growing, and stale
    /// measurements.
    pub fn warnings(&self) -> Vec<Warning> {
        let mut warnings = Vec::new();
        let canvas = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        let drawn: Vec<(&Region<Id>, Rect)> = self
            .regions
            .iter()
            .filter(|region| !in_overlay(&region.path))
            .filter_map(|region| match region.kind {
                RegionKind::Leaf => Some((region, region.content)),
                RegionKind::Slab { .. } => Some((region, region.slot)),
                RegionKind::Row { .. }
                | RegionKind::Column { .. }
                | RegionKind::Grid { .. }
                | RegionKind::Wrap { .. }
                | RegionKind::Flow { .. } => None,
            })
            .collect();
        for (index, (region, rect)) in drawn.iter().enumerate() {
            if !matches!(region.kind, RegionKind::Slab { .. }) {
                continue;
            }
            for (other_index, (other, other_rect)) in drawn.iter().enumerate() {
                let both_slabs = matches!(other.kind, RegionKind::Slab { .. });
                if other_index == index
                    || (both_slabs && other_index < index)
                    || region.path.starts_with(&other.path)
                {
                    continue;
                }
                if overlaps(rect, other_rect) {
                    warnings.push(Warning::Overlap {
                        slab: region.path.clone(),
                        other: other.path.clone(),
                    });
                }
            }
        }
        for (region, rect) in &drawn {
            if rect.x < -TOLERANCE
                || rect.y < -TOLERANCE
                || rect.right() > canvas.right() + TOLERANCE
                || rect.bottom() > canvas.bottom() + TOLERANCE
            {
                warnings.push(Warning::PastCanvas {
                    path: region.path.clone(),
                });
            }
        }
        warnings.extend(
            self.explanation
                .aspect_fits
                .iter()
                .cloned()
                .map(Warning::AspectFit),
        );
        if self.explanation.stopped_at_limit {
            warnings.push(Warning::StillGrowing);
        }
        for id in &self.stale {
            if let Some(region) = self.region(id) {
                warnings.push(Warning::Stale {
                    path: region.path.clone(),
                });
            }
        }
        warnings
    }

    /// The regions, tracks and gaps under `point`, with the explanation of each track and gap
    /// when the solve explained.
    pub fn explain_at(&self, point: [f32; 2]) -> PointExplanation<'_, Id> {
        let regions = self
            .regions
            .iter()
            .filter(|region| region.slot.contains(point))
            .collect();
        let mut tracks = Vec::new();
        let mut gaps = Vec::new();
        let owners = self
            .explanation
            .sizes
            .iter()
            .flat_map(|sizes| &sizes.owners);
        for owner in owners {
            let Some(region) = self.at_path(&owner.path) else {
                continue;
            };
            let d = owner.direction.index();
            let cross = owner.direction.cross().index();
            let (cross_start, cross_end) = ranges(&region.content)[cross];
            if point[cross] < cross_start || point[cross] > cross_end {
                continue;
            }
            for (index, track) in owner.tracks.iter().enumerate() {
                if within(point[d], track.start, track.length) {
                    tracks.push(TrackAt {
                        owner,
                        index,
                        track,
                    });
                }
            }
            for (index, gap) in owner.gaps.iter().enumerate() {
                if gap.length > 0.0 && within(point[d], gap.start, gap.length) {
                    gaps.push(GapAt { owner, index, gap });
                }
            }
        }
        PointExplanation {
            regions,
            tracks,
            gaps,
        }
    }

    /// A plain-text report: the canvas, the runs and merges, the warnings, and when the solve
    /// explained, each owner's tracks and gaps with what decided them.
    pub fn report(&self) -> String {
        let mut text = String::new();
        let explanation = &self.explanation;
        let _ = writeln!(
            text,
            "canvas {} x {}, {} region(s), {} solve(s)",
            number(self.size.width),
            number(self.size.height),
            self.regions.len(),
            explanation.solves
        );
        for raised in &explanation.raised_widths {
            let _ = writeln!(
                text,
                "raised width: {} from {} to {}",
                path_text(&raised.path),
                number(raised.from),
                number(raised.to)
            );
        }
        for raised in &explanation.raised_overhangs {
            let _ = writeln!(
                text,
                "raised overhang: {} from {}, {} to {}, {}",
                path_text(&raised.path),
                number(raised.from[0]),
                number(raised.from[1]),
                number(raised.to[0]),
                number(raised.to[1])
            );
        }
        for representation in &explanation.representations {
            let members: Vec<String> = representation
                .members
                .iter()
                .map(|path| path_text(path))
                .collect();
            let _ = writeln!(
                text,
                "{} {} ({}) on {:?}: {} for {}",
                match representation.kind {
                    RepresentationKind::Run => "run",
                    RepresentationKind::Merge => "merge",
                },
                representation.tag,
                representation.content_key,
                representation.side,
                path_text(&representation.representative),
                members.join(", ")
            );
        }
        let warnings = self.warnings();
        if !warnings.is_empty() {
            let _ = writeln!(text, "warnings:");
            for warning in warnings {
                let _ = writeln!(text, "  {warning}");
            }
        }
        let Some(owners) = explanation.sizes.as_ref().map(|sizes| &sizes.owners) else {
            let _ = writeln!(
                text,
                "tracks and gaps were not recorded; solve with `explain` set to see them"
            );
            return text;
        };
        for owner in owners {
            let kind = match self.at_path(&owner.path).map(|region| &region.kind) {
                Some(RegionKind::Row { .. }) => "Row",
                Some(RegionKind::Column { .. }) => "Column",
                Some(RegionKind::Wrap { .. }) => "Wrap",
                Some(RegionKind::Flow { .. }) => "Flow",
                _ => "Grid",
            };
            let _ = writeln!(
                text,
                "{} {kind} ({})",
                path_text(&owner.path),
                match owner.direction {
                    Direction::Horizontal => "columns",
                    Direction::Vertical => "rows",
                }
            );
            for (index, track) in owner.tracks.iter().enumerate() {
                let _ = write!(
                    text,
                    "  track {index}: {}, at {} for {}, base size {}",
                    track_size(&track.size),
                    number(track.start),
                    number(track.length),
                    number(track.base_size)
                );
                if track.growth_limit > track.base_size + TOLERANCE {
                    let _ = write!(text, ", growth limit {}", number(track.growth_limit));
                }
                if let Some(tag) = &track.tag {
                    let _ = write!(text, ", tag {tag}");
                }
                if let Some(path) = &track.set_by {
                    let _ = write!(text, ", set by {}", path_text(path));
                }
                if let (Some(from), Some(tag)) = (track.raised_from, &track.tag) {
                    let _ = write!(text, ", raised from {} by match {tag}", number(from));
                }
                text.push('\n');
            }
            for (index, gap) in owner.gaps.iter().enumerate() {
                let _ = write!(
                    text,
                    "  gap {index}: at {} for {}, ",
                    number(gap.start),
                    number(gap.length)
                );
                match &gap.term {
                    GapTerm::Minimum => {
                        let _ = write!(text, "the minimum of {}", path_text(&gap.grid));
                    }
                    GapTerm::Overflow { trailing, leading } => {
                        let side = |path: &Option<Vec<PathStep>>| {
                            path.as_deref().map_or("nothing".to_string(), path_text)
                        };
                        let _ = write!(
                            text,
                            "the overflow of {} and {} plus the clearance of {}",
                            side(trailing),
                            side(leading),
                            path_text(&gap.grid)
                        );
                    }
                }
                if let (Some(from), Some(tag)) = (gap.raised_from, &gap.tag) {
                    let _ = write!(text, ", raised from {} by match {tag}", number(from));
                }
                text.push('\n');
            }
        }
        text
    }
}

pub(crate) fn in_overlay(path: &[PathStep]) -> bool {
    path.iter().any(|step| matches!(step, PathStep::Overlay(_)))
}

pub(crate) fn overlaps(a: &Rect, b: &Rect) -> bool {
    let width = a.right().min(b.right()) - a.x.max(b.x);
    let height = a.bottom().min(b.bottom()) - a.y.max(b.y);
    width > TOLERANCE && height > TOLERANCE
}

fn within(value: f32, start: f32, length: f32) -> bool {
    value >= start && value <= start + length
}

/// The horizontal and vertical ranges of a rectangle.
pub(crate) fn ranges(rect: &Rect) -> [(f32, f32); 2] {
    [(rect.x, rect.right()), (rect.y, rect.bottom())]
}

fn track_size(size: &TrackSize) -> String {
    match size {
        TrackSize::Auto => "Auto".to_string(),
        TrackSize::Px(length) => format!("{}px", number(*length)),
        TrackSize::FitContent(limit) => format!("fit-content {}", number(*limit)),
        TrackSize::Fr { weight, basis, max } => {
            let mut text = format!("{}fr", number(*weight));
            if *basis > 0.0 {
                text = format!("{}px + {text}", number(*basis));
            }
            if let Some(max) = max {
                text.push_str(&format!(" up to {}", number(*max)));
            }
            text
        }
        TrackSize::Aspect { column, ratio } => {
            format!("Aspect of column {column} at {}", number(*ratio))
        }
    }
}

/// A number with at most two decimals and no trailing zeros.
pub(crate) fn number(value: f32) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}
