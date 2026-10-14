//! Each scale's domain fitted to data through `domain_summary` and `fit_domain`.

use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, Float64Array, StringArray, StructArray,
    TimestampMillisecondArray,
};
use arrow::compute::cast;
use arrow::datatypes::{
    DataType, Date32Type, Field, Fields, Float64Type, TimestampMillisecondType,
};
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::domain_summary::{
    fit_continuous, interval_summary, linear_single_value_span, CategoryOrder, DomainFitError,
    DomainOptions, DomainSummary, Extents, IntervalFit, IntervalSummary, NiceStep,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use avenger_scales::scales::nested_band::NestedBandScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::point::PointScale;
use avenger_scales::scales::pow::PowScale;
use avenger_scales::scales::quantile::QuantileScale;
use avenger_scales::scales::quantize::QuantizeScale;
use avenger_scales::scales::symlog::SymlogScale;
use avenger_scales::scales::threshold::ThresholdScale;
use avenger_scales::scales::time::TimeScale;
use avenger_scales::scales::{ConfiguredScale, DomainKind, ScaleConfig, ScaleImpl};

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn colors() -> ArrayRef {
    strings(&["white", "black"])
}

/// The scale fitted to `values`.
fn fit(
    scale: &ConfiguredScale,
    values: &ArrayRef,
    extents: Extents,
    options: &DomainOptions,
) -> Result<ConfiguredScale, AvengerScaleError> {
    let summary = scale.summarize(values, extents)?;
    scale.clone().fit_domain(options, &summary)
}

fn interval(scale: &ConfiguredScale) -> (f64, f64) {
    scale.config.numeric_interval_domain().unwrap()
}

fn assert_close(actual: (f64, f64), expected: (f64, f64)) {
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * (1.0 + b.abs());
    assert!(
        close(actual.0, expected.0) && close(actual.1, expected.1),
        "{actual:?} != {expected:?}"
    );
}

fn nice(count: f64) -> DomainOptions {
    DomainOptions {
        nice: Some(count),
        ..Default::default()
    }
}

fn zero() -> DomainOptions {
    DomainOptions {
        zero: true,
        ..Default::default()
    }
}

#[test]
fn linear_scales_fit_zero_nice_one_value_and_no_data() {
    let scale = LinearScale::unfitted((0.0, 100.0));
    let data = numbers(&[2.0, 8.0]);
    let none = DomainOptions::default();
    assert_eq!(
        interval(&fit(&scale, &data, Extents::None, &none).unwrap()),
        (2.0, 8.0)
    );
    assert_eq!(
        interval(&fit(&scale, &data, Extents::None, &zero()).unwrap()),
        (0.0, 8.0)
    );
    // An explicit domain becomes nice as a summary without extents, as the README shows.
    let explicit = IntervalSummary::new(&[1.23, 8.97], &[0.0; 2], &[0.0; 2]).unwrap();
    assert_eq!(explicit.fit_nice(1.0, NiceStep::Auto(10.0)), Ok((1.0, 9.0)));
    let rough = numbers(&[0.3, 9.7]);
    assert_eq!(
        interval(&fit(&scale, &rough, Extents::None, &nice(10.0)).unwrap()),
        (0.0, 10.0)
    );
    // One value spans 10% of its magnitude, and at least 0.1.
    assert_close(
        interval(&fit(&scale, &numbers(&[5.0]), Extents::None, &none).unwrap()),
        (4.75, 5.25),
    );
    assert_close(
        interval(&fit(&scale, &numbers(&[0.0]), Extents::None, &none).unwrap()),
        (-0.05, 0.05),
    );
    // No data gives (0, 1) or the options' empty domain.
    let empty = numbers(&[]);
    assert_eq!(
        interval(&fit(&scale, &empty, Extents::None, &none).unwrap()),
        (0.0, 1.0)
    );
    let given = DomainOptions {
        empty: Some(numbers(&[-5.0, 5.0])),
        ..Default::default()
    };
    assert_eq!(
        interval(&fit(&scale, &empty, Extents::None, &given).unwrap()),
        (-5.0, 5.0)
    );
}

/// Circles of radius 10 at 0 and 10 in 100 px leave 80 px for the data, so the domain is 12.5
/// wide, and nulls and non-finite values are skipped.
#[test]
fn linear_scales_leave_room_for_marks() {
    let scale = LinearScale::unfitted((0.0, 100.0));
    let data: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(0.0),
        None,
        Some(10.0),
        Some(f64::NAN),
    ]));
    let circles = Extents::Constant {
        lower: 10.0,
        upper: 10.0,
    };
    let fitted = fit(&scale, &data, circles, &DomainOptions::default()).unwrap();
    assert_close(interval(&fitted), (-1.25, 11.25));
    // A color range has no pixels, so marks cannot reach past their positions.
    let color = LinearScale::unfitted((0.0, 1.0)).with_range(colors());
    let none = DomainOptions::default();
    assert!(matches!(
        fit(&color, &data, circles, &none),
        Err(AvengerScaleError::DomainFit(DomainFitError::InvalidInput(
            _
        )))
    ));
    assert_eq!(
        interval(&fit(&color, &data, Extents::None, &none).unwrap()),
        (0.0, 10.0)
    );
}

