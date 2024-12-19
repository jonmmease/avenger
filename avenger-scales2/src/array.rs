/// Generate approximately count ticks within the given range
pub fn ticks(start: f32, stop: f32, count: f32) -> Vec<f32> {
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
    if !(i2 >= i1) {
        return vec![];
    }

    // JS: const n = i2 - i1 + 1
    let n = ((i2 - i1 + 1.0) as usize).max(0);
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
                let val = ((i2 - i as f32) / -inc) as f32;
                ticks.push(val);
            }
        } else {
            for i in 0..n {
                let val = ((i2 - i as f32) * inc) as f32;
                ticks.push(val);
            }
        }
    } else {
        if inc < 0.0 {
            for i in 0..n {
                let val = ((i1 + i as f32) / -inc) as f32;
                ticks.push(val);
            }
        } else {
            for i in 0..n {
                let val = ((i1 + i as f32) * inc) as f32;
                ticks.push(val);
            }
        }
    }

    ticks
}

/// Helper function to calculate tick specifications
/// JS: function tickSpec(start, stop, count)
fn tick_spec(start: f32, stop: f32, count: f32) -> (f32, f32, f32) {
    // JS: const step = (stop - start) / Math.max(0, count)
    let step = (stop - start) / count.max(0.0);

    // JS: const power = Math.floor(Math.log10(step))
    let power = step.log10().floor();

    // JS: const error = step / Math.pow(10, power)
    let error = step / 10.0_f32.powf(power);

    // JS: const factor = error >= e10 ? 10 : error >= e5 ? 5 : error >= e2 ? 2 : 1
    let factor = if error >= 7.071067811865476 {
        // e10 = Math.sqrt(50)
        10.0
    } else if error >= 3.162277660168379 {
        // e5 = Math.sqrt(10)
        5.0
    } else if error >= 1.4142135623730951 {
        // e2 = Math.sqrt(2)
        2.0
    } else {
        1.0
    };

    let (mut i1, mut i2, inc);

    // JS: if (power < 0) { ... } else { ... }
    if power < 0.0 {
        let temp_inc = 10.0_f32.powf(-power) / factor;
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
        inc = 10.0_f32.powf(power) * factor;
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
    if i2 < i1 && 0.5 <= count && count < 2.0 {
        return tick_spec(start, stop, count * 2.0);
    }

    (i1, i2, inc)
}

/// Calculate the tick increment for the given range and count
pub fn tick_increment(start: f32, stop: f32, count: f32) -> f32 {
    // JS: if (!(count > 0)) return NaN;
    if !(count > 0.0) {
        return f32::NAN;
    }

    // JS: if (stop === start) return -Infinity;
    if start == stop {
        return f32::NEG_INFINITY;
    }

    let step = (stop - start) / count.max(0.0);
    // Handle step = 0 case (which happens with infinite count)
    if step == 0.0 {
        return f32::NAN;
    }

    let power = step.log10().floor();
    let error = step / 10.0_f32.powf(power);
    let factor = if error >= 7.071067811865476 {
        10.0
    } else if error >= 3.162277660168379 {
        5.0
    } else if error >= 1.4142135623730951 {
        2.0
    } else {
        1.0
    };
    10.0_f32.powf(power) * factor
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(ticks(f32::NAN, 1.0, 1.0), Vec::<f32>::new());
        assert_eq!(ticks(0.0, f32::NAN, 1.0), Vec::<f32>::new());
        assert_eq!(ticks(0.0, 1.0, f32::NAN), Vec::<f32>::new());
        assert_eq!(ticks(0.0, 1.0, 0.0), Vec::<f32>::new());
        assert_eq!(ticks(0.0, 1.0, -1.0), Vec::<f32>::new());
        assert_eq!(ticks(1.0, 1.0, 1.0), vec![1.0]);
        assert_eq!(ticks(1.0, 1.0, 10.0), vec![1.0]);
        assert_eq!(ticks(0.0, 1.0, f32::INFINITY), Vec::<f32>::new());
    }

    #[test]
    fn test_ticks_fractional_count() {
        assert_eq!(ticks(1.0, 364.0, 0.1), Vec::<f32>::new());
        assert_eq!(ticks(1.0, 364.0, 0.499), Vec::<f32>::new());
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
        assert!(tick_increment(f32::NAN, 1.0, 1.0).is_nan());
        assert!(tick_increment(0.0, f32::NAN, 1.0).is_nan());
        assert!(tick_increment(0.0, 1.0, f32::NAN).is_nan());
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
        assert!(tick_increment(0.0, 1.0, f32::INFINITY).is_nan());
    }
}
