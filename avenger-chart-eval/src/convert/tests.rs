//! The conversions of the format: scale types and options, ranges from computed styles, domains
//! and their fits, selection states, selection domains, predicates, and format syntaxes.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array, RecordBatch},
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{
    build::{self, ChartParts},
    list_value,
    proto::{self, cartesian_layer, condition, domain, scale_spec, Channel},
    Chart,
};
use avenger_scales::scales::domain_summary::{CategoryOrder, DomainOptions};
use avenger_style::{
    Cascade, ComputedStyles, Element, Env, HostInputs, NodeId, ParseOptions, Sheet, SourceId,
    StyleTable, StyleTree,
};

use super::{
    domain::{domain_options, source, with_ends, DomainSource},
    format,
    scale::{options, range, range_shape, scale_impl, RangeShape, ScaleRange},
    selection::{domain_values, predicate, states, SelectionValues},
};
use crate::{EvalError, Registry};

/// The computed styles of one `scale` node of `channel` and `kind`, which `css` styles after the
/// base stylesheet.
fn scale_style(css: &str, channel: &str, kind: &str, marks: &str) -> (ComputedStyles, NodeId) {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let text = format!("@import 'std:base.css';\n{css}");
    let (sheet, diagnostics) = Sheet::load(&text, None, SourceId(1), options, &mut no_files);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let cascade = Cascade::new(&StyleTable::new().with_author(sheet, Env::new()));
    let mut tree = StyleTree::new(Element::Chart);
    let plot = tree.add(tree.root(), Element::Plot);
    let node = tree.add(plot, Element::Scale);
    tree.set_attribute(node, "channel", channel.to_string());
    tree.set_attribute(node, "kind", kind.to_string());
    tree.set_attribute_forms(node, "marks", marks, marks);
    (
        cascade.compute(&tree, &HostInputs::sized(400.0, 300.0)),
        node,
    )
}

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn floats(range: &ScaleRange) -> Vec<f64> {
    match range {
        ScaleRange::Values(values) => values.as_primitive::<Float64Type>().values().to_vec(),
        other => panic!("numbers, not {other:?}"),
    }
}

