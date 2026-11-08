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
        self, cartesian_layer, condition, domain, domain_fit, encoding, node, plot, predicate,
        scale, spherical, tool, CartesianLayer, Concat, Encoding, Plot, Selection, SymbolMark,
        ValueRef,
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
fn x_scale(message: &mut proto::Chart) -> &mut proto::Scale {
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

const X: &str = "root.plot.cartesian.layers[0].symbol.x";

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
        "`coord` is required: one of `cartesian`, `polar`, `geo`",
    );

    let mut parts = base();
    layer_mut(&mut parts.0).mark = None;
    invalid(parts, "root.plot.cartesian.layers[0]", "`mark` is required");

    let mut parts = base();
    layer_mut(&mut parts.0).table = None;
    invalid(
        parts,
        "root.plot.cartesian.layers[0].table",
        "`table` is required",
    );

    let mut parts = base();
    symbol_mut(&mut parts.0).x.as_mut().unwrap().kind = None;
    invalid(parts, X, "`kind` is required");

    // The reader infers an absent scale type and domain.
    let mut parts = base();
    x_scale(&mut parts.0).r#type = None;
    x_scale(&mut parts.0).domain = None;
    read(parts).unwrap();
}

#[test]
fn a_plot_has_a_layer() {
    let mut parts = base();
    if let Some(plot::Coord::Cartesian(plot)) = plot_mut(&mut parts.0).coord.as_mut() {
        plot.layers.clear();
    }
    invalid(
        parts,
        "root.plot.cartesian.layers",
        "`layers` holds one item or more",
    );
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
    invalid(
        parts.clone(),
        "root.concat.direction",
        "`direction` is required and not `DIRECTION_UNSPECIFIED`",
    );
    // The zero value of a field that is not required is absent, and valid.
    concat_mut(&mut parts).direction = proto::Direction::Row as i32;
    concat_mut(&mut parts).projection = 0;
    read(parts.clone()).unwrap();
    concat_mut(&mut parts).projection = proto::Sharing::Shared as i32;
    read(parts).unwrap();
}

/// `x` and `y` take a scaled field or a scaled value through their scale, or a direct number,
/// which is a fraction of the plot.
#[test]
fn x_and_y_take_scaled_values_through_a_scale_or_direct_numbers() {
    let mut parts = base();
    symbol_mut(&mut parts.0).x.as_mut().unwrap().kind =
        Some(encoding::Kind::ScaledField("missing".to_string()));
    invalid(parts, X, "no column `missing`");

    let mut parts = base();
    symbol_mut(&mut parts.0).x.as_mut().unwrap().kind =
        Some(encoding::Kind::DirectField("a".to_string()));
    invalid(parts, X, "a `scaled_field` or a `scaled_value`");

    let mut parts = base();
    symbol_mut(&mut parts.0).x = Some(Encoding {
        kind: Some(encoding::Kind::DirectValue(ValueRef { index: 0 })),
        ..Default::default()
    });
    parts.2 = vec![Arc::new(Float64Array::from(vec![1.0]))];
    read(parts.clone()).unwrap();
    parts.2 = vec![Arc::new(StringArray::from(vec!["right"]))];
    invalid(parts, X, "`x` values without a scale cannot have type Utf8");
}

#[test]
fn an_encoding_has_a_scale_exactly_when_it_has_a_scaled_value() {
    // A plain column with a scale.
    let mut parts = base();
    symbol_mut(&mut parts.0).size = Some(Encoding {
        kind: Some(encoding::Kind::DirectField("a".to_string())),
        scale: Some(linear_data()),
        ..Default::default()
    });
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.size",
        "has no scale",
    );

    // A channel that takes no scale takes no scaled value.
    let mut parts = base();
    layer_mut(&mut parts.0).mark = Some(cartesian_layer::Mark::Line(proto::LineMark {
        x: Some(column("a", linear_data())),
        y: Some(column("b", linear_data())),
        detail: Some(Encoding {
            kind: Some(encoding::Kind::ScaledField("a".to_string())),
            ..Default::default()
        }),
        ..Default::default()
    }));
    invalid(
        parts,
        "root.plot.cartesian.layers[0].line.detail",
        "takes no scale",
    );
}

