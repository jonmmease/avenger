//! Fit a linear domain around marker extents.
//!
//! A [`DomainFit`] holds markers, each with a value and two pixel extents: the lower extent
//! reaches toward smaller values, and the upper extent toward larger values. [`DomainFit::fit`]
//! returns the narrowest increasing domain whose scale keeps every marker inside a plot of a given
//! width, in pixels.
//!
//! The fit is linear in the scale's coordinate. Linear and time scales fit their values. Log, pow,
//! sqrt, and symlog scales fit their transformed values, such as `log10(value)`, and invert the
//! two endpoints of the fitted domain.
//!
//! Only markers on two convex hull chains can decide a fit: the upper-left hull of (value, lower
//! extent) and the upper-right hull of (value, upper extent). Which markers those are does not
//! depend on the width, so one `DomainFit` serves every width, and a resize calls `fit` again.
//! [`DomainFit::union`] combines two fits' markers. A domain shared by plots of the same width is
//! the fit of their union, and appending data is a union with a fit of the new markers. Both fits
//! must use the same coordinate. [`DomainFit::new`] costs O(n log n) for n markers, or O(n) for a
//! side whose extents are all equal, where the chain is one marker. `union` costs O(k log k) for the
//! k markers on the two fits' hull chains, and `fit` costs O(log k).
//!
//! A fit of no markers is `(0, 1)`. Markers with zero extents fit exactly the data's extent. When
//! all values are equal, the domain is one unit wide, with the markers centered in the pixels where
//! they fit. [`DomainFitError::Infeasible`] reports markers that cannot fit in the width, so that a
//! caller can choose a visual fallback.
//!
//! A scale's `zero` and `nice` options are constraints on the fit, not steps after it. Rounding a
//! fitted domain outward compresses the data toward the end that moved less, which can push the
//! marker that touches that end past it. [`DomainFit::including`] adds a value that every fitted
//! domain contains, so `including(0.0)` is `zero`. [`DomainFit::fit_nice`] returns the narrowest
//! domain that `LinearScale`'s `nice` leaves unchanged and that fits every marker. It scans nice
//! spans upward from the narrowest fit, which usually takes one or two spans, each costing one pass
//! over the hull chains. Through the coordinate, it is also the `nice` of pow, sqrt, and symlog
//! scales, which round their transformed domains the same way. Log and time scales round to powers
//! of the base and to calendar units instead, which `fit_nice` does not cover.
//!
//! ```
//! use avenger_scales::scales::domain_fit::DomainFit;
//! // Two markers that reach 5 px on each side.
//! let markers = DomainFit::new(&[0.0, 10.0], &[5.0, 5.0], &[5.0, 5.0]).unwrap();
//! let (start, end) = markers.fit(100.0).unwrap();
//! assert!(start < 0.0 && end > 10.0);
//! // A wider plot needs less room around the data.
//! let (wide_start, wide_end) = markers.fit(400.0).unwrap();
//! assert!(start < wide_start && wide_end < end);
//! // Markers at 5 and 10, in a domain that includes zero and ends on round values.
//! let markers = DomainFit::new(&[5.0, 10.0], &[5.0, 5.0], &[5.0, 5.0]).unwrap();
//! let zeroed = markers.including(0.0).unwrap();
//! assert_eq!(zeroed.fit_nice(100.0, 10.0).unwrap(), (0.0, 11.0));
//! ```

use std::cmp::Ordering;

use thiserror::Error;

use crate::array::tick_increment;
use crate::scales::linear::LinearScale;

/// The most candidate spans that [`DomainFit::fit_nice`] tries before it reports no fit.
const MAX_NICE_SPANS: usize = 1000;

/// How far below a grid point a computed value may fall and still round up to it.
const GRID_TOLERANCE: f64 = 1e-9;

/// Why markers have no fitted domain.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DomainFitError {
    /// A value, extent, or width that cannot be fitted: not finite, a negative extent, a
    /// non-positive width, mismatched lengths, or values whose span exceeds the `f64` range.
    #[error("invalid domain fit input: {0}")]
    InvalidInput(String),
    /// Markers that do not fit in the width at any domain.
    #[error("marker extents cannot fit: {0}")]
    Infeasible(String),
}