#[test]
fn scale_types_take_their_options_and_custom_scales_their_registry() {
    let (styles, node) = scale_style("scale { padding-inner: 0.3; }", "x", "categorical", "bar");
    let style = &styles[node];
    let band = proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Band(proto::BandScale {
            align: Some(0.2),
            ..Default::default()
        })),
        ..Default::default()
    };
    let converted = options(&band, style);
    assert_eq!(converted["align"].as_f64().unwrap(), 0.2);
    assert!((converted["padding_inner"].as_f64().unwrap() - 0.3).abs() < 1e-6);
    let log = proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Log(proto::LogScale {
            base: Some(2.0),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(options(&log, style)["base"].as_f64().unwrap(), 2.0);

    let custom = proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Custom(proto::CustomScale {
            r#type: "warp".to_string(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(matches!(
        scale_impl(&custom, &Registry::new()),
        Err(EvalError::UnknownScale(name)) if name == "warp"
    ));
    let registry = Registry::new().with_scale(
        "warp",
        Arc::new(avenger_scales::scales::linear::LinearScale),
    );
    assert!(scale_impl(&custom, &registry).is_ok());
}

#[test]
fn ranges_come_from_the_computed_scheme_or_range() {
    // A categorical scheme samples a color for each category.
    let (styles, node) = scale_style("", "color", "categorical", "symbol");
    let Some(ScaleRange::Colors { colors, space }) =
        range(&styles[node], &styles, RangeShape::Discrete(3))
    else {
        panic!("colors");
    };
    assert_eq!((colors.len(), space), (3, None));

    // A continuous scale takes the stops of its palette and its space.
    let (styles, node) = scale_style("", "fill", "sequential", "symbol");
    let Some(ScaleRange::Colors { colors, space }) =
        range(&styles[node], &styles, RangeShape::Continuous)
    else {
        panic!("colors");
    };
    let blues = styles.palette("blues").unwrap();
    assert_eq!(colors, blues.colors);
    assert_eq!(space, Some(blues.space()));

    // The bins of a quantize scale take the scheme's count, reversed.
    let (styles, node) = scale_style(
        "scale { scheme: blues 4 reverse; }",
        "color",
        "sequential",
        "rect",
    );
    let Some(ScaleRange::Colors { colors, .. }) = range(&styles[node], &styles, RangeShape::Bins)
    else {
        panic!("colors");
    };
    assert_eq!(colors.len(), 4);
    assert_eq!(colors[3], blues.interpolate(&[0.0])[0]);

    // The base stylesheet's ranges of sizes, by mark.
    let (styles, node) = scale_style("", "size", "sequential", "symbol");
    let symbols = range(&styles[node], &styles, RangeShape::Continuous).unwrap();
    assert_eq!(floats(&symbols), [9.0, 361.0]);
    let (styles, node) = scale_style("", "size", "sequential", "text");
    let texts = range(&styles[node], &styles, RangeShape::Continuous).unwrap();
    assert_eq!(floats(&texts), [8.0, 40.0]);

    // Shapes are names, and dashes lists of lengths.
    let (styles, node) = scale_style("", "shape", "categorical", "symbol");
    let Some(ScaleRange::Values(shapes)) = range(&styles[node], &styles, RangeShape::Discrete(2))
    else {
        panic!("shapes");
    };
    assert_eq!(shapes.as_string::<i32>().value(0), "circle");
    let (styles, node) = scale_style("", "stroke_dash", "categorical", "line");
    let Some(ScaleRange::Values(dashes)) = range(&styles[node], &styles, RangeShape::Discrete(2))
    else {
        panic!("dashes");
    };
    assert_eq!(dashes.data_type().to_string(), "List(Float64)");

    // A scale without a scheme or a range has no range.
    let (styles, node) = scale_style(
        "scale { scheme: auto; range: auto; }",
        "x",
        "sequential",
        "symbol",
    );
    assert_eq!(range(&styles[node], &styles, RangeShape::Continuous), None);
}

#[test]
fn range_shapes_follow_the_scale_type() {
    let spec = |kind: scale_spec::Type| proto::ScaleSpec {
        r#type: Some(kind),
        ..Default::default()
    };
    assert_eq!(
        range_shape(&spec(scale_spec::Type::Ordinal(Default::default())), 4),
        RangeShape::Discrete(4)
    );
    assert_eq!(
        range_shape(&spec(scale_spec::Type::Threshold(Default::default())), 2),
        RangeShape::Discrete(3)
    );
    assert_eq!(
        range_shape(&spec(scale_spec::Type::Quantile(Default::default())), 9),
        RangeShape::Bins
    );
    assert_eq!(
        range_shape(&spec(scale_spec::Type::Linear(Default::default())), 2),
        RangeShape::Continuous
    );
}

/// A chart of symbols of `a` and `b`, whose `x` scale is `x`, with `selections`.
fn chart_with(
    x: proto::ScaleSpec,
    selections: Vec<proto::Selection>,
    values: Vec<ArrayRef>,
) -> Chart {
    let mut parts = ChartParts::new();
    let table = parts.table(
        RecordBatch::try_from_iter([
            ("a", numbers(&[1.0, 2.0, 3.0])),
            ("b", numbers(&[4.0, 6.0, 5.0])),
        ])
        .unwrap(),
    );
    for value in values {
        parts.value(value);
    }
    let mut x_encoding = build::column("a");
    x_encoding.scale = Some(x);
    let mut root = build::cartesian(vec![proto::CartesianLayer {
        id: "points".to_string(),
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(x_encoding),
            y: Some(build::scaled_column("b")),
            ..Default::default()
        })),
        tools: vec![proto::LayerTool {
            kind: Some(proto::layer_tool::Kind::Select(proto::Select {
                selection: "brush".to_string(),
                form: Some(proto::select::Form::Interval(
                    proto::IntervalSelect::default(),
                )),
                ..Default::default()
            })),
        }],
        ..Default::default()
    }]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.id = "scatter".to_string();
        plot.tools = vec![proto::Tool {
            kind: Some(proto::tool::Kind::Select(proto::Select {
                selection: "brush".to_string(),
                form: Some(proto::select::Form::Interval(
                    proto::IntervalSelect::default(),
                )),
                ..Default::default()
            })),
        }];
    }
    let mut message = build::message(root);
    message.selections = match selections.is_empty() {
        true => vec![brush(proto::SelectionResolution::Global, vec![])],
        false => selections,
    };
    parts.finish(message).unwrap()
}

fn linear(domain: domain::Domain) -> proto::ScaleSpec {
    proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Linear(proto::LinearScale::default())),
        domain: Some(proto::Domain {
            domain: Some(domain),
        }),
        ..Default::default()
    }
}

