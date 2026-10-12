use arrow::array::{ArrayRef, Float64Array};
use std::sync::Arc;

use crate::error::AvengerScaleError;

/// The most ticks that start-step tick generation makes before it reports an error.
pub const MAX_START_STEP_TICKS: usize = 10_000;

/// Tick values as a Float64 array.
pub fn tick_array(ticks: impl IntoIterator<Item = f64>) -> ArrayRef {
    Arc::new(Float64Array::from_iter_values(ticks))
}

/// Generate approximately count ticks within the given range
pub fn ticks(start: f64, stop: f64, count: f64) -> Vec<f64> {
    // JS: if (!(count > 0)) return [];
    if count <= 0.0 || count.is_nan() {
        return vec![];
    }

    // JS: if (start === stop) return [start];
    if start == stop {
        return vec![start];
    }

    // JS: const reverse = stop < start
    let reverse = stop < start;

    // JS: const [i1, i2, inc] = reverse ? tickSpec(stop, start, count) : tickSpec(start, stop, count)
    let (i1, i2, inc) = if reverse {
        tick_spec(stop, start, count)
    } else {
        tick_spec(start, stop, count)
    };

    // JS: if (!(i2 >= i1)) return [];
    if i2 < i1 {
        return vec![];
    }

    // JS: const n = i2 - i1 + 1
    let n = (i2 - i1 + 1.0) as usize;
    let mut ticks = Vec::with_capacity(n);

    // JS: if (reverse) {
    //       if (inc < 0) for (let i = 0; i < n; ++i) ticks[i] = (i2 - i) / -inc;
    //       else for (let i = 0; i < n; ++i) ticks[i] = (i2 - i) * inc;
    //     } else {
    //       if (inc < 0) for (let i = 0; i < n; ++i) ticks[i] = (i1 + i) / -inc;
    //       else for (let i = 0; i < n; ++i) ticks[i] = (i1 + i) * inc;
    //     }
    if reverse {
        if inc < 0.0 {
            for i in 0..n {
                let val = (i2 - i as f64) / -inc;
                ticks.push(val);
            }
        } else {
            for i in 0..n {
                let val = (i2 - i as f64) * inc;
                ticks.push(val);
            }
        }
    } else if inc < 0.0 {
        for i in 0..n {
            let val = (i1 + i as f64) / -inc;
            ticks.push(val);
        }
    } else {
        for i in 0..n {
            let val = (i1 + i as f64) * inc;
            ticks.push(val);
        }
    }

    ticks
}

/// Helper function to calculate tick specifications
/// JS: function tickSpec(start, stop, count)
fn tick_spec(start: f64, stop: f64, count: f64) -> (f64, f64, f64) {
    // JS: const step = (stop - start) / Math.max(0, count)
    let step = (stop - start) / count.max(0.0);

    // JS: const power = Math.floor(Math.log10(step))
    let power = step.log10().floor();

    // JS: const error = step / Math.pow(10, power)
    let error = step / 10.0_f64.powf(power);

    // JS: const factor = error >= e10 ? 10 : error >= e5 ? 5 : error >= e2 ? 2 : 1
    let factor = if error >= 7.071_067_811_865_476 {
        // e10 = Math.sqrt(50)
        10.0
    } else if error >= 3.162_277_660_168_379_5 {
        // e5 = Math.sqrt(10)
        5.0
    } else if error >= std::f64::consts::SQRT_2 {
        // e2 = Math.sqrt(2)
        2.0
    } else {
        1.0
    };

    let (mut i1, mut i2, inc);

    // JS: if (power < 0) { ... } else { ... }
    if power < 0.0 {
        let temp_inc = 10.0_f64.powf(-power) / factor;
        i1 = (start * temp_inc).round();
        i2 = (stop * temp_inc).round();
        if i1 / temp_inc < start {
            i1 += 1.0;
        }
        if i2 / temp_inc > stop {
            i2 -= 1.0;
        }
        inc = -temp_inc;
    } else {
        inc = 10.0_f64.powf(power) * factor;
        i1 = (start / inc).round();
        i2 = (stop / inc).round();
        if i1 * inc < start {
            i1 += 1.0;
        }
        if i2 * inc > stop {
            i2 -= 1.0;
        }
    }

    // JS: if (i2 < i1 && 0.5 <= count && count < 2) return tickSpec(start, stop, count * 2);
    if i2 < i1 && (0.5..2.0).contains(&count) {
        return tick_spec(start, stop, count * 2.0);
    }

    (i1, i2, inc)
}

