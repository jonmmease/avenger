//! The inference of the scale types and domains that marks and the types of their values imply.

mod support;

use std::sync::Arc;

use arrow::{
    array::{
        ArrayRef, BooleanArray, Date32Array, DurationMillisecondArray, Float64Array, StringArray,
        StructArray, TimestampMillisecondArray,
    },
    datatypes::{DataType, Field},
    record_batch::RecordBatch,
};
use avenger_chart::{
    prelude::*,
    proto::{domain, node, plot, scale},
    MarkMessage,
};

/// A table with a column of each kind of values: `number`, `time`, `date`, `duration`, `name`,
/// `flag`, and `path`.
fn table() -> RecordBatch {
    let path = StructArray::from(vec![
        (
            Arc::new(Field::new("outer", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["a", "a", "b"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("inner", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["x", "y", "x"])) as ArrayRef,
        ),
    ]);
    RecordBatch::try_from_iter([
        (
            "number",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
        ),
        (
            "time",
            Arc::new(TimestampMillisecondArray::from(vec![0, 1, 2])) as ArrayRef,
        ),
        (
            "date",
            Arc::new(Date32Array::from(vec![0, 1, 2])) as ArrayRef,
        ),
        (
            "duration",
            Arc::new(DurationMillisecondArray::from(vec![0, 1, 2])) as ArrayRef,
        ),
        (
            "name",
            Arc::new(StringArray::from(vec!["p", "q", "r"])) as ArrayRef,
        ),
        (
            "flag",
            Arc::new(BooleanArray::from(vec![true, false, true])) as ArrayRef,
        ),
        ("path", Arc::new(path) as ArrayRef),
    ])
    .unwrap()
}

/// The encoding of `channel` of the first layer of `message`.
fn encoding(message: &ChartMessage, channel: Channel) -> Encoding {
    let Some(node::Node::Plot(plot)) = &message.root.as_ref().unwrap().node else {
        panic!("the root is a plot")
    };
    let Some(plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("the plot is Cartesian")
    };
    let mark = cartesian.layers[0].mark.as_ref().unwrap() as &dyn MarkMessage;
    mark.encoding(channel).unwrap().clone()
}

/// The name of the scale type of `encoding`, or `None` without one.
fn type_name(encoding: &Encoding) -> Option<&'static str> {
    use scale::Type;
    Some(match encoding.scale.as_ref()?.r#type.as_ref()? {
        Type::Linear(_) => "linear",
        Type::Time(_) => "time",
        Type::Band(_) => "band",
        Type::Point(_) => "point",
        Type::NestedBand(_) => "nested band",
        Type::Ordinal(_) => "ordinal",
        _ => "other",
    })
}