#[test]
fn a_plain_conditional_value_in_a_scaled_encoding_is_drawn_as_it_is() {
    let gray = |parts: &mut (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>), value| {
        symbol_mut(&mut parts.0).size = Some(Encoding {
            kind: Some(encoding::Kind::ScaledField("a".to_string())),
            scale: Some(linear_data()),
            conditional: vec![proto::ConditionalValue {
                predicate: Some(proto::Predicate {
                    predicate: Some(predicate::Predicate::All(proto::PredicateList::default())),
                }),
                kind: Some(value),
            }],
            ..Default::default()
        });
    };
    // A plain constant of the form of `size` reads, and a plain column of strings does not.
    let mut parts = base();
    parts.2 = vec![Arc::new(Float64Array::from(vec![4.0]))];
    gray(
        &mut parts,
        proto::conditional_value::Kind::DirectValue(ValueRef { index: 0 }),
    );
    read(parts).unwrap();

    let mut parts = base();
    parts.1 = vec![RecordBatch::try_from_iter([
        (
            "a",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
        ),
        (
            "b",
            Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0])) as ArrayRef,
        ),
        (
            "c",
            Arc::new(StringArray::from(vec!["x", "y", "z"])) as ArrayRef,
        ),
    ])
    .unwrap()];
    gray(
        &mut parts,
        proto::conditional_value::Kind::DirectField("c".to_string()),
    );
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.size.conditional[0]",
        "cannot have type Utf8",
    );
}

#[test]
fn the_second_channel_of_a_pair_follows_the_scale_of_the_first() {
    let bar = |theta2: Encoding| {
        let layer = proto::PolarLayer {
            table: Some(proto::TableRef { index: 0 }),
            mark: Some(proto::polar_layer::Mark::Bar(proto::PolarBarMark {
                theta: Some(column("a", linear_data())),
                theta2: Some(theta2),
                radius: Some(column("b", linear_data())),
                ..Default::default()
            })),
            ..Default::default()
        };
        let root = proto::Node {
            node: Some(node::Node::Plot(Plot {
                coord: Some(plot::Coord::Polar(proto::PolarPlot {
                    layers: vec![layer],
                    ..Default::default()
                })),
                ..Default::default()
            })),
        };
        (chart(root), vec![points()], vec![])
    };
    read(bar(Encoding {
        kind: Some(encoding::Kind::ScaledField("b".to_string())),
        ..Default::default()
    }))
    .unwrap();
    invalid(
        bar(Encoding {
            kind: Some(encoding::Kind::DirectField("b".to_string())),
            ..Default::default()
        }),
        "root.plot.polar.layers[0].bar.theta2",
        "goes through the scale of `theta`",
    );
}

#[test]
fn values_without_a_scale_have_the_form_of_their_channel() {
    let mut parts = base();
    symbol_mut(&mut parts.0).color = Some(Encoding {
        kind: Some(encoding::Kind::DirectField("a".to_string())),
        ..Default::default()
    });
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.color",
        "cannot have type Float64",
    );
}

#[test]
fn scale_options_are_in_range() {
    let mut parts = base();
    x_scale(&mut parts.0).r#type = Some(scale::Type::Log(proto::LogScale {
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
    x_scale(&mut parts.0).r#type = Some(scale::Type::Quantile(proto::QuantileScale::default()));
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
    x_scale(&mut parts.0).legend = Some(proto::Legend::default());
    invalid(parts, &format!("{X}.scale.legend"), "other than a position");
}

#[test]
fn a_pattern_needs_its_syntax() {
    let mut parts = base();
    x_scale(&mut parts.0).axis = Some(proto::Axis {
        labels: Some(proto::Labels {
            format: ",.0f".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    invalid(
        parts.clone(),
        &format!("{X}.scale.axis.labels.format"),
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
                kind: Some(encoding::Kind::DirectField("a".to_string())),
                ..Default::default()
            }),
            y: Some(column("b", linear_data())),
            ..Default::default()
        })),
        ..Default::default()
    };
    invalid(
        parts,
        "root.plot.cartesian.layers[0].bar.x2",
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
        "root.plot.cartesian.layers[0].text.text",
        "`text` is required",
    );
}

