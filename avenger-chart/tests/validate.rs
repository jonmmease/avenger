//! The rules of the format that one message decides: each test breaks one rule of a valid chart
//! and checks the path and the reason of the error, and the lenient spellings that a reader
//! accepts.

mod support;

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    record_batch::RecordBatch,
};
use avenger_chart::{
    list_value,
    proto::{
        self, cartesian_layer, condition, domain, domain_fit, encoding, guide, node, plot,
        predicate, scale_spec, spherical, tool, CartesianLayer, Concat, Encoding, Plot, Selection,
        SymbolMark, ValueRef,
    },
    Chart, ChartError,
};
use support::*;

/// A valid scatter plot: its message, its tables, and its values.
fn base() -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
    (chart(plot(vec![symbols(0)])), vec![points()], vec![])
}

fn plot_mut(message: &mut proto::Chart) -> &mut Plot {
    match message.root.as_mut().and_then(|root| root.node.as_mut()) {
        Some(node::Node::Plot(plot)) => plot,
        _ => panic!("the root is a plot"),
    }
}

fn layer_mut(message: &mut proto::Chart) -> &mut CartesianLayer {
    match plot_mut(message).coord.as_mut() {
        Some(plot::Coord::Cartesian(plot)) => &mut plot.layers[0],
        _ => panic!("the plot is Cartesian"),
    }
}

fn symbol_mut(message: &mut proto::Chart) -> &mut SymbolMark {
    match layer_mut(message).mark.as_mut() {
        Some(cartesian_layer::Mark::Symbol(mark)) => mark,
        _ => panic!("the layer draws symbols"),
    }
}

/// The scale of `x`.
fn x_scale(message: &mut proto::Chart) -> &mut proto::ScaleSpec {
    symbol_mut(message)
        .x
        .as_mut()
        .unwrap()
        .scale
        .as_mut()
        .unwrap()
}

fn read(parts: (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>)) -> Result<Chart, ChartError> {
    Chart::from_parts(parts.0, parts.1, parts.2)
}