/// Summaries of two layers merge into the summary of all their marks.
#[test]
fn layers_merge_their_summaries() {
    let scale = LinearScale::unfitted((0.0, 100.0));
    let points = scale
        .summarize(
            &numbers(&[0.0, 10.0]),
            Extents::Constant {
                lower: 10.0,
                upper: 10.0,
            },
        )
        .unwrap();
    let labels = scale
        .summarize(
            &numbers(&[10.0]),
            Extents::PerRow {
                lower: &[0.0],
                upper: &[30.0],
            },
        )
        .unwrap();
    let both = points.merge(&labels).unwrap();
    let fitted = scale.fit_domain(&DomainOptions::default(), &both).unwrap();
    let (start, end) = interval(&fitted);
    let x = |value: f64| (value - start) * 100.0 / (end - start);
    assert_close((x(0.0) - 10.0, x(10.0) + 30.0), (0.0, 100.0));
}

#[test]
fn log_nice_domains_end_on_powers_of_the_base() {
    let scale = LogScale::unfitted((0.0, 100.0));
    let none = DomainOptions::default();
    let data = numbers(&[2.0, 50.0]);
    assert_close(
        interval(&fit(&scale, &data, Extents::None, &none).unwrap()),
        (2.0, 50.0),
    );
    assert_close(
        interval(&fit(&scale, &data, Extents::None, &nice(10.0)).unwrap()),
        (1.0, 100.0),
    );
    for (data, expected) in [([0.7, 11.001], (0.1, 100.0)), ([0.01, 0.49], (0.01, 1.0))] {
        let fitted = fit(&scale, &numbers(&data), Extents::None, &nice(10.0)).unwrap();
        assert_close(interval(&fitted), expected);
    }
    // One value spans one power of the base, centered in the log coordinate.
    let one = fit(&scale, &numbers(&[30.0]), Extents::None, &none).unwrap();
    assert_close(interval(&one), (30.0 / 10f64.sqrt(), 30.0 * 10f64.sqrt()));
    assert_close(
        interval(&fit(&scale, &numbers(&[]), Extents::None, &none).unwrap()),
        (1.0, 10.0),
    );
    // Log scales infer domains from positive data only.
    for bad in [
        numbers(&[0.0, 5.0]),
        numbers(&[-1.0, 5.0]),
        numbers(&[-50.0, -2.0]),
    ] {
        assert!(matches!(
            scale.summarize(&bad, Extents::None),
            Err(AvengerScaleError::InvalidScalePropertyValue(_))
        ));
    }
    assert!(matches!(
        fit(&scale, &data, Extents::None, &zero()),
        Err(AvengerScaleError::InvalidScalePropertyValue(_))
    ));
}