/// Calculate the tick increment for the given range and count
pub fn tick_increment(start: f64, stop: f64, count: f64) -> f64 {
    // JS: if (!(count > 0)) return NaN;
    if count <= 0.0 || count.is_nan() {
        return f64::NAN;
    }

    // JS: if (stop === start) return -Infinity;
    if start == stop {
        return f64::NEG_INFINITY;
    }

    let step = (stop - start) / count.max(0.0);
    // Handle step = 0 case (which happens with infinite count)
    if step == 0.0 {
        return f64::NAN;
    }

    let power = step.log10().floor();
    let error = step / 10.0_f64.powf(power);
    let factor = if error >= 7.071_067_811_865_476 {
        10.0
    } else if error >= 3.162_277_660_168_379_5 {
        5.0
    } else if error >= std::f64::consts::SQRT_2 {
        2.0
    } else {
        1.0
    };
    10.0_f64.powf(power) * factor
}

/// Ticks at `start + k * step` for whole numbers `k`, from `lo` to `hi`. Each tick is computed
/// from `start`, and ticks within a ten-thousandth of a step of either end, or of zero, snap to
/// it.
pub fn start_step_ticks(
    lo: f64,
    hi: f64,
    start: f64,
    step: f64,
) -> Result<Vec<f64>, AvengerScaleError> {
    let invalid = |message: &str| Err(AvengerScaleError::InvalidScalePropertyValue(message.into()));
    if !start.is_finite() {
        return invalid("tick start must be finite");
    }
    if !step.is_finite() || step <= 0.0 {
        return invalid("tick step must be finite and greater than zero");
    }
    if !lo.is_finite() || !hi.is_finite() {
        return invalid("start-step ticks need a finite domain");
    }
    let epsilon = step * 1e-4;
    let first = ((lo - epsilon - start) / step).ceil();
    let last = ((hi + epsilon - start) / step).floor();
    if !first.is_finite() || !last.is_finite() {
        return invalid("start-step ticks are out of range");
    }
    if last < first {
        return Ok(vec![]);
    }
    if last - first >= MAX_START_STEP_TICKS as f64 {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "start-step ticks would make more than {MAX_START_STEP_TICKS} ticks"
        )));
    }
    Ok((0..=(last - first) as usize)
        .map(|i| {
            let tick = start + (first + i as f64) * step;
            if tick.abs() <= epsilon {
                0.0
            } else {
                tick
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_step_ticks_cover_the_domain() {
        assert_eq!(
            start_step_ticks(1.2, 8.8, 0.0, 2.0).unwrap(),
            [2.0, 4.0, 6.0, 8.0]
        );
        // The start anchors ticks before and after it.
        assert_eq!(
            start_step_ticks(0.0, 10.0, 5.0, 2.5).unwrap(),
            [0.0, 2.5, 5.0, 7.5, 10.0]
        );
        // Each tick is computed from the start, so the last of ten tenths is exactly 1.
        let tenths = start_step_ticks(0.0, 1.0, 0.0, 0.1).unwrap();
        assert_eq!(tenths.len(), 11);
        assert_eq!(tenths[10], 1.0);
        assert_eq!(start_step_ticks(0.25, 0.75, 0.0, 1.0).unwrap(), [0.0; 0]);
    }

    #[test]
    fn start_step_ticks_reject_bad_steps_and_too_many_ticks() {
        for (start, step) in [
            (0.0, 0.0),
            (0.0, -1.0),
            (0.0, f64::NAN),
            (f64::INFINITY, 1.0),
        ] {
            assert!(
                start_step_ticks(0.0, 10.0, start, step).is_err(),
                "{start} {step}"
            );
        }
        assert!(start_step_ticks(0.0, 1e9, 0.0, 1.0).is_err());
    }

    #[test]
    fn test_ticks() {
        assert_eq!(
            ticks(0.0, 1.0, 10.0),
            vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]
        );
        assert_eq!(
            ticks(0.0, 1.0, 9.0),
            vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]
        );
        assert_eq!(
            ticks(0.0, 1.0, 8.0),
            vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]
        );
        assert_eq!(ticks(0.0, 1.0, 7.0), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(ticks(0.0, 1.0, 6.0), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(ticks(0.0, 1.0, 5.0), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(ticks(0.0, 1.0, 4.0), vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(ticks(0.0, 1.0, 3.0), vec![0.0, 0.5, 1.0]);
        assert_eq!(ticks(0.0, 1.0, 2.0), vec![0.0, 0.5, 1.0]);
        assert_eq!(ticks(0.0, 1.0, 1.0), vec![0.0, 1.0]);
    }

    #[test]
    fn test_ticks_edge_cases() {
        assert_eq!(ticks(f64::NAN, 1.0, 1.0), Vec::<f64>::new());
        assert_eq!(ticks(0.0, f64::NAN, 1.0), Vec::<f64>::new());
        assert_eq!(ticks(0.0, 1.0, f64::NAN), Vec::<f64>::new());
        assert_eq!(ticks(0.0, 1.0, 0.0), Vec::<f64>::new());
        assert_eq!(ticks(0.0, 1.0, -1.0), Vec::<f64>::new());
        assert_eq!(ticks(1.0, 1.0, 1.0), vec![1.0]);
        assert_eq!(ticks(1.0, 1.0, 10.0), vec![1.0]);
        assert_eq!(ticks(0.0, 1.0, f64::INFINITY), Vec::<f64>::new());
    }

    #[test]
    fn test_ticks_fractional_count() {
        assert_eq!(ticks(1.0, 364.0, 0.1), Vec::<f64>::new());
        assert_eq!(ticks(1.0, 364.0, 0.499), Vec::<f64>::new());
        assert_eq!(ticks(1.0, 364.0, 0.5), vec![200.0]);
        assert_eq!(ticks(1.0, 364.0, 1.0), vec![200.0]);
        assert_eq!(ticks(1.0, 364.0, 1.5), vec![200.0]);
    }

    #[test]
    fn test_tick_increment() {
        assert_eq!(tick_increment(0.0, 1.0, 10.0), 0.1);
        assert_eq!(tick_increment(0.0, 1.0, 9.0), 0.1);
        assert_eq!(tick_increment(0.0, 1.0, 8.0), 0.1);
        assert_eq!(tick_increment(0.0, 1.0, 7.0), 0.2);
        assert_eq!(tick_increment(0.0, 1.0, 6.0), 0.2);
        assert_eq!(tick_increment(0.0, 1.0, 5.0), 0.2);
        assert_eq!(tick_increment(0.0, 1.0, 4.0), 0.2);
        assert_eq!(tick_increment(0.0, 1.0, 3.0), 0.5);
        assert_eq!(tick_increment(0.0, 1.0, 2.0), 0.5);
        assert_eq!(tick_increment(0.0, 1.0, 1.0), 1.0);
    }

    #[test]
    fn test_tick_increment_edge_cases() {
        assert!(tick_increment(f64::NAN, 1.0, 1.0).is_nan());
        assert!(tick_increment(0.0, f64::NAN, 1.0).is_nan());
        assert!(tick_increment(0.0, 1.0, f64::NAN).is_nan());
        assert!(
            tick_increment(1.0, 1.0, 1.0).is_infinite()
                && tick_increment(1.0, 1.0, 1.0).is_sign_negative()
        );
        assert!(
            tick_increment(1.0, 1.0, 10.0).is_infinite()
                && tick_increment(1.0, 1.0, 10.0).is_sign_negative()
        );
        assert!(tick_increment(0.0, 1.0, 0.0).is_nan());
        assert!(tick_increment(0.0, 1.0, -1.0).is_nan());
        assert!(tick_increment(0.0, 1.0, f64::INFINITY).is_nan());
    }
}
