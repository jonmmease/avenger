//! The rules of the format that one message decides: each test breaks one rule of a valid chart
//! and checks the path and the reason of the error, and the lenient spellings that a reader
//! accepts.

mod support;

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Date32Array, Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::{
    list_value,
    proto::{
        self, axis, cartesian_layer, condition, encoding, legend, node, plot, predicate,
        projection, scale, tool, CartesianLayer, Concat, Encoding, Plot, ProjectionType, Selection,
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

/// The encoding of `x`.
fn x_encoding(message: &mut proto::Chart) -> &mut proto::Encoding {
    symbol_mut(message).x.as_mut().unwrap()
}

/// The scale of `x`.
fn x_scale(message: &mut proto::Chart) -> &mut proto::Scale {
    x_encoding(message).scale.as_mut().unwrap()
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

    // The reader infers an absent scale type, and an absent domain is the data domain.
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
    concat_mut(&mut parts).horizontal_distribute = 0;
    read(parts.clone()).unwrap();
    concat_mut(&mut parts).horizontal_distribute = proto::Distribute::Center as i32;
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
    x_scale(&mut parts.0).domain = Some(scale::Domain::Explicit(ValueRef { index: 0 }));
    parts.2 = vec![floats(vec![0.0, 5.0, 10.0])];
    invalid(parts.clone(), &format!("{X}.scale.explicit"), "two ends");
    parts.2 = vec![floats(vec![10.0, 0.0])];
    invalid(parts, &format!("{X}.scale.explicit"), "ascending");
}

#[test]
fn a_summary_has_the_layout_of_its_scale() {
    let mut parts = base();
    x_scale(&mut parts.0).domain = Some(scale::Domain::Summary(ValueRef { index: 0 }));
    parts.2 = vec![floats(vec![1.0])];
    invalid(parts, &format!("{X}.scale.summary"), "is a struct");
}

#[test]
fn the_fit_fields_are_of_the_kind_of_its_scale() {
    let mut parts = base();
    x_scale(&mut parts.0).sort = Some(scale::Sort::Direction(
        proto::CategorySort::Descending as i32,
    ));
    invalid(
        parts.clone(),
        &format!("{X}.scale.sort"),
        "which a linear scale does not fit",
    );
    // A scale takes the fit fields of one kind.
    x_scale(&mut parts.0).zero = Some(false);
    invalid(parts.clone(), &format!("{X}.scale.sort"), "of one kind");
    // A band scale fits categories, not an interval.
    x_scale(&mut parts.0).sort = None;
    x_scale(&mut parts.0).r#type = Some(scale::Type::Band(proto::BandScale::default()));
    invalid(
        parts.clone(),
        &format!("{X}.scale.zero"),
        "which a band scale does not fit",
    );
    // A quantile scale takes no fit field.
    x_scale(&mut parts.0).r#type = Some(scale::Type::Quantile(proto::QuantileScale::default()));
    invalid(
        parts,
        &format!("{X}.scale.zero"),
        "which a quantile scale does not fit",
    );
    // A nested band scale sorts its paths, and takes no preferred order.
    let mut banded = nested(proto::Axis::default());
    x_scale(&mut banded.0).sort = Some(scale::Sort::Preferred(ValueRef { index: 0 }));
    banded.2 = vec![floats(vec![1.0])];
    invalid(
        banded,
        &format!("{X}.scale.preferred"),
        "a preferred order is invalid on a nested band scale",
    );
}

/// The fit fields say how the renderer fits a domain, so they are invalid with an explicit
/// domain and with the explicit fallback of a selection domain, which it does not fit.
#[test]
fn the_fit_fields_need_a_fitted_domain() {
    let mut parts = base();
    x_scale(&mut parts.0).domain = Some(scale::Domain::Explicit(ValueRef { index: 0 }));
    x_scale(&mut parts.0).zero = Some(false);
    parts.2 = vec![floats(vec![0.0, 10.0])];
    invalid(
        parts.clone(),
        &format!("{X}.scale.zero"),
        "invalid with an explicit domain",
    );
    // The data fallback of a selection domain fits.
    parts.0.selections = vec![Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        ..Default::default()
    }];
    x_scale(&mut parts.0).domain = Some(scale::Domain::Selection(proto::SelectionDomain {
        selection: "brush".to_string(),
        channel: proto::Channel::X as i32,
        fallback: None,
    }));
    read(parts.clone()).unwrap();
    // Its explicit fallback does not.
    x_scale(&mut parts.0).domain = Some(scale::Domain::Selection(proto::SelectionDomain {
        selection: "brush".to_string(),
        channel: proto::Channel::X as i32,
        fallback: Some(proto::selection_domain::Fallback::Explicit(ValueRef {
            index: 0,
        })),
    }));
    invalid(
        parts,
        &format!("{X}.scale.zero"),
        "invalid with an explicit domain or an explicit fallback",
    );
}

/// A threshold scale takes only an explicit domain, so an unset domain and an unset fallback,
/// which are the data domain, are invalid on it.
#[test]
fn a_threshold_scale_has_no_data_domain() {
    let mut parts = base();
    x_scale(&mut parts.0).r#type = Some(scale::Type::Threshold(proto::ThresholdScale::default()));
    invalid(
        parts.clone(),
        &format!("{X}.scale.domain"),
        "a data domain is invalid on a threshold scale",
    );
    parts.0.selections = vec![Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        ..Default::default()
    }];
    x_scale(&mut parts.0).domain = Some(scale::Domain::Selection(proto::SelectionDomain {
        selection: "brush".to_string(),
        channel: proto::Channel::X as i32,
        fallback: None,
    }));
    invalid(
        parts,
        &format!("{X}.scale.selection.fallback"),
        "a data domain is invalid on a threshold scale",
    );
}