#[test]
fn pow_and_symlog_scales_fit_in_their_coordinates() {
    let square = PowScale::unfitted((0.0, 100.0)).with_option("exponent", 2.0);
    let none = DomainOptions::default();
    let data = numbers(&[1.0, 3.0]);
    assert_close(
        interval(&fit(&square, &data, Extents::None, &none).unwrap()),
        (1.0, 3.0),
    );
    assert_close(
        interval(&fit(&square, &data, Extents::None, &zero()).unwrap()),
        (0.0, 3.0),
    );
    // 2 squares to 4, which spans 0.4 in the coordinate.
    let one = fit(&square, &numbers(&[2.0]), Extents::None, &none).unwrap();
    assert_close(interval(&one), (3.8f64.sqrt(), 4.2f64.sqrt()));
    // A circle's radius takes the same pixels on both sides, in the squared coordinate.
    let circles = Extents::Constant {
        lower: 10.0,
        upper: 10.0,
    };
    let (start, end) = interval(&fit(&square, &data, circles, &none).unwrap());
    let x = |value: f64| (value * value - start * start) * 100.0 / (end * end - start * start);
    assert_close((x(1.0), x(3.0)), (10.0, 90.0));
    let symlog = SymlogScale::unfitted((0.0, 100.0));
    let data = numbers(&[-10.0, 100.0]);
    assert_close(
        interval(&fit(&symlog, &data, Extents::None, &none).unwrap()),
        (-10.0, 100.0),
    );
}

#[test]
fn time_scales_fit_dates_and_timestamps() {
    let days: ArrayRef = Arc::new(Date32Array::from(vec![19723, 19730]));
    let scale = TimeScale::unfitted((0.0, 100.0));
    let none = DomainOptions::default();
    let domain = |scale: &ConfiguredScale| {
        let dates = scale.config.domain.as_primitive::<Date32Type>();
        (dates.value(0), dates.value(1))
    };
    assert_eq!(
        domain(&fit(&scale, &days, Extents::None, &none).unwrap()),
        (19723, 19730)
    );
    // Marks need room, and dates round outward to whole days.
    let circles = Extents::Constant {
        lower: 20.0,
        upper: 20.0,
    };
    let (start, end) = domain(&fit(&scale, &days, circles, &none).unwrap());
    assert!(start < 19723 && end > 19730);
    let x = |day: i32| (day - start) as f64 * 100.0 / (end - start) as f64;
    assert!(x(19723) >= 20.0 && x(19730) <= 80.0);
    // One date spans a day centered on it, rounded outward to whole days.
    let one: ArrayRef = Arc::new(Date32Array::from(vec![19723]));
    assert_eq!(
        domain(&fit(&scale, &one, Extents::None, &none).unwrap()),
        (19722, 19724)
    );
    // A timestamp spans a day centered on it.
    let noon = 1_704_110_400_000;
    let stamps: ArrayRef = Arc::new(TimestampMillisecondArray::from(vec![noon]));
    let stamp_scale = TimeScale::unfitted((0.0, 100.0));
    let fitted = fit(&stamp_scale, &stamps, Extents::None, &none).unwrap();
    let millis = fitted
        .config
        .domain
        .as_primitive::<TimestampMillisecondType>();
    assert_eq!(
        (millis.value(0), millis.value(1)),
        (noon - 43_200_000, noon + 43_200_000)
    );
    for options in [nice(10.0), zero()] {
        assert!(matches!(
            fit(&scale, &days, Extents::None, &options),
            Err(AvengerScaleError::InvalidScalePropertyValue(_))
        ));
    }
    let empty = fit(&scale, &days.slice(0, 0), Extents::None, &none).unwrap();
    assert_eq!(domain(&empty), (0, 1));
}