/// A marker's value and one of its extents, in pixels.
type Support = (f64, f64);

/// Markers prepared for fitting a linear domain at any width. See the [module docs](self).
///
/// A domain maps value `d` to `a·d + b` pixels. Every marker fits at slope `a` when an offset `b`
/// satisfies `A(a) ≤ b ≤ width − B(a)`, with `A(a) = max(lower − a·d)` and
/// `B(a) = max(upper + a·d)`, so when `h(a) = A(a) + B(a)` is at most the width. `h` is convex and
/// piecewise linear, and the narrowest domain, the steepest slope that fits, lies on a piece where
/// `h` increases.
#[derive(Debug, Clone, PartialEq)]
pub struct DomainFit {
    /// (value, lower extent) on the upper-left hull, from the smallest value up to the largest
    /// lower extent. These markers form `A`.
    lower: Vec<Support>,
    /// (value, upper extent) on the upper-right hull, from the largest upper extent up to the
    /// largest value. These markers form `B`.
    upper: Vec<Support>,
    /// The pieces of `h` where it increases, by increasing slope.
    pieces: Vec<Piece>,
}

/// A piece of `h` on which the same two markers bind.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Piece {
    /// `h` at the piece's smallest slope: the narrowest width at which the piece applies.
    start_width: f64,
    /// The marker whose left edge is at 0 px.
    lower: Support,
    /// The marker whose right edge is at the width.
    upper: Support,
}

impl DomainFit {
    /// Prepares markers from their values and their lower and upper extents in pixels.
    pub fn new(
        values: &[f64],
        lower_extents: &[f64],
        upper_extents: &[f64],
    ) -> Result<Self, DomainFitError> {
        if values.len() != lower_extents.len() || values.len() != upper_extents.len() {
            return Err(DomainFitError::InvalidInput(
                "values and extents must have the same length".into(),
            ));
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(DomainFitError::InvalidInput("values must be finite".into()));
        }
        if lower_extents
            .iter()
            .chain(upper_extents)
            .any(|extent| !extent.is_finite() || *extent < 0.0)
        {
            return Err(DomainFitError::InvalidInput(
                "extents must be finite and nonnegative".into(),
            ));
        }
        let supports = |extents: &[f64]| {
            values
                .iter()
                .copied()
                .zip(extents.iter().copied())
                .collect()
        };
        Ok(Self::from_supports(
            supports(lower_extents),
            supports(upper_extents),
        ))
    }

    /// The markers of both fits, as one fit.
    pub fn union(&self, other: &Self) -> Self {
        let both = |a: &[Support], b: &[Support]| a.iter().chain(b).copied().collect();
        Self::from_supports(
            both(&self.lower, &other.lower),
            both(&self.upper, &other.upper),
        )
    }

    /// The narrowest increasing domain that keeps every marker inside a plot `width` pixels wide.
    pub fn fit(&self, width: f64) -> Result<(f64, f64), DomainFitError> {
        if !width.is_finite() || width <= 0.0 {
            return Err(DomainFitError::InvalidInput(
                "width must be finite and positive".into(),
            ));
        }
        let (Some(&(min, _)), Some(&(max, _))) = (self.lower.first(), self.upper.last()) else {
            return Ok((0.0, 1.0));
        };
        if !(max - min).is_finite() {
            return Err(DomainFitError::InvalidInput(
                "the values' span exceeds the f64 range".into(),
            ));
        }
        if min == max {
            // One value, so each chain is one marker with the largest extent on its side.
            let (value, lower) = self.lower[0];
            let upper = self.upper[0].1;
            if lower + upper > width {
                return Err(DomainFitError::Infeasible(format!(
                    "the markers need {} px, but the plot is {width} px",
                    lower + upper
                )));
            }
            let start = value - (lower + width - upper) / 2.0 / width;
            return Ok((start, start + 1.0));
        }
        let narrowest = self.pieces[0].start_width;
        let piece = self
            .pieces
            .partition_point(|piece| piece.start_width <= width)
            .checked_sub(1)
            .map(|index| self.pieces[index]);
        let infeasible = || {
            DomainFitError::Infeasible(format!(
                "the markers need a plot wider than {narrowest} px, but it is {width} px"
            ))
        };
        let Piece {
            lower: (lower_value, lower),
            upper: (upper_value, upper),
            ..
        } = piece.ok_or_else(infeasible)?;
        let slope = (width - lower - upper) / (upper_value - lower_value);
        if slope <= 0.0 {
            return Err(infeasible());
        }
        Ok((lower_value - lower / slope, upper_value + upper / slope))
    }