#[test]
fn scale_types_follow_values_channels_and_marks() {
    let cases = [
        (Channel::X, "number", Some("linear")),
        (Channel::X, "time", Some("time")),
        (Channel::X, "date", Some("time")),
        // A time scale takes dates and timestamps, not durations.
        (Channel::X, "duration", None),
        (Channel::X, "name", Some("point")),
        (Channel::X, "flag", Some("point")),
        (Channel::X, "path", Some("nested band")),
        (Channel::Color, "number", Some("linear")),
        (Channel::Color, "time", Some("time")),
        (Channel::Color, "name", Some("ordinal")),
        (Channel::Color, "path", None),
        (Channel::Color, "duration", None),
        (Channel::Size, "number", Some("linear")),
        (Channel::Size, "time", Some("time")),
        (Channel::Size, "name", Some("ordinal")),
        (Channel::Shape, "number", Some("ordinal")),
        (Channel::Shape, "time", Some("ordinal")),
        (Channel::Shape, "name", Some("ordinal")),
    ];
    for (channel, column, expected) in cases {
        let mark = SymbolMark::builder().y(Encoding::scaled_field("number"));
        let mark = match channel {
            Channel::X => mark.x(Encoding::scaled_field(column)),
            Channel::Color => mark
                .x(Encoding::scaled_field("number"))
                .color(Encoding::scaled_field(column)),
            Channel::Size => mark
                .x(Encoding::scaled_field("number"))
                .size(Encoding::scaled_field(column)),
            Channel::Shape => mark
                .x(Encoding::scaled_field("number"))
                .shape(Encoding::scaled_field(column)),
            other => panic!("the cases do not encode {other:?}"),
        };
        let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([
            CartesianLayer::symbol(table(), mark),
        ])))
        .build();
        let Some(expected) = expected else {
            // Values without a scale type leave the type absent, and the chart is invalid.
            match chart {
                Err(ChartError::Invalid { reason, .. }) => {
                    assert!(
                        reason.contains("the type of a scale is required"),
                        "{reason}"
                    )
                }
                other => panic!("{channel:?} of `{column}` has no scale type, got {other:?}"),
            }
            continue;
        };
        let chart = chart.unwrap();
        let encoding = encoding(chart.message(), channel);
        assert_eq!(
            type_name(&encoding),
            Some(expected),
            "{channel:?} of `{column}`"
        );
        assert!(
            matches!(
                encoding
                    .scale
                    .as_ref()
                    .and_then(|spec| spec.domain.as_ref()),
                Some(Domain {
                    domain: Some(domain::Domain::Data(_))
                })
            ),
            "the domain of {channel:?} of `{column}`"
        );
    }
    // Categories on a position of a bar are bands.
    let bars = CartesianLayer::bar(
        table(),
        BarMark::builder()
            .x(Encoding::scaled_field("name"))
            .y(Encoding::scaled_field("number")),
    );
    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([bars])))
        .build()
        .unwrap();
    assert_eq!(
        type_name(&encoding(chart.message(), Channel::X)),
        Some("band")
    );
}

#[test]
fn present_fields_and_plain_values_stay() {
    let mark = SymbolMark::builder()
        .x(Encoding::scaled_field("number").scale(Scale::log(LogScale::builder().base(2.0))))
        .y(Encoding::scaled_field("number"))
        .color(Encoding::direct_field("name"));
    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([
        CartesianLayer::symbol(table(), mark),
    ])))
    .build()
    .unwrap();
    let message = chart.message();
    let x = encoding(message, Channel::X).scale.unwrap();
    assert!(matches!(
        x.r#type,
        Some(scale::Type::Log(LogScale {
            base: Some(2.0),
            ..
        }))
    ));
    // The domain was absent, so it is the data domain.
    assert!(matches!(
        x.domain.unwrap().domain,
        Some(domain::Domain::Data(_))
    ));
    // `y`, a scaled column without a scale, gets one, and `color`, a plain column, has none.
    assert_eq!(type_name(&encoding(message, Channel::Y)), Some("linear"));
    assert!(encoding(message, Channel::Color).scale.is_none());
    // Fits, guides, and titles stay absent.
    let y = encoding(message, Channel::Y);
    assert!(y.scale.as_ref().unwrap().axis.is_none());
    assert!(y.title.is_none());
}

#[test]
fn inferring_twice_changes_nothing() {
    let symbols = CartesianLayer::symbol(
        table(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("name"))
            .y(Encoding::scaled_field("number"))
            .color(Encoding::scaled_field("number")),
    );
    let once = Chart::builder(Plot::cartesian(CartesianPlot::new([symbols])))
        .build()
        .unwrap();
    let twice = Chart::builder(once.clone()).build().unwrap();
    assert_eq!(once, twice);
}

#[test]
fn layers_with_inferred_scales_share_them() {
    let bars = CartesianLayer::bar(
        table(),
        BarMark::builder()
            .x(Encoding::scaled_field("name"))
            .y(Encoding::scaled_field("number")),
    );
    let symbols = CartesianLayer::symbol(
        table(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("name"))
            .y(Encoding::scaled_field("number")),
    );
    let chart = Chart::builder(Plot::cartesian(CartesianPlot::new([bars, symbols])))
        .build()
        .unwrap();
    let scales = chart.resolve().scales;
    let x = scales
        .iter()
        .find(|group| group.channel == Channel::X)
        .unwrap();
    // A band and a point scale share a band scale.
    assert_eq!(x.members.len(), 2);
    assert!(matches!(x.spec.r#type, Some(scale::Type::Band(_))));
}
