//! Values in labels: `datetime`, which builds dates and datetimes, and `bind`, which writes
//! values into markup.

mod common;

use std::sync::Arc;

use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_typst_label::{
    CompiledLabel, FrameItem, LabelEngine, LabelError, LabelFrame, LabelOptions,
    LabelValue, LabelValues, bind,
};
use indexmap::IndexMap;

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(
            D3DateTimeFormatProvider::new().with_timezone(chrono_tz::UTC),
        ))
}

/// The text of a label.
fn text(source: &str) -> String {
    engine()
        .compile(source, &LabelOptions::default())
        .unwrap()
        .semantic_text
}

/// The message and hints of the error that a source compiles to.
fn error(source: &str) -> (String, Vec<String>) {
    match engine().compile(source, &LabelOptions::default()).unwrap_err() {
        LabelError::Source { message, hints, .. } => (message, hints),
        other => panic!("{source}: {other:?}"),
    }
}

/// `datetime` builds dates, naive datetimes and instants, to the nanosecond.
#[test]
fn datetime_builds_dates_naive_datetimes_and_instants() {
    let instant = chrono::NaiveDate::from_ymd_opt(2024, 1, 5)
        .unwrap()
        .and_hms_nano_opt(13, 4, 5, 123_456_789)
        .unwrap()
        .and_utc();
    let date = "year: 2024, month: 1, day: 5";
    let time = "hour: 13, minute: 4, second: 5, nanosecond: 123456789";
    for (source, expected) in [
        (
            format!("#datetimefmt(datetime({date}), \"%B %-d, %Y\")"),
            "January 5, 2024".to_string(),
        ),
        (
            format!("#datetimefmt(datetime({date}, {time}), \"%Y-%m-%d %H:%M:%S.%L\")"),
            "2024-01-05 13:04:05.123".to_string(),
        ),
        (
            format!(
                "#datetimefmt(datetime({date}, {time}, utc: true), \"%H:%M:%S.%L %Q\")"
            ),
            format!("13:04:05.123 {}", instant.timestamp_millis()),
        ),
    ] {
        assert_eq!(text(&source), expected, "{source}");
    }
}

/// `datetime`'s errors are upstream's, but for the arguments a label adds and for times without
/// dates, which labels don't have.
#[test]
fn datetime_reports_incomplete_and_invalid_arguments() {
    let date = "year: 2024, month: 1, day: 5";
    let add_time = "add the `hour`, `minute`, and `second` arguments to get a valid time";
    let add_date = "add the `year`, `month`, and `day` arguments to get a valid date";
    let cases = [
        (
            "#datetime(year: 2024)".to_string(),
            "date is incomplete",
            vec!["add the `month` and `day` arguments to get a valid date"],
        ),
        (
            format!("#datetime({date}, minute: 3)"),
            "time is incomplete",
            vec!["add the `hour` and `second` arguments to get a valid time"],
        ),
        (
            "#datetime(year: 2023, month: 2, day: 29)".to_string(),
            "date is invalid",
            vec![],
        ),
        (
            format!("#datetime({date}, hour: 24, minute: 0, second: 0)"),
            "time is invalid",
            vec![],
        ),
        (
            "#datetime(hour: 1, minute: 2, second: 3)".to_string(),
            "times without dates are not supported in labels",
            vec![add_date],
        ),
        (
            format!("#datetime({date}, nanosecond: 1)"),
            "`nanosecond` needs a time",
            vec![add_time],
        ),
        (format!("#datetime({date}, utc: true)"), "`utc` needs a time", vec![add_time]),
        (
            "#datetime()".to_string(),
            "at least one of date or time must be fully specified",
            vec![add_time, add_date],
        ),
    ];
    for (source, message, hints) in cases {
        assert_eq!(
            error(&source),
            (message.to_string(), hints.iter().map(|h| h.to_string()).collect()),
            "{source}"
        );
    }
}

/// A frame without its items' source ranges, which binding moves.
fn without_sources(frame: &LabelFrame) -> LabelFrame {
    let mut frame = frame.clone();
    for (_, item) in &mut frame.items {
        match item {
            FrameItem::Group(group) => group.frame = without_sources(&group.frame),
            FrameItem::Text(text) => {
                text.source = 0..0;
                for glyph in &mut text.glyphs {
                    glyph.source = 0..0;
                }
            }
            FrameItem::Shape(_) => {}
        }
    }
    frame
}

/// What a compiled label draws and measures, but not the source it comes from.
fn drawn(
    label: Result<CompiledLabel, LabelError>,
) -> Result<(LabelFrame, String, String), (String, Vec<String>)> {
    match label {
        Ok(label) => Ok((
            without_sources(&label.frame),
            format!("{:?} {:?}", label.metrics, label.flags),
            label.semantic_text,
        )),
        Err(LabelError::Source { message, hints, .. }) => Err((message, hints)),
        Err(other) => panic!("{other:?}"),
    }
}

