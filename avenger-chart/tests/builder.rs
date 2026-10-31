//! The builder: the scale types that it infers, the values that it writes, the combination of
//! charts, and the errors of the charts that it checks.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, Date32Array, Float64Array, Int64Array, RecordBatch, StringArray, StructArray,
};
use arrow::datatypes::{DataType, Field};
use avenger_chart::{
    proto::{self, axis, domain, domain_fit, guide, node, Channel},
    Chart, ChartError, Child, Contribution, Encode, Scale, Selection, TupleTest,
};
use avenger_scales::scalar::Scalar;

/// A table of cars: a number, a date, two categories, and CSS colors.
fn cars() -> RecordBatch {
    let columns: Vec<(&str, ArrayRef)> = vec![
        (
            "weight",
            Arc::new(Float64Array::from(vec![2.0, 3.5, 3.0, 4.0])),
        ),
        (
            "mpg",
            Arc::new(Float64Array::from(vec![30.0, 18.0, 24.0, 15.0])),
        ),
        ("count", Arc::new(Int64Array::from(vec![3, 5, 2, 7]))),
        (
            "day",
            Arc::new(Date32Array::from(vec![19_700, 19_701, 19_702, 19_703])),
        ),
        (
            "origin",
            Arc::new(StringArray::from(vec!["US", "EU", "EU", "JP"])),
        ),
        (
            "cylinders",
            Arc::new(StringArray::from(vec!["4", "8", "6", "8"])),
        ),
        (
            "css",
            Arc::new(StringArray::from(vec!["red", "blue", "blue", "green"])),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

fn unit(chart: &Chart) -> &proto::Unit {
    match &chart.root().node {
        Some(node::Node::Unit(unit)) => unit,
        other => panic!("expected a unit, got {other:?}"),
    }
}

fn encoding(chart: &Chart, channel: Channel) -> &proto::Encoding {
    unit(chart)
        .encodings
        .iter()
        .find(|encoding| encoding.channel == channel as i32)
        .unwrap()
}

fn spec(chart: &Chart, channel: Channel) -> &proto::ScaleSpec {
    encoding(chart, channel).scale.as_ref().unwrap()
}

fn interval(spec: &proto::ScaleSpec) -> &proto::IntervalFit {
    match spec.fit.as_ref().and_then(|fit| fit.fit.as_ref()) {
        Some(domain_fit::Fit::Interval(interval)) => interval,
        other => panic!("expected an interval fit, got {other:?}"),
    }
}

fn guide(spec: &proto::ScaleSpec) -> &guide::Guide {
    spec.guide.as_ref().unwrap().guide.as_ref().unwrap()
}

#[track_caller]
fn invalid(result: Result<Chart, ChartError>, path: &str, reason: &str) {
    match result {
        Err(ChartError::Invalid {
            path: actual,
            reason: actual_reason,
        }) => {
            assert_eq!(actual, path, "{actual_reason}");
            assert!(
                actual_reason.contains(reason),
                "`{actual_reason}` does not contain `{reason}`"
            );
        }
        other => panic!("expected an invalid chart at {path}, got {other:?}"),
    }
}

#[test]
fn scale_types_come_from_the_column_and_the_mark() {
    let bars = Chart::unit(cars(), proto::BarMark::default())
        .x("origin")
        .y("mpg")
        .color("cylinders")
        .opacity("count")
        .build()
        .unwrap();
    assert_eq!(spec(&bars, Channel::X).r#type, "band");
    assert_eq!(spec(&bars, Channel::Y).r#type, "linear");
    assert_eq!(spec(&bars, Channel::Color).r#type, "ordinal");
    assert_eq!(spec(&bars, Channel::Opacity).r#type, "linear");
    // The measure of a vertical bar contains zero.
    assert_eq!(interval(spec(&bars, Channel::Y)).zero, Some(true));

    let points = Chart::unit(cars(), proto::SymbolMark::default())
        .x("day")
        .y("origin")
        .shape("count")
        .build()
        .unwrap();
    assert_eq!(spec(&points, Channel::X).r#type, "time");
    assert_eq!(spec(&points, Channel::Y).r#type, "point");
    // A number takes an ordinal scale on a channel of discrete values.
    assert_eq!(spec(&points, Channel::Shape).r#type, "ordinal");
    // Only the measure of a bar or an area contains zero.
    assert_eq!(interval(spec(&points, Channel::X)).zero, None);

    // A horizontal bar measures along x.
    let horizontal = Chart::unit(cars(), proto::BarMark::default())
        .x("mpg")
        .y("origin")
        .build()
        .unwrap();
    assert_eq!(interval(spec(&horizontal, Channel::X)).zero, Some(true));

    // Paths of categories take a nested band scale.
    let paths: ArrayRef = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("origin", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec!["US", "EU", "EU", "JP"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("cylinders", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec!["4", "8", "6", "8"])) as ArrayRef,
        ),
    ]));
    let nested = RecordBatch::try_from_iter([
        ("path", paths),
        (
            "mpg",
            Arc::new(Float64Array::from(vec![30.0, 18.0, 24.0, 15.0])) as ArrayRef,
        ),
    ])
    .unwrap();
    let grouped = Chart::unit(nested, proto::BarMark::default())
        .x("path")
        .y("mpg")
        .build()
        .unwrap();
    assert_eq!(spec(&grouped, Channel::X).r#type, "nested_band");
}

#[test]
fn encodings_hold_titles_and_specs_hold_only_the_guides_that_callers_give() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y(Encode::column("mpg")
            .scale(Scale::of("log"))
            .title("Miles per gallon"))
        .color(Encode::column("count").scale(Scale::new().guide(proto::Legend::default())))
        .shape(Encode::column("origin").untitled())
        .build()
        .unwrap();
    // The renderer gives the default guide, titled with the column's name.
    assert!(spec(&chart, Channel::X).guide.is_none());
    assert_eq!(encoding(&chart, Channel::X).title, None);
    assert_eq!(
        encoding(&chart, Channel::Y).title.as_deref(),
        Some("Miles per gallon")
    );
    assert_eq!(encoding(&chart, Channel::Shape).title.as_deref(), Some(""));
    // A caller's guide keeps its kind and takes no title from the builder.
    match guide(spec(&chart, Channel::Color)) {
        guide::Guide::Legend(legend) => assert_eq!(legend.title, None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn scale_options_reach_the_spec() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x(Encode::column("weight").scale(
            Scale::new()
                .option("round", true)
                .option("clamp", true)
                .nice(5.0)
                .zero(true)
                .family("weights"),
        ))
        .y(Encode::column("mpg")
            .scale(Scale::new().domain(Arc::new(Float64Array::from(vec![0.0, 40.0])))))
        .color(
            Encode::column("origin").scale(
                Scale::new()
                    .preferred(Arc::new(StringArray::from(vec!["US"])))
                    .palette("tableau10"),
            ),
        )
        .build()
        .unwrap();
    let x = spec(&chart, Channel::X);
    let names: Vec<&str> = x
        .options
        .iter()
        .map(|option| option.name.as_str())
        .collect();
    assert_eq!(names, ["clamp", "round"]);
    assert_eq!(interval(x).nice, Some(5.0));
    assert_eq!(interval(x).zero, Some(true));
    assert_eq!(x.family, "weights");
    let y = spec(&chart, Channel::Y);
    assert!(matches!(
        y.domain.as_ref().unwrap().domain,
        Some(domain::Domain::Explicit(_))
    ));
    assert!(y.fit.is_none());
    let color = spec(&chart, Channel::Color);
    assert!(matches!(
        color.range.as_ref().unwrap().range,
        Some(proto::range::Range::Palette(_))
    ));
}

#[test]
fn constants_and_scaled_constants_bind_channels() {
    let rule = Chart::unit(cars(), proto::RuleMark::default())
        .y(Encode::scaled_constant(20.0))
        .stroke(Encode::constant("gray"))
        .build()
        .unwrap();
    assert_eq!(spec(&rule, Channel::Y).r#type, "linear");
    assert!(encoding(&rule, Channel::Stroke).scale.is_none());
}

#[test]
fn builder_errors_name_the_part() {
    invalid(
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y("missing")
            .build(),
        "root/encodings[1]",
        "no column `missing`",
    );
    invalid(
        Chart::unit(cars(), proto::LineMark::default())
            .x("weight")
            .y("mpg")
            .shape("origin")
            .build(),
        "root/encodings[2]",
        "a line mark in a Cartesian plot does not take `CHANNEL_SHAPE`",
    );
    invalid(
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y(Encode::column("mpg")
                .scale(Scale::new().domain(Arc::new(Float64Array::from(vec![40.0, 0.0])))))
            .build(),
        "root/encodings[1]/scale/domain",
        "ascending order",
    );
    invalid(
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y("mpg")
            .color_values("count")
            .build(),
        "root/encodings[2]",
        "without a scale cannot hold values of type Int64",
    );
    invalid(
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y("mpg")
            .tool(proto::Navigate::default())
            .build(),
        "root",
        "a node that declares tools has an id",
    );
    let plot = |id: &str| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .id(id)
            .x("weight")
            .y("mpg")
            .build()
            .unwrap()
    };
    invalid(
        Chart::overlay([plot("a"), plot("a")]).build(),
        "root/children[1]",
        "two nodes have the id `a` and one partition",
    );
}

/// The chart of the cars of one origin, with an id that every facet shares, a brush selection
/// that it owns, and a credit.
fn facet(origin: &str) -> Chart {
    let rows: Vec<usize> = cars()
        .column_by_name("origin")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, value)| *value == Some(origin))
        .map(|(row, _)| row)
        .collect();
    let indices = arrow::array::UInt32Array::from_iter_values(rows.iter().map(|row| *row as u32));
    let table = arrow::compute::take_record_batch(&cars(), &indices).unwrap();
    Chart::unit(table, proto::SymbolMark::default())
        .id("plot")
        .partition("dataset", "cars")
        .partition("origin", origin)
        .x("weight")
        .y("mpg")
        .tool(proto::Select {
            selection: "brush".into(),
            ..Default::default()
        })
        .selection(
            Selection::new("brush", proto::SelectionResolution::Union).initial(
                Contribution::new("plot")
                    .partition("dataset", "cars")
                    .partition("origin", origin)
                    .tuple([TupleTest::Set {
                        column: "origin".into(),
                        channel: Channel::Unspecified,
                        values: Arc::new(StringArray::from(vec![origin])),
                    }]),
            ),
        )
        .attribution(proto::Attribution {
            credits: vec!["Source: StatLib".into()],
            ..Default::default()
        })
        .build()
        .unwrap()
}

#[test]
fn facet_charts_combine_into_a_facet_grid() {
    let row = Chart::row([facet("US"), facet("EU")])
        .facet("origin")
        .build()
        .unwrap();
    let Some(node::Node::Concat(concat)) = &row.root().node else {
        panic!("expected a concat");
    };
    // The value that every child declares moves to the row, and the facet's column stays.
    let columns = |partition: &[proto::PartitionValue]| -> Vec<String> {
        partition.iter().map(|value| value.column.clone()).collect()
    };
    assert_eq!(columns(&concat.partition), ["dataset"]);
    // Each unit refers to the table of its own chart, after the tables of the charts before it.
    for (index, child) in concat.children.iter().enumerate() {
        let Some(node::Node::Unit(unit)) = &child.node else {
            panic!("expected a unit");
        };
        assert_eq!(columns(&unit.partition), ["origin"]);
        assert_eq!(unit.table, index as u32);
    }
    // The facet has the default header, which the renderer titles with its column.
    assert_eq!(concat.facet.as_ref().unwrap().header, None);
    // The selections of one name become one, with the contributions of both plots.
    assert_eq!(row.selections().len(), 1);
    assert_eq!(row.selections()[0].initial.len(), 2);
    assert_eq!(row.tables().len(), 2);

    // A column of the rows of two datasets keeps each dataset on its row.
    let other = Chart::row([facet("JP")]).facet("origin").build().unwrap();
    Chart::column([row, other]).build().unwrap();
}

#[test]
fn an_empty_cell_stands_for_a_facet_without_data() {
    let row = Chart::row([
        Child::from(facet("US")),
        Child::from(
            Chart::empty()
                .partition("dataset", "cars")
                .partition("origin", "BR"),
        ),
    ])
    .facet("origin")
    .build()
    .unwrap();
    let Some(node::Node::Concat(concat)) = &row.root().node else {
        panic!("expected a concat");
    };
    assert!(matches!(
        concat.children[1].node,
        Some(node::Node::Empty(_))
    ));
}

#[test]
fn a_plot_holds_the_tools_of_its_nodes_in_tree_order() {
    let points = Chart::unit(cars(), proto::SymbolMark::default())
        .id("points")
        .x("weight")
        .y("mpg")
        .tool(proto::Tooltip::default())
        .build()
        .unwrap();
    let line = Chart::unit(cars(), proto::LineMark::default())
        .x("weight")
        .y("mpg")
        .build()
        .unwrap();
    let navigate = proto::Navigate {
        channels: vec![Channel::X as i32],
    };
    let chart = Chart::overlay([points, line])
        .id("plot")
        .tool(navigate.clone())
        .build()
        .unwrap();
    let tree = avenger_chart::resolve::tree(&chart);
    let tools: Vec<proto::Tool> = tree.plots[0]
        .tools
        .iter()
        .map(|&tool| tool.clone())
        .collect();
    assert_eq!(tools, [navigate.into(), proto::Tooltip::default().into()]);
}

#[test]
fn an_overlay_drops_the_titles_of_its_children_and_keeps_their_credits() {
    let titled = |credit: &str| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .title("A plot")
            .x("weight")
            .y("mpg")
            .attribution(proto::Attribution {
                credits: vec![credit.into(), "Source: StatLib".into()],
                ..Default::default()
            })
            .build()
            .unwrap()
    };
    let overlay = Chart::overlay([titled("Source: EPA"), titled("Source: DOE")])
        .title("Fuel economy")
        .build()
        .unwrap();
    let Some(node::Node::Overlay(overlay_node)) = &overlay.root().node else {
        panic!("expected an overlay");
    };
    assert_eq!(overlay_node.title.as_ref().unwrap().text, "Fuel economy");
    assert_eq!(
        overlay_node.attribution.as_ref().unwrap().credits,
        ["Source: EPA", "Source: StatLib", "Source: DOE"]
    );
    for child in &overlay_node.children {
        let Some(node::Node::Unit(unit)) = &child.node else {
            panic!("expected a unit");
        };
        assert!(unit.title.is_none() && unit.attribution.is_none());
    }
}

#[test]
fn combinations_fail_for_selections_of_two_resolutions_and_for_facets_without_values() {
    let with_selection = |resolution: proto::SelectionResolution| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y("mpg")
            .selection(Selection::new("brush", resolution))
            .build()
            .unwrap()
    };
    invalid(
        Chart::row([
            with_selection(proto::SelectionResolution::Union),
            with_selection(proto::SelectionResolution::Global),
        ])
        .build(),
        "selections",
        "the selection `brush` has two resolutions",
    );
    let plain = || {
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y("mpg")
            .build()
            .unwrap()
    };
    invalid(
        Chart::row([plain(), plain()]).facet("origin").build(),
        "root/facet",
        "child 0 declares no value of the facet's column `origin`",
    );
    invalid(
        Chart::column([facet("US"), facet("EU")])
            .facet("origin")
            .facet_header(proto::FacetHeader {
                side: proto::Side::Top as i32,
                ..Default::default()
            })
            .build(),
        "root/facet/shown",
        "the header of a row stands on the top or the bottom",
    );
}

#[test]
fn built_charts_round_trip_through_bytes() {
    let row = Chart::row([facet("US"), facet("EU")])
        .facet("origin")
        .partition("study", Scalar(Arc::new(Date32Array::from(vec![19_000]))))
        .build()
        .unwrap();
    let decoded = Chart::from_bytes(row.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded, row);
}

#[test]
fn values_of_fields_reach_the_chart() {
    let ticks: ArrayRef = Arc::new(Float64Array::from(vec![0.0, 2.0, 4.0]));
    let domain: ArrayRef = Arc::new(Float64Array::from(vec![0.0, 40.0]));
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .x(Encode::column("weight").scale(
            Scale::new()
                .guide(proto::Axis::default())
                .guide_values(ticks.clone()),
        ))
        .y(Encode::column("mpg").scale(Scale::new().domain(domain.clone())))
        .fill(Encode::constant("steelblue"))
        .build()
        .unwrap();
    let Some(guide::Guide::Axis(axis)) = guide_of(&chart, Channel::X) else {
        panic!("expected an axis");
    };
    let Some(axis::Ticks::TickValues(index)) = axis.ticks else {
        panic!("expected tick values");
    };
    assert_eq!(chart.list(index).as_ref(), ticks.as_ref());
    let Some(domain::Domain::Explicit(index)) =
        spec(&chart, Channel::Y).domain.as_ref().unwrap().domain
    else {
        panic!("expected an explicit domain");
    };
    assert_eq!(chart.list(index).as_ref(), domain.as_ref());
    let Some(proto::encoding::Value::Constant(index)) = encoding(&chart, Channel::Fill).value
    else {
        panic!("expected a constant");
    };
    assert_eq!(
        chart.value(index).as_ref(),
        &StringArray::from(vec!["steelblue"]) as &dyn arrow::array::Array
    );
    // The values of each type stand together, in the order of the fields that refer to them.
    let types: Vec<&DataType> = chart
        .values()
        .iter()
        .map(|value| value.data_type())
        .collect();
    assert_eq!(types.len(), 3);
    assert!(matches!(types[0], DataType::List(_)) && matches!(types[1], DataType::List(_)));
    assert_eq!(types[2], &DataType::Utf8);

    // Guide values need a guide whose kind takes them.
    invalid(
        Chart::unit(cars(), proto::SymbolMark::default())
            .x(Encode::column("weight").scale(Scale::new().guide_values(ticks)))
            .y("mpg")
            .build(),
        "root/encodings[0]/scale/guide",
        "guide values belong to an axis, a legend, or a colorbar",
    );
}

fn guide_of(chart: &Chart, channel: Channel) -> Option<guide::Guide> {
    spec(chart, channel)
        .guide
        .as_ref()
        .and_then(|guide| guide.guide.clone())
}

/// A chart of symbols of `mpg` over `weight`, whose `y` scale is `scale`.
fn symbols(scale: Scale) -> Chart {
    Chart::unit(cars(), proto::SymbolMark::default())
        .x("weight")
        .y(Encode::column("mpg").scale(scale))
        .build()
        .unwrap()
}

fn axis_titled(title: &str) -> Scale {
    Scale::new().guide(proto::Axis {
        title: Some(title.into()),
        ..Default::default()
    })
}

#[test]
fn shared_scales_whose_specs_disagree_fail() {
    let disagree = |children: Vec<Chart>, earlier: &str, reason: &str| {
        invalid(
            Chart::overlay(children.clone()).build(),
            "root/children[1]",
            &format!(
                "the `CHANNEL_Y` specs of `root/children[{earlier}]` and `root/children[1]` share \
                 a scale and {reason}"
            ),
        );
        // The children keep their own scales of an independent `y`.
        Chart::overlay(children)
            .independent(Channel::Y)
            .build()
            .unwrap();
    };
    disagree(
        vec![
            symbols(Scale::new()),
            symbols(Scale::new().domain(Arc::new(Float64Array::from(vec![0.0, 40.0])))),
        ],
        "0",
        "set an explicit or a selection domain and a summary or a data domain",
    );
    disagree(
        vec![
            symbols(axis_titled("Fuel")),
            symbols(axis_titled("Economy")),
        ],
        "0",
        "set `guide.axis.title` to different values",
    );
    disagree(
        vec![
            symbols(Scale::new().guide(proto::Axis::default())),
            symbols(Scale::new().guide(proto::NoGuide {})),
        ],
        "0",
        "set guides of two kinds",
    );
    disagree(
        vec![
            symbols(Scale::new().zero(true)),
            symbols(Scale::new().zero(false)),
        ],
        "0",
        "set `fit.interval.zero` to different values",
    );
    disagree(
        vec![symbols(Scale::of("log")), symbols(Scale::new())],
        "0",
        "set `type` to different values",
    );

    // A band and a point scale share a band scale, unless the point scale sets an option that a
    // band scale does not take.
    let bars = Chart::unit(cars(), proto::BarMark::default())
        .x("origin")
        .y("count")
        .build()
        .unwrap();
    let points = |scale: Scale| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .x(Encode::column("origin").scale(scale))
            .y("count")
            .build()
            .unwrap()
    };
    Chart::overlay([points(Scale::new().option("align", 0.5)), bars.clone()])
        .build()
        .unwrap();
    invalid(
        Chart::overlay([bars, points(Scale::new().option("padding", 0.5))]).build(),
        "root/children[1]",
        "set the point option `padding`, which a band scale does not take",
    );

    // A field that one spec sets is the merged value, so the third spec disagrees with the first,
    // which sets the title, and not with the second.
    invalid(
        Chart::overlay([
            symbols(axis_titled("Fuel")),
            symbols(Scale::new().guide(proto::Axis {
                format: ".1f".into(),
                ..Default::default()
            })),
            symbols(axis_titled("Economy")),
        ])
        .build(),
        "root/children[2]",
        "the `CHANNEL_Y` specs of `root/children[0]` and `root/children[2]` share a scale",
    );

    // Specs that set different fields, or a field that the other does not set, agree.
    Chart::overlay([
        symbols(axis_titled("Fuel").zero(true)),
        symbols(Scale::new().guide(proto::Axis {
            format: ".1f".into(),
            ..Default::default()
        })),
    ])
    .build()
    .unwrap();

    // A row keeps an independent `y` for each child by default, and a facet shares it.
    let panel = |panel: &str, scale: Scale| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .partition("panel", panel)
            .x("weight")
            .y(Encode::column("mpg").scale(scale))
            .build()
            .unwrap()
    };
    Chart::row([
        panel("a", axis_titled("Fuel")),
        panel("b", axis_titled("Economy")),
    ])
    .build()
    .unwrap();
    invalid(
        Chart::row([
            panel("a", axis_titled("Fuel")),
            panel("b", axis_titled("Economy")),
        ])
        .facet("panel")
        .build(),
        "root/children[1]",
        "set `guide.axis.title` to different values",
    );
}

#[test]
fn a_plot_has_at_most_two_scales_of_a_direction() {
    let plot = |column: &str| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .x("weight")
            .y(column)
            .build()
            .unwrap()
    };
    Chart::overlay([plot("mpg"), plot("count")])
        .independent(Channel::Y)
        .build()
        .unwrap();
    invalid(
        Chart::overlay([plot("mpg"), plot("count"), plot("weight")])
            .independent(Channel::Y)
            .build(),
        "root",
        "a plot has at most two scales of `CHANNEL_Y`, one on each side, and it has 3",
    );
    // An inner overlay that shares `y` holds one scale of an outer independent `y`.
    let inner = Chart::overlay([plot("count"), plot("weight")])
        .build()
        .unwrap();
    Chart::overlay([plot("mpg"), inner.clone()])
        .independent(Channel::Y)
        .build()
        .unwrap();
    // An inner independent `y` wins over the outer shared `y`.
    let separate = Chart::overlay([plot("count"), plot("weight")])
        .independent(Channel::Y)
        .build()
        .unwrap();
    invalid(
        Chart::overlay([plot("mpg"), separate]).build(),
        "root",
        "and it has 3",
    );
}

fn share(weight: f32) -> proto::Track {
    proto::Track {
        size: Some(proto::track::Size::Share(proto::Share {
            weight,
            max: None,
        })),
    }
}

fn length(pixels: f32) -> proto::Track {
    proto::Track {
        size: Some(proto::track::Size::Length(pixels)),
    }
}

fn tracks(tracks: Vec<proto::Track>) -> proto::ConcatLayout {
    proto::ConcatLayout {
        tracks,
        ..Default::default()
    }
}

#[test]
fn the_tracks_of_a_table_have_one_size() {
    let plot = || symbols(Scale::new());
    // A row inside a row adds its children to the tracks of its parent, which its own layout
    // sizes.
    invalid(
        Chart::row([Chart::row([plot(), plot()]).build().unwrap(), plot()])
            .layout(tracks(vec![share(1.0), share(1.0)]))
            .build(),
        "root/layout/tracks[0]",
        "a child that is a concat of the same direction, or that spans several tracks, takes an \
         empty track",
    );
    invalid(
        Chart::row([plot(), plot()])
            .layout(tracks(vec![share(0.0), share(1.0)]))
            .build(),
        "root/layout/tracks[0]",
        "a share has a positive weight",
    );
    // The rows of a column share their columns, so they declare each column alike.
    let row = |first: proto::Track| {
        Chart::row([plot(), plot()])
            .layout(tracks(vec![first, share(1.0)]))
            .build()
            .unwrap()
    };
    Chart::column([row(length(100.0)), row(length(100.0))])
        .build()
        .unwrap();
    invalid(
        Chart::column([row(length(100.0)), row(length(200.0))]).build(),
        "root/children[1]/layout/tracks[0]",
        "the concats `root/children[0]` and `root/children[1]` set different sizes for column 0 \
         of their table",
    );
}

#[test]
fn the_plots_of_a_shared_position_scale_take_one_length() {
    let plot = || symbols(Scale::new());
    Chart::row([plot(), plot()])
        .share(Channel::X)
        .layout(tracks(vec![share(2.0), share(2.0)]))
        .build()
        .unwrap();
    invalid(
        Chart::row([plot(), plot()])
            .share(Channel::X)
            .layout(tracks(vec![share(1.0), share(2.0)]))
            .build(),
        "root",
        "the plots that share `CHANNEL_X` stand in column 0 of the table of \
         `root/children[0]` and column 1 of the table of `root/children[1]`, whose sizes differ",
    );
    // The weights of an independent `x` are free.
    Chart::row([plot(), plot()])
        .layout(tracks(vec![share(1.0), share(2.0)]))
        .build()
        .unwrap();
    // A plot below a row of two plots spans both columns. The row shares `x` too, because an
    // inner independent rule wins over an outer shared rule.
    let shared_row = || {
        Chart::row([plot(), plot()])
            .share(Channel::X)
            .build()
            .unwrap()
    };
    Chart::column([Chart::row([plot(), plot()]).build().unwrap(), plot()])
        .share(Channel::X)
        .build()
        .unwrap();
    invalid(
        Chart::column([shared_row(), plot()])
            .share(Channel::X)
            .build(),
        "root",
        "the plot `root/children[1]` spans 2 columns",
    );
}

#[test]
fn shared_axes_of_a_concat_agree() {
    invalid(
        Chart::column([
            symbols(axis_titled("Fuel")),
            symbols(axis_titled("Economy")),
        ])
        .share(Channel::Y)
        .build(),
        "root/children[1]",
        "the `CHANNEL_Y` specs of `root/children[0]` and `root/children[1]` share a scale and set \
         `guide.axis.title` to different values",
    );
    // A row shares `y` across its plots only when it says so.
    Chart::row([
        symbols(axis_titled("Fuel")),
        symbols(axis_titled("Economy")),
    ])
    .build()
    .unwrap();
}

#[test]
fn the_children_of_a_facet_agree() {
    let plot = |panel: &str, group: &str| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .partition("panel", panel)
            .partition("group", group)
            .x("weight")
            .y("mpg")
            .color("origin")
            .build()
            .unwrap()
    };
    let row = |panel: &str| Chart::row([plot(panel, "a"), plot(panel, "b")]).facet("group");
    // Rows that share each channel alike agree, also when one restates a default.
    Chart::column([
        row("first").independent(Channel::X).build().unwrap(),
        row("second").independent(Channel::X).build().unwrap(),
    ])
    .facet("panel")
    .build()
    .unwrap();
    Chart::column([
        row("first").share(Channel::Color).build().unwrap(),
        row("second").build().unwrap(),
    ])
    .facet("panel")
    .build()
    .unwrap();
    // One row of a facet grid cannot separate the colors that the other shares.
    invalid(
        Chart::column([
            row("first").independent(Channel::Color).build().unwrap(),
            row("second").build().unwrap(),
        ])
        .facet("panel")
        .build(),
        "root/children[1]",
        "the children 0 and 1 of a facet share `CHANNEL_COLOR` differently",
    );
    // Nor separate the projections that the other shares.
    invalid(
        Chart::column([
            row("first")
                .projection(proto::Sharing::Independent)
                .build()
                .unwrap(),
            row("second").build().unwrap(),
        ])
        .facet("panel")
        .build(),
        "root/children[1]",
        "the children 0 and 1 of a facet share the projection differently",
    );
    // Nor facet another column.
    let other = Chart::row([plot("second", "a"), plot("second", "b")])
        .build()
        .unwrap();
    invalid(
        Chart::column([row("first").build().unwrap(), other])
            .facet("panel")
            .build(),
        "root/children[1]",
        "the children 0 and 1 of a facet facet `group` and no column",
    );
}