#[test]
fn a_line_holds_one_value_for_each_series() {
    let mut parts = base();
    layer_mut(&mut parts.0).mark = Some(cartesian_layer::Mark::Line(proto::LineMark {
        x: Some(column("a", linear_data())),
        y: Some(column("b", linear_data())),
        opacity: Some(Encoding {
            kind: Some(encoding::Kind::DirectField("a".to_string())),
            ..Default::default()
        }),
        ..Default::default()
    }));
    invalid(
        parts,
        "root.plot.cartesian.layers[0].line.opacity",
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
                            kind: Some(encoding::Kind::DirectField("a".to_string())),
                            ..Default::default()
                        }),
                        latitude: Some(Encoding {
                            kind: Some(encoding::Kind::DirectField("b".to_string())),
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
        "`projection` is required",
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
    // A map's surface has no default, so a geo plot always sets its options.
    if let Some(node::Node::Plot(plot)) = parts.0.root.as_mut().and_then(|root| root.node.as_mut())
    {
        if let Some(plot::Coord::Geo(map)) = &mut plot.coord {
            map.options = None;
        }
    }
    invalid(parts, "root.plot.geo.options", "`options` is required");
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

/// A tooltip's title shows only its field's value, and the labels of tooltip fields take the
/// tooltip's style.
#[test]
fn a_tooltip_title_has_no_field_title_and_its_labels_no_style() {
    let tooltip = |tooltip: proto::Tooltip| {
        let mut parts = base();
        let plot = plot_mut(&mut parts.0);
        plot.id = "scatter".to_string();
        plot.tools = vec![proto::Tool {
            kind: Some(tool::Kind::Tooltip(tooltip)),
        }];
        parts
    };
    let field = |title: Option<&str>, labels: Option<proto::Labels>| proto::TooltipField {
        column: "a".to_string(),
        title: title.map(str::to_string),
        labels,
    };
    let markup = proto::Labels {
        markup: "#value km".to_string(),
        ..Default::default()
    };
    read(tooltip(proto::Tooltip {
        title: Some(field(None, Some(markup.clone()))),
        fields: vec![field(Some("A"), Some(markup))],
        ..Default::default()
    }))
    .unwrap();

    invalid(
        tooltip(proto::Tooltip {
            title: Some(field(Some("A"), None)),
            ..Default::default()
        }),
        "root.plot.tools[0].tooltip.title.title",
        "shows only its value",
    );
    let styled = proto::Labels {
        style: Some(proto::TextStyle::default()),
        ..Default::default()
    };
    invalid(
        tooltip(proto::Tooltip {
            fields: vec![field(None, Some(styled))],
            ..Default::default()
        }),
        "root.plot.tools[0].tooltip.fields[0].labels.style",
        "takes the tooltip's style",
    );
}

#[test]
fn selections_set_their_resolution_and_their_clauses_hold_conditions() {
    let mut parts = base();
    parts.0.selections = vec![Selection {
        name: "brush".to_string(),
        ..Default::default()
    }];
    invalid(
        parts.clone(),
        "selections[0].resolution",
        "`resolution` is required and not `SELECTION_RESOLUTION_UNSPECIFIED`",
    );
    parts.0.selections[0].resolution = proto::SelectionResolution::Global as i32;
    parts.0.selections[0].initial = vec![proto::Clause {
        owner: "scatter".to_string(),
        ..Default::default()
    }];
    invalid(
        parts.clone(),
        "selections[0].initial[0].conditions",
        "`conditions` holds one item or more",
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

/// A range has one end or two: an absent end does not bound it, and a range has at least one.
#[test]
fn a_range_has_one_end_or_two() {
    let clause = |low: Option<u32>, high: Option<u32>| proto::Clause {
        owner: "scatter".to_string(),
        conditions: vec![proto::Condition {
            kind: Some(condition::Kind::Range(proto::ValueRange {
                column: "a".to_string(),
                low: low.map(|index| ValueRef { index }),
                high: high.map(|index| ValueRef { index }),
                ..Default::default()
            })),
        }],
        ..Default::default()
    };
    let parts = |clause: proto::Clause| {
        let mut message = chart(selecting());
        message.selections = vec![Selection {
            initial: vec![clause],
            ..brush(proto::SelectionResolution::Global)
        }];
        (
            message,
            vec![points()],
            vec![Arc::new(Float64Array::from(vec![2.0])) as ArrayRef],
        )
    };
    read(parts(clause(Some(0), None))).unwrap();
    read(parts(clause(None, Some(0)))).unwrap();
    invalid(
        parts(clause(None, None)),
        "selections[0].initial[0].conditions[0].range",
        "a low end, a high end, or both",
    );
}

#[test]
fn a_predicate_names_a_selection_of_the_chart() {
    let mut parts = base();
    symbol_mut(&mut parts.0).opacity = Some(Encoding {
        kind: Some(encoding::Kind::DirectValue(ValueRef { index: 0 })),
        conditional: vec![proto::ConditionalValue {
            predicate: Some(proto::Predicate {
                predicate: Some(predicate::Predicate::Selected(proto::Selected {
                    selection: "brush".to_string(),
                    ..Default::default()
                })),
            }),
            kind: Some(proto::conditional_value::Kind::DirectValue(ValueRef {
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
        "root.plot.cartesian.layers[0].symbol.opacity.conditional[0].predicate.selected.selection",
        "no selection `brush`",
    );
}

#[test]
fn a_navigation_is_valid_only_on_a_continuous_scale_of_x_or_y() {
    let mut parts = base();
    x_scale(&mut parts.0).navigation_limits = Some(proto::NavigationLimits::default());
    read(parts.clone()).unwrap();
    x_scale(&mut parts.0).r#type = Some(scale::Type::Band(proto::BandScale::default()));
    invalid(
        parts,
        &format!("{X}.scale.navigation_limits"),
        "valid only on a continuous scale of `x` or `y`",
    );
    let mut parts = base();
    let mut color = column("b", linear_data());
    color.scale.as_mut().unwrap().navigation_limits = Some(proto::NavigationLimits::default());
    symbol_mut(&mut parts.0).color = Some(color);
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.color.scale.navigation_limits",
        "valid only on a continuous scale of `x` or `y`",
    );
}

/// A chart of symbols whose `x` is a column `a` of paths of two levels, `region` and `city`,
/// through a nested band scale with `guide`.
fn nested(guide: proto::Axis) -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
    use arrow::{
        array::{Array, StructArray},
        datatypes::{DataType, Field, Schema},
    };
    let paths = StructArray::from(vec![
        (
            Arc::new(Field::new("region", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["east", "east", "west"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("city", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["NYC", "BOS", "SFO"])) as ArrayRef,
        ),
    ]);
    let schema = Schema::new(vec![
        Field::new("a", paths.data_type().clone(), false),
        Field::new("b", DataType::Float64, false),
    ]);
    let table = RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(paths),
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
        ],
    )
    .unwrap();
    let mut parts = (chart(plot(vec![symbols(0)])), vec![table], vec![]);
    let x = x_scale(&mut parts.0);
    x.r#type = Some(scale::Type::NestedBand(proto::NestedBandScale::default()));
    x.axis = Some(guide);
    parts
}

#[test]
fn the_levels_of_an_axis_belong_to_a_nested_band_scale() {
    let level = |show_labels| proto::NestedAxisLevel {
        show_labels,
        ..Default::default()
    };
    let axis = |levels| proto::Axis {
        levels,
        ..Default::default()
    };
    read(nested(axis(vec![level(Some(false)), level(None)]))).unwrap();
    invalid(
        nested(axis(vec![level(None), level(None), level(None)])),
        &format!("{X}.scale.axis.levels"),
        "the axis has 3 levels, and the paths of its scale have 2",
    );
    // A level's pattern of numbers needs its syntax, and the values of these levels are strings.
    let formatted = proto::NestedAxisLevel {
        format: ",.0f".to_string(),
        ..Default::default()
    };
    read(nested(axis(vec![formatted]))).unwrap();
    let mut parts = base();
    x_scale(&mut parts.0).axis = Some(axis(vec![level(None)]));
    invalid(
        parts,
        &format!("{X}.scale.axis.levels"),
        "valid only on the axis of a nested band scale",
    );
}

#[test]
fn the_ticks_of_a_category_axis_stand_where_its_scale_allows() {
    let ticks = |at| proto::Axis {
        ticks: Some(proto::Ticks {
            at: Some(at),
            ..Default::default()
        }),
        ..Default::default()
    };
    invalid(
        nested(ticks(proto::ticks::At::Count(3.0))),
        &format!("{X}.scale.axis.ticks.count"),
        "ticks at the bands of its leaf",
    );
    let band = |at| {
        let mut parts = base();
        parts.1 = vec![table(&[
            ("a", vec![1.0, 2.0, 3.0]),
            ("b", vec![4.0, 6.0, 5.0]),
        ])];
        let x = x_scale(&mut parts.0);
        x.r#type = Some(scale::Type::Band(proto::BandScale::default()));
        x.axis = Some(ticks(at));
        parts
    };
    invalid(
        band(proto::ticks::At::Count(3.0)),
        &format!("{X}.scale.axis.ticks.count"),
        "only a list of ticks is valid",
    );
    let mut listed = band(proto::ticks::At::Values(ValueRef { index: 0 }));
    listed.2 = vec![list_value(Arc::new(Float64Array::from(vec![1.0, 3.0])))];
    read(listed).unwrap();
}

#[test]
fn a_class_is_not_empty_and_holds_no_whitespace() {
    let strings = |classes: &[&str]| classes.iter().map(|class| class.to_string()).collect();
    let mut parts = base();
    plot_mut(&mut parts.0).classes = strings(&["wide", ""]);
    invalid(parts, "root.plot.classes[1]", "a class is not empty");

    let mut parts = base();
    layer_mut(&mut parts.0).classes = strings(&["muted dim"]);
    invalid(
        parts,
        "root.plot.cartesian.layers[0].classes[0]",
        "holds no whitespace",
    );

    let mut parts = base();
    x_scale(&mut parts.0).axis = Some(proto::Axis {
        classes: strings(&["a\tb"]),
        ..Default::default()
    });
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.x.scale.axis.classes[0]",
        "holds no whitespace",
    );

    let mut parts = base();
    plot_mut(&mut parts.0).classes = strings(&["report", "wide"]);
    layer_mut(&mut parts.0).classes = strings(&["muted"]);
    read(parts).unwrap();
}

/// The base chart with a selection `filters` and `widget` declared on its layer.
fn with_widget(
    widget: proto::Widget,
    values: Vec<ArrayRef>,
) -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
    let mut parts = base();
    parts.0.selections = vec![Selection {
        name: "filters".to_string(),
        resolution: proto::SelectionResolution::Intersect as i32,
        ..Default::default()
    }];
    layer_mut(&mut parts.0).widgets = vec![widget];
    parts.2 = values;
    parts
}

/// A widget of `kind` that writes `filters` over the column `a`.
fn widget(kind: proto::widget::Kind) -> proto::Widget {
    let mut widget = proto::Widget {
        key: "a".to_string(),
        kind: Some(kind),
        ..Default::default()
    };
    *target_of(&mut widget) = Some(proto::WidgetTarget {
        selection: "filters".to_string(),
        column: "a".to_string(),
    });
    widget
}

/// The target of a widget's kind, to change.
fn target_of(widget: &mut proto::Widget) -> &mut Option<proto::WidgetTarget> {
    use proto::widget::Kind;
    match widget.kind.as_mut().expect("a widget has a kind") {
        Kind::Slider(slider) => &mut slider.target,
        Kind::RangeSlider(slider) => &mut slider.target,
        Kind::Radio(radio) => &mut radio.target,
        Kind::Checkboxes(checkboxes) => &mut checkboxes.target,
        Kind::Checkbox(checkbox) => &mut checkbox.target,
    }
}

fn data_domain() -> Option<proto::WidgetDomain> {
    Some(proto::WidgetDomain {
        domain: Some(proto::widget_domain::Domain::Data(
            proto::WidgetDataDomain::default(),
        )),
    })
}

fn explicit_domain(index: u32) -> Option<proto::WidgetDomain> {
    Some(proto::WidgetDomain {
        domain: Some(proto::widget_domain::Domain::Explicit(ValueRef { index })),
    })
}

fn slider(comparison: proto::SliderComparison, step: Option<u32>) -> proto::widget::Kind {
    proto::widget::Kind::Slider(proto::SliderWidget {
        domain: data_domain(),
        comparison: comparison as i32,
        step: step.map(|index| ValueRef { index }),
        ..Default::default()
    })
}

const WIDGET: &str = "root.plot.cartesian.layers[0].widgets[0]";

fn one(value: f64) -> ArrayRef {
    Arc::new(Float64Array::from(vec![value]))
}

#[test]
fn each_kind_of_widget_reads() {
    let step = vec![one(0.5)];
    for (kind, values) in [
        (
            slider(proto::SliderComparison::Equal, Some(0)),
            step.clone(),
        ),
        (slider(proto::SliderComparison::AtLeast, None), vec![]),
        (
            slider(proto::SliderComparison::AtMost, Some(0)),
            step.clone(),
        ),
        (
            proto::widget::Kind::RangeSlider(proto::RangeSliderWidget {
                domain: explicit_domain(0),
                ..Default::default()
            }),
            vec![floats(vec![0.0, 10.0])],
        ),
        (
            proto::widget::Kind::Radio(proto::RadioWidget {
                domain: Some(proto::WidgetDomain {
                    domain: Some(proto::widget_domain::Domain::Summary(
                        proto::WidgetSummaryDomain {
                            values: Some(ValueRef { index: 0 }),
                            fit: Some(proto::DomainFit {
                                fit: Some(domain_fit::Fit::Categories(
                                    proto::CategoryFit::default(),
                                )),
                            }),
                        },
                    )),
                }),
                clear_label: "All".to_string(),
                ..Default::default()
            }),
            vec![floats(vec![3.0, 1.0, 3.0])],
        ),
        (
            proto::widget::Kind::Checkboxes(proto::CheckboxesWidget {
                domain: data_domain(),
                ..Default::default()
            }),
            vec![],
        ),
        (
            proto::widget::Kind::Checkbox(proto::CheckboxWidget {
                value: Some(ValueRef { index: 0 }),
                ..Default::default()
            }),
            vec![one(2.0)],
        ),
    ] {
        read(with_widget(widget(kind), values)).unwrap();
    }
}

#[test]
fn a_widget_names_its_key_selection_column_and_kind() {
    let valid = || widget(slider(proto::SliderComparison::AtLeast, None));
    let mut missing = valid();
    missing.key.clear();
    invalid(
        with_widget(missing, vec![]),
        &format!("{WIDGET}.key"),
        "`key` is required and not empty",
    );
    let mut unknown = valid();
    target_of(&mut unknown).as_mut().unwrap().selection = "brush".to_string();
    invalid(
        with_widget(unknown, vec![]),
        &format!("{WIDGET}.slider.target.selection"),
        "no selection `brush`",
    );
    let mut absent = valid();
    target_of(&mut absent).as_mut().unwrap().column = "c".to_string();
    invalid(
        with_widget(absent, vec![]),
        &format!("{WIDGET}.slider.target.column"),
        "no column `c`",
    );
    let mut untargeted = valid();
    *target_of(&mut untargeted) = None;
    invalid(
        with_widget(untargeted, vec![]),
        &format!("{WIDGET}.slider.target"),
        "`target` is required",
    );
    let mut kindless = valid();
    kindless.kind = None;
    invalid(with_widget(kindless, vec![]), WIDGET, "`kind` is required");
    let mut parts = with_widget(valid(), vec![]);
    layer_mut(&mut parts.0).widgets.push(valid());
    invalid(
        parts,
        "root.plot.cartesian.layers[0].widgets[1].key",
        "at most once",
    );
}

#[test]
fn a_widget_domain_fits_its_kind_and_its_column() {
    let slider_path = format!("{WIDGET}.slider");
    let mut no_domain = widget(slider(proto::SliderComparison::AtLeast, None));
    if let Some(proto::widget::Kind::Slider(slider)) = no_domain.kind.as_mut() {
        slider.domain = None;
    }
    invalid(
        with_widget(no_domain, vec![]),
        &format!("{slider_path}.domain"),
        "`domain` is required",
    );
    let range = |values: ArrayRef| {
        with_widget(
            widget(proto::widget::Kind::RangeSlider(proto::RangeSliderWidget {
                domain: explicit_domain(0),
                ..Default::default()
            })),
            vec![values],
        )
    };
    invalid(
        range(floats(vec![10.0, 0.0])),
        &format!("{WIDGET}.range_slider.domain.explicit"),
        "ascending order",
    );
    invalid(
        range(floats(vec![1.0, 2.0, 3.0])),
        &format!("{WIDGET}.range_slider.domain.explicit"),
        "two ends",
    );
    invalid(
        range(list_value(Arc::new(arrow::array::Int64Array::from(vec![
            0, 10,
        ])))),
        &format!("{WIDGET}.range_slider.domain"),
        "column holds Float64",
    );
    let items = with_widget(
        widget(proto::widget::Kind::Checkboxes(proto::CheckboxesWidget {
            domain: explicit_domain(0),
            ..Default::default()
        })),
        vec![floats(vec![])],
    );
    invalid(
        items,
        &format!("{WIDGET}.checkboxes.domain.explicit"),
        "one or more",
    );
    let mut fit = widget(slider(proto::SliderComparison::AtLeast, None));
    if let Some(proto::widget::Kind::Slider(slider)) = fit.kind.as_mut() {
        slider.domain = Some(proto::WidgetDomain {
            domain: Some(proto::widget_domain::Domain::Data(
                proto::WidgetDataDomain {
                    fit: Some(proto::DomainFit {
                        fit: Some(domain_fit::Fit::Categories(proto::CategoryFit::default())),
                    }),
                },
            )),
        });
    }
    invalid(
        with_widget(fit, vec![]),
        &format!("{slider_path}.domain.data.fit"),
        "fits as an interval",
    );
    // A slider runs over numbers, dates, or timestamps.
    let mut strings = with_widget(
        widget(slider(proto::SliderComparison::AtLeast, None)),
        vec![],
    );
    strings.1 = vec![RecordBatch::try_from_iter([
        (
            "a",
            Arc::new(StringArray::from(vec!["x", "y", "z"])) as ArrayRef,
        ),
        (
            "b",
            Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0])) as ArrayRef,
        ),
    ])
    .unwrap()];
    symbol_mut(&mut strings.0).x = Some(column(
        "a",
        proto::Scale {
            r#type: Some(scale::Type::Band(proto::BandScale::default())),
            domain: Some(proto::Domain {
                domain: Some(domain::Domain::Data(proto::DataDomain::default())),
            }),
            ..Default::default()
        },
    ));
    invalid(
        strings,
        &format!("{slider_path}.domain"),
        "numbers, dates, or timestamps",
    );
}

#[test]
fn a_slider_step_is_positive_and_an_equal_slider_has_one() {
    let step = format!("{WIDGET}.slider.step");
    invalid(
        with_widget(widget(slider(proto::SliderComparison::Equal, None)), vec![]),
        &step,
        "writes the step's bin",
    );
    invalid(
        with_widget(
            widget(slider(proto::SliderComparison::Unspecified, None)),
            vec![],
        ),
        &step,
        "writes the step's bin",
    );
    invalid(
        with_widget(
            widget(slider(proto::SliderComparison::AtLeast, Some(0))),
            vec![one(0.0)],
        ),
        &step,
        "more than 0",
    );
    let duration: ArrayRef = Arc::new(arrow::array::DurationSecondArray::from(vec![60]));
    invalid(
        with_widget(
            widget(slider(proto::SliderComparison::AtLeast, Some(0))),
            vec![duration],
        ),
        &step,
        "a number for a domain of numbers",
    );
}

#[test]
fn a_checkbox_value_has_the_column_type() {
    invalid(
        with_widget(
            widget(proto::widget::Kind::Checkbox(proto::CheckboxWidget {
                value: Some(ValueRef { index: 0 }),
                ..Default::default()
            })),
            vec![Arc::new(StringArray::from(vec!["yes"]))],
        ),
        &format!("{WIDGET}.checkbox.value"),
        "column holds Float64",
    );
}

#[test]
fn a_resolve_rule_names_a_channel_or_a_widget_key() {
    let mut parts = base();
    plot_mut(&mut parts.0).resolve = vec![proto::Resolve {
        key: None,
        sharing: proto::Sharing::Shared as i32,
    }];
    invalid(
        parts.clone(),
        "root.plot.resolve[0]",
        "`key` is required: one of `channel`, `widget`",
    );
    plot_mut(&mut parts.0).resolve[0].key = Some(proto::resolve::Key::Widget(String::new()));
    invalid(
        parts.clone(),
        "root.plot.resolve[0].widget",
        "`widget` is required and not empty",
    );
    plot_mut(&mut parts.0).resolve[0].key =
        Some(proto::resolve::Key::Widget("magnitude".to_string()));
    read(parts).unwrap();
}

/// The tick count of nice ends is more than 0, since `nice: false` turns nice ends off.
#[test]
fn a_nice_count_is_more_than_zero() {
    let mut parts = base();
    *data_fit(x_scale(&mut parts.0)) = Some(proto::DomainFit {
        fit: Some(domain_fit::Fit::Interval(proto::IntervalFit {
            nice_ends: Some(proto::interval_fit::NiceEnds::NiceCount(0.0)),
            ..Default::default()
        })),
    });
    invalid(
        parts,
        &format!("{X}.scale.domain.data.fit.interval.nice_count"),
        "more than 0",
    );
}

/// A fit pins one value, a scalar that the chart's values hold.
#[test]
fn a_pin_of_a_fit_is_a_scalar() {
    let mut parts = base();
    *data_fit(x_scale(&mut parts.0)) = Some(proto::DomainFit {
        fit: Some(domain_fit::Fit::Interval(proto::IntervalFit {
            domain_pin: Some(proto::interval_fit::DomainPin::Mid(ValueRef { index: 0 })),
            ..Default::default()
        })),
    });
    parts.2 = vec![Arc::new(Float64Array::from(vec![0.0]))];
    read(parts.clone()).unwrap();
    parts.2 = vec![floats(vec![0.0, 1.0])];
    invalid(
        parts,
        &format!("{X}.scale.domain.data.fit.interval.mid"),
        "is invalid on a linear scale",
    );
}

/// A band position and a band level place a position's mark in its band, so they need a band or
/// a nested band scale, and a level of the scale's paths.
#[test]
fn a_band_placement_needs_a_band_of_its_position() {
    let mut parts = base();
    symbol_mut(&mut parts.0).x.as_mut().unwrap().band_position = Some(0.5);
    invalid(
        parts.clone(),
        &format!("{X}.band_position"),
        "through a band or a nested band scale",
    );
    x_scale(&mut parts.0).r#type = Some(scale::Type::Band(proto::BandScale::default()));
    x_scale(&mut parts.0).domain = None;
    let mut banded = nested(proto::Axis::default());
    symbol_mut(&mut banded.0).x.as_mut().unwrap().band_position = Some(1.5);
    invalid(banded.clone(), &format!("{X}.band_position"), "");
    let x = symbol_mut(&mut banded.0).x.as_mut().unwrap();
    x.band_position = Some(0.0);
    x.band_level = Some(0);
    read(banded.clone()).unwrap();
    symbol_mut(&mut banded.0).x.as_mut().unwrap().band_level = Some(2);
    invalid(
        banded,
        &format!("{X}.band_level"),
        "the paths have 2 levels",
    );
    let mut color = base();
    symbol_mut(&mut color.0).y.as_mut().unwrap().band_level = Some(0);
    invalid(
        color,
        "root.plot.cartesian.layers[0].symbol.y.band_level",
        "through a nested band scale",
    );
}

#[test]
fn a_title_has_text() {
    let mut parts = base();
    plot_mut(&mut parts.0).title = Some(proto::Title::default());
    invalid(
        parts,
        "root.plot.title.text",
        "`text` is required and not empty",
    );
}

#[test]
fn only_the_title_of_a_node_has_a_subtitle() {
    let mut parts = base();
    x_scale(&mut parts.0).axis = Some(proto::Axis {
        title: Some(proto::Title {
            text: "x".to_string(),
            subtitle: "units".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    invalid(
        parts,
        &format!("{X}.scale.axis.title.subtitle"),
        "only the title of a plot",
    );
}