/// A selection `brush` of `resolution` with `initial` clauses.
fn brush(resolution: proto::SelectionResolution, initial: Vec<proto::Clause>) -> proto::Selection {
    proto::Selection {
        name: "brush".to_string(),
        resolution: resolution as i32,
        initial,
    }
}

/// A clause of `owner` with a range of `a` through `x` from the value `low` to the value `high`.
fn range_clause(owner: &str, low: Option<u32>, high: Option<u32>) -> proto::Clause {
    proto::Clause {
        owner: owner.to_string(),
        conditions: vec![proto::Condition {
            kind: Some(condition::Kind::Range(proto::ValueRange {
                column: "a".to_string(),
                channel: Channel::X as i32,
                low: low.map(|index| proto::ValueRef { index }),
                high: high.map(|index| proto::ValueRef { index }),
                ..Default::default()
            })),
        }],
        ..Default::default()
    }
}

fn x_group(chart: &Chart) -> avenger_chart::resolve::ScaleGroup {
    let resolved = chart.resolve();
    let index = (resolved.scales.iter())
        .position(|group| group.channel == Channel::X)
        .unwrap();
    resolved.scales.into_iter().nth(index).unwrap()
}

fn scalar(value: f64) -> ArrayRef {
    numbers(&[value])
}

fn f64s(values: &ArrayRef) -> Vec<f64> {
    let values = arrow::compute::cast(values, &DataType::Float64).unwrap();
    values.as_primitive::<Float64Type>().values().to_vec()
}

#[test]
fn explicit_summary_and_data_domains_have_their_sources() {
    let explicit = chart_with(
        linear(domain::Domain::Explicit(proto::ValueRef { index: 0 })),
        vec![],
        vec![list_value(numbers(&[0.0, 10.0]))],
    );
    let group = x_group(&explicit);
    match source(&explicit, &group, &BTreeMap::new()).unwrap() {
        DomainSource::Explicit(values) => assert_eq!(f64s(&values), [0.0, 10.0]),
        other => panic!("an explicit domain, not {other:?}"),
    }

    let data = chart_with(
        linear(domain::Domain::Data(proto::DataDomain {
            fit: Some(proto::DomainFit {
                fit: Some(proto::domain_fit::Fit::Interval(proto::IntervalFit {
                    zero: Some(true),
                    nice: Some(5.0),
                    ..Default::default()
                })),
            }),
        })),
        vec![],
        vec![],
    );
    let group = x_group(&data);
    let DomainSource::Inferred {
        summaries,
        data: of_data,
    } = source(&data, &group, &BTreeMap::new()).unwrap()
    else {
        panic!("an inferred domain");
    };
    assert!(summaries.is_empty());
    assert_eq!(of_data, [true]);
    let fit = domain_options(&data, &group.spec);
    let DomainOptions::Interval(interval) = fit.options else {
        panic!("an interval fit");
    };
    assert!(interval.zero);
    assert_eq!(interval.nice, Some(5.0));
}

#[test]
fn a_preferred_order_moves_the_listed_categories_first() {
    let spec = proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Band(proto::BandScale::default())),
        domain: Some(proto::Domain {
            domain: Some(domain::Domain::Data(proto::DataDomain {
                fit: Some(proto::DomainFit {
                    fit: Some(proto::domain_fit::Fit::Categories(proto::CategoryFit {
                        order: Some(proto::category_fit::Order::Preferred(proto::ValueRef {
                            index: 0,
                        })),
                    })),
                }),
            })),
        }),
        ..Default::default()
    };
    let chart = chart_with(spec, vec![], vec![list_value(numbers(&[3.0, 9.0, 1.0]))]);
    let group = x_group(&chart);
    let fit = domain_options(&chart, &group.spec);
    assert!(matches!(
        fit.options,
        DomainOptions::Categories(ref options) if matches!(options.order, CategoryOrder::Ascending)
    ));
    let ordered = fit.order(numbers(&[1.0, 2.0, 3.0, 4.0])).unwrap();
    // 9 has no data, so it is not in the domain.
    assert_eq!(f64s(&ordered), [3.0, 1.0, 2.0, 4.0]);
}

