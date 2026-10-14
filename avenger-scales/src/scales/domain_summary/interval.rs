use std::cmp::Ordering;

use super::DomainFitError;
use crate::array::tick_increment;

/// How many times wider than the narrowest fit, or than one step when the step is wider,
/// [`IntervalSummary::fit_nice`] and [`IntervalSummary::fit_grid`] let a nice domain be. Ordinary
/// data needs at most about 2×. Markers that leave almost no room can need thousands of times the
/// width, which would crush the data into a sliver.
const MAX_NICE_WIDENING: f64 = 3.0;

/// How far below a grid point a computed value may fall and still round up to it.
const GRID_TOLERANCE: f64 = 1e-9;

/// A marker's value and one of its extents, in pixels.
type Marker = (f64, f64);

/// A summary of data for an interval domain: markers prepared for fitting a linear domain at any
/// width.
/// See the [module docs](super).
///
/// A domain maps value `d` to `a·d + b` pixels. Every marker fits at slope `a` when an offset `b`
/// satisfies `A(a) ≤ b ≤ width − B(a)`, with `A(a) = max(lower − a·d)` and
/// `B(a) = max(upper + a·d)`, so when `h(a) = A(a) + B(a)` is at most the width. `h` is convex and
/// piecewise linear, and the narrowest domain, the steepest slope that fits, lies on a piece where
/// `h` increases.
#[derive(Debug, Clone, PartialEq)]
pub struct IntervalSummary {
    /// (value, lower extent) on the upper-left hull, from the smallest value up to the largest
    /// lower extent. These markers form `A`.
    lower: Vec<Marker>,
    /// (value, upper extent) on the upper-right hull, from the largest upper extent up to the
    /// largest value. These markers form `B`.
    upper: Vec<Marker>,
    /// The pieces of `h` where it increases, by increasing slope.
    pieces: Vec<Piece>,
}

/// A piece of `h` on which the same two markers bind.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Piece {
    /// `h` at the piece's smallest slope: the narrowest width at which the piece applies.
    start_width: f64,
    /// The marker whose left edge is at 0 px.
    lower: Marker,
    /// The marker whose right edge is at the width.
    upper: Marker,
}

