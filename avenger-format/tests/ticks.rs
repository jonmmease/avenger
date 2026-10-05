//! Tick step inference and the default tick formatting.

use avenger_format::{FormattedNumber, PreparedNumberFormatter, TickSpacing, TickStep};

fn ticks(step: f64, magnitude: f64, resolution: i32) -> Option<TickStep> {
    Some(TickStep {
        step,
        magnitude,
        resolution,
    })
}

#[test]
fn evenly_spaced_ticks_recover_their_step() {
    let tenths: Vec<f64> = (0..=10).map(|i| f64::from(i) / 10.0).collect();
    assert_eq!(TickStep::infer(&tenths), ticks(0.1, 1.0, -1));
    // Multiplication leaves rounding error, such as 0.30000000000000004.
    let products: Vec<f64> = (0..=10).map(|i| f64::from(i) * 0.1).collect();
    assert_eq!(TickStep::infer(&products), ticks(0.1, 1.0, -1));
    // Widening f32 ticks with `as` leaves error near the eighth significant digit.
    let widened: Vec<f64> = (0..=5u8).map(|i| f64::from(f32::from(i) * 0.2)).collect();
    let inferred = TickStep::infer(&widened).unwrap();
    assert_eq!((inferred.step, inferred.resolution), (0.2, -1));
    for (values, expected) in [
        (
            vec![0.0, 200.0, 400.0, 600.0, 800.0, 1000.0],
            ticks(200.0, 1000.0, 2),
        ),
        (vec![-0.4, -0.2, 0.0, 0.2], ticks(0.2, 0.4, -1)),
        (vec![1000.0, 1000.1, 1000.2, 1000.3], ticks(0.1, 1000.3, -1)),
        (vec![0.0, 0.25, 0.5, 0.75, 1.0], ticks(0.25, 1.0, -2)),
        (vec![1e-7, 2e-7, 3e-7], ticks(1e-7, 3e-7, -7)),
        (vec![0.0, 5e9, 1e10], ticks(5e9, 1e10, 9)),
    ] {
        assert_eq!(TickStep::infer(&values), expected, "{values:?}");
    }
}

#[test]
fn resolution_covers_offsets_and_values_off_a_decimal_grid() {
    // The step's own digits would drop the offset.
    assert_eq!(TickStep::infer(&[0.05, 1.05, 2.05]), ticks(1.0, 2.05, -2));
    // No decimal grid holds thirds, so they keep four places below the step's leading digit.
    assert_eq!(
        TickStep::infer(&[0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]),
        ticks(0.3333, 1.0, -4)
    );
}

#[test]
fn a_single_value_uses_its_last_significant_digit() {
    for (value, step, resolution) in [
        (0.3, 0.1, -1),
        (0.1 + 0.2, 0.1, -1),
        (1234.5, 0.1, -1),
        (58.0, 1.0, 0),
        (1.2e6, 1e5, 5),
        (-0.25, 0.01, -2),
        (0.0, 1.0, 0),
    ] {
        assert_eq!(
            TickStep::infer(&[value]),
            ticks(step, value.abs(), resolution),
            "{value}"
        );
    }
}

#[test]
fn inference_ignores_order_duplicates_and_non_finite_values() {
    let values = [1.0, f64::NAN, 0.5, 0.0, 1.0, f64::INFINITY, -0.0];
    assert_eq!(TickStep::infer(&values), ticks(0.5, 1.0, -1));
    assert_eq!(TickStep::infer(&[]), None);
    assert_eq!(TickStep::infer(&[f64::NAN, f64::NEG_INFINITY]), None);
}

#[derive(Debug)]
struct TwoPlaces;

impl PreparedNumberFormatter for TwoPlaces {
    fn format(&self, value: f64) -> FormattedNumber {
        FormattedNumber::plain(format!("{value:.2}"))
    }
}

#[test]
fn default_tick_formatting_formats_each_value() {
    for spacing in [TickSpacing::Uniform, TickSpacing::Varying] {
        let labels: Vec<_> = TwoPlaces
            .format_ticks(&[0.0, 0.5, f64::NAN], spacing)
            .into_iter()
            .map(|label| label.text)
            .collect();
        assert_eq!(labels, ["0.00", "0.50", "NaN"]);
    }
}