#[test]
fn the_plot_gives_the_range_of_a_position() {
    let mut parts = base();
    x_scale(&mut parts.0).range = Some(scale::Range::Palette("tableau10".to_string()));
    invalid(
        parts.clone(),
        &format!("{X}.scale.range"),
        "takes only `reverse`",
    );
    x_scale(&mut parts.0).range = None;
    x_scale(&mut parts.0).reverse = true;
    read(parts).unwrap();
}

#[test]
fn a_guide_is_of_a_kind_that_its_channel_takes() {
    // A legend on a position.
    let mut parts = base();
    x_encoding(&mut parts.0).legend = Some(proto::Legend::default());
    invalid(
        parts,
        &format!("{X}.legend"),
        "a legend is valid only on a channel other than a position",
    );
    // An axis on a channel other than a position.
    let mut parts = base();
    symbol_mut(&mut parts.0).color = Some(Encoding {
        axis: Some(proto::Axis::default()),
        ..column("a", linear_data())
    });
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.color.axis",
        "an axis is valid only on a position with a scale of its own",
    );
    // An axis on the second channel of a pair, which follows the scale of the first.
    let rule = |x2: Encoding| proto::CartesianLayer {
        table: Some(proto::TableRef { index: 0 }),
        mark: Some(cartesian_layer::Mark::Rule(proto::RuleMark {
            x: Some(column("a", linear_data())),
            x2: Some(x2),
            y: Some(column("b", linear_data())),
            ..Default::default()
        })),
        ..Default::default()
    };
    let x2 = || Encoding {
        kind: Some(encoding::Kind::ScaledField("b".to_string())),
        ..Default::default()
    };
    read((chart(plot(vec![rule(x2())])), vec![points()], vec![])).unwrap();
    invalid(
        (
            chart(plot(vec![rule(Encoding {
                axis: Some(proto::Axis::default()),
                ..x2()
            })])),
            vec![points()],
            vec![],
        ),
        "root.plot.cartesian.layers[0].rule.x2.axis",
        "an axis is valid only on a position with a scale of its own",
    );
    // An axis or a legend on an encoding without a scale.
    let mut parts = base();
    *x_encoding(&mut parts.0) = Encoding {
        kind: Some(encoding::Kind::DirectValue(ValueRef { index: 0 })),
        axis: Some(proto::Axis::default()),
        ..Default::default()
    };
    parts.2 = vec![Arc::new(Float64Array::from(vec![0.5])) as ArrayRef];
    invalid(
        parts,
        &format!("{X}.axis"),
        "an encoding without a scale has no axis",
    );
    let mut parts = base();
    symbol_mut(&mut parts.0).fill = Some(Encoding {
        kind: Some(encoding::Kind::DirectValue(ValueRef { index: 0 })),
        legend: Some(proto::Legend::default()),
        ..Default::default()
    });
    parts.2 = vec![Arc::new(StringArray::from(vec!["red"])) as ArrayRef];
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.fill.legend",
        "an encoding without a scale has no legend",
    );
}