#[test]
fn quantize_quantile_and_ordinal_scales_fit_their_domains() {
    let quantize = QuantizeScale::unfitted(colors());
    assert_eq!(
        interval(&fit(&quantize, &numbers(&[0.3, 9.7]), Extents::None, &nice(10.0)).unwrap()),
        (0.0, 10.0)
    );
    let quantile = QuantileScale::unfitted(colors());
    let fitted = fit(
        &quantile,
        &numbers(&[3.0, 1.0, 2.0]),
        Extents::None,
        &DomainOptions::default(),
    )
    .unwrap();
    assert_eq!(
        fitted
            .config
            .domain
            .as_primitive::<arrow::datatypes::Float64Type>()
            .values(),
        &[1.0, 2.0, 3.0]
    );
    let ordinal = OrdinalScale::unfitted(colors());
    let data = strings(&["b", "a", "b"]);
    let descending = DomainOptions {
        order: CategoryOrder::Descending,
        ..Default::default()
    };
    let fitted = fit(&ordinal, &data, Extents::None, &descending).unwrap();
    let names: Vec<_> = fitted
        .config
        .domain
        .as_string::<i32>()
        .iter()
        .flatten()
        .collect();
    assert_eq!(names, ["b", "a"]);
    assert!(matches!(
        ordinal.summarize(
            &data,
            Extents::Constant {
                lower: 1.0,
                upper: 1.0
            }
        ),
        Err(AvengerScaleError::DomainFit(DomainFitError::InvalidInput(
            _
        )))
    ));
    let threshold = ThresholdScale::configured(vec![1.0], colors());
    assert!(matches!(
        threshold.summarize(&numbers(&[1.0]), Extents::None),
        Err(AvengerScaleError::ScaleOperationNotSupported(_))
    ));
}

/// The fitted clip padding of a discrete scale, and where its marks reach.
fn discrete_fit(
    scale: ConfiguredScale,
    keys: &[&str],
    lower: &[f64],
    upper: &[f64],
    options: &DomainOptions,
) -> ((f64, f64), (f64, f64)) {
    let values = strings(keys);
    let fitted = fit(&scale, &values, Extents::PerRow { lower, upper }, options).unwrap();
    let positions = fitted
        .scale_to_numeric(&values)
        .unwrap()
        .as_vec(keys.len(), None);
    let reach = positions.iter().enumerate().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(low, high), (i, x)| {
            (
                low.min(*x as f64 - lower[i]),
                high.max(*x as f64 + upper[i]),
            )
        },
    );
    let padding = (
        fitted.config.option_f64("clip_padding_lower", 0.0),
        fitted.config.option_f64("clip_padding_upper", 0.0),
    );
    (padding, reach)
}

fn assert_near(actual: (f64, f64), expected: (f64, f64)) {
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-3;
    assert!(
        near(actual.0, expected.0) && near(actual.1, expected.1),
        "{actual:?} != {expected:?}"
    );
}

