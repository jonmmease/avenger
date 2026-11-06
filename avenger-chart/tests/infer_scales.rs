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
    build::{self, ChartParts},
    proto::{self, cartesian_layer::Mark, domain, scale_spec, Channel, Encoding},
    Chart, MarkMessage,
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

/// The chart message of one layer of `mark`, with its scales inferred.
fn inferred(mark: Mark) -> proto::Chart {
    let layer = proto::CartesianLayer {
        table: Some(proto::TableRef { index: 0 }),
        mark: Some(mark),
        ..Default::default()
    };
    let mut message = build::message(build::cartesian(vec![layer]));
    build::infer_scales(&mut message, &[table()], &[]);
    message
}

/// The encoding of `channel` of the first layer of `message`.
fn encoding(message: &proto::Chart, channel: Channel) -> Encoding {
    let Some(proto::node::Node::Plot(plot)) = &message.root.as_ref().unwrap().node else {
        panic!("the root is a plot")
    };
    let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord else {
        panic!("the plot is Cartesian")
    };
    let mark = cartesian.layers[0].mark.as_ref().unwrap() as &dyn MarkMessage;
    mark.encoding(channel).unwrap().clone()
}

/// The name of the scale type of `encoding`, or `None` without one.
fn type_name(encoding: &Encoding) -> Option<&'static str> {
    use scale_spec::Type;
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

/// Symbols at `x` of the column `x`, with `y` of `number`, and `channel` of the column `value`
/// through a scale to infer.
fn symbols(x: &str, channel: Option<(Channel, &str)>) -> Mark {
    let mut mark = proto::SymbolMark {
        x: Some(build::column(x)),
        y: Some(build::column("number")),
        ..Default::default()
    };
    if let Some((channel, value)) = channel {
        let encoding = Some(build::scaled_column(value));
        match channel {
            Channel::Color => mark.color = encoding,
            Channel::Size => mark.size = encoding,
            Channel::Shape => mark.shape = encoding,
            other => panic!("the cases do not encode {other:?}"),
        }
    }
    Mark::Symbol(mark)
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
        let mark = match channel {
            Channel::X => symbols(column, None),
            _ => symbols("number", Some((channel, column))),
        };
        let message = inferred(mark);
        let encoding = encoding(&message, channel);
        assert_eq!(type_name(&encoding), expected, "{channel:?} of `{column}`");
        let data = matches!(
            encoding
                .scale
                .as_ref()
                .and_then(|spec| spec.domain.as_ref()),
            Some(proto::Domain {
                domain: Some(domain::Domain::Data(_))
            })
        );
        assert_eq!(
            data,
            expected.is_some(),
            "the domain of {channel:?} of `{column}`"
        );
    }
    // Categories on a position of a bar are bands.
    let bar = Mark::Bar(proto::BarMark {
        x: Some(build::column("name")),
        y: Some(build::column("number")),
        ..Default::default()
    });
    assert_eq!(
        type_name(&encoding(&inferred(bar), Channel::X)),
        Some("band")
    );
}

#[test]
fn present_fields_and_raw_values_stay() {
    let mut mark = proto::SymbolMark {
        x: Some(build::column("number")),
        y: Some(build::column("number")),
        color: Some(build::column("name")),
        ..Default::default()
    };
    let x = mark.x.as_mut().unwrap();
    x.scale = Some(proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Log(proto::LogScale {
            base: Some(2.0),
            ..Default::default()
        })),
        ..Default::default()
    });
    let message = inferred(Mark::Symbol(mark));
    let x = encoding(&message, Channel::X).scale.unwrap();
    assert!(matches!(
        x.r#type,
        Some(scale_spec::Type::Log(proto::LogScale {
            base: Some(2.0),
            ..
        }))
    ));
    // The domain was absent, so it is the data domain.
    assert!(matches!(
        x.domain.unwrap().domain,
        Some(domain::Domain::Data(_))
    ));
    // `x` without a scale gets one, and `color` without a scale stays raw.
    assert_eq!(type_name(&encoding(&message, Channel::Y)), Some("linear"));
    assert!(encoding(&message, Channel::Color).scale.is_none());
    // Fits, guides, and titles stay absent.
    let y = encoding(&message, Channel::Y);
    assert!(y.scale.as_ref().unwrap().guide.is_none());
    assert!(y.title.is_none());
}

#[test]
fn inferring_twice_changes_nothing() {
    let once = inferred(symbols("name", Some((Channel::Color, "number"))));
    let mut twice = once.clone();
    build::infer_scales(&mut twice, &[table()], &[]);
    assert_eq!(once, twice);
}

#[test]
fn layers_with_inferred_scales_share_them() {
    let mut parts = ChartParts::new();
    let table = parts.table(table());
    let layer = |mark| proto::CartesianLayer {
        table: Some(table),
        mark: Some(mark),
        ..Default::default()
    };
    let bar = Mark::Bar(proto::BarMark {
        x: Some(build::column("name")),
        y: Some(build::column("number")),
        ..Default::default()
    });
    // `ChartParts::finish` infers the scales before it reads the chart.
    let message = build::message(build::cartesian(vec![
        layer(bar),
        layer(symbols("name", None)),
    ]));
    let chart: Chart = parts.finish(message).unwrap();
    let scales = chart.resolve().scales;
    let x = scales
        .iter()
        .find(|group| group.channel == Channel::X)
        .unwrap();
    // A band and a point scale share a band scale.
    assert_eq!(x.members.len(), 2);
    assert!(matches!(x.spec.r#type, Some(scale_spec::Type::Band(_))));
}