/// Fails unless `parts` is invalid at `path`, for a reason that contains `reason`.
#[track_caller]
fn invalid(parts: (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>), path: &str, reason: &str) {
    match read(parts) {
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

/// A value of one element.
fn floats(values: Vec<f64>) -> ArrayRef {
    list_value(Arc::new(Float64Array::from(values)))
}

const X: &str = "root.plot.cartesian.layers[0].mark.symbol.x";

#[test]
fn the_base_chart_is_valid() {
    read(base()).unwrap();
}

#[test]
fn required_fields_and_oneofs_are_set() {
    let mut parts = base();
    plot_mut(&mut parts.0).coord = None;
    invalid(
        parts,
        "root.plot",
        "the coordinate system of a plot is required",
    );

    let mut parts = base();
    layer_mut(&mut parts.0).mark = None;
    invalid(
        parts,
        "root.plot.cartesian.layers[0].mark",
        "the mark of a layer is required",
    );

    let mut parts = base();
    layer_mut(&mut parts.0).table = None;
    invalid(
        parts,
        "root.plot.cartesian.layers[0].table",
        "always refers to its table",
    );

    let mut parts = base();
    symbol_mut(&mut parts.0).x.as_mut().unwrap().value = None;
    invalid(parts, X, "the value of an encoding is required");

    let mut parts = base();
    x_scale(&mut parts.0).r#type = None;
    invalid(
        parts,
        &format!("{X}.scale"),
        "the type of a scale spec is required",
    );

    let mut parts = base();
    x_scale(&mut parts.0).domain = None;
    invalid(
        parts,
        &format!("{X}.scale.domain"),
        "the domain of a scale spec is required",
    );
}

#[test]
fn a_plot_has_a_layer() {
    let mut parts = base();
    if let Some(plot::Coord::Cartesian(plot)) = plot_mut(&mut parts.0).coord.as_mut() {
        plot.layers.clear();
    }
    invalid(parts, "root.plot.cartesian.layers", "a layer or more");
}

#[test]
fn enum_fields_hold_declared_values() {
    let mut parts = base();
    parts.0.root = Some(proto::Node {
        node: Some(node::Node::Concat(Concat {
            direction: 7,
            children: vec![plot(vec![symbols(0)])],
            ..Default::default()
        })),
    });
    invalid(
        parts.clone(),
        "root.concat.direction",
        "is not a value of `Direction`",
    );
    fn concat_mut(parts: &mut (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>)) -> &mut Concat {
        match parts.0.root.as_mut().and_then(|root| root.node.as_mut()) {
            Some(node::Node::Concat(concat)) => concat,
            _ => panic!("the root is a concat"),
        }
    }
    concat_mut(&mut parts).direction = 0;
    invalid(parts.clone(), "root.concat.direction", "is absent");
    // An `optional` enum field that is not set is valid, and its zero value is not.
    concat_mut(&mut parts).direction = proto::Direction::Row as i32;
    read(parts.clone()).unwrap();
    concat_mut(&mut parts).projection = Some(proto::Sharing::Shared as i32);
    read(parts.clone()).unwrap();
    concat_mut(&mut parts).projection = Some(0);
    invalid(parts, "root.concat.projection", "is absent");
}

#[test]
fn x_and_y_take_a_column_or_a_scaled_constant_through_a_scale() {
    let mut parts = base();
    symbol_mut(&mut parts.0).x.as_mut().unwrap().scale = None;
    invalid(parts, X, "always have a scale");

    let mut parts = base();
    symbol_mut(&mut parts.0).x.as_mut().unwrap().value =
        Some(encoding::Value::Column("missing".to_string()));
    invalid(parts, X, "no column `missing`");

    let mut parts = base();
    symbol_mut(&mut parts.0).x = Some(Encoding {
        value: Some(encoding::Value::Constant(ValueRef { index: 0 })),
        ..Default::default()
    });
    parts.2 = vec![Arc::new(Float64Array::from(vec![1.0]))];
    invalid(parts, X, "a column or a scaled constant");
}

#[test]
fn values_without_a_scale_have_the_form_of_their_channel() {
    let mut parts = base();
    symbol_mut(&mut parts.0).color = Some(Encoding {
        value: Some(encoding::Value::Column("a".to_string())),
        ..Default::default()
    });
    invalid(
        parts,
        "root.plot.cartesian.layers[0].mark.symbol.color",
        "cannot have type Float64",
    );
}

#[test]
fn scale_options_are_in_range() {
    let mut parts = base();
    x_scale(&mut parts.0).r#type = Some(scale_spec::Type::Log(proto::LogScale {
        base: Some(1.0),
        ..Default::default()
    }));
    invalid(parts, &format!("{X}.scale.log.base"), "not 1");
}

#[test]
fn a_scale_takes_values_of_its_kind() {
    let mut parts = base();
    let labels = RecordBatch::try_from_iter([
        (
            "a",
            Arc::new(StringArray::from(vec!["p", "q", "r"])) as ArrayRef,
        ),
        (
            "b",
            Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0])) as ArrayRef,
        ),
    ])
    .unwrap();
    parts.1 = vec![labels];
    invalid(
        parts,
        &format!("{X}.scale"),
        "values of type Utf8 are invalid",
    );
}

#[test]
fn an_explicit_interval_domain_holds_two_ascending_ends() {
    let mut parts = base();
    x_scale(&mut parts.0).domain = Some(proto::Domain {
        domain: Some(domain::Domain::Explicit(ValueRef { index: 0 })),
    });
    parts.2 = vec![floats(vec![0.0, 5.0, 10.0])];
    invalid(
        parts.clone(),
        &format!("{X}.scale.domain.explicit"),
        "two ends",
    );
    parts.2 = vec![floats(vec![10.0, 0.0])];
    invalid(parts, &format!("{X}.scale.domain.explicit"), "ascending");
}