/// The selection domain of `x` of a chart whose brush has `initial` clauses.
fn selection_domain(
    resolution: proto::SelectionResolution,
    initial: Vec<proto::Clause>,
    values: Vec<ArrayRef>,
) -> DomainSource {
    let spec = linear(domain::Domain::Selection(proto::SelectionDomain {
        selection: "brush".to_string(),
        channel: Channel::X as i32,
        fallback: Some(proto::selection_domain::Fallback::Explicit(
            proto::ValueRef { index: 0 },
        )),
    }));
    let mut all = vec![list_value(numbers(&[-100.0, 100.0]))];
    all.extend(values);
    let chart = chart_with(spec, vec![brush(resolution, initial)], all);
    let group = x_group(&chart);
    source(&chart, &group, &states(&chart).unwrap()).unwrap()
}

fn explicit(source: DomainSource) -> Vec<f64> {
    match source {
        DomainSource::Explicit(values) => f64s(&values),
        other => panic!("an explicit domain, not {other:?}"),
    }
}

#[test]
fn selection_domains_take_the_envelope_or_the_intersection_of_ranges() {
    use proto::SelectionResolution::{Global, Intersect, Union};
    // Values: 1 = 2.0, 2 = 5.0, 3 = 3.0, 4 = 8.0.
    let values = || vec![scalar(2.0), scalar(5.0), scalar(3.0), scalar(8.0)];
    // Without clauses, the fallback.
    assert_eq!(
        explicit(selection_domain(Global, vec![], values())),
        [-100.0, 100.0]
    );
    // One clause gives its range.
    assert_eq!(
        explicit(selection_domain(
            Global,
            vec![range_clause("scatter", Some(1), Some(2))],
            values()
        )),
        [2.0, 5.0]
    );
    // A union takes the envelope; an intersection the overlap.
    let two = || {
        vec![
            range_clause("scatter", Some(1), Some(2)),
            range_clause("points", Some(3), Some(4)),
        ]
    };
    let with_other = |resolution| selection_domain(resolution, two(), values());
    assert_eq!(explicit(with_other(Union)), [2.0, 8.0]);
    assert_eq!(explicit(with_other(Intersect)), [3.0, 5.0]);
}

#[test]
fn a_range_with_one_end_takes_the_other_from_the_fallback() {
    use proto::SelectionResolution::Global;
    let source = selection_domain(
        Global,
        vec![range_clause("scatter", Some(1), None)],
        vec![scalar(2.0)],
    );
    let DomainSource::Ends {
        low,
        high,
        fallback,
    } = source
    else {
        panic!("a domain with an open end");
    };
    assert!(high.is_none());
    let fallback = match *fallback {
        DomainSource::Explicit(values) => values,
        other => panic!("an explicit fallback, not {other:?}"),
    };
    let domain = with_ends(low.as_ref(), high.as_ref(), &fallback).unwrap();
    assert_eq!(f64s(&domain), [2.0, 100.0]);
}

#[test]
fn an_intersection_of_ranges_with_open_ends_stays_open_only_where_every_range_is() {
    let state = avenger_selection::SelectionState::new(
        avenger_selection::Resolution::Intersect,
        vec![
            avenger_selection::Clause::new("a").condition(avenger_selection::Condition::Range {
                column: "a".into(),
                channel: Some("x".into()),
                ends: Arc::new(Float64Array::from(vec![Some(2.0), None])),
                exclude_end: false,
                cells: None,
            }),
            avenger_selection::Clause::new("b").condition(avenger_selection::Condition::Range {
                column: "a".into(),
                channel: Some("x".into()),
                ends: Arc::new(Float64Array::from(vec![None, Some(9.0)])),
                exclude_end: false,
                cells: None,
            }),
        ],
    );
    let Some(SelectionValues::Range { low, high }) = domain_values(&state, Channel::X).unwrap()
    else {
        panic!("a range");
    };
    assert_eq!(f64s(&low.unwrap()), [2.0]);
    assert_eq!(f64s(&high.unwrap()), [9.0]);
}