fn values(values: impl IntoIterator<Item = (&'static str, LabelValue)>) -> LabelValues {
    values
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

/// Bound markup draws and measures as the same markup with its values written in does, and
/// fails as it does.
#[test]
fn bound_markup_draws_as_written_values() {
    use LabelValue::{Array, Bool, Date, Dict, Float, Int, Str, ZonedDateTime};
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();
    let instant = date.and_hms_nano_opt(13, 4, 5, 123_456_789).unwrap().and_utc();
    let stroke: IndexMap<String, LabelValue> = [
        ("cap", Str("round".into())),
        ("join", Str("bevel".into())),
        ("dash", Str("dashed".into())),
        ("miter-limit", Float(2.0)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();
    let renders = [
        (
            "#series_name >= #threshold",
            values([("series_name", Str("Revenue".into())), ("threshold", Float(-2.5))]),
            "Revenue >= #(-2.5)",
        ),
        (
            "Series #upper[#series_name]",
            values([("series_name", Str("revenue".into()))]),
            "Series #upper[revenue]",
        ),
        // A `*` after a value stays text.
        ("#n*2 #(n*2) #n;", values([("n", Int(3))]), "3*2 6 3"),
        (
            "#numfmt(value, \",.1f\") #underline[#numfmt(value, \".1e\")]",
            values([("value", Float(1234.5))]),
            "#numfmt(1234.5, \",.1f\") #underline[#numfmt(1234.5, \".1e\")]",
        ),
        (
            "#datetimefmt(day, \"%B %-d, %Y\") #datetimefmt(instant, \"%H:%M:%S.%L %Q\")",
            values([("day", Date(date)), ("instant", ZonedDateTime(instant))]),
            "January 5, 2024 #datetimefmt(datetime(year: 2024, month: 1, day: 5, hour: 13, \
             minute: 4, second: 5, nanosecond: 123456789, utc: true), \"%H:%M:%S.%L %Q\")",
        ),
        (
            "$y = #slope x + #intercept$ $rate_1 x$",
            values([("slope", Float(2.5)), ("intercept", Int(7)), ("rate", Float(0.5))]),
            "$y = #2.5 x + #7$ $#0.5_1 x$",
        ),
        // Values shadow the library's names.
        (
            "#upper $frac$",
            values([("upper", Str("UPPER".into())), ("frac", Str("FRAC".into()))]),
            "UPPER $\"FRAC\"$",
        ),
        (
            "#underline(stroke: 1.5pt + rgb(color))[Series]",
            values([("color", Str("tomato".into()))]),
            "#underline(stroke: 1.5pt + rgb(\"tomato\"))[Series]",
        ),
        (
            "#underline(stroke: stroke)[Series]",
            values([("stroke", Dict(stroke))]),
            "#underline(stroke: (cap: \"round\", join: \"bevel\", dash: \"dashed\", \
             miter-limit: 2.0))[Series]",
        ),
        (
            "#underline(offset: offset * 1pt, extent: extent * 1em, background: background, \
             evade: evade)[care] #super(baseline: baseline * 1em, size: size * 1pt)[N]",
            values([
                ("offset", Float(2.0)),
                ("extent", Float(-0.5)),
                ("background", Bool(true)),
                ("evade", Bool(false)),
                ("baseline", Float(-0.25)),
                ("size", Float(8.0)),
            ]),
            "#underline(offset: 2pt, extent: -0.5em, background: true, evade: false)[care] \
             #super(baseline: -0.25em, size: 8pt)[N]",
        ),
    ];
    for (source, values, written) in renders {
        // One engine, since font references compare font instances.
        let engine = engine();
        let bound = bind(source, &values).unwrap();
        let expected = drawn(engine.compile(written, &LabelOptions::default()));
        assert!(expected.is_ok(), "{written}: {expected:?}");
        assert_eq!(
            drawn(engine.compile(&bound, &LabelOptions::default())),
            expected,
            "{source} as {bound}"
        );
    }

    let fails = [
        (
            "#items $#items$",
            values([("items", Array(vec![Int(1)]))]),
            "cannot display array in a label",
        ),
        (
            "#active",
            values([("active", Bool(true))]),
            "cannot display boolean in a label",
        ),
        (
            "#underline(offset: text)[care]",
            values([("text", Str("2pt".into()))]),
            "expected length or auto, found string",
        ),
        ("#missing", values([]), "unknown variable: missing"),
    ];
    for (source, values, message) in fails {
        let bound = bind(source, &values).unwrap();
        assert_eq!(error(&bound).0, message, "{source} as {bound}");
    }
}

/// Math that calls a value's name is an error, since bind can't write the call.
#[test]
fn bind_rejects_math_that_calls_a_value() {
    let values = values([("rate", LabelValue::Float(0.5))]);
    for source in ["$rate(x)$", "$rate{x}/2$"] {
        match bind(source, &values) {
            Err(LabelError::Source { range, message, hints }) => {
                assert_eq!(range, 1..5, "{source}");
                assert_eq!(message, "cannot bind `rate`, which math calls", "{source}");
                assert_eq!(hints, ["add a space after `rate`"], "{source}");
            }
            other => panic!("{source}: {other:?}"),
        }
    }
    assert_eq!(bind("$rate (x)$", &values).unwrap(), "$#((0.5)); (x)$");
    // Markup that doesn't parse returns its syntax error.
    assert!(matches!(bind("#(", &values), Err(LabelError::Source { .. })));
}