#[test]
fn a_summary_has_the_layout_of_its_scale() {
    let mut parts = base();
    x_scale(&mut parts.0).domain = Some(proto::Domain {
        domain: Some(domain::Domain::Summary(proto::SummaryDomain {
            value: Some(ValueRef { index: 0 }),
            fit: None,
        })),
    });
    parts.2 = vec![floats(vec![1.0])];
    invalid(
        parts,
        &format!("{X}.scale.domain.summary.value"),
        "is a struct",
    );
}

#[test]
fn a_fit_is_of_the_kind_of_its_scale() {
    let mut parts = base();
    *data_fit(x_scale(&mut parts.0)) = Some(proto::DomainFit {
        fit: Some(domain_fit::Fit::Categories(proto::CategoryFit::default())),
    });
    invalid(
        parts.clone(),
        &format!("{X}.scale.domain.data.fit"),
        "not of the kind",
    );
    // A fit is invalid on a quantile scale.
    x_scale(&mut parts.0).r#type =
        Some(scale_spec::Type::Quantile(proto::QuantileScale::default()));
    *data_fit(x_scale(&mut parts.0)) = Some(proto::DomainFit {
        fit: Some(domain_fit::Fit::Interval(proto::IntervalFit::default())),
    });
    invalid(
        parts,
        &format!("{X}.scale.domain.data.fit"),
        "not of the kind",
    );
}

#[test]
fn the_plot_gives_the_range_of_a_position() {
    let mut parts = base();
    x_scale(&mut parts.0).range = Some(proto::Range {
        range: Some(proto::range::Range::Palette("tableau10".to_string())),
        reverse: false,
    });
    invalid(parts, &format!("{X}.scale.range"), "sets only `reverse`");
}

#[test]
fn a_guide_is_of_a_kind_that_its_channel_takes() {
    let mut parts = base();
    x_scale(&mut parts.0).guide = Some(proto::Guide {
        guide: Some(guide::Guide::Legend(proto::Legend::default())),
    });
    invalid(
        parts,
        &format!("{X}.scale.guide.legend"),
        "other than a position",
    );
}