#[test]
fn a_polar_text_turns_by_its_orient_or_by_its_angle() {
    let text = |orient: proto::PolarTextOrient, angle: Option<Encoding>| {
        let layer = proto::PolarLayer {
            table: Some(proto::TableRef { index: 0 }),
            mark: Some(proto::polar_layer::Mark::Text(proto::PolarTextMark {
                theta: Some(column("a", linear_data())),
                radius: Some(column("b", linear_data())),
                text: Some(Encoding {
                    kind: Some(encoding::Kind::DirectField("a".to_string())),
                    ..Default::default()
                }),
                orient: orient as i32,
                angle,
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
    let angle = || {
        Some(Encoding {
            kind: Some(encoding::Kind::DirectField("b".to_string())),
            ..Default::default()
        })
    };
    read(text(proto::PolarTextOrient::Radial, None)).unwrap();
    read(text(proto::PolarTextOrient::Unspecified, angle())).unwrap();
    invalid(
        text(proto::PolarTextOrient::Tangential, angle()),
        "root.plot.polar.layers[0].text.angle",
        "by its `orient` or by its `angle`, not both",
    );
}

#[test]
fn a_pattern_needs_its_syntax() {
    let mut parts = base();
    x_encoding(&mut parts.0).axis = Some(proto::Axis {
        labels: Some(proto::Labels {
            format: ",.0f".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    invalid(
        parts.clone(),
        &format!("{X}.axis.labels.format"),
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
    parts.0.padding_left = Some(-2.0);
    invalid(parts, "padding_left", "at least 0");

    let mut parts = base();
    let plot = plot_mut(&mut parts.0);
    plot.width = Some(300.0);
    plot.height = Some(200.0);
    if let Some(plot::Coord::Cartesian(cartesian)) = plot.coord.as_mut() {
        cartesian.options = Some(proto::Cartesian { aspect: Some(1.5) });
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

/// A geo plot of a symbol layer at the places of the columns `a` and `b`, with the options `geo`.
fn geo_plot(geo: Option<proto::Geo>) -> proto::Node {
    proto::Node {
        node: Some(node::Node::Plot(Plot {
            coord: Some(plot::Coord::Geo(proto::GeoPlot {
                options: geo,
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
    }
}

/// The options of a geo plot of the built-in projection `builtin`, changed by `change`.
fn geo_options(builtin: ProjectionType, change: impl FnOnce(&mut proto::Geo)) -> proto::Geo {
    let mut geo = proto::Geo {
        projection: Some(proto::Projection {
            r#type: Some(projection::Type::Builtin(builtin as i32)),
            ..Default::default()
        }),
        ..Default::default()
    };
    change(&mut geo);
    geo
}

/// The parts of a valid chart whose root is a geo plot with the options `geo`.
fn geo_parts(geo: proto::Geo) -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
    let mut parts = base();
    parts.0.root = Some(geo_plot(Some(geo)));
    parts
}

#[test]
fn geo_plots_set_a_projection_within_range() {
    let mut parts = base();
    parts.0.root = Some(geo_plot(Some(proto::Geo::default())));
    invalid(
        parts.clone(),
        "root.plot.geo.options.projection",
        "`projection` is required",
    );
    parts.0.root = Some(geo_plot(Some(proto::Geo {
        projection: Some(proto::Projection::default()),
        ..Default::default()
    })));
    invalid(
        parts.clone(),
        "root.plot.geo.options.projection",
        "`type` is required",
    );
    invalid(
        geo_parts(geo_options(ProjectionType::Unspecified, |_| {})),
        "root.plot.geo.options.projection.builtin",
        "is required",
    );
    invalid(
        geo_parts(geo_options(ProjectionType::ConicConformal, |geo| {
            geo.projection.as_mut().unwrap().parallels = vec![100.0, 30.0];
        })),
        "root.plot.geo.options.projection.parallels",
        "from -90 to 90",
    );
    // A map's projection has no default, so a geo plot always sets its options.
    parts.0.root = Some(geo_plot(None));
    invalid(parts, "root.plot.geo.options", "`options` is required");
}

#[test]
fn a_projection_takes_only_the_parameters_of_its_type() {
    let with = |builtin, change: fn(&mut proto::Projection)| {
        geo_parts(geo_options(builtin, |geo| {
            change(geo.projection.as_mut().unwrap())
        }))
    };
    read(with(ProjectionType::Hammer, |p| p.coefficient = Some(2.0))).unwrap();
    invalid(
        with(ProjectionType::Mercator, |p| p.coefficient = Some(2.0)),
        "root.plot.geo.options.projection.coefficient",
        "`projection.coefficient` applies only to `hammer`, not to `mercator`",
    );
    read(with(ProjectionType::ConicEqualArea, |p| {
        p.parallels = vec![20.0]
    }))
    .unwrap();
    invalid(
        with(ProjectionType::Robinson, |p| p.parallels = vec![20.0, 50.0]),
        "root.plot.geo.options.projection.parallels",
        "applies only to `conic_conformal`, `conic_equal_area`, `conic_equidistant`",
    );
    invalid(
        with(ProjectionType::Mercator, |p| p.parallel = Some(30.0)),
        "root.plot.geo.options.projection.parallel",
        "applies only to `cylindrical_equal_area`",
    );
    read(with(ProjectionType::Identity, |p| {
        p.reflect_x = true;
        p.reflect_y = true;
    }))
    .unwrap();
    invalid(
        with(ProjectionType::Mercator, |p| p.reflect_x = true),
        "root.plot.geo.options.projection.reflect_x",
        "applies only to `identity`",
    );
    for (change, parameter) in [
        (
            (|p: &mut proto::Projection| p.rotate = vec![10.0, 0.0]) as fn(&mut _),
            "rotate",
        ),
        (|p| p.center = vec![10.0, 0.0], "center"),
        (|p| p.precision = Some(1.0), "precision"),
        (
            |p| p.fit = Some(projection::Fit::WholeSphere(proto::WholeSphere {})),
            "whole_sphere",
        ),
    ] {
        invalid(
            with(ProjectionType::Identity, change),
            &format!("root.plot.geo.options.projection.{parameter}"),
            "applies only to a projection of the sphere, not to `identity`",
        );
    }
    // A projection of the sphere draws a graticule, a sphere, and tiles, and `identity` does not.
    let graticule = |builtin| {
        geo_parts(geo_options(builtin, |geo| {
            geo.graticule = Some(proto::Graticule::default());
        }))
    };
    read(graticule(ProjectionType::Mercator)).unwrap();
    // A hidden graticule is no graticule.
    read(geo_parts(geo_options(ProjectionType::Identity, |geo| {
        geo.graticule = Some(proto::Graticule {
            show: Some(false),
            ..Default::default()
        });
    })))
    .unwrap();
    invalid(
        graticule(ProjectionType::Identity),
        "root.plot.geo.options.graticule",
        "not to `identity`",
    );
    invalid(
        geo_parts(geo_options(ProjectionType::Identity, |geo| {
            geo.navigation = Some(proto::GeoNavigation {
                pan_mode: proto::PanMode::Sphere as i32,
                ..Default::default()
            });
        })),
        "root.plot.geo.options.navigation.pan_mode",
        "not to `identity`",
    );
    // A custom projection takes the parameters of the sphere, and its options.
    let mut custom = geo_options(ProjectionType::Mercator, |geo| {
        let projection = geo.projection.as_mut().unwrap();
        projection.rotate = vec![10.0, 20.0, 30.0];
    });
    custom.projection.as_mut().unwrap().r#type =
        Some(projection::Type::Custom(proto::CustomProjection {
            name: "healpix".to_string(),
            options: [(
                "lobes".to_string(),
                proto::OptionValue {
                    value: Some(proto::option_value::Value::Integer(4)),
                },
            )]
            .into(),
        }));
    read(geo_parts(custom.clone())).unwrap();
    custom.projection.as_mut().unwrap().coefficient = Some(2.0);
    invalid(
        geo_parts(custom),
        "root.plot.geo.options.projection.coefficient",
        "not to a custom projection",
    );
}

#[test]
fn the_arrays_of_a_projection_have_their_lengths() {
    let with = |builtin, change: fn(&mut proto::Geo)| geo_parts(geo_options(builtin, change));
    read(with(ProjectionType::Orthographic, |geo| {
        geo.projection.as_mut().unwrap().rotate = vec![96.0, 0.0];
    }))
    .unwrap();
    read(with(ProjectionType::Orthographic, |geo| {
        geo.projection.as_mut().unwrap().rotate = vec![96.0, 0.0, 10.0];
    }))
    .unwrap();
    invalid(
        with(ProjectionType::Orthographic, |geo| {
            geo.projection.as_mut().unwrap().rotate = vec![96.0, 0.0, 0.0, 1.0];
        }),
        "root.plot.geo.options.projection.rotate",
        "a rotation takes two or three angles: the yaw, the pitch, and an optional roll, not 4",
    );
    invalid(
        with(ProjectionType::ConicConformal, |geo| {
            geo.projection.as_mut().unwrap().parallels = vec![20.0, 30.0, 40.0];
        }),
        "root.plot.geo.options.projection.parallels",
        "one or two standard parallels, not 3",
    );
    invalid(
        with(ProjectionType::Mercator, |geo| {
            geo.projection.as_mut().unwrap().center = vec![10.0];
        }),
        "root.plot.geo.options.projection.center",
        "its longitude and its latitude, not 1",
    );
    invalid(
        with(ProjectionType::Mercator, |geo| {
            geo.projection.as_mut().unwrap().center = vec![10.0, 100.0];
        }),
        "root.plot.geo.options.projection.center",
        "a latitude from -90 to 90",
    );
    invalid(
        with(ProjectionType::Mercator, |geo| {
            geo.projection.as_mut().unwrap().fit = Some(projection::Fit::Box(proto::FitBox {
                bounds: vec![-10.0, 0.0, 10.0],
            }));
        }),
        "root.plot.geo.options.projection.box.bounds",
        "four numbers",
    );
    invalid(
        with(ProjectionType::Mercator, |geo| {
            geo.projection.as_mut().unwrap().fit = Some(projection::Fit::Box(proto::FitBox {
                bounds: vec![-10.0, 50.0, 10.0, 20.0],
            }));
        }),
        "root.plot.geo.options.projection.box.bounds",
        "the south edge of a box is at most its north edge",
    );
    invalid(
        with(ProjectionType::Mercator, |geo| {
            geo.graticule = Some(proto::Graticule {
                step: vec![10.0],
                ..Default::default()
            });
        }),
        "root.plot.geo.options.graticule.step",
        "two numbers of degrees, not 1",
    );
    invalid(
        with(ProjectionType::Mercator, |geo| {
            geo.graticule = Some(proto::Graticule {
                extent_major: vec![-180.0, -90.0, 180.0],
                ..Default::default()
            });
        }),
        "root.plot.geo.options.graticule.extent_major",
        "four numbers",
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
    // A navigation needs no id: the plot's path identifies it.
    let mut parts = base();
    plot_mut(&mut parts.0).tools = vec![navigate()];
    read(parts.clone()).unwrap();
    plot_mut(&mut parts.0).tools = vec![navigate(), navigate()];
    invalid(parts, "root.plot.tools[1]", "at most one navigation");

    // A selection's clauses name their owner by id, so a plot that declares one has an id.
    let mut parts = base();
    parts.0.selections = vec![Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        ..Default::default()
    }];
    plot_mut(&mut parts.0).tools = vec![proto::Tool {
        kind: Some(tool::Kind::Select(proto::Select {
            selection: "brush".to_string(),
            ..Default::default()
        })),
    }];
    invalid(
        parts,
        "root.plot",
        "a plot that declares a selection has an id, which its clauses name",
    );

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

/// A plot whose one tool is `tooltip`.
fn with_tooltip(tooltip: proto::Tooltip) -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
    let mut parts = base();
    let plot = plot_mut(&mut parts.0);
    plot.id = "scatter".to_string();
    plot.tools = vec![proto::Tool {
        kind: Some(tool::Kind::Tooltip(tooltip)),
    }];
    parts
}

/// The title and the fields of a tooltip read their values with a pattern and markup, each over
/// a column of the plot.
#[test]
fn a_tooltip_title_and_its_fields_read_their_columns() {
    read(with_tooltip(proto::Tooltip {
        title: Some(proto::TooltipTitle {
            column: "a".to_string(),
            markup: "#value km".to_string(),
            ..Default::default()
        }),
        fields: vec![proto::TooltipField {
            column: "a".to_string(),
            title: Some("A".to_string()),
            markup: "#value km".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }))
    .unwrap();
    invalid(
        with_tooltip(proto::Tooltip {
            title: Some(proto::TooltipTitle {
                column: "c".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        "root.plot.tools[0].tooltip.title.column",
        "no table of the plot and no partition has a column `c`",
    );
}

/// A nearest tooltip takes a radius and a metric, and a tooltip without `nearest` takes neither.
#[test]
fn a_tooltip_takes_a_radius_and_a_metric_only_with_nearest() {
    read(with_tooltip(proto::Tooltip {
        nearest: true,
        max_radius: Some(10.0),
        metric: proto::NearestMetric::AlongX as i32,
        ..Default::default()
    }))
    .unwrap();
    read(with_tooltip(proto::Tooltip {
        nearest: true,
        ..Default::default()
    }))
    .unwrap();
    invalid(
        with_tooltip(proto::Tooltip {
            max_radius: Some(0.0),
            ..Default::default()
        }),
        "root.plot.tools[0].tooltip.max_radius",
        "a tooltip without `nearest`",
    );
    invalid(
        with_tooltip(proto::Tooltip {
            metric: proto::NearestMetric::AlongY as i32,
            ..Default::default()
        }),
        "root.plot.tools[0].tooltip.metric",
        "a tooltip without `nearest`",
    );
    invalid(
        with_tooltip(proto::Tooltip {
            nearest: true,
            max_radius: Some(-1.0),
            ..Default::default()
        }),
        "root.plot.tools[0].tooltip.max_radius",
        "at least 0",
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
    x_scale(&mut parts.0).navigation = Some(proto::NavigationLimits::default());
    read(parts.clone()).unwrap();
    x_scale(&mut parts.0).r#type = Some(scale::Type::Band(proto::BandScale::default()));
    invalid(
        parts,
        &format!("{X}.scale.navigation"),
        "valid only on a continuous scale of `x` or `y`",
    );
    let mut parts = base();
    let mut color = column("b", linear_data());
    color.scale.as_mut().unwrap().navigation = Some(proto::NavigationLimits::default());
    symbol_mut(&mut parts.0).color = Some(color);
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.color.scale.navigation",
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
    x_scale(&mut parts.0).r#type = Some(scale::Type::NestedBand(proto::NestedBandScale::default()));
    x_encoding(&mut parts.0).axis = Some(guide);
    parts
}

#[test]
fn the_levels_of_an_axis_belong_to_a_nested_band_scale() {
    let level = |show| proto::NestedAxisLevel {
        labels: Some(proto::Labels {
            show,
            ..Default::default()
        }),
        ..Default::default()
    };
    let axis = |levels| proto::Axis {
        levels,
        ..Default::default()
    };
    read(nested(axis(vec![level(Some(false)), level(None)]))).unwrap();
    invalid(
        nested(axis(vec![level(None), level(None), level(None)])),
        &format!("{X}.axis.levels"),
        "the axis has 3 levels, and the paths of its scale have 2",
    );
    // A level's pattern of numbers needs its syntax, and the values of these levels are strings.
    let formatted = proto::NestedAxisLevel {
        labels: Some(proto::Labels {
            format: ",.0f".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    read(nested(axis(vec![formatted]))).unwrap();
    // Every level takes the markup and the font of the axis's labels.
    let styled = proto::NestedAxisLevel {
        labels: Some(proto::Labels {
            font_size: Some(9.0),
            ..Default::default()
        }),
        ..Default::default()
    };
    invalid(
        nested(axis(vec![styled])),
        &format!("{X}.axis.levels[0].labels.font_size"),
        "take the markup and the font of the axis's labels",
    );
    let mut parts = base();
    x_encoding(&mut parts.0).axis = Some(axis(vec![level(None)]));
    invalid(
        parts,
        &format!("{X}.axis.levels"),
        "valid only on the axis of a nested band scale",
    );
}

#[test]
fn the_ticks_of_a_category_axis_stand_where_its_scale_allows() {
    let ticks = |placement| proto::Axis {
        tick_placement: Some(placement),
        ..Default::default()
    };
    invalid(
        nested(ticks(axis::TickPlacement::TickCount(3.0))),
        &format!("{X}.axis.tick_count"),
        "ticks at the bands of its leaf",
    );
    let band = |placement| {
        let mut parts = base();
        parts.1 = vec![table(&[
            ("a", vec![1.0, 2.0, 3.0]),
            ("b", vec![4.0, 6.0, 5.0]),
        ])];
        x_scale(&mut parts.0).r#type = Some(scale::Type::Band(proto::BandScale::default()));
        x_encoding(&mut parts.0).axis = Some(ticks(placement));
        parts
    };
    invalid(
        band(axis::TickPlacement::TickCount(3.0)),
        &format!("{X}.axis.tick_count"),
        "only a list of ticks is valid",
    );
    let mut listed = band(axis::TickPlacement::Values(ValueRef { index: 0 }));
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
    x_encoding(&mut parts.0).axis = Some(proto::Axis {
        classes: strings(&["a\tb"]),
        ..Default::default()
    });
    invalid(
        parts,
        "root.plot.cartesian.layers[0].symbol.x.axis.classes[0]",
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
    let (selection, column) = target_of(&mut widget);
    *selection = "filters".to_string();
    *column = "a".to_string();
    widget
}

/// The selection and the column of a widget's kind, to change.
fn target_of(widget: &mut proto::Widget) -> (&mut String, &mut String) {
    use proto::widget::Kind;
    match widget.kind.as_mut().expect("a widget has a kind") {
        Kind::Slider(slider) => (&mut slider.selection, &mut slider.column),
        Kind::RangeSlider(slider) => (&mut slider.selection, &mut slider.column),
        Kind::Radio(radio) => (&mut radio.selection, &mut radio.column),
        Kind::Checkboxes(checkboxes) => (&mut checkboxes.selection, &mut checkboxes.column),
        Kind::Checkbox(checkbox) => (&mut checkbox.selection, &mut checkbox.column),
        Kind::Mode(_) => panic!("a mode widget has no selection"),
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
                            sort: Some(proto::widget_summary_domain::Sort::Direction(
                                proto::CategorySort::Ascending as i32,
                            )),
                            ..Default::default()
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
fn a_widget_names_its_selection_column_and_kind() {
    let valid = || widget(slider(proto::SliderComparison::AtLeast, None));
    let mut unknown = valid();
    *target_of(&mut unknown).0 = "brush".to_string();
    invalid(
        with_widget(unknown, vec![]),
        &format!("{WIDGET}.slider.selection"),
        "no selection `brush`",
    );
    let mut absent = valid();
    *target_of(&mut absent).1 = "c".to_string();
    invalid(
        with_widget(absent, vec![]),
        &format!("{WIDGET}.slider.column"),
        "no column `c`",
    );
    let mut unselected = valid();
    target_of(&mut unselected).0.clear();
    invalid(
        with_widget(unselected, vec![]),
        &format!("{WIDGET}.slider.selection"),
        "`selection` is required",
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
fn a_widget_without_a_key_takes_the_name_of_its_selection() {
    let mut keyless = widget(slider(proto::SliderComparison::AtLeast, None));
    keyless.key.clear();
    let chart = read(with_widget(keyless, vec![])).unwrap();
    let mut message = chart.message().clone();
    assert_eq!(layer_mut(&mut message).widgets[0].key, "filters");
}

#[test]
fn two_widgets_of_one_selection_without_keys_in_one_layer_fail_at_the_default_key() {
    let keyless = || {
        let mut keyless = widget(slider(proto::SliderComparison::AtLeast, None));
        keyless.key.clear();
        keyless
    };
    let mut parts = with_widget(keyless(), vec![]);
    layer_mut(&mut parts.0).widgets.push(keyless());
    let error = read(parts).unwrap_err().to_string();
    assert!(
        error.contains("root.plot.cartesian.layers[0].widgets[1].key")
            && error.contains("declares the key `filters` at most once")
            && error
                .contains("`filters` is the default key of a widget of the selection `filters`")
            && error.contains("give each widget an explicit `key`"),
        "{error}"
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
                    sort: Some(proto::widget_data_domain::Sort::Direction(
                        proto::CategorySort::Ascending as i32,
                    )),
                    ..Default::default()
                },
            )),
        });
    }
    invalid(
        with_widget(fit, vec![]),
        &format!("{slider_path}.domain.data.sort"),
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

/// A plot's layers share one projection, so its resolve sets none, and the keys of independent
/// widgets are not empty and appear once.
#[test]
fn a_resolve_names_each_independent_widget_once() {
    let mut parts = base();
    plot_mut(&mut parts.0).resolve = Some(proto::Resolve {
        projection: proto::Sharing::Shared as i32,
        ..Default::default()
    });
    invalid(
        parts.clone(),
        "root.plot.resolve.projection",
        "sets no `projection`",
    );
    let resolve = |keys: &[&str]| {
        Some(proto::Resolve {
            independent_widgets: keys.iter().map(|key| key.to_string()).collect(),
            ..Default::default()
        })
    };
    plot_mut(&mut parts.0).resolve = resolve(&[""]);
    invalid(
        parts.clone(),
        "root.plot.resolve.independent_widgets[0]",
        "not empty",
    );
    plot_mut(&mut parts.0).resolve = resolve(&["magnitude", "magnitude"]);
    invalid(
        parts.clone(),
        "root.plot.resolve.independent_widgets[1]",
        "appears once",
    );
    plot_mut(&mut parts.0).resolve = resolve(&["magnitude", "depth"]);
    read(parts).unwrap();
}

/// A track's share is more than 0, and a track has a maximum only with a share, in the tracks of
/// a concat and in the row height of a wrap.
#[test]
fn a_track_has_a_maximum_only_with_a_share() {
    use proto::track::Size;
    let track = |size: Option<Size>, max: Option<f32>| proto::Track { size, max };
    let concat = |track: proto::Track| {
        let mut parts = base();
        parts.0.root = Some(proto::Node {
            node: Some(node::Node::Concat(Concat {
                direction: proto::Direction::Row as i32,
                children: vec![plot(vec![symbols(0)])],
                tracks: vec![track],
                ..Default::default()
            })),
        });
        parts
    };
    read(concat(track(Some(Size::Share(2.0)), Some(600.0)))).unwrap();
    read(concat(track(Some(Size::Length(200.0)), None))).unwrap();
    invalid(
        concat(track(Some(Size::Length(200.0)), Some(600.0))),
        "root.concat.tracks[0].max",
        "a maximum only with a share",
    );
    invalid(
        concat(track(None, Some(600.0))),
        "root.concat.tracks[0].max",
        "a maximum only with a share",
    );
    invalid(
        concat(track(Some(Size::Share(0.0)), None)),
        "root.concat.tracks[0].share",
        "more than 0",
    );
    invalid(
        concat(track(Some(Size::Share(1.0)), Some(0.0))),
        "root.concat.tracks[0].max",
        "more than 0",
    );
    invalid(
        concat(track(Some(Size::Length(-1.0)), None)),
        "root.concat.tracks[0].length",
        "at least 0",
    );
    let wrap = |track: proto::Track| {
        let mut parts = base();
        parts.0.root = Some(proto::Node {
            node: Some(node::Node::Wrap(proto::Wrap {
                children: vec![plot(vec![symbols(0)])],
                row_height: Some(track),
                ..Default::default()
            })),
        });
        parts
    };
    read(wrap(track(Some(Size::Share(1.0)), Some(300.0)))).unwrap();
    invalid(
        wrap(track(Some(Size::Length(200.0)), Some(300.0))),
        "root.wrap.row_height.max",
        "a maximum only with a share",
    );
    invalid(
        wrap(track(Some(Size::Share(-1.0)), None)),
        "root.wrap.row_height.share",
        "more than 0",
    );
}

/// The tick count of nice ends is more than 0, since `nice: false` turns nice ends off.
#[test]
fn a_nice_tick_count_is_more_than_zero() {
    let mut parts = base();
    x_scale(&mut parts.0).nice = Some(scale::Nice::TickCount(0.0));
    invalid(parts, &format!("{X}.scale.tick_count"), "more than 0");
}

/// A fit pins one value, a scalar that the chart's values hold.
#[test]
fn a_pin_of_a_fit_is_a_scalar() {
    let mut parts = base();
    x_scale(&mut parts.0).pin = Some(proto::DomainPin {
        value: Some(ValueRef { index: 0 }),
        position: proto::PinPosition::Mid as i32,
    });
    parts.2 = vec![Arc::new(Float64Array::from(vec![0.0]))];
    read(parts.clone()).unwrap();
    parts.2 = vec![floats(vec![0.0, 1.0])];
    invalid(
        parts,
        &format!("{X}.scale.pin.value"),
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
fn the_title_of_a_node_has_text_and_always_shows() {
    let mut parts = base();
    plot_mut(&mut parts.0).title = Some(proto::Title::default());
    invalid(
        parts,
        "root.plot.title.text",
        "the title of a plot, a concat, or a wrap has a text",
    );
    let mut parts = base();
    plot_mut(&mut parts.0).title = Some(proto::Title {
        text: "Penguins".to_string(),
        show: Some(false),
        ..Default::default()
    });
    invalid(parts, "root.plot.title.show", "always shows");
    // The title of a guide styles the title that its guide gives without a text of its own, and
    // hides it.
    let mut parts = base();
    x_encoding(&mut parts.0).axis = Some(proto::Axis {
        title: Some(proto::Title {
            font_size: Some(14.0),
            show: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    });
    read(parts).unwrap();
}

#[test]
fn a_subtitle_font_is_only_on_the_title_of_a_node() {
    let mut parts = base();
    x_encoding(&mut parts.0).axis = Some(proto::Axis {
        title: Some(proto::Title {
            subtitle_color: "gray".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    invalid(
        parts,
        &format!("{X}.axis.title.subtitle_color"),
        "only the title of a plot",
    );
}

/// The base chart with a color encoding of `b` through a linear scale with `legend`.
fn with_legend(legend: proto::Legend) -> (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>) {
    let mut parts = base();
    symbol_mut(&mut parts.0).color = Some(proto::Encoding {
        legend: Some(legend),
        ..column("b", linear_data())
    });
    parts
}

const COLOR: &str = "root.plot.cartesian.layers[0].symbol.color.legend";

#[test]
fn a_legend_takes_the_fields_of_its_type() {
    let symbol = |legend: proto::Legend| proto::Legend {
        r#type: proto::LegendForm::Symbol as i32,
        ..legend
    };
    // A count of columns is a grid, which a flow is not.
    read(with_legend(symbol(proto::Legend {
        columns: Some(2),
        ..Default::default()
    })))
    .unwrap();
    invalid(
        with_legend(symbol(proto::Legend {
            columns: Some(2),
            wrap: proto::LegendWrap::Flow as i32,
            ..Default::default()
        })),
        &format!("{COLOR}.columns"),
        "invalid with a flow",
    );
    // A colorbar draws no grid of entries, and a symbol legend no axis.
    invalid(
        with_legend(proto::Legend {
            columns: Some(2),
            ..Default::default()
        }),
        &format!("{COLOR}.columns"),
        "a colorbar has no such part",
    );
    invalid(
        with_legend(symbol(proto::Legend {
            rule: Some(false),
            ..Default::default()
        })),
        &format!("{COLOR}.rule"),
        "a symbol legend has no such part",
    );
    // A tick count and values place the entries of a symbol legend and the ticks of a colorbar,
    // and a step places only the ticks of a colorbar.
    let count = Some(legend::TickPlacement::TickCount(4.0));
    read(with_legend(symbol(proto::Legend {
        tick_placement: count,
        ..Default::default()
    })))
    .unwrap();
    read(with_legend(proto::Legend {
        tick_placement: count,
        ..Default::default()
    }))
    .unwrap();
    let step = || {
        Some(legend::TickPlacement::TickStep(proto::TickStep {
            step: 1.0,
            ..Default::default()
        }))
    };
    read(with_legend(proto::Legend {
        tick_placement: step(),
        ..Default::default()
    }))
    .unwrap();
    invalid(
        with_legend(symbol(proto::Legend {
            tick_placement: step(),
            ..Default::default()
        })),
        &format!("{COLOR}.tick_step"),
        "a symbol legend has no such part",
    );
}

#[test]
fn a_tick_step_fits_its_scale() {
    let stepped = |step: proto::TickStep, values: Vec<ArrayRef>| {
        let mut parts = base();
        x_encoding(&mut parts.0).axis = Some(proto::Axis {
            tick_placement: Some(axis::TickPlacement::TickStep(step)),
            ..Default::default()
        });
        parts.2 = values;
        parts
    };
    let path = format!("{X}.axis.tick_step");
    read(stepped(
        proto::TickStep {
            step: 0.5,
            start: Some(ValueRef { index: 0 }),
            ..Default::default()
        },
        vec![one(0.25)],
    ))
    .unwrap();
    invalid(
        stepped(proto::TickStep::default(), vec![]),
        &format!("{path}.step"),
        "more than 0",
    );
    invalid(
        stepped(
            proto::TickStep {
                step: 3.0,
                interval: proto::CalendarUnit::Month as i32,
                ..Default::default()
            },
            vec![],
        ),
        &format!("{path}.interval"),
        "valid only on a time scale",
    );
    invalid(
        stepped(
            proto::TickStep {
                step: 1.0,
                start: Some(ValueRef { index: 0 }),
                ..Default::default()
            },
            vec![Arc::new(StringArray::from(vec!["a"]))],
        ),
        &format!("{path}.start"),
        "a start of type Utf8 is invalid on a linear scale",
    );
    // On a time scale of dates, the step has a unit and is a whole number of it, from a date.
    let timed = |step: proto::TickStep| {
        let mut parts = stepped(step, vec![Arc::new(Date32Array::from(vec![19723]))]);
        let days: ArrayRef = Arc::new(Date32Array::from(vec![19723, 19754, 19783]));
        let schema = Schema::new(vec![
            Field::new("a", DataType::Date32, false),
            Field::new("b", DataType::Float64, false),
        ]);
        let b: ArrayRef = Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0]));
        parts.1 = vec![RecordBatch::try_new(Arc::new(schema), vec![days, b]).unwrap()];
        x_scale(&mut parts.0).r#type = Some(scale::Type::Time(proto::TimeScale::default()));
        parts
    };
    let months = |step: f64| proto::TickStep {
        step,
        interval: proto::CalendarUnit::Month as i32,
        start: Some(ValueRef { index: 0 }),
    };
    read(timed(months(3.0))).unwrap();
    invalid(
        timed(proto::TickStep {
            step: 3.0,
            ..Default::default()
        }),
        &format!("{path}.interval"),
        "the step of the ticks of a time scale has a unit",
    );
    invalid(
        timed(months(1.5)),
        &format!("{path}.step"),
        "a whole number of its unit",
    );
}

#[test]
fn facets_and_widgets_always_show_their_labels() {
    let hidden = Some(proto::Labels {
        show: Some(false),
        ..Default::default()
    });
    let mut root = concat(
        proto::Direction::Row,
        vec![plot_with("", vec![value("sex", 0)], vec![symbols(0)])],
    );
    concat_mut(&mut root).facet = Some(proto::Facet {
        column: "sex".to_string(),
        labels: hidden.clone(),
        ..Default::default()
    });
    invalid(
        parts(root, strings(&["female"])),
        "root.concat.facet.labels.show",
        "a facet always shows its labels",
    );
    invalid(
        with_widget(
            widget(proto::widget::Kind::Radio(proto::RadioWidget {
                domain: data_domain(),
                labels: hidden,
                ..Default::default()
            })),
            vec![],
        ),
        &format!("{WIDGET}.radio.labels.show"),
        "a widget always shows its labels",
    );
}

#[test]
fn only_the_title_of_a_node_has_a_subtitle() {
    let mut parts = base();
    x_encoding(&mut parts.0).axis = Some(proto::Axis {
        title: Some(proto::Title {
            text: "x".to_string(),
            subtitle: "units".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    invalid(
        parts,
        &format!("{X}.axis.title.subtitle"),
        "only the title of a plot",
    );
}