/// A widget's initial clause and a brush of the plot with the widget's anchor id have different
/// owners, so a conditional value that skips the plot's own clause still reads the widget's.
#[test]
fn a_widget_clause_and_a_brush_of_one_plot_keep_apart() {
    let widget = proto::Clause {
        widget: "magnitude".to_string(),
        ..range_clause("", Some(0), Some(1))
    };
    let brush_clause = range_clause("scatter", Some(2), Some(3));
    let mut parts = ChartParts::new();
    let table = parts.table(
        RecordBatch::try_from_iter([
            ("a", numbers(&[1.0, 2.0, 3.0, 4.0])),
            ("b", numbers(&[1.0, 1.0, 1.0, 1.0])),
        ])
        .unwrap(),
    );
    for value in [1.0, 2.5, 2.5, 4.0] {
        parts.value(scalar(value));
    }
    let mut root = build::cartesian(vec![proto::CartesianLayer {
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(build::scaled_column("a")),
            y: Some(build::scaled_column("b")),
            ..Default::default()
        })),
        widgets: vec![proto::Widget {
            key: "magnitude".to_string(),
            kind: Some(proto::widget::Kind::RangeSlider(proto::RangeSliderWidget {
                target: Some(proto::WidgetTarget {
                    selection: "brush".to_string(),
                    column: "a".to_string(),
                }),
                domain: Some(proto::WidgetDomain {
                    domain: Some(proto::widget_domain::Domain::Data(
                        proto::WidgetDataDomain::default(),
                    )),
                }),
                ..Default::default()
            })),
            ..Default::default()
        }],
        ..Default::default()
    }]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.id = "scatter".to_string();
        plot.tools = vec![proto::Tool {
            kind: Some(proto::tool::Kind::Select(proto::Select {
                selection: "brush".to_string(),
                form: Some(proto::select::Form::Interval(
                    proto::IntervalSelect::default(),
                )),
                ..Default::default()
            })),
        }];
    }
    let mut message = build::message(root);
    message.selections = vec![brush(
        proto::SelectionResolution::Intersect,
        vec![widget, brush_clause],
    )];
    let chart = parts.finish(message).unwrap();
    let states = states(&chart).unwrap();
    let owners: Vec<&str> = (states["brush"].clauses.iter())
        .map(|clause| clause.owner.as_str())
        .collect();
    // The canonical form orders the clauses by their owners: the plot, then the widgets of its
    // layers.
    assert_eq!(owners, ["scatter", "\0magnitude\0"]);

    let selected = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selected(proto::Selected {
            selection: "brush".to_string(),
            exclude_own: true,
            ..Default::default()
        })),
    };
    let test = predicate(&selected, &["scatter".to_string()], false);
    let rows = RecordBatch::try_from_iter([("a", numbers(&[1.0, 2.0, 3.0, 4.0]))]).unwrap();
    let mask = test.mask(&states, &rows).unwrap();
    // The brush of the plot is its own, so only the widget's range from 1 to 2.5 applies.
    assert_eq!(
        mask.values().iter().collect::<Vec<_>>(),
        [true, true, false, false]
    );
}

#[test]
fn a_syntax_that_the_engine_lacks_fails() {
    let mut parts = ChartParts::new();
    let table = parts.table(RecordBatch::try_from_iter([("a", numbers(&[1.0]))]).unwrap());
    let root = build::cartesian(vec![proto::CartesianLayer {
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(build::scaled_column("a")),
            y: Some(build::scaled_column("a")),
            ..Default::default()
        })),
        ..Default::default()
    }]);
    let mut message = build::message(root);
    message.number_format_syntax = "d3".to_string();
    let chart = parts.finish(message).unwrap();
    let bare = avenger_typst_label::bundled_label_engine();
    assert!(matches!(
        format::check(&chart, &bare),
        Err(EvalError::MissingFormatter {
            kind: "numbers",
            ..
        })
    ));
    let d3 = bare.with_number_formatting(Arc::new(
        avenger_format_number_d3::D3NumberFormatProvider::new(),
    ));
    format::check(&chart, &d3).unwrap();
}

/// A summary domain holds the producer's summary, and the evaluator does not summarize the data.
#[test]
fn a_summary_domain_holds_the_producers_summary() {
    use avenger_scales::scales::{domain_summary::Extents, linear::LinearScale};
    let summary = LinearScale::configured((0.0, 1.0), (0.0, 1.0))
        .summarize(&numbers(&[-5.0, 50.0]), Extents::None)
        .unwrap();
    let value: ArrayRef = Arc::new(summary.to_arrow());
    let chart = chart_with(
        linear(domain::Domain::Summary(proto::SummaryDomain {
            value: Some(proto::ValueRef { index: 0 }),
            fit: None,
        })),
        vec![],
        vec![value],
    );
    let group = x_group(&chart);
    let DomainSource::Inferred { summaries, data } =
        source(&chart, &group, &BTreeMap::new()).unwrap()
    else {
        panic!("an inferred domain");
    };
    assert_eq!(summaries, [summary]);
    assert_eq!(data, [false]);
}
