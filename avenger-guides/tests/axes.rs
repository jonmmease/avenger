//! Axes and colorbars label numeric ticks with a D3 number formatter and temporal ticks with D3
//! datetime formatters.

use arrow::{
    array::{
        ArrayRef, Date32Array, Float32Array, Float64Array, Int64Array, StringArray,
        TimestampMillisecondArray,
    },
    datatypes::{DataType, TimeUnit},
};
use avenger_format::{
    DateTimeFormatProvider, NumberFormatProvider, PreparedFormatter, PreparedNumberFormatter,
    TickSpacing, ValueKind,
};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::{
    axis::{
        band::make_band_axis_marks,
        continuous::make_continuous_axis_marks,
        guide_format, label_values,
        opts::{AxisConfig, AxisOrientation},
        point::make_point_axis_marks,
    },
    error::AvengerGuidesError,
    legend::colorbar::{make_colorbar_marks, ColorbarConfig, ColorbarOrientation},
};
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, log::LogScale, point::PointScale, time::TimeScale,
    ConfiguredScale,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use chrono::{NaiveDate, TimeZone};
use chrono_tz::{America::New_York, UTC};
use std::sync::Arc;

fn d3(pattern: &str) -> Arc<dyn PreparedNumberFormatter> {
    D3NumberFormatProvider::new().prepare(pattern).unwrap()
}