/// Clip padding fitted to marks at the outer categories, for point and band scales.
#[test]
fn discrete_scales_fit_clip_padding_to_their_marks() {
    let none = DomainOptions::default();
    // A lollipop: circles of radius 60 on three points with half a step of outer padding.
    let lollipop = PointScale::unfitted((0.0, 300.0)).with_option("padding", 0.5);
    let (padding, reach) = discrete_fit(
        lollipop.clone(),
        &["a", "b", "c"],
        &[60.0; 3],
        &[60.0; 3],
        &none,
    );
    assert_near(padding, (15.0, 15.0));
    assert_near(reach, (0.0, 300.0));
    // Only the first circle is large.
    let wide = [80.0, 10.0, 10.0, 10.0, 10.0];
    let points = PointScale::unfitted((0.0, 400.0));
    let (padding, reach) = discrete_fit(points, &["a", "b", "c", "d", "e"], &wide, &wide, &none);
    assert_near(padding, (80.0, 10.0));
    assert_near(reach, (0.0, 400.0));
    // Small circles fit inside the outer padding.
    let padded = PointScale::unfitted((0.0, 400.0)).with_option("padding", 0.5);
    let (padding, _) = discrete_fit(padded, &["a", "b", "c", "d"], &[5.0; 4], &[5.0; 4], &none);
    assert_near(padding, (0.0, 0.0));
    // Dots at band centers.
    let bands = BandScale::unfitted((0.0, 400.0))
        .with_option("padding_inner", 0.2)
        .with_option("band", 0.5);
    let (padding, reach) =
        discrete_fit(bands, &["a", "b", "c", "d"], &[60.0; 4], &[60.0; 4], &none);
    assert_near(padding, (68.0 / 3.0, 68.0 / 3.0));
    assert_near(reach, (0.0, 400.0));
    // With align 0 the outer padding follows the last point, and its label reaches into it.
    let aligned = PointScale::unfitted((0.0, 400.0))
        .with_option("padding", 0.3)
        .with_option("align", 0.0);
    let (padding, reach) = discrete_fit(
        aligned,
        &["a", "b", "c", "d"],
        &[4.0; 4],
        &[4.0, 4.0, 4.0, 120.0],
        &none,
    );
    assert_near(padding, (4.0, 64.8));
    assert_near(reach, (0.0, 400.0));
    // A configured clip padding is a minimum, and the other side still fits exactly.
    let minimum = lollipop.with_option("clip_padding_lower", 30.0);
    let (padding, reach) = discrete_fit(minimum, &["a", "b", "c"], &[60.0; 3], &[60.0; 3], &none);
    assert!(
        padding.0 >= 30.0 - 1e-9 && reach.0 >= -1e-3,
        "{padding:?} {reach:?}"
    );
    assert_near((reach.1, 0.0), (300.0, 0.0));
}

