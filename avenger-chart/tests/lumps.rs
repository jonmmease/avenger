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

/// Bars of the lumped ports with a point selection `pick` of `points` and the initial clauses
/// `initial`.
fn picked(points: PointSelectBuilder, initial: Vec<ClauseBuilder>) -> Result<Chart, ChartError> {
    let bars = CartesianLayer::bar(ports(), BarMark::builder().x("port").y("tons"))
        .id("bars")
        .tools([Select::points("pick", points)]);
    Chart::builder(Plot::cartesian([bars]))
        .selections([Selection::new("pick", SelectionResolution::Union).initial(initial)])
        .build()
}

#[test]
fn a_point_selection_names_the_source_of_its_lump() {
    let points = |keys: Vec<PointKeyBuilder>| PointSelect::builder().keys(keys);
    let path = "root.plot.cartesian.layers[0].tools[0].select.points.keys[0].source";
    picked(
        points(vec![PointKey::new("port").source("harbor")]),
        Vec::new(),
    )
    .unwrap();
    // The source of a column that is not a lump fails.
    invalid(
        picked(
            points(vec![PointKey::new("tons").source("weight")]),
            Vec::new(),
        ),
        path,
        "a column of lumps",
    );
    // A key with a source is the one key.
    invalid(
        picked(
            points(vec![
                PointKey::new("port").source("harbor"),
                PointKey::new("tons"),
            ]),
            Vec::new(),
        ),
        path,
        "one key",
    );
}

#[test]
fn a_complement_round_trips() {
    let clause = Clause::new([Condition::set(
        ValueSetBuilder::from([SetColumn::new("harbor", ["Rotterdam", "Hamburg"])])
            .complement(true),
    )])
    .owner("bars");
    let chart = picked(PointSelect::builder().keys(["port"]), vec![clause]).unwrap();
    let read = Chart::from_bytes(chart.to_bytes().unwrap()).unwrap();
    let set = match &read.selections()[0].initial[0].conditions[0].kind {
        Some(avenger_chart::proto::condition::Kind::Set(set)) => set.clone(),
        kind => panic!("a set, and not {kind:?}"),
    };
    assert!(set.complement);
    let values = read.list_elements(set.columns[0].values.as_ref().unwrap());
    let values = values.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(
        values.iter().flatten().collect::<Vec<_>>(),
        ["Rotterdam", "Hamburg"]
    );
}