fn config(format: impl Into<PreparedFormatter>) -> AxisConfig {
    AxisConfig {
        orientation: AxisOrientation::Bottom,
        dimensions: [400.0, 300.0],
        grid: false,
        format: format.into(),
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

fn numeric_labels(scale: &ConfiguredScale, pattern: &str) -> Vec<String> {
    axis_labels(scale, d3(pattern))
}

fn axis_labels(scale: &ConfiguredScale, format: impl Into<PreparedFormatter>) -> Vec<String> {
    labels(
        &make_continuous_axis_marks(
            scale,
            "Title",
            [0.0, 0.0],
            &config(format),
            &avenger_typst_label::bundled_label_engine(),
        )
        .unwrap(),
    )
}

fn axis_error(scale: &ConfiguredScale, format: impl Into<PreparedFormatter>) -> AvengerGuidesError {
    make_continuous_axis_marks(
        scale,
        "Title",
        [0.0, 0.0],
        &config(format),
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap_err()
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
fn date_axes_label_ticks_by_calendar_boundary() {
    let provider = D3DateTimeFormatProvider::new();
    let format = provider
        .default_calendar_patterns()
        .prepare_date(&provider)
        .unwrap();
    assert_eq!(axis_labels(&dates_2024(), format), MONTHS);
    // An explicit pattern labels every tick the same way.
    let format = provider.prepare_date("%b %d").unwrap();
    assert_eq!(
        axis_labels(&dates_2024(), format)[..2],
        ["Jan 01", "Feb 01"]
    );
}

#[test]
fn timestamp_axes_label_naive_and_zoned_ticks() {
    let provider = D3DateTimeFormatProvider::new();
    let patterns = provider.default_calendar_patterns();
    let naive = patterns.prepare_naive(&provider).unwrap();
    assert_eq!(axis_labels(&one_day(None), naive), HOURS);
    let zoned = patterns
        .prepare_zoned(&provider.with_timezone(New_York))
        .unwrap();
    assert_eq!(axis_labels(&one_day(Some("UTC")), zoned), HOURS);
}

#[test]
fn zoned_formatters_must_use_the_scales_timezone() {
    let provider = D3DateTimeFormatProvider::new();
    let utc = provider
        .default_calendar_patterns()
        .prepare_zoned(&provider)
        .unwrap();
    assert!(matches!(
        axis_error(&one_day(Some("UTC")), utc),
        AvengerGuidesError::TimezoneMismatch { scale, formatter } if scale == New_York && formatter == UTC
    ));
}

#[test]
fn formatters_must_match_the_ticks() {
    let provider = D3DateTimeFormatProvider::new();
    assert!(matches!(
        axis_error(&dates_2024(), d3(",f")),
        AvengerGuidesError::FormatMismatch {
            formatter: ValueKind::Number,
            ticks: DataType::Date32
        }
    ));
    let linear = LinearScale::configured((0.0, 1.0), (0.0, 400.0));
    assert!(matches!(
        axis_error(&linear, provider.prepare_date("%Y").unwrap()),
        AvengerGuidesError::FormatMismatch {
            formatter: ValueKind::Date,
            ticks: DataType::Float64
        }
    ));
    assert!(matches!(
        axis_error(&one_day(None), provider.prepare_zoned("%H").unwrap()),
        AvengerGuidesError::FormatMismatch {
            formatter: ValueKind::ZonedDateTime,
            ..
        }
    ));
}

#[test]
fn band_axes_format_numeric_categories() {
    let numbers = Arc::new(Float32Array::from(vec![1000.0, 2000.0, 2500.0])) as ArrayRef;
    let scale = BandScale::configured(numbers, (0.0, 400.0));
    let axis = make_band_axis_marks(
        &scale,
        "Title",
        [0.0, 0.0],
        &config(d3(",")),
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap();
    assert_eq!(labels(&axis), ["1,000", "2,000", "2,500"]);

    let names = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
    let scale = BandScale::configured(names, (0.0, 400.0));
    let axis = make_band_axis_marks(
        &scale,
        "Title",
        [0.0, 0.0],
        &config(d3(",")),
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap();
    assert_eq!(labels(&axis), ["a", "b", "c"]);
}

#[test]
fn band_and_point_axes_label_temporal_and_64_bit_categories() {
    let dates = Arc::new(Date32Array::from(vec![19723, 19754, 19783])) as ArrayRef;
    let scale = BandScale::configured(dates, (0.0, 400.0));
    let format = D3DateTimeFormatProvider::new()
        .prepare_date("%b %d")
        .unwrap();
    let axis = make_band_axis_marks(
        &scale,
        "Title",
        [0.0, 0.0],
        &config(format),
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap();
    assert_eq!(labels(&axis), ["Jan 01", "Feb 01", "Mar 01"]);

    let numbers = Arc::new(Float64Array::from(vec![1000.0, 2000.0, 2500.0])) as ArrayRef;
    let scale = BandScale::configured(numbers, (0.0, 400.0));
    let axis = make_band_axis_marks(
        &scale,
        "Title",
        [0.0, 0.0],
        &config(d3(",")),
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap();
    assert_eq!(labels(&axis), ["1,000", "2,000", "2,500"]);

    let counts = Arc::new(Int64Array::from(vec![1000, 2000, 2500])) as ArrayRef;
    let scale = PointScale::configured(counts, (0.0, 400.0));
    let axis = make_point_axis_marks(
        scale,
        "Title",
        [0.0, 0.0],
        &config(d3(",")),
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap();
    assert_eq!(labels(&axis), ["1,000", "2,000", "2,500"]);
}

#[test]
fn colorbars_label_ticks_with_their_format() {
    let scale = LinearScale::configured_color((0.0, 100.0), vec!["white", "blue"]);
    let config = ColorbarConfig {
        orientation: ColorbarOrientation::Right,
        dimensions: [20.0, 200.0],
        format: d3(",f").into(),
        style: Default::default(),
    };
    let colorbar = make_colorbar_marks(
        &scale,
        "Title",
        [0.0, 0.0],
        &config,
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap();
    let expected: Vec<String> = (0..=10).map(|i| (i * 10).to_string()).collect();
    assert_eq!(labels(&colorbar), expected);
}

#[test]
fn time_colorbars_label_dates() {
    let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
    let end = Arc::new(Date32Array::from(vec![20088])) as ArrayRef;
    let scale = TimeScale::configured_color((start, end), vec!["white", "blue"]);
    let provider = D3DateTimeFormatProvider::new();
    let format = provider
        .default_calendar_patterns()
        .prepare_date(&provider)
        .unwrap();
    let config = ColorbarConfig {
        orientation: ColorbarOrientation::Right,
        dimensions: [20.0, 200.0],
        format: format.into(),
        style: Default::default(),
    };
    let colorbar = make_colorbar_marks(
        &scale,
        "Title",
        [0.0, 0.0],
        &config,
        &avenger_typst_label::bundled_label_engine(),
    )
    .unwrap();
    // A 200 px colorbar fits quarterly ticks.
    assert_eq!(labels(&colorbar), ["2024", "April", "July", "October"]);
}

#[test]
fn guide_formats_follow_the_value_type() {
    let number = D3NumberFormatProvider::new();
    let datetime = D3DateTimeFormatProvider::new();
    let format = |data_type: &DataType| guide_format(data_type, &number, "c", &datetime).unwrap();
    for (data_type, kind) in [
        (DataType::Int64, ValueKind::Number),
        (DataType::Utf8, ValueKind::Number),
        (DataType::Date64, ValueKind::Date),
        (
            DataType::Timestamp(TimeUnit::Millisecond, None),
            ValueKind::NaiveDateTime,
        ),
        (
            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            ValueKind::ZonedDateTime,
        ),
    ] {
        assert_eq!(format(&data_type).kind(), kind, "{data_type}");
    }

    // Dates use the provider's calendar patterns, and "c" shows numbers as written
    let dates = Arc::new(Date32Array::from(vec![19723, 19754])) as ArrayRef;
    let labels = label_values(&dates, &format(dates.data_type()), TickSpacing::Varying);
    assert_eq!(labels.unwrap(), ["2024", "February"]);
    let years = Arc::new(Int64Array::from(vec![2020, 2021])) as ArrayRef;
    let labels = label_values(&years, &format(years.data_type()), TickSpacing::Varying);
    assert_eq!(labels.unwrap(), ["2020", "2021"]);
    let words = Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef;
    let labels = label_values(&words, &format(words.data_type()), TickSpacing::Varying);
    assert_eq!(labels.unwrap(), ["a", "b"]);
}