#[test]
fn discrete_scales_order_their_categories() {
    let scale = PointScale::unfitted((0.0, 100.0));
    let data = strings(&["b", "c", "a"]);
    let domain = |options: &DomainOptions| {
        let fitted = fit(&scale, &data, Extents::None, options).unwrap();
        fitted
            .config
            .domain
            .as_string::<i32>()
            .iter()
            .flatten()
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(domain(&DomainOptions::default()), ["a", "b", "c"]);
    let listed = DomainOptions {
        order: CategoryOrder::Explicit(strings(&["c", "z", "a"])),
        ..Default::default()
    };
    assert_eq!(domain(&listed), ["c", "z", "a"]);
}

#[test]
fn nested_bands_fit_clip_padding_to_their_marks() {
    let fields = Fields::from(vec![
        Field::new("outer", DataType::Utf8, false),
        Field::new("inner", DataType::Utf8, false),
    ]);
    let paths: ArrayRef = Arc::new(StructArray::new(
        fields,
        vec![
            strings(&["b", "a", "a", "b"]),
            strings(&["x", "y", "x", "y"]),
        ],
        None,
    ));
    let scale = NestedBandScale::unfitted((0.0, 400.0)).with_option("band", 0.5);
    let circles = Extents::Constant {
        lower: 70.0,
        upper: 70.0,
    };
    let fitted = fit(&scale, &paths, circles, &DomainOptions::default()).unwrap();
    let positions = fitted
        .scale_to_numeric(&paths)
        .unwrap()
        .as_vec(paths.len(), None);
    let low = positions
        .iter()
        .fold(f64::INFINITY, |low, x| low.min(*x as f64));
    let high = positions
        .iter()
        .fold(f64::NEG_INFINITY, |high, x| high.max(*x as f64));
    assert_near((low - 70.0, high + 70.0), (0.0, 400.0));
    let outer = fitted
        .config
        .domain
        .as_struct()
        .column(0)
        .as_string::<i32>();
    assert_eq!(
        outer.iter().flatten().collect::<Vec<_>>(),
        ["a", "a", "b", "b"]
    );
}

#[test]
fn mismatched_summaries_do_not_merge() {
    let a = LinearScale::unfitted((0.0, 100.0))
        .summarize(&numbers(&[1.0, 5.0]), Extents::None)
        .unwrap();
    // Dates and timestamps summarize values of different types.
    let dates: ArrayRef = Arc::new(Date32Array::from(vec![19723]));
    let stamps: ArrayRef = Arc::new(TimestampMillisecondArray::from(vec![0]));
    let time = TimeScale::unfitted((0.0, 100.0));
    let date_summary = time.summarize(&dates, Extents::None).unwrap();
    let stamp_summary = time.summarize(&stamps, Extents::None).unwrap();
    assert!(matches!(
        date_summary.merge(&stamp_summary),
        Err(DomainFitError::InvalidInput(_))
    ));
    let categories = PointScale::unfitted((0.0, 100.0))
        .summarize(&strings(&["a"]), Extents::None)
        .unwrap();
    assert!(matches!(
        a.merge(&categories),
        Err(DomainFitError::InvalidInput(_))
    ));
    assert!(matches!(categories, DomainSummary::Categories(_)));
}

/// A scale made without a domain fails to scale until its domain is fitted.
#[test]
fn scales_without_a_domain_fail_until_fitted() {
    let values = numbers(&[1.0]);
    assert!(LinearScale::unfitted((0.0, 100.0))
        .scale_to_numeric(&values)
        .is_err());
    let stamps: ArrayRef = Arc::new(TimestampMillisecondArray::from(vec![0]));
    assert!(TimeScale::unfitted((0.0, 100.0))
        .scale_to_numeric(&stamps)
        .is_err());
    assert!(BandScale::unfitted((0.0, 100.0))
        .scale_to_numeric(&strings(&["a"]))
        .is_err());
    let fitted = fit(
        &LinearScale::unfitted((0.0, 100.0)),
        &numbers(&[2.0, 8.0]),
        Extents::None,
        &DomainOptions::default(),
    )
    .unwrap();
    assert!(fitted.scale_to_numeric(&values).is_ok());
}

/// A scale defined outside avenger-scales: a logit scale, linear in `ln(p / (1 − p))`.
#[derive(Debug)]
struct LogitScale;

impl ScaleImpl for LogitScale {
    fn scale_type(&self) -> &'static str {
        "logit"
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn scale(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "scale".to_string(),
        ))
    }

    fn summarize(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        let values = cast(values, &DataType::Float64)?;
        let logits = values
            .as_primitive::<Float64Type>()
            .unary::<_, Float64Type>(|p| (p / (1.0 - p)).ln());
        interval_summary(DataType::Float64, &logits, extents)
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        if options.zero {
            return Err(AvengerScaleError::InvalidScalePropertyValue(
                "logit scales do not support zero".to_string(),
            ));
        }
        let fit = IntervalFit {
            nice: options.nice.map(NiceStep::Auto),
            zero: None,
            single_value_span: linear_single_value_span,
        };
        let sigmoid = |t: f64| 1.0 / (1.0 + (-t).exp());
        fit_continuous(config, options, summary, fit, sigmoid, (0.25, 0.75))
    }
}

/// A scale from another crate infers its domain with the same public helpers as the built-in
/// scales.
#[test]
fn scales_from_other_crates_fit_domains_with_the_public_helpers() {
    let scale = ConfiguredScale {
        scale_impl: Arc::new(LogitScale),
        config: ScaleConfig::empty(),
    }
    .with_range_interval((0.0, 100.0));
    let data = numbers(&[0.2, 0.8]);
    let none = DomainOptions::default();
    assert_close(
        interval(&fit(&scale, &data, Extents::None, &none).unwrap()),
        (0.2, 0.8),
    );
    // Circles of radius 10 sit 10 px inside each end, in the logit coordinate.
    let circles = Extents::Constant {
        lower: 10.0,
        upper: 10.0,
    };
    let (start, end) = interval(&fit(&scale, &data, circles, &none).unwrap());
    let logit = |p: f64| (p / (1.0 - p)).ln();
    let x = |p: f64| (logit(p) - logit(start)) * 100.0 / (logit(end) - logit(start));
    assert_close((x(0.2), x(0.8)), (10.0, 90.0));
    assert!(matches!(
        fit(&scale, &data, Extents::None, &zero()),
        Err(AvengerScaleError::InvalidScalePropertyValue(_))
    ));
}