    /// This fit with a marker at `value` that has zero extents, so that every fitted domain
    /// contains `value`. A scale's `zero` option is `including(0.0)`.
    pub fn including(&self, value: f64) -> Result<Self, DomainFitError> {
        Ok(self.union(&Self::new(&[value], &[0.0], &[0.0])?))
    }

    /// The narrowest domain that keeps every marker inside a plot `width` pixels wide and that
    /// `LinearScale`'s `nice` option with `count` ticks leaves unchanged. No markers give the nice
    /// domain of `(0, 1)`.
    pub fn fit_nice(&self, width: f64, count: f64) -> Result<(f64, f64), DomainFitError> {
        self.scan_nice(width, count).map(|(domain, _)| domain)
    }

    /// `fit_nice`'s domain, and how many candidate spans it tried.
    fn scan_nice(&self, width: f64, count: f64) -> Result<((f64, f64), usize), DomainFitError> {
        if !count.is_finite() || count <= 0.0 {
            return Err(DomainFitError::InvalidInput(
                "the tick count must be finite and positive".into(),
            ));
        }
        let fitted = self.fit(width)?;
        if self.lower.is_empty() {
            return Ok((nice(fitted, count), 0));
        }
        let spans = nice_spans(fitted.1 - fitted.0, count).take(MAX_NICE_SPANS);
        for (tried, (span, steps, step)) in spans.enumerate() {
            // Every marker fits at this slope for an offset b with A(slope) ≤ b ≤ width − B(slope),
            // so for a domain that starts between −(width − B) / slope and −A / slope.
            let slope = width / span;
            let low = envelope(&self.lower, -slope);
            let high = width - envelope(&self.upper, slope);
            if low > high {
                continue;
            }
            let (earliest, latest) = (-high / slope, -low / slope);
            // Nice writes a domain's ends as whole multiples of the step, `k × step`.
            let mut k = (earliest / step - GRID_TOLERANCE).ceil();
            while k * step <= latest + GRID_TOLERANCE * span {
                let domain = (k * step, (k + steps) * step);
                if nice(domain, count) == domain {
                    return Ok((domain, tried + 1));
                }
                k += 1.0;
            }
        }
        Err(DomainFitError::Infeasible(format!(
            "no nice domain fits within {MAX_NICE_SPANS} candidate spans"
        )))
    }

    fn from_supports(lower: Vec<Support>, upper: Vec<Support>) -> Self {
        let lower = lower_chain(lower);
        let upper = upper_chain(upper);
        let pieces = increasing_pieces(&lower, &upper);
        Self {
            lower,
            upper,
            pieces,
        }
    }
}

/// `A`'s chain: the upper hull from the smallest value up to the first largest extent. When every
/// extent is equal, that is the marker with the smallest value, found without a sort.
fn lower_chain(points: Vec<Support>) -> Vec<Support> {
    if let Some(extent) = constant_extent(&points) {
        let min = points
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min);
        return vec![(min, extent)];
    }
    let mut hull = upper_hull(points);
    let top = (0..hull.len()).fold(0, |top, i| if hull[i].1 > hull[top].1 { i } else { top });
    hull.truncate(top + 1);
    hull
}

/// `B`'s chain: the upper hull from the last largest extent up to the largest value. When every
/// extent is equal, that is the marker with the largest value, found without a sort.
fn upper_chain(points: Vec<Support>) -> Vec<Support> {
    if let Some(extent) = constant_extent(&points) {
        let max = points
            .iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max);
        return vec![(max, extent)];
    }
    let mut hull = upper_hull(points);
    let top = (0..hull.len()).fold(0, |top, i| if hull[i].1 >= hull[top].1 { i } else { top });
    hull.drain(..top);
    hull
}