#[test]
fn a_pattern_needs_its_syntax() {
    let mut parts = base();
    x_scale(&mut parts.0).guide = Some(proto::Guide {
        guide: Some(guide::Guide::Axis(proto::Axis {
            labels: Some(proto::GuideLabels {
                format: ",.0f".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        })),
    });
    invalid(
        parts.clone(),
        &format!("{X}.scale.guide.axis.labels.format"),
        "number_format_syntax",
    );
    parts.0.number_format_syntax = "d3".to_string();
    read(parts).unwrap();
}

#[test]
fn a_second_channel_follows_its_first() {
    let mut parts = base();
    *layer_mut(&mut parts.0) = CartesianLayer {
        table: Some(proto::TableRef { index: 0 }),
        mark: Some(cartesian_layer::Mark::Bar(proto::BarMark {
            x2: Some(Encoding {
                value: Some(encoding::Value::Column("a".to_string())),
                ..Default::default()
            }),
            y: Some(column("b", linear_data())),
            ..Default::default()
        })),
        ..Default::default()
    };
    invalid(
        parts,
        "root.plot.cartesian.layers[0].mark.bar.x2",
        "second channel of `x`",
    );
}

#[test]
fn a_text_mark_needs_its_text() {
    let mut parts = base();
    layer_mut(&mut parts.0).mark = Some(cartesian_layer::Mark::Text(proto::TextMark {
        x: Some(column("a", linear_data())),
        y: Some(column("b", linear_data())),
        ..Default::default()
    }));
    invalid(
        parts,
        "root.plot.cartesian.layers[0].mark.text",
        "needs `text`",
    );
}

#[test]
fn a_line_holds_one_value_for_each_series() {
    let mut parts = base();
    layer_mut(&mut parts.0).mark = Some(cartesian_layer::Mark::Line(proto::LineMark {
        x: Some(column("a", linear_data())),
        y: Some(column("b", linear_data())),
        opacity: Some(Encoding {
            value: Some(encoding::Value::Column("a".to_string())),
            ..Default::default()
        }),
        ..Default::default()
    }));
    invalid(
        parts,
        "root.plot.cartesian.layers[0].mark.line.opacity",
        "one value for each series",
    );
}

#[test]
fn sizes_and_paddings_are_in_range() {
    let mut parts = base();
    plot_mut(&mut parts.0).width = Some(-1.0);
    invalid(parts, "root.plot.width", "more than 0");

    let mut parts = base();
    parts.0.padding = Some(proto::Padding {
        left: Some(-2.0),
        ..Default::default()
    });
    invalid(parts, "padding.left", "at least 0");

    let mut parts = base();
    let plot = plot_mut(&mut parts.0);
    plot.width = Some(300.0);
    plot.height = Some(200.0);
    if let Some(plot::Coord::Cartesian(cartesian)) = plot.coord.as_mut() {
        cartesian.options = Some(proto::Cartesian {
            aspect_ratio: Some(1.5),
        });
    }
    invalid(parts, "root.plot", "is a conflict");
}

#[test]
fn keyed_lists_accept_equal_duplicates_and_reject_different_ones() {
    let value = |index| proto::PartitionValue {
        column: "sex".to_string(),
        value: Some(ValueRef { index }),
    };
    let strings = |value: &str| -> ArrayRef { Arc::new(StringArray::from(vec![value])) };
    let mut parts = base();
    plot_mut(&mut parts.0).partition = vec![value(0), value(1)];
    parts.2 = vec![strings("female"), strings("female")];
    read(parts.clone()).unwrap();
    parts.2 = vec![strings("female"), strings("male")];
    invalid(parts, "root.plot.partition", "is a conflict");
}

#[test]
fn spherical_maps_set_a_projection_within_range() {
    let geo = |projection| proto::Node {
        node: Some(node::Node::Plot(Plot {
            coord: Some(plot::Coord::Geo(proto::GeoPlot {
                options: Some(proto::Geo {
                    surface: Some(proto::geo::Surface::Spherical(proto::Spherical {
                        projection,
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                layers: vec![proto::GeoLayer {
                    table: Some(proto::TableRef { index: 0 }),
                    mark: Some(proto::geo_layer::Mark::Symbol(proto::GeoSymbolMark {
                        longitude: Some(Encoding {
                            value: Some(encoding::Value::Column("a".to_string())),
                            ..Default::default()
                        }),
                        latitude: Some(Encoding {
                            value: Some(encoding::Value::Column("b".to_string())),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })),
                    ..Default::default()
                }],
            })),
            ..Default::default()
        })),
    };
    let mut parts = base();
    parts.0.root = Some(geo(None));
    invalid(
        parts.clone(),
        "root.plot.geo.options.spherical",
        "the projection of a spherical map is required",
    );
    parts.0.root = Some(geo(Some(spherical::Projection::ConicConformal(
        proto::ConicProjection {
            parallel1: 100.0,
            parallel2: 30.0,
        },
    ))));
    invalid(
        parts.clone(),
        "root.plot.geo.options.spherical.parallel1",
        "from -90 to 90",
    );
    // A map's surface has no default, so a map plot always sets its options.
    if let Some(node::Node::Plot(plot)) = parts.0.root.as_mut().and_then(|root| root.node.as_mut())
    {
        if let Some(plot::Coord::Geo(map)) = &mut plot.coord {
            map.options = None;
        }
    }
    invalid(
        parts,
        "root.plot.geo.options",
        "the options of a map plot are required",
    );
}

#[test]
fn each_value_is_one_element() {
    let mut parts = base();
    parts.2 = vec![
        Arc::new(Float64Array::from(vec![1.0, 2.0])),
        Arc::new(Float64Array::from(vec![3.0])),
    ];
    invalid(parts, "values", "value 0 has 2 elements");
}

#[test]
fn tools_fit_their_plot() {
    let navigate = || proto::Tool {
        kind: Some(tool::Kind::Navigate(proto::Navigate::default())),
    };
    let mut parts = base();
    plot_mut(&mut parts.0).tools = vec![navigate()];
    invalid(parts.clone(), "root.plot", "has an id");
    plot_mut(&mut parts.0).id = "scatter".to_string();
    read(parts.clone()).unwrap();
    plot_mut(&mut parts.0).tools = vec![navigate(), navigate()];
    invalid(parts, "root.plot.tools[1]", "at most one navigation");

    let mut parts = base();
    let plot = plot_mut(&mut parts.0);
    plot.id = "scatter".to_string();
    plot.tools = vec![proto::Tool {
        kind: Some(tool::Kind::Select(proto::Select {
            selection: "brush".to_string(),
            ..Default::default()
        })),
    }];
    invalid(
        parts.clone(),
        "root.plot.tools[0].select.selection",
        "no selection `brush`",
    );
    parts.0.selections = vec![Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        ..Default::default()
    }];
    read(parts).unwrap();
}

#[test]
fn selections_set_their_resolution_and_their_clauses_hold_conditions() {
    let mut parts = base();
    parts.0.selections = vec![Selection {
        name: "brush".to_string(),
        ..Default::default()
    }];
    invalid(parts.clone(), "selections[0].resolution", "is absent");
    parts.0.selections[0].resolution = proto::SelectionResolution::Global as i32;
    parts.0.selections[0].initial = vec![proto::Clause {
        owner: "scatter".to_string(),
        ..Default::default()
    }];
    invalid(
        parts.clone(),
        "selections[0].initial[0].conditions",
        "a condition or more",
    );
    parts.0.selections[0].initial[0].conditions = vec![proto::Condition {
        kind: Some(condition::Kind::Range(proto::ValueRange {
            column: "a".to_string(),
            low: Some(ValueRef { index: 0 }),
            high: Some(ValueRef { index: 1 }),
            ..Default::default()
        })),
    }];
    parts.2 = vec![
        Arc::new(Float64Array::from(vec![5.0])),
        Arc::new(Float64Array::from(vec![1.0])),
    ];
    invalid(
        parts,
        "selections[0].initial[0].conditions[0].range",
        "at or above its low end",
    );
}

#[test]
fn a_predicate_names_a_selection_of_the_chart() {
    let mut parts = base();
    symbol_mut(&mut parts.0).opacity = Some(Encoding {
        value: Some(encoding::Value::Constant(ValueRef { index: 0 })),
        conditional: vec![proto::ConditionalValue {
            predicate: Some(proto::Predicate {
                predicate: Some(predicate::Predicate::Selected(proto::Selected {
                    selection: "brush".to_string(),
                    ..Default::default()
                })),
            }),
            value: Some(proto::conditional_value::Value::Constant(ValueRef {
                index: 1,
            })),
        }],
        ..Default::default()
    });
    parts.2 = vec![
        Arc::new(Float64Array::from(vec![1.0])),
        Arc::new(Float64Array::from(vec![0.3])),
    ];
    invalid(
        parts,
        "root.plot.cartesian.layers[0].mark.symbol.opacity.conditional[0].predicate.selected.selection",
        "no selection `brush`",
    );
}

#[test]
fn a_navigation_is_valid_only_on_a_continuous_scale_of_x_or_y() {
    let mut parts = base();
    x_scale(&mut parts.0).navigation = Some(proto::ScaleNavigation::default());
    read(parts.clone()).unwrap();
    x_scale(&mut parts.0).r#type = Some(scale_spec::Type::Band(proto::BandScale::default()));
    invalid(
        parts,
        &format!("{X}.scale.navigation"),
        "valid only on a continuous scale of `x` or `y`",
    );
    let mut parts = base();
    let mut color = column("b", linear_data());
    color.scale.as_mut().unwrap().navigation = Some(proto::ScaleNavigation::default());
    symbol_mut(&mut parts.0).color = Some(color);
    invalid(
        parts,
        "root.plot.cartesian.layers[0].mark.symbol.color.scale.navigation",
        "valid only on a continuous scale of `x` or `y`",
    );
}