impl IntervalSummary {
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
        let markers = |extents: &[f64]| {
            values
                .iter()
                .copied()
                .zip(extents.iter().copied())
                .collect()
        };
        Ok(Self::from_markers(
            markers(lower_extents),
            markers(upper_extents),
        ))
    }

    /// The markers of both fits, as one fit.
    pub fn merge(&self, other: &Self) -> Self {
        let both = |a: &[Marker], b: &[Marker]| a.iter().chain(b).copied().collect();
        Self::from_markers(
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
        Ok(self.merge(&Self::new(&[value], &[0.0], &[0.0])?))
    }

    /// Whether some marker reaches past its value.
    pub(crate) fn has_extents(&self) -> bool {
        self.lower
            .iter()
            .chain(&self.upper)
            .any(|&(_, extent)| extent > 0.0)
    }

    /// The smallest and largest value, or `None` without markers.
    pub fn extent(&self) -> Option<(f64, f64)> {
        Some((self.lower.first()?.0, self.upper.last()?.0))
    }

    /// This summary with every extent multiplied by `factor`. Plots of different widths share a
    /// domain by scaling each plot's summary by `1 / width`, merging them, and fitting at
    /// width 1.
    pub fn scale_extents(&self, factor: f64) -> Result<Self, DomainFitError> {
        if !factor.is_finite() || factor < 0.0 {
            return Err(DomainFitError::InvalidInput(
                "the extent factor must be finite and nonnegative".into(),
            ));
        }
        // Scaling the extents scales the hull, so the chains still hold every marker that can bind.
        let scaled = |chain: &[Marker]| {
            chain
                .iter()
                .map(|&(value, extent)| (value, extent * factor))
                .collect()
        };
        Ok(Self::from_markers(scaled(&self.lower), scaled(&self.upper)))
    }

    /// The narrowest domain that keeps every marker inside a plot `width` pixels wide and that
    /// nice leaves unchanged with `count` ticks. No markers give the nice domain of
    /// `(0, 1)`. A nice domain more than 3× as wide as the narrowest fit is `Infeasible`, so that
    /// the caller can use `fit` instead.
    pub fn fit_nice(&self, width: f64, count: f64) -> Result<(f64, f64), DomainFitError> {
        self.fit_nice_step(width, NiceStep::Auto(count))
    }

    /// The narrowest domain that keeps every marker inside a plot `width` pixels wide and whose
    /// ends are whole multiples of `step`. In a log scale's coordinate, whole numbers are powers of
    /// the base, so its nice domain is `fit_grid(width, 1.0)`. No markers give the grid's domain
    /// around `(0, 1)`. A domain more than 3× as wide as the narrowest fit, or as one step when the
    /// step is wider, is `Infeasible`, so that the caller can use `fit` instead.
    pub fn fit_grid(&self, width: f64, step: f64) -> Result<(f64, f64), DomainFitError> {
        self.fit_nice_step(width, NiceStep::Fixed(step))
    }

    /// The narrowest nice domain that fits, with ends on whole multiples of `step`.
    pub(crate) fn fit_nice_step(
        &self,
        width: f64,
        step: NiceStep,
    ) -> Result<(f64, f64), DomainFitError> {
        self.scan(width, step).map(|(domain, _)| domain)
    }

    /// The domain of `fit_nice_step`, and how many candidate spans it tried.
    fn scan(&self, width: f64, nice: NiceStep) -> Result<((f64, f64), usize), DomainFitError> {
        nice.validate()?;
        let fitted = self.fit(width)?;
        if self.lower.is_empty() {
            return Ok((nice.domain_around(fitted), 0));
        }
        let narrowest = fitted.1 - fitted.0;
        let first_step = nice.first_step(narrowest);
        let widest = MAX_NICE_WIDENING * narrowest.max(first_step);
        let spans = nice
            .spans(narrowest)
            .take_while(|&(span, _, _)| span <= widest);
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
            // A nice domain's ends are whole multiples of the step, `k × step`.
            let mut k = (earliest / step - GRID_TOLERANCE).ceil();
            while k * step <= latest + GRID_TOLERANCE * span {
                let domain = (k * step, (k + steps) * step);
                if nice.accepts(domain) {
                    return Ok((domain, tried + 1));
                }
                k += 1.0;
            }
        }
        Err(DomainFitError::Infeasible(if first_step > narrowest {
            format!("no nice domain fits within {MAX_NICE_WIDENING}× the grid step of {first_step}")
        } else {
            format!(
                "no nice domain fits within {MAX_NICE_WIDENING}× the narrowest fit of {narrowest}"
            )
        }))
    }

    fn from_markers(lower: Vec<Marker>, upper: Vec<Marker>) -> Self {
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
fn lower_chain(points: Vec<Marker>) -> Vec<Marker> {
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
fn upper_chain(points: Vec<Marker>) -> Vec<Marker> {
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
fn constant_extent(points: &[Marker]) -> Option<f64> {
    let extent = points.first()?.1;
    points
        .iter()
        .all(|point| point.1 == extent)
        .then_some(extent)
}

/// `max(extent + slope·value)` over a chain.
fn envelope(chain: &[Marker], slope: f64) -> f64 {
    chain
        .iter()
        .map(|&(value, extent)| extent + slope * value)
        .fold(f64::NEG_INFINITY, f64::max)
}

/// The nice domain around `domain` for `count` ticks: both ends move outward to whole multiples of
/// the tick step, until the step stops changing. Its fixed points are the domains that `fit_nice`
/// searches.
fn nice(domain: (f64, f64), count: f64) -> (f64, f64) {
    let (domain_start, domain_end) = domain;

    if domain_start == domain_end || domain_start.is_nan() || domain_end.is_nan() {
        return domain;
    }

    let (mut start, mut stop) = if domain_start <= domain_end {
        (domain_start, domain_end)
    } else {
        (domain_end, domain_start)
    };

    let mut prestep = 0.0;
    let mut max_iter = 10;

    while max_iter > 0 {
        let step = tick_increment(start, stop, count);

        if step == prestep {
            if domain_start <= domain_end {
                return (start, stop);
            } else {
                return (stop, start);
            }
        } else if step > 0.0 {
            start = (start / step).floor() * step;
            stop = (stop / step).ceil() * step;
        } else if step < 0.0 {
            start = (start * step).ceil() / step;
            stop = (stop * step).floor() / step;
        } else {
            break;
        }

        prestep = step;
        max_iter -= 1;
    }

    if domain_start <= domain_end {
        (start, stop)
    } else {
        (stop, start)
    }
}

/// The step whose whole multiples are a nice domain's ends, in the scale's coordinate.
#[derive(Debug, Clone, Copy)]
pub enum NiceStep {
    /// The step that nice chooses for each span, to give about this many ticks.
    Auto(f64),
    /// One step for every span, such as 1 for powers of the base in a log coordinate.
    Fixed(f64),
}

impl NiceStep {
    fn validate(self) -> Result<(), DomainFitError> {
        match self {
            NiceStep::Auto(count) if !count.is_finite() || count <= 0.0 => Err(
                DomainFitError::InvalidInput("the tick count must be finite and positive".into()),
            ),
            NiceStep::Fixed(step) if !step.is_finite() || step <= 0.0 => Err(
                DomainFitError::InvalidInput("the grid step must be finite and positive".into()),
            ),
            _ => Ok(()),
        }
    }

    /// The nice domain around `domain`.
    fn domain_around(self, domain: (f64, f64)) -> (f64, f64) {
        match self {
            NiceStep::Auto(count) => nice(domain, count),
            NiceStep::Fixed(step) => (
                (domain.0 / step).floor() * step,
                (domain.1 / step).ceil() * step,
            ),
        }
    }

    /// The step of the narrowest span that the scan considers.
    fn first_step(self, narrowest: f64) -> f64 {
        match self {
            NiceStep::Auto(count) => tick_increment(0.0, narrowest, count),
            NiceStep::Fixed(step) => step,
        }
    }

    /// Candidate spans in increasing order from `narrowest`, each with its number of steps and its
    /// step.
    fn spans(self, narrowest: f64) -> Box<dyn Iterator<Item = (f64, f64, f64)>> {
        match self {
            NiceStep::Auto(count) => Box::new(nice_spans(narrowest, count)),
            NiceStep::Fixed(step) => {
                let first = (narrowest / step - GRID_TOLERANCE).ceil().max(1.0);
                Box::new(
                    std::iter::successors(Some(first), |steps| Some(steps + 1.0))
                        .map(move |steps| (steps * step, steps, step)),
                )
            }
        }
    }

    /// Whether a candidate domain is nice.
    fn accepts(self, domain: (f64, f64)) -> bool {
        match self {
            NiceStep::Auto(count) => nice(domain, count) == domain,
            NiceStep::Fixed(_) => true,
        }
    }
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
fn upper_hull(mut points: Vec<Marker>) -> Vec<Marker> {
    // Values are finite, so they always compare.
    points.sort_by(|p, q| {
        p.0.partial_cmp(&q.0)
            .unwrap_or(Ordering::Equal)
            .then(q.1.total_cmp(&p.1))
    });
    points.dedup_by(|later, kept| later.0 == kept.0);
    let mut hull: Vec<Marker> = Vec::with_capacity(points.len());
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
fn increasing_pieces(lower: &[Marker], upper: &[Marker]) -> Vec<Piece> {
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
        IntervalSummary::new(values, lower, upper)?.fit(width)
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
            let markers = IntervalSummary::new(&values, &lower, &upper).unwrap();
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
    fn merge_fits_like_one_fit_of_all_markers() {
        let mut seed = 5;
        let none = IntervalSummary::new(&[], &[], &[]).unwrap();
        for _ in 0..300 {
            let n = 4 + (random(&mut seed) * 8.0) as usize;
            let values: Vec<f64> = (0..n).map(|_| random(&mut seed) * 50.0).collect();
            let lower: Vec<f64> = (0..n).map(|_| random(&mut seed) * 30.0).collect();
            let upper: Vec<f64> = (0..n).map(|_| random(&mut seed) * 30.0).collect();
            let split = 1 + (random(&mut seed) * (n - 1) as f64) as usize;
            let all = IntervalSummary::new(&values, &lower, &upper).unwrap();
            let first =
                IntervalSummary::new(&values[..split], &lower[..split], &upper[..split]).unwrap();
            let second =
                IntervalSummary::new(&values[split..], &lower[split..], &upper[split..]).unwrap();
            let merged = first.merge(&second);
            assert_eq!(merged, second.merge(&first));
            assert_eq!(merged.merge(&none), merged);
            for width in [80.0, 150.0, 400.0] {
                assert_eq!(merged.fit(width), all.fit(width));
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
        let markers = IntervalSummary::new(&[0.0, 10.0], &[10.0, 0.0], &[0.0, 20.0]).unwrap();
        assert!(infeasible(markers.fit(30.0)));
        assert!(markers.fit(30.01).is_ok());
    }

    #[test]
    fn invalid_input_is_rejected() {
        fn invalid<T>(result: Result<T, DomainFitError>) -> bool {
            matches!(result, Err(DomainFitError::InvalidInput(_)))
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(invalid(IntervalSummary::new(
                &[0.0, bad],
                &[0.0; 2],
                &[0.0; 2]
            )));
            assert!(invalid(IntervalSummary::new(
                &[0.0, 1.0],
                &[bad, 0.0],
                &[0.0; 2]
            )));
            assert!(invalid(fit(&[0.0, 1.0], &[0.0; 2], &[0.0; 2], bad)));
        }
        assert!(invalid(IntervalSummary::new(&[0.0], &[-1.0], &[0.0])));
        assert!(invalid(IntervalSummary::new(
            &[0.0, 1.0],
            &[0.0],
            &[0.0, 0.0]
        )));
        for width in [0.0, -10.0] {
            assert!(invalid(fit(&[0.0, 1.0], &[0.0; 2], &[0.0; 2], width)));
        }
        let low = IntervalSummary::new(&[-f64::MAX], &[0.0], &[0.0]).unwrap();
        let high = IntervalSummary::new(&[f64::MAX], &[0.0], &[0.0]).unwrap();
        assert!(invalid(low.merge(&high).fit(100.0)));
    }

    /// A side whose extents are all equal is one marker, the extreme value, and still fits like the
    /// brute-force search.
    #[test]
    fn constant_extents_reduce_a_chain_to_one_marker() {
        let values = [4.0, -3.0, 9.5, 0.0, 2.0];
        let constant = [6.0; 5];
        let varying = [1.0, 7.0, 3.0, 12.0, 5.0];
        let both = IntervalSummary::new(&values, &constant, &constant).unwrap();
        assert_eq!(
            (both.lower.as_slice(), both.upper.as_slice()),
            (&[(-3.0, 6.0)][..], &[(9.5, 6.0)][..])
        );
        let slope = (100.0 - 12.0) / 12.5;
        assert_close(
            both.fit(100.0).unwrap(),
            (-3.0 - 6.0 / slope, 9.5 + 6.0 / slope),
        );
        let mixed = IntervalSummary::new(&values, &constant, &varying).unwrap();
        assert_eq!(mixed.lower, [(-3.0, 6.0)]);
        for width in [60.0, 100.0, 250.0] {
            let (start, end) = mixed.fit(width).unwrap();
            let narrowest = brute_force(&values, &constant, &varying, width).unwrap();
            assert!(((end - start) - narrowest).abs() <= 1e-9 * narrowest);
        }
    }

    #[test]
    fn including_adds_a_value_the_domain_must_contain() {
        let markers = IntervalSummary::new(&[5.0, 15.0], &[10.0, 0.0], &[0.0, 10.0]).unwrap();
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
        markers: &IntervalSummary,
        values: &[f64],
        width: f64,
        count: f64,
        fits: &impl Fn((f64, f64)) -> bool,
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
            let mut markers = IntervalSummary::new(&values, &lower, &upper).unwrap();
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
            let best = narrowest_nice(&markers, &values, width, count, &fits);
            assert!(
                best.is_finite(),
                "the brute force found no nice domain for case {case}"
            );
            let narrowest = markers.fit(width).map(|(start, end)| end - start).unwrap();
            if best > MAX_NICE_WIDENING * narrowest * (1.0 + 1e-9) {
                assert!(matches!(
                    markers.fit_nice(width, count),
                    Err(DomainFitError::Infeasible(_))
                ));
                continue;
            }
            let (domain, tried) = markers.scan(width, NiceStep::Auto(count)).unwrap();
            most_tried = most_tried.max(tried);
            assert!(fits(domain), "{domain:?} clips");
            assert_eq!(nice(domain, count), domain, "nice changes {domain:?}");
            assert!(
                ((domain.1 - domain.0) - best).abs() <= 1e-9 * best,
                "{domain:?} is wider than {best}"
            );
        }
        assert!(most_tried <= 20, "a fit tried {most_tried} spans");
    }

    /// Nice writes a domain's ends as `k × step`, so a nice domain with fractional steps can end at
    /// 9.200000000000001 rather than 9.2. For zero extents, the fit is nice's domain of the data.
    #[test]
    fn fit_nice_gives_the_ends_that_nice_writes() {
        for (values, expected) in [([8.31, 9.15], (8.3, 9.2)), ([0.95, 2.05], (0.9, 2.1))] {
            let markers = IntervalSummary::new(&values, &[0.0; 2], &[0.0; 2]).unwrap();
            let domain = markers.fit_nice(100.0, 10.0).unwrap();
            assert_eq!(domain, nice((values[0], values[1]), 10.0));
            assert_close(domain, expected);
        }
    }

    /// Markers that leave a thousandth of the plot free fit, but any nice domain that fits is far
    /// more than 3× as wide, so `fit_nice` refuses it.
    #[test]
    fn fit_nice_refuses_nice_domains_far_wider_than_the_fit() {
        let markers = IntervalSummary::new(&[3.7, 4.6], &[199.8, 0.0], &[0.0, 199.8]).unwrap();
        assert!(markers.fit(400.0).is_ok());
        assert!(matches!(
            markers.fit_nice(400.0, 10.0),
            Err(DomainFitError::Infeasible(_))
        ));
    }

    #[test]
    fn fit_nice_handles_no_markers_one_value_and_invalid_counts() {
        let none = IntervalSummary::new(&[], &[], &[]).unwrap();
        assert_eq!(none.fit_nice(100.0, 10.0), Ok(nice((0.0, 1.0), 10.0)));
        let single = IntervalSummary::new(&[3.7], &[20.0], &[40.0]).unwrap();
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
        let markers = IntervalSummary::new(&values, &extents, &extents).unwrap();
        for width in [150.0, 300.0, 600.0] {
            let (start, end) = markers.fit(width).unwrap();
            let slope = width / (end - start);
            for i in 0..n {
                let x = (values[i] - start) * slope;
                assert!(x - extents[i] >= -1e-6 && x + extents[i] <= width + 1e-6);
            }
        }
    }

    #[test]
    fn nice_widens_to_multiples_of_its_step() {
        assert_eq!(nice((1.1, 10.9), 10.0), (1.0, 11.0));
        assert_eq!(nice((10.9, 1.1), 10.0), (11.0, 1.0));
    }

    #[test]
    fn extent_is_the_smallest_and_largest_value() {
        assert_eq!(IntervalSummary::new(&[], &[], &[]).unwrap().extent(), None);
        let markers =
            IntervalSummary::new(&[3.0, -1.0, 7.0], &[1.0, 5.0, 0.0], &[2.0, 0.0, 9.0]).unwrap();
        assert_eq!(markers.extent(), Some((-1.0, 7.0)));
        let constant = IntervalSummary::new(&[3.0, -1.0, 7.0], &[4.0; 3], &[4.0; 3]).unwrap();
        assert_eq!(constant.extent(), Some((-1.0, 7.0)));
    }

    /// Scaling the extents by `1 / width` and fitting at width 1 is the fit at `width`, so plots of
    /// different widths share a domain in which each plot's markers fit.
    #[test]
    fn scaled_extents_let_plots_of_different_widths_share_a_domain() {
        let markers = IntervalSummary::new(
            &[0.0, 4.0, 9.0, 3.0],
            &[10.0, 25.0, 0.0, 5.0],
            &[0.0, 8.0, 30.0, 12.0],
        )
        .unwrap();
        for width in [80.0, 200.0, 640.0] {
            assert_close(
                markers
                    .scale_extents(1.0 / width)
                    .unwrap()
                    .fit(1.0)
                    .unwrap(),
                markers.fit(width).unwrap(),
            );
        }
        // 10 px markers in plots 400 and 200 px wide: the value 0 binds at plot A's left edge and
        // the value 12 at plot B's right edge, where 10 px is a larger part of the plot.
        let a = IntervalSummary::new(&[0.0, 10.0], &[10.0; 2], &[10.0; 2]).unwrap();
        let b = IntervalSummary::new(&[2.0, 12.0], &[10.0; 2], &[10.0; 2]).unwrap();
        let shared = a
            .scale_extents(1.0 / 400.0)
            .unwrap()
            .merge(&b.scale_extents(1.0 / 200.0).unwrap())
            .fit(1.0)
            .unwrap();
        let span = 12.0 / 0.925;
        assert_close(shared, (-0.025 * span, 0.975 * span));
        assert_eq!(a.scale_extents(0.0).unwrap().fit(100.0), Ok((0.0, 10.0)));
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                a.scale_extents(bad),
                Err(DomainFitError::InvalidInput(_))
            ));
        }
    }

    /// `fit_grid` gives the narrowest domain on whole multiples of the step that fits, by a
    /// brute-force search, including data narrower than one step.
    #[test]
    fn fit_grid_matches_a_brute_force_search_over_grid_domains() {
        let mut seed = 31;
        let (mut fitted, mut refused) = (0, 0);
        for case in 0..300 {
            // Coincident values have no narrowest domain, so `fit` gives them a unit span.
            let n = 2 + case % 5;
            let offset = (random(&mut seed) - 0.5) * 8.0;
            let spread = [0.2, 1.0, 4.0][case % 3];
            let values: Vec<f64> = (0..n)
                .map(|_| offset + random(&mut seed) * spread)
                .collect();
            let lower: Vec<f64> = (0..n).map(|_| random(&mut seed) * 40.0).collect();
            let upper: Vec<f64> = (0..n).map(|_| random(&mut seed) * 40.0).collect();
            let width = 200.0 + random(&mut seed) * 400.0;
            let step = [1.0, 0.5, 2.5][(case / 3) % 3];
            let markers = IntervalSummary::new(&values, &lower, &upper).unwrap();
            let Ok(narrowest) = markers.fit(width) else {
                continue;
            };
            let fits = |domain: (f64, f64)| {
                let slope = width / (domain.1 - domain.0);
                (0..n).all(|i| {
                    let x = (values[i] - domain.0) * slope;
                    x - lower[i] >= -1e-7 && x + upper[i] <= width + 1e-7
                })
            };
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let mut best = f64::INFINITY;
            for i in 0..80 {
                for j in 0..80 {
                    let domain = (
                        ((low / step).floor() - i as f64) * step,
                        ((high / step).ceil() + j as f64) * step,
                    );
                    if domain.1 - domain.0 < best && fits(domain) {
                        best = domain.1 - domain.0;
                    }
                }
            }
            let limit = MAX_NICE_WIDENING * (narrowest.1 - narrowest.0).max(step);
            match markers.fit_grid(width, step) {
                Ok(domain) => {
                    assert!(fits(domain), "{domain:?} clips");
                    assert!(
                        ((domain.1 - domain.0) - best).abs() <= 1e-9 * best,
                        "{domain:?} is not {best} wide"
                    );
                    for end in [domain.0, domain.1] {
                        assert!(
                            ((end / step).round() * step - end).abs() <= 1e-9 * step.max(end.abs())
                        );
                    }
                    fitted += 1;
                }
                Err(DomainFitError::Infeasible(_)) => {
                    assert!(best > limit * (1.0 - 1e-9), "refused a domain of {best}");
                    refused += 1;
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(fitted > 100, "{fitted} fitted, {refused} refused");
        // Data within one step still reaches a whole step, and no markers give the grid's (0, 1).
        let narrow = IntervalSummary::new(&[1.3, 1.6], &[0.0; 2], &[0.0; 2]).unwrap();
        assert_eq!(narrow.fit_grid(100.0, 1.0), Ok((1.0, 2.0)));
        let none = IntervalSummary::new(&[], &[], &[]).unwrap();
        assert_eq!(none.fit_grid(100.0, 1.0), Ok((0.0, 1.0)));
        for step in [0.0, -1.0, f64::NAN] {
            assert!(matches!(
                narrow.fit_grid(100.0, step),
                Err(DomainFitError::InvalidInput(_))
            ));
        }
    }
}
