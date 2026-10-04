//! Axes and colorbars label ticks with a D3 number formatter.

use arrow::array::{ArrayRef, Date32Array, Float32Array, StringArray};
use avenger_format::{NumberFormatProvider, PreparedNumberFormatter};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::{
    axis::{
        band::make_band_axis_marks,
        numeric::make_numeric_axis_marks,
        opts::{AxisConfig, AxisOrientation},
    },
    error::AvengerGuidesError,
    legend::colorbar::{make_colorbar_marks, ColorbarConfig, ColorbarOrientation},
};
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, log::LogScale, time::TimeScale, ConfiguredScale,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use std::sync::Arc;

fn d3(pattern: &str) -> Arc<dyn PreparedNumberFormatter> {
    D3NumberFormatProvider::new().prepare(pattern).unwrap()
}

fn config(pattern: &str) -> AxisConfig {
    AxisConfig {
        orientation: AxisOrientation::Bottom,
        dimensions: [400.0, 300.0],
        grid: false,
        format: d3(pattern),
    }
}

/// The tick labels: the first text mark with several values, searching nested groups.
fn labels(group: &SceneGroup) -> Vec<String> {
    group
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.len > 1 => {
                Some(text.text.as_vec(text.len as usize, None))
            }
            SceneMark::Group(group) => Some(labels(group)).filter(|found| !found.is_empty()),
            _ => None,
        })
        .unwrap_or_default()
}

fn numeric_labels(scale: &ConfiguredScale, pattern: &str) -> Vec<String> {
    labels(&make_numeric_axis_marks(scale, "Title", [0.0, 0.0], &config(pattern)).unwrap())
}

#[test]
fn linear_axes_label_ticks_like_vega() {
    let scale = LinearScale::configured((0.0, 1.0), (0.0, 400.0));
    let tenths: Vec<String> = (0..=10).map(|i| format!("{}.{}", i / 10, i % 10)).collect();
    // `,f` is Vega's default axis format.
    assert_eq!(numeric_labels(&scale, ",f"), tenths);
    let scale = LinearScale::configured((0.0, 1e6), (0.0, 400.0));
    let millions: Vec<String> = tenths.iter().map(|label| format!("{label}M")).collect();
    assert_eq!(numeric_labels(&scale, "s"), millions);
}

#[test]
fn log_axes_label_each_tick_by_its_own_digits() {
    let scale = LogScale::configured((1.0, 1000.0), (0.0, 400.0));
    let labels = numeric_labels(&scale, "s");
    assert_eq!(labels[..3], ["1", "2", "3"]);
    assert_eq!(labels.last().unwrap(), "1k");
}

#[test]
fn numeric_axes_reject_temporal_ticks() {
    let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
    let end = Arc::new(Date32Array::from(vec![20088])) as ArrayRef;
    let scale = TimeScale::configured((start, end), (0.0, 400.0));
    let result = make_numeric_axis_marks(&scale, "Title", [0.0, 0.0], &config(",f"));
    assert!(matches!(
        result,
        Err(AvengerGuidesError::NonNumericTicks(_))
    ));
}

#[test]
fn band_axes_format_numeric_categories() {
    let numbers = Arc::new(Float32Array::from(vec![1000.0, 2000.0, 2500.0])) as ArrayRef;
    let scale = BandScale::configured(numbers, (0.0, 400.0));
    let axis = make_band_axis_marks(&scale, "Title", [0.0, 0.0], &config(",")).unwrap();
    assert_eq!(labels(&axis), ["1,000", "2,000", "2,500"]);

    let names = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
    let scale = BandScale::configured(names, (0.0, 400.0));
    let axis = make_band_axis_marks(&scale, "Title", [0.0, 0.0], &config(",")).unwrap();
    assert_eq!(labels(&axis), ["a", "b", "c"]);
}

#[test]
fn colorbars_label_ticks_with_their_format() {
    let scale = LinearScale::configured_color((0.0, 100.0), vec!["white", "blue"]);
    let config = ColorbarConfig {
        orientation: ColorbarOrientation::Right,
        dimensions: [20.0, 200.0],
        format: d3(",f"),
    };
    let colorbar = make_colorbar_marks(&scale, "Title", [0.0, 0.0], &config).unwrap();
    let expected: Vec<String> = (0..=10).map(|i| (i * 10).to_string()).collect();
    assert_eq!(labels(&colorbar), expected);
}
