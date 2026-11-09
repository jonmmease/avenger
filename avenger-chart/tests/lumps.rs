//! Columns of lumps: the categorical scales that they infer and take, and the encodings that
//! cannot read them.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray, StructArray},
    datatypes::{DataType, Field, Fields},
};
use avenger_chart::{lump, prelude::*, proto::scale, ChartError};

/// Ports by tons, lumped to the top 2: Rotterdam and Hamburg are kept, and Antwerp and Le Havre
/// are the Other group.
fn ports() -> RecordBatch {
    let fields = Fields::from(vec![
        Field::new("value", DataType::Utf8, true),
        Field::new("other", DataType::Utf8, true),
        Field::new("weight", DataType::Float64, true),
        Field::new("category_count", DataType::Int64, false),
    ]);
    let lumps = StructArray::new(
        fields,
        vec![
            Arc::new(StringArray::from(vec![
                Some("Rotterdam"),
                Some("Hamburg"),
                None,
                None,
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                None,
                None,
                Some("Other"),
                Some("Other"),
            ])),
            Arc::new(Float64Array::from(vec![40.0, 30.0, 25.0, 25.0])),
            Arc::new(Int64Array::from(vec![1, 1, 2, 2])),
        ],
        None,
    );
    RecordBatch::try_from_iter([
        ("port", Arc::new(lumps) as ArrayRef),
        (
            "tons",
            Arc::new(Float64Array::from(vec![40.0, 30.0, 15.0, 10.0])),
        ),
    ])
    .unwrap()
}

fn bars(x: EncodingBuilder) -> Result<Chart, ChartError> {
    let bars = CartesianLayer::bar(ports(), BarMark::builder().x(x).y("tons"));
    Chart::builder(Plot::cartesian([bars])).build()
}

/// The scale type of the encoding of `channel` of the first mark of `chart`.
fn scale_type(chart: &Chart, x: bool) -> scale::Type {
    let resolved = chart.resolve();
    let group = resolved
        .scales
        .iter()
        .find(|group| (group.channel == Channel::X) == x)
        .expect("a scale of the channel");
    group.spec.r#type.clone().expect("an inferred type")
}

#[track_caller]
fn invalid(built: Result<Chart, ChartError>, path: &str, reason: &str) {
    match built {
        Err(ChartError::Invalid {
            path: at,
            reason: why,
        }) => {
            assert_eq!(at, path, "the error names the path ({why})");
            assert!(why.contains(reason), "`{why}` says `{reason}`");
        }
        other => panic!("expected an error at {path}, got {other:?}"),
    }
}

#[test]
fn a_lump_is_a_struct_of_four_fields() {
    let port = ports().column(0).clone();
    assert!(lump::is_lump(port.data_type()));
    assert_eq!(lump::values_type(port.data_type()), DataType::Utf8);
    let others = lump::others(&port).unwrap();
    assert_eq!(
        others.iter().collect::<Vec<_>>(),
        [Some(false), Some(false), Some(true), Some(true)]
    );
    let keys = lump::keys(&port).unwrap();
    assert_eq!(keys.data_type(), &lump::key_type(port.data_type()).unwrap());
    // A struct of another shape, such as a span, is not a lump.
    let span = DataType::Struct(Fields::from(vec![
        Field::new("start", DataType::Float64, true),
        Field::new("end", DataType::Float64, true),
    ]));
    assert!(!lump::is_lump(&span));
}

#[test]
fn a_lump_infers_a_categorical_scale() {
    let chart = bars(Encoding::scaled_field("port")).unwrap();
    assert!(matches!(scale_type(&chart, true), scale::Type::Band(_)));

    let symbols = CartesianLayer::symbol(
        ports(),
        SymbolMark::builder().x("tons").y("tons").color("port"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let resolved = chart.resolve();
    let color = (resolved.scales.iter())
        .find(|group| group.channel == Channel::Color)
        .unwrap();
    assert!(matches!(color.spec.r#type, Some(scale::Type::Ordinal(_))));
}

#[test]
fn a_lump_takes_only_a_categorical_scale() {
    let path = "root.plot.cartesian.layers[0].bar.x.scale";
    invalid(
        bars(Encoding::scaled_field("port").scale(Scale::builder().linear(LinearScale::builder()))),
        path,
        "a column of lumps",
    );
    invalid(
        bars(
            Encoding::scaled_field("port")
                .scale(Scale::builder().nested_band(NestedBandScale::builder())),
        ),
        path,
        "a column of lumps",
    );
    // A lump goes through its scale, so it is never a direct field.
    let symbols = CartesianLayer::symbol(
        ports(),
        SymbolMark::builder()
            .x("tons")
            .y("tons")
            .color(Encoding::direct_field("port")),
    );
    invalid(
        Chart::builder(Plot::cartesian([symbols])).build(),
        "root.plot.cartesian.layers[0].symbol.color",
        "a `scaled_field`",
    );
}

#[test]
fn the_explicit_categories_of_a_lump_are_kept_values() {
    let domain = Domain::explicit(vec!["Hamburg", "Rotterdam"]);
    bars(
        Encoding::scaled_field("port")
            .scale(Scale::builder().band(BandScale::builder()).domain(domain)),
    )
    .unwrap();
}