/// The extent that every point has, if there are points and they all have the same one.
fn constant_extent(points: &[Support]) -> Option<f64> {
    let extent = points.first()?.1;
    points
        .iter()
        .all(|point| point.1 == extent)
        .then_some(extent)
}

/// `max(extent + slope·value)` over a chain.
fn envelope(chain: &[Support], slope: f64) -> f64 {
    chain
        .iter()
        .map(|&(value, extent)| extent + slope * value)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// `LinearScale`'s nice domain for `count` ticks.
fn nice(domain: (f64, f64), count: f64) -> (f64, f64) {
    LinearScale::apply_nice_internal(domain, count).unwrap_or(domain)
}

/// Spans that nice leaves unchanged, in increasing order from `narrowest`, each with its number of
/// steps and its step. A span is nice when it is a multiple of the step that nice picks for it.
/// That step never shrinks as the span grows, so the spans are, for each step in turn, its
/// multiples that are long enough to have that step and short enough to keep it.
fn nice_spans(narrowest: f64, count: f64) -> impl Iterator<Item = (f64, f64, f64)> {
    let mut step = tick_increment(0.0, narrowest, count);
    let mut k = (narrowest / step - GRID_TOLERANCE).ceil();
    std::iter::from_fn(move || loop {
        let span = k * step;
        let found = tick_increment(0.0, span, count);
        if (found - step).abs() <= GRID_TOLERANCE * step {
            k += 1.0;
            // Nice computes the domain's ends from the step that it finds for this span.
            let steps = (span / found).round();
            return Some((steps * found, steps, found));
        }
        if found < step {
            // Too short for the current step. No nice span lies here: a shorter step's multiples
            // all came before.
            k += 1.0;
        } else {
            // The step grows before this span: continue with the larger step, from its first
            // multiple past the last span considered.
            let last = (k - 1.0) * step;
            step = found;
            k = ((last / step + GRID_TOLERANCE).floor() + 1.0)
                .max((narrowest / step - GRID_TOLERANCE).ceil());
        }
    })
}

/// The upper convex hull of the points, by increasing value, keeping the largest extent of equal
/// values.
fn upper_hull(mut points: Vec<Support>) -> Vec<Support> {
    // Values are finite, so they always compare.
    points.sort_by(|p, q| {
        p.0.partial_cmp(&q.0)
            .unwrap_or(Ordering::Equal)
            .then(q.1.total_cmp(&p.1))
    });
    points.dedup_by(|later, kept| later.0 == kept.0);
    let mut hull: Vec<Support> = Vec::with_capacity(points.len());
    for point in points {
        while let [.., o, a] = hull[..] {
            if (a.0 - o.0) * (point.1 - o.1) - (a.1 - o.1) * (point.0 - o.0) < 0.0 {
                break;
            }
            hull.pop();
        }
        hull.push(point);
    }
    hull
}

/// The pieces of `h` where it increases, by increasing slope. From the steepest slope down, `A`'s
/// binding marker moves from the smallest value toward the largest lower extent, and `B`'s from the
/// largest value toward the largest upper extent, each at the slope of the chain edge it crosses.
fn increasing_pieces(lower: &[Support], upper: &[Support]) -> Vec<Piece> {
    if lower.is_empty() {
        return Vec::new();
    }
    // The slopes at which a binding marker changes, and whether it is `A`'s.
    let mut changes: Vec<(f64, bool)> = lower
        .windows(2)
        .map(|edge| ((edge[1].1 - edge[0].1) / (edge[1].0 - edge[0].0), true))
        .chain(
            upper
                .windows(2)
                .map(|edge| ((edge[0].1 - edge[1].1) / (edge[1].0 - edge[0].0), false)),
        )
        .collect();
    changes.sort_by(|p, q| q.0.total_cmp(&p.0));
    let mut changes = changes.into_iter().peekable();
    let (mut a, mut b) = (0, upper.len() - 1);
    let mut pieces = Vec::new();
    loop {
        let (lower, upper) = (lower[a], upper[b]);
        let rate = upper.0 - lower.0;
        if rate <= 0.0 {
            break;
        }
        let start = changes.peek().map_or(0.0, |change| change.0);
        pieces.push(Piece {
            start_width: lower.1 + upper.1 + rate * start,
            lower,
            upper,
        });
        match changes.next() {
            None => break,
            Some((_, true)) => a += 1,
            Some((_, false)) => b -= 1,
        }
    }
    pieces.reverse();
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fit(
        values: &[f64],
        lower: &[f64],
        upper: &[f64],
        width: f64,
    ) -> Result<(f64, f64), DomainFitError> {
        DomainFit::new(values, lower, upper)?.fit(width)
    }

    fn assert_close(actual: (f64, f64), expected: (f64, f64)) {
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-12 * (1.0 + b.abs());
        assert!(
            close(actual.0, expected.0) && close(actual.1, expected.1),
            "{actual:?} != {expected:?}"
        );
    }

    /// The narrowest domain's width by brute force, or `None` when no domain fits: the largest
    /// width that a value-ordered pair needs, checked against every marker.
    fn brute_force(values: &[f64], lower: &[f64], upper: &[f64], width: f64) -> Option<f64> {
        let mut narrowest: f64 = 0.0;
        for i in 0..values.len() {
            for j in 0..values.len() {
                if values[i] < values[j] {
                    let room = width - lower[i] - upper[j];
                    if room <= 0.0 {
                        return None;
                    }
                    narrowest = narrowest.max((values[j] - values[i]) * width / room);
                }
            }
        }
        let slope = width / narrowest;
        let low = (0..values.len())
            .map(|i| lower[i] - slope * values[i])
            .fold(f64::NEG_INFINITY, f64::max);
        let high = (0..values.len())
            .map(|i| width - upper[i] - slope * values[i])
            .fold(f64::INFINITY, f64::min);
        (low <= high + 1e-9).then_some(narrowest)
    }

    fn random(seed: &mut u64) -> f64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn coincident_values_get_a_centered_unit_span() {
        assert_close(fit(&[50.0], &[20.0], &[20.0], 100.0).unwrap(), (49.5, 50.5));
        assert_close(
            fit(&[2.0, 2.0], &[30.0, 10.0], &[5.0, 40.0], 100.0).unwrap(),
            (1.55, 2.55),
        );
        assert_close(
            fit(&[3.0, 3.0], &[0.0; 2], &[0.0; 2], 100.0).unwrap(),
            (2.5, 3.5),
        );
        assert!(matches!(
            fit(&[2.0, 2.0], &[80.0, 0.0], &[0.0, 80.0], 100.0),
            Err(DomainFitError::Infeasible(_))
        ));
    }

    #[test]
    fn distinct_values_get_the_exact_narrowest_domain() {
        // The first marker's left edge binds at 0 px and the second's right edge at 100 px, so the
        // value 0 is at 10 px, the value 10 at 80 px, and the domain is 100/7 wide.
        assert_close(
            fit(&[0.0, 10.0], &[10.0, 0.0], &[0.0, 20.0], 100.0).unwrap(),
            (-10.0 / 7.0, 90.0 / 7.0),
        );
    }

    /// One prepared fit serves every width: each matches a brute-force search, including markers
    /// that force a minimum slope and markers that cannot fit.
    #[test]
    fn fits_match_a_brute_force_search_at_every_width() {
        let mut seed = 17;
        let (mut fitted, mut infeasible) = (0, 0);
        for case in 0..3000 {
            let n = 2 + case % 7;
            let scale = [10.0, 40.0, 95.0][case % 3];
            let values: Vec<f64> = (0..n).map(|_| (random(&mut seed) * 20.0).round()).collect();
            let lower: Vec<f64> = (0..n)
                .map(|_| {
                    if case % 5 == 0 {
                        0.0
                    } else {
                        random(&mut seed) * scale
                    }
                })
                .collect();
            let upper: Vec<f64> = (0..n).map(|_| random(&mut seed) * scale).collect();
            if values.iter().all(|value| *value == values[0]) {
                continue;
            }
            let markers = DomainFit::new(&values, &lower, &upper).unwrap();
            for width in [60.0, 100.0, 250.0] {
                match (
                    markers.fit(width),
                    brute_force(&values, &lower, &upper, width),
                ) {
                    (Ok((start, end)), Some(narrowest)) => {
                        assert!(
                            ((end - start) - narrowest).abs() <= 1e-9 * narrowest,
                            "{values:?} {lower:?} {upper:?} at {width}"
                        );
                        let slope = width / (end - start);
                        for i in 0..n {
                            let x = (values[i] - start) * slope;
                            assert!(x - lower[i] >= -1e-7 && x + upper[i] <= width + 1e-7);
                        }
                        fitted += 1;
                    }
                    (Err(DomainFitError::Infeasible(_)), None) => infeasible += 1,
                    other => panic!("{values:?} {lower:?} {upper:?} at {width}: {other:?}"),
                }
            }
        }
        assert!(
            fitted > 1000 && infeasible > 1000,
            "{fitted} fitted, {infeasible} infeasible"
        );
    }

    #[test]
    fn union_fits_like_one_fit_of_all_markers() {
        let mut seed = 5;
        let none = DomainFit::new(&[], &[], &[]).unwrap();
        for _ in 0..300 {
            let n = 4 + (random(&mut seed) * 8.0) as usize;
            let values: Vec<f64> = (0..n).map(|_| random(&mut seed) * 50.0).collect();
            let lower: Vec<f64> = (0..n).map(|_| random(&mut seed) * 30.0).collect();
            let upper: Vec<f64> = (0..n).map(|_| random(&mut seed) * 30.0).collect();
            let split = 1 + (random(&mut seed) * (n - 1) as f64) as usize;
            let all = DomainFit::new(&values, &lower, &upper).unwrap();
            let first = DomainFit::new(&values[..split], &lower[..split], &upper[..split]).unwrap();
            let second =
                DomainFit::new(&values[split..], &lower[split..], &upper[split..]).unwrap();
            let union = first.union(&second);
            assert_eq!(union, second.union(&first));
            assert_eq!(union.union(&none), union);
            for width in [80.0, 150.0, 400.0] {
                assert_eq!(union.fit(width), all.fit(width));
            }
        }
    }

    #[test]
    fn markers_that_cannot_fit_are_infeasible() {
        let infeasible = |result| matches!(result, Err(DomainFitError::Infeasible(_)));
        // One marker wider than the plot, alone or among others.
        assert!(infeasible(fit(&[0.0], &[60.0], &[60.0], 100.0)));
        assert!(infeasible(fit(
            &[0.0, 1.0],
            &[60.0, 0.0],
            &[60.0, 0.0],
            100.0
        )));
        // A smaller value's lower extent and a larger value's upper extent exceed the width.
        assert!(infeasible(fit(
            &[0.0, 1.0],
            &[60.0, 0.0],
            &[0.0, 50.0],
            100.0
        )));
        // These markers need a plot wider than 30 px, and fit just beyond it.
        let markers = DomainFit::new(&[0.0, 10.0], &[10.0, 0.0], &[0.0, 20.0]).unwrap();
        assert!(infeasible(markers.fit(30.0)));
        assert!(markers.fit(30.01).is_ok());
    }

    #[test]
    fn invalid_input_is_rejected() {
        fn invalid<T>(result: Result<T, DomainFitError>) -> bool {
            matches!(result, Err(DomainFitError::InvalidInput(_)))
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(invalid(DomainFit::new(&[0.0, bad], &[0.0; 2], &[0.0; 2])));
            assert!(invalid(DomainFit::new(&[0.0, 1.0], &[bad, 0.0], &[0.0; 2])));
            assert!(invalid(fit(&[0.0, 1.0], &[0.0; 2], &[0.0; 2], bad)));
        }
        assert!(invalid(DomainFit::new(&[0.0], &[-1.0], &[0.0])));
        assert!(invalid(DomainFit::new(&[0.0, 1.0], &[0.0], &[0.0, 0.0])));
        for width in [0.0, -10.0] {
            assert!(invalid(fit(&[0.0, 1.0], &[0.0; 2], &[0.0; 2], width)));
        }
        let low = DomainFit::new(&[-f64::MAX], &[0.0], &[0.0]).unwrap();
        let high = DomainFit::new(&[f64::MAX], &[0.0], &[0.0]).unwrap();
        assert!(invalid(low.union(&high).fit(100.0)));
    }

    /// A side whose extents are all equal is one marker, the extreme value, and still fits like the
    /// brute-force search.
    #[test]
    fn constant_extents_reduce_a_chain_to_one_marker() {
        let values = [4.0, -3.0, 9.5, 0.0, 2.0];
        let constant = [6.0; 5];
        let varying = [1.0, 7.0, 3.0, 12.0, 5.0];
        let both = DomainFit::new(&values, &constant, &constant).unwrap();
        assert_eq!(
            (both.lower.as_slice(), both.upper.as_slice()),
            (&[(-3.0, 6.0)][..], &[(9.5, 6.0)][..])
        );
        let slope = (100.0 - 12.0) / 12.5;
        assert_close(
            both.fit(100.0).unwrap(),
            (-3.0 - 6.0 / slope, 9.5 + 6.0 / slope),
        );
        let mixed = DomainFit::new(&values, &constant, &varying).unwrap();
        assert_eq!(mixed.lower, [(-3.0, 6.0)]);
        for width in [60.0, 100.0, 250.0] {
            let (start, end) = mixed.fit(width).unwrap();
            let narrowest = brute_force(&values, &constant, &varying, width).unwrap();
            assert!(((end - start) - narrowest).abs() <= 1e-9 * narrowest);
        }
    }

    #[test]
    fn including_adds_a_value_the_domain_must_contain() {
        let markers = DomainFit::new(&[5.0, 15.0], &[10.0, 0.0], &[0.0, 10.0]).unwrap();
        assert_close(markers.fit(100.0).unwrap(), (3.75, 16.25));
        assert_close(
            markers.including(0.0).unwrap().fit(100.0).unwrap(),
            (0.0, 100.0 / 6.0),
        );
        assert_eq!(
            markers.including(10.0).unwrap().fit(100.0),
            markers.fit(100.0)
        );
        for bad in [f64::NAN, f64::INFINITY] {
            assert!(matches!(
                markers.including(bad),
                Err(DomainFitError::InvalidInput(_))
            ));
        }
    }

    /// The narrowest domain on a 1, 2, 5 grid that nice leaves unchanged and that fits every
    /// marker, by brute force.
    fn narrowest_nice(
        markers: &DomainFit,
        values: &[f64],
        width: f64,
        count: f64,
        fits: impl Fn((f64, f64)) -> bool,
    ) -> f64 {
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let narrowest = markers.fit(width).unwrap();
        let first = ((narrowest.1 - narrowest.0) / 100.0).log10().floor() as i32;
        let mut best = f64::INFINITY;
        for power in first..first + 6 {
            for k in [1.0, 2.0, 5.0] {
                let step = 10f64.powf(power as f64) * k;
                let (i0, j0) = (
                    (low.min(narrowest.0) / step).floor(),
                    (high.max(narrowest.1) / step).ceil(),
                );
                for di in 0..40 {
                    for dj in 0..40 {
                        let domain = ((i0 - di as f64) * step, (j0 + dj as f64) * step);
                        if domain.1 - domain.0 < best
                            && nice(domain, count) == domain
                            && fits(domain)
                        {
                            best = domain.1 - domain.0;
                        }
                    }
                }
            }
        }
        best
    }

    /// `fit_nice` gives the narrowest nice domain that fits, by a brute-force search, with and
    /// without zero.
    #[test]
    fn fit_nice_matches_a_brute_force_search_over_nice_domains() {
        let mut seed = 23;
        let mut most_tried = 0;
        for case in 0..240 {
            let n = 2 + case % 8;
            let magnitude = 10f64.powf(random(&mut seed) * 3.0 - 1.0);
            let offset = (random(&mut seed) * 4.0 - 1.0) * 30.0 * magnitude;
            let mut values: Vec<f64> = (0..n)
                .map(|_| offset + random(&mut seed) * 10.0 * magnitude)
                .collect();
            let mut lower: Vec<f64> = (0..n).map(|_| random(&mut seed) * 30.0).collect();
            let mut upper: Vec<f64> = (0..n).map(|_| random(&mut seed) * 30.0).collect();
            let width = 150.0 + random(&mut seed) * 500.0;
            let count = [5.0, 10.0, 20.0][case % 3];
            let mut markers = DomainFit::new(&values, &lower, &upper).unwrap();
            if case % 2 == 1 {
                markers = markers.including(0.0).unwrap();
                values.push(0.0);
                lower.push(0.0);
                upper.push(0.0);
            }
            let fits = |domain: (f64, f64)| {
                let slope = width / (domain.1 - domain.0);
                (0..values.len()).all(|i| {
                    let x = (values[i] - domain.0) * slope;
                    x - lower[i] >= -1e-7 && x + upper[i] <= width + 1e-7
                })
            };
            let (domain, tried) = markers.scan_nice(width, count).unwrap();
            most_tried = most_tried.max(tried);
            assert!(fits(domain), "{domain:?} clips");
            assert_eq!(nice(domain, count), domain, "nice changes {domain:?}");
            let best = narrowest_nice(&markers, &values, width, count, fits);
            assert!(
                best.is_finite(),
                "the brute force found no nice domain for case {case}"
            );
            assert!(
                ((domain.1 - domain.0) - best).abs() <= 1e-9 * best,
                "{domain:?} is wider than {best}"
            );
        }
        // The cap of 1,000 candidate spans is a backstop far above what fits need.
        assert!(most_tried <= 20, "a fit tried {most_tried} spans");
    }

    /// Nice writes a domain's ends as `k × step`, so a nice domain with fractional steps can end at
    /// 9.200000000000001 rather than 9.2. For zero extents, the fit is nice's domain of the data.
    #[test]
    fn fit_nice_gives_the_ends_that_nice_writes() {
        for (values, expected) in [([8.31, 9.15], (8.3, 9.2)), ([0.95, 2.05], (0.9, 2.1))] {
            let markers = DomainFit::new(&values, &[0.0; 2], &[0.0; 2]).unwrap();
            let domain = markers.fit_nice(100.0, 10.0).unwrap();
            assert_eq!(domain, nice((values[0], values[1]), 10.0));
            assert_close(domain, expected);
        }
    }

    #[test]
    fn fit_nice_handles_no_markers_one_value_and_invalid_counts() {
        let none = DomainFit::new(&[], &[], &[]).unwrap();
        assert_eq!(none.fit_nice(100.0, 10.0), Ok(nice((0.0, 1.0), 10.0)));
        let single = DomainFit::new(&[3.7], &[20.0], &[40.0]).unwrap();
        let (start, end) = single.fit_nice(200.0, 10.0).unwrap();
        let x = (3.7 - start) * 200.0 / (end - start);
        assert!(
            end - start >= 1.0 && x >= 20.0 && x + 40.0 <= 200.0,
            "{start}..{end}"
        );
        assert_eq!(nice((start, end), 10.0), (start, end));
        for count in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                single.fit_nice(200.0, count),
                Err(DomainFitError::InvalidInput(_))
            ));
        }
        assert!(matches!(
            single.fit_nice(50.0, 10.0),
            Err(DomainFitError::Infeasible(_))
        ));
    }

    #[test]
    fn no_markers_and_zero_extents_fit_the_data() {
        assert_eq!(fit(&[], &[], &[], 100.0), Ok((0.0, 1.0)));
        assert_eq!(
            fit(&[3.0, -2.0, 7.0], &[0.0; 3], &[0.0; 3], 100.0),
            Ok((-2.0, 7.0))
        );
    }

    /// Almost every marker is on a hull chain when the extents are a smooth concave function of
    /// the value, so a quadratic preparation or fit would hang here.
    #[test]
    fn large_inputs_prepare_once_and_fit_quickly() {
        let n = 100_000;
        let values: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let extents: Vec<f64> = (0..n)
            .map(|i| 4.0 + 40.0 * (std::f64::consts::PI * i as f64 / (n - 1) as f64).sin())
            .collect();
        let markers = DomainFit::new(&values, &extents, &extents).unwrap();
        for width in [150.0, 300.0, 600.0] {
            let (start, end) = markers.fit(width).unwrap();
            let slope = width / (end - start);
            for i in 0..n {
                let x = (values[i] - start) * slope;
                assert!(x - extents[i] >= -1e-6 && x + extents[i] <= width + 1e-6);
            }
        }
    }
}
