//! Axes and colorbars label ticks through a text engine with D3 number and datetime formatting.

use arrow::array::{
    ArrayRef, Date32Array, Float32Array, Float64Array, Int64Array, StringArray,
    TimestampMillisecondArray,
};
use avenger_format::ValueKind;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::{
    axis::{Axis, AxisConfig},
    error::AvengerGuidesError,
    legend::{Colorbar, ColorbarConfig},
    GuideSide,
};
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, log::LogScale, point::PointScale, time::TimeScale,
    ConfiguredScale,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use avenger_typst_label::{LabelEngine, LabelError};
use chrono::{NaiveDate, TimeZone};
use chrono_tz::{America::New_York, Tz, UTC};
use std::sync::Arc;

/// The bundled fonts, with D3 number formatting, and D3 datetime formatting in a timezone.
fn engine(timezone: Tz) -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(
            D3DateTimeFormatProvider::new().with_timezone(timezone),
        ))
}

/// The plot that the axes stand under.
const PLOT: [f32; 2] = [400.0, 300.0];

fn config(pattern: &str) -> AxisConfig {
    AxisConfig {
        side: GuideSide::Bottom,
        tick_format: pattern.to_string(),
        tick_label: None,
        style: Default::default(),
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

fn axis(
    scale: &ConfiguredScale,
    pattern: &str,
    engine: &LabelEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    Axis::continuous(scale, "Title", &config(pattern), engine).map(|axis| axis.marks(PLOT))
}

fn band_axis(scale: &ConfiguredScale, pattern: &str, engine: &LabelEngine) -> SceneGroup {
    Axis::band(scale, "Title", &config(pattern), engine)
        .unwrap()
        .marks(PLOT)
}

fn axis_labels(scale: &ConfiguredScale, pattern: &str) -> Vec<String> {
    labels(&axis(scale, pattern, &engine(UTC)).unwrap())
}

/// A time scale over 2024's dates, which ticks on the 1st of each month.
fn dates_2024() -> ConfiguredScale {
    let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
    let end = Arc::new(Date32Array::from(vec![20088])) as ArrayRef;
    TimeScale::configured((start, end), (0.0, 400.0))
}

const MONTHS: [&str; 12] = [
    "2024",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A time scale over one day of timestamps, which ticks every three hours.
fn one_day(timezone: Option<&str>) -> ConfiguredScale {
    let midnight = |day| match timezone {
        Some(_) => New_York
            .with_ymd_and_hms(2024, 3, day, 0, 0, 0)
            .unwrap()
            .timestamp_millis(),
        None => NaiveDate::from_ymd_opt(2024, 3, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis(),
    };
    let array = |day| {
        Arc::new(TimestampMillisecondArray::from(vec![midnight(day)]).with_timezone_opt(timezone))
            as ArrayRef
    };
    let scale = TimeScale::configured((array(5), array(6)), (0.0, 400.0));
    match timezone {
        Some(_) => scale.with_option("timezone", "America/New_York"),
        None => scale,
    }
}

const HOURS: [&str; 9] = [
    "Tue 05", "03 AM", "06 AM", "09 AM", "12 PM", "03 PM", "06 PM", "09 PM", "Wed 06",
];

#[test]
fn linear_axes_label_ticks_like_vega() {
    let scale = LinearScale::configured((0.0, 1.0), (0.0, 400.0));
    let tenths: Vec<String> = (0..=10).map(|i| format!("{}.{}", i / 10, i % 10)).collect();
    // `,f` is Vega's default axis format.
    assert_eq!(axis_labels(&scale, ",f"), tenths);
    let scale = LinearScale::configured((0.0, 1e6), (0.0, 400.0));
    let millions: Vec<String> = tenths.iter().map(|label| format!("{label}M")).collect();
    assert_eq!(axis_labels(&scale, "s"), millions);
}

#[test]
fn log_axes_label_each_tick_by_its_own_digits() {
    let scale = LogScale::configured((1.0, 1000.0), (0.0, 400.0));
    let labels = axis_labels(&scale, "s");
    assert_eq!(labels[..3], ["1", "2", "3"]);
    assert_eq!(labels.last().unwrap(), "1k");
}

#[test]
fn date_axes_label_ticks_by_calendar_boundary() {
    // An empty pattern takes the provider's calendar patterns.
    assert_eq!(axis_labels(&dates_2024(), ""), MONTHS);
    // An explicit pattern labels every tick the same way.
    assert_eq!(
        axis_labels(&dates_2024(), "%b %d")[..2],
        ["Jan 01", "Feb 01"]
    );
}

#[test]
fn timestamp_axes_label_naive_and_zoned_ticks() {
    assert_eq!(axis_labels(&one_day(None), ""), HOURS);
    let zoned = axis(&one_day(Some("UTC")), "", &engine(New_York)).unwrap();
    assert_eq!(labels(&zoned), HOURS);
}

#[test]
fn zoned_ticks_must_show_in_the_scales_timezone() {
    assert!(matches!(
        axis(&one_day(Some("UTC")), "", &engine(UTC)).unwrap_err(),
        AvengerGuidesError::TimezoneMismatch { scale, formatter } if scale == New_York && formatter == UTC
    ));
}

/// Numbers need the engine's number provider and dates its datetime provider, but text needs
/// neither.
#[test]
fn axes_need_the_provider_for_their_ticks() {
    let bundled = avenger_typst_label::bundled_label_engine();
    let linear = LinearScale::configured((0.0, 1.0), (0.0, 400.0));
    for (scale, kind) in [(linear, ValueKind::Number), (dates_2024(), ValueKind::Date)] {
        assert!(matches!(
            axis(&scale, "", &bundled).unwrap_err(),
            AvengerGuidesError::Text(LabelError::MissingFormatter(missing)) if missing == kind
        ));
    }
    let names = Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef;
    let scale = BandScale::configured(names, (0.0, 400.0));
    let axis = band_axis(&scale, "", &bundled);
    assert_eq!(labels(&axis), ["a", "b"]);
}

#[test]
fn band_axes_format_numeric_categories() {
    let numbers = Arc::new(Float32Array::from(vec![1000.0, 2000.0, 2500.0])) as ArrayRef;
    let scale = BandScale::configured(numbers, (0.0, 400.0));
    let axis = band_axis(&scale, ",", &engine(UTC));
    assert_eq!(labels(&axis), ["1,000", "2,000", "2,500"]);

    let names = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
    let scale = BandScale::configured(names, (0.0, 400.0));
    let axis = band_axis(&scale, ",", &engine(UTC));
    assert_eq!(labels(&axis), ["a", "b", "c"]);
}

#[test]
fn band_and_point_axes_label_temporal_and_64_bit_categories() {
    let dates = Arc::new(Date32Array::from(vec![19723, 19754, 19783])) as ArrayRef;
    let scale = BandScale::configured(dates, (0.0, 400.0));
    let axis = band_axis(&scale, "%b %d", &engine(UTC));
    assert_eq!(labels(&axis), ["Jan 01", "Feb 01", "Mar 01"]);

    let numbers = Arc::new(Float64Array::from(vec![1000.0, 2000.0, 2500.0])) as ArrayRef;
    let scale = BandScale::configured(numbers, (0.0, 400.0));
    let axis = band_axis(&scale, ",", &engine(UTC));
    assert_eq!(labels(&axis), ["1,000", "2,000", "2,500"]);

    let counts = Arc::new(Int64Array::from(vec![1000, 2000, 2500])) as ArrayRef;
    let scale = PointScale::configured(counts, (0.0, 400.0));
    let axis = Axis::point(&scale, "Title", &config(","), &engine(UTC))
        .unwrap()
        .marks(PLOT);
    assert_eq!(labels(&axis), ["1,000", "2,000", "2,500"]);
}

/// A right colorbar's whole length beside a plot, title and background included.
const COLORBAR_LENGTH: f32 = 220.0;

fn colorbar_config(pattern: &str) -> ColorbarConfig {
    let defaults = ColorbarConfig::default();
    ColorbarConfig {
        axis: AxisConfig {
            tick_format: pattern.into(),
            ..defaults.axis
        },
        ..defaults
    }
}

#[test]
fn colorbars_label_ticks_with_their_format() {
    let scale = LinearScale::configured_color((0.0, 100.0), vec!["white", "blue"]);
    let colorbar = Colorbar::new(&scale, "Title", &colorbar_config(",f"), &engine(UTC))
        .unwrap()
        .marks(COLORBAR_LENGTH)
        .group;
    let expected: Vec<String> = (0..=10).map(|i| (i * 10).to_string()).collect();
    assert_eq!(labels(&colorbar), expected);
}

#[test]
fn time_colorbars_label_dates() {
    let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
    let end = Arc::new(Date32Array::from(vec![20088])) as ArrayRef;
    let scale = TimeScale::configured_color((start, end), vec!["white", "blue"]);
    let colorbar = Colorbar::new(&scale, "Title", &colorbar_config(""), &engine(UTC))
        .unwrap()
        .marks(COLORBAR_LENGTH)
        .group;
    assert_eq!(labels(&colorbar), MONTHS);
}
