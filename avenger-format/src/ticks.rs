/// How a set of tick values is spaced, which decides what their labels share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TickSpacing {
    /// Evenly spaced values, such as a linear scale's ticks. Labels can share one precision
    /// and one SI or compact unit.
    #[default]
    Uniform,
    /// Values spanning several magnitudes, such as a log scale's ticks. Each label keeps its
    /// own precision and unit.
    Varying,
}

/// The spacing and magnitude of a set of tick values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickStep {
    /// Smallest distance between distinct values, rounded to the resolution.
    /// For a single value, this is the place value of its last significant digit.
    pub step: f64,
    /// Largest absolute value.
    pub magnitude: f64,
    /// Power of ten of the finest decimal place the values need, such as -2 for 0.05, 1.05,
    /// and 2.05. Each value lies within 0.1% of the step from a multiple of this place.
    pub resolution: i32,
}

impl TickStep {
    /// Infer spacing from the values, ignoring NaN and infinities.
    /// Returns `None` when no value is finite.
    pub fn infer(values: &[f64]) -> Option<Self> {
        let mut finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        finite.sort_by(f64::total_cmp);
        finite.dedup();
        let magnitude = finite.iter().fold(0.0_f64, |max, v| max.max(v.abs()));
        let (step, resolution) = match finite.as_slice() {
            [] => return None,
            [value] => {
                let resolution = last_digit_exponent(*value);
                (power_of_ten(resolution), resolution)
            }
            values => grid_step(values),
        };
        Some(Self {
            step,
            magnitude,
            resolution,
        })
    }
}

/// The smallest gap and the coarsest power-of-ten grid that holds every value within 0.1% of
/// that gap. The tolerance absorbs rounding in computed ticks, such as 0.30000000000000004.
fn grid_step(values: &[f64]) -> (f64, i32) {
    let gap = values
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(f64::INFINITY, f64::min);
    let tolerance = gap * 1e-3;
    // Starting one place coarser handles a gap just below a power of ten, such as 0.0999999.
    // Four places below the gap's leading digit, rounding error stays within the tolerance,
    // so values off any decimal grid, such as thirds, keep about four digits.
    let coarsest = gap.log10().floor() as i32 + 1;
    let resolution = (coarsest - 3..=coarsest)
        .rev()
        .find(|&exponent| {
            values
                .iter()
                .all(|&value| (value - round_to(value, exponent)).abs() <= tolerance)
        })
        .unwrap_or(coarsest - 4);
    (round_to(gap, resolution), resolution)
}

/// Round to a multiple of 10^exponent. Fractional places divide by an exact power of ten,
/// so 3 tenths becomes 0.3 rather than 0.30000000000000004.
fn round_to(value: f64, exponent: i32) -> f64 {
    let unit = 10_f64.powi(exponent.abs());
    if exponent < 0 {
        (value * unit).round() / unit
    } else {
        (value / unit).round() * unit
    }
}

/// 10^exponent, dividing by an exact power of ten for fractional places.
fn power_of_ten(exponent: i32) -> f64 {
    let unit = 10_f64.powi(exponent.abs());
    if exponent < 0 {
        1.0 / unit
    } else {
        unit
    }
}

/// The power of ten of the last significant digit, ignoring rounding error beyond 15 digits.
fn last_digit_exponent(value: f64) -> i32 {
    if value == 0.0 {
        return 0;
    }
    let text = format!("{:.14e}", value.abs());
    let (mantissa, exponent) = text.split_once('e').expect("exponential notation");
    let digits = mantissa.replace('.', "").trim_end_matches('0').len() as i32;
    exponent.parse::<i32>().expect("integer exponent") + 1 - digits
}
