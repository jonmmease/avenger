//! The conversions of the format: scale types and options, ranges from computed styles, domains
//! and their fits, selection states, selection domains, predicates, and format syntaxes.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array, RecordBatch},
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{prelude::*, FitKind};
use avenger_scales::scales::domain_summary::{CategoryOrder, DomainOptions, DomainPin};
use avenger_style::{
    Cascade, ComputedStyles, Element, Env, HostInputs, NodeId, ParseOptions, Sheet, SourceId,
    StyleTable, StyleTree,
};

use super::{
    domain::{domain_options, source, DomainSource, FitDefaults},
    format,
    scale::{options, range, range_shape, scale_impl, RangeShape, ScaleRange},
    selection::{domain_values, predicate, states, SelectionValues},
};
use crate::{EvalError, EvalWarning, Registry};

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
    let band = Scale::band(BandScale::builder().align(0.2))
        .build()
        .unwrap();
    let converted = options(&band, style);
    assert_eq!(converted["align"].as_f64().unwrap(), 0.2);
    assert!((converted["padding_inner"].as_f64().unwrap() - 0.3).abs() < 1e-6);
    let log = Scale::log(LogScale::builder().base(2.0)).build().unwrap();
    assert_eq!(options(&log, style)["base"].as_f64().unwrap(), 2.0);

    let custom = Scale::custom("warp").build().unwrap();
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
    let ordinal = Scale::ordinal(OrdinalScale::builder()).build().unwrap();
    assert_eq!(range_shape(&ordinal, 4), RangeShape::Discrete(4));
    let threshold = Scale::threshold().build().unwrap();
    assert_eq!(range_shape(&threshold, 2), RangeShape::Discrete(3));
    let quantile = Scale::quantile().build().unwrap();
    assert_eq!(range_shape(&quantile, 9), RangeShape::Bins);
    let linear = Scale::linear(LinearScale::builder()).build().unwrap();
    assert_eq!(range_shape(&linear, 2), RangeShape::Continuous);
}

/// The table of `a` against `b` of the charts of the domain tests.
fn points() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("a", numbers(&[1.0, 2.0, 3.0])),
        ("b", numbers(&[4.0, 6.0, 5.0])),
    ])
    .unwrap()
}

/// A clause of `owner` with a range of `a` through `x`.
fn range_clause(owner: &str, range: ValueRangeBuilder) -> ClauseBuilder {
    Clause::new([range.channel(Channel::X)]).owner(owner)
}

fn x_group(chart: &Chart) -> avenger_chart::resolve::ScaleGroup {
    let resolved = chart.resolve();
    let index = (resolved.scales.iter())
        .position(|group| group.channel == Channel::X)
        .unwrap();
    resolved.scales.into_iter().nth(index).unwrap()
}

fn f64s(values: &ArrayRef) -> Vec<f64> {
    let values = arrow::compute::cast(values, &DataType::Float64).unwrap();
    values.as_primitive::<Float64Type>().values().to_vec()
}

#[test]
fn explicit_summary_and_data_domains_have_their_sources() {
    let symbols = CartesianLayer::symbol(
        points(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("a")
                .scale(Scale::linear(LinearScale::builder()).domain(Domain::explicit([0.0, 10.0]))))
            .y("b"),
    );
    let explicit = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let group = x_group(&explicit);
    match source(&explicit, &group, &BTreeMap::new()).unwrap() {
        DomainSource::Explicit(values) => assert_eq!(f64s(&values), [0.0, 10.0]),
        other => panic!("an explicit domain, not {other:?}"),
    }

    let symbols = CartesianLayer::symbol(
        points(),
        SymbolMark::builder()
            .x(
                Encoding::scaled_field("a").scale(Scale::linear(LinearScale::builder()).domain(
                    DataDomain::builder().fit(IntervalFit::builder().zero(true).nice_count(5.0)),
                )),
            )
            .y("b"),
    );
    let data = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let group = x_group(&data);
    let DomainSource::Inferred {
        summaries,
        data: of_data,
        ..
    } = source(&data, &group, &BTreeMap::new()).unwrap()
    else {
        panic!("an inferred domain");
    };
    assert!(summaries.is_empty());
    assert_eq!(of_data, [true]);
    let fit = domain_options(
        &data,
        &group.spec,
        FitKind::Interval,
        FitDefaults::default(),
    );
    let DomainOptions::Interval(interval) = fit.options else {
        panic!("an interval fit");
    };
    assert!(interval.zero);
    assert_eq!(interval.nice, Some(5.0));
}

/// `nice: false` turns off the default nice ends, `nice: true` rounds at a count of 10 where no
/// default does, and an absent fit takes the default.
#[test]
fn nice_turns_nice_ends_on_and_off() {
    let nice = |fit: IntervalFitBuilder, defaults: Option<f64>| {
        let symbols = CartesianLayer::symbol(
            points(),
            SymbolMark::builder()
                .x(Encoding::scaled_field("a").scale(
                    Scale::linear(LinearScale::builder()).domain(DataDomain::builder().fit(fit)),
                ))
                .y("b"),
        );
        let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
        let group = x_group(&chart);
        let defaults = FitDefaults {
            zero: false,
            nice: defaults,
        };
        let fit = domain_options(&chart, &group.spec, FitKind::Interval, defaults);
        let DomainOptions::Interval(interval) = fit.options else {
            panic!("an interval fit");
        };
        interval.nice
    };
    assert_eq!(nice(IntervalFit::builder(), Some(10.0)), Some(10.0));
    assert_eq!(nice(IntervalFit::builder().nice(false), Some(10.0)), None);
    assert_eq!(nice(IntervalFit::builder().nice(true), None), Some(10.0));
    assert_eq!(
        nice(IntervalFit::builder().nice_count(4.0), Some(10.0)),
        Some(4.0)
    );
}

#[test]
fn a_preferred_order_moves_the_listed_categories_first() {
    let symbols = CartesianLayer::symbol(
        points(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("a").scale(
                Scale::band(BandScale::builder())
                    .domain(DataDomain::builder().fit(CategoryFit::preferred([3.0, 9.0, 1.0]))),
            ))
            .y("b"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let group = x_group(&chart);
    let fit = domain_options(
        &chart,
        &group.spec,
        FitKind::Categories,
        FitDefaults::default(),
    );
    assert!(matches!(
        fit.options,
        DomainOptions::Categories(ref options) if matches!(options.order, CategoryOrder::Ascending)
    ));
    let ordered = fit.order(numbers(&[1.0, 2.0, 3.0, 4.0])).unwrap();
    // 9 has no data, so it is not in the domain.
    assert_eq!(f64s(&ordered), [3.0, 1.0, 2.0, 4.0]);
}

fn explicit(source: DomainSource) -> Vec<f64> {
    match source {
        DomainSource::Explicit(values) => f64s(&values),
        other => panic!("an explicit domain, not {other:?}"),
    }
}

#[test]
fn selection_domains_take_the_envelope_or_the_intersection_of_ranges() {
    use SelectionResolution::{Global, Intersect, Union};
    // The selection domain of `x`, with an explicit fallback, of a chart whose brush has
    // `initial` clauses.
    let domain =
        |resolution: SelectionResolution, initial: Vec<ClauseBuilder>| {
            let symbols =
                CartesianLayer::symbol(
                    points(),
                    SymbolMark::builder()
                        .x(Encoding::scaled_field("a").scale(
                            Scale::linear(LinearScale::builder()).domain(
                                SelectionDomain::explicit("brush", Channel::X, [-100.0, 100.0]),
                            ),
                        ))
                        .y("b"),
                )
                .id("points")
                .tools([Select::interval("brush", IntervalSelect::builder())]);
            let plot = Plot::cartesian([symbols])
                .id("scatter")
                .tools([Select::interval("brush", IntervalSelect::builder())]);
            let chart = Chart::builder(plot)
                .selections([Selection::new("brush", resolution).initial(initial)])
                .build()
                .unwrap();
            let group = x_group(&chart);
            source(&chart, &group, &states(&chart).unwrap()).unwrap()
        };
    // Without clauses, the fallback.
    assert_eq!(explicit(domain(Global, vec![])), [-100.0, 100.0]);
    // One clause gives its range.
    let scatter = || range_clause("scatter", ValueRange::new("a").low(2.0).high(5.0));
    assert_eq!(explicit(domain(Global, vec![scatter()])), [2.0, 5.0]);
    // A union takes the envelope; an intersection the overlap.
    let two = || {
        vec![
            scatter(),
            range_clause("points", ValueRange::new("a").low(3.0).high(8.0)),
        ]
    };
    assert_eq!(explicit(domain(Union, two())), [2.0, 8.0]);
    assert_eq!(explicit(domain(Intersect, two())), [3.0, 5.0]);
}

/// A range with one end takes the other from an explicit fallback, in the fallback's type, and an
/// end past the fallback's other end closes the domain there.
#[test]
fn a_range_with_one_end_takes_the_other_from_an_explicit_fallback() {
    let domain =
        |range: ValueRangeBuilder| {
            let symbols =
                CartesianLayer::symbol(
                    points(),
                    SymbolMark::builder()
                        .x(Encoding::scaled_field("a").scale(
                            Scale::linear(LinearScale::builder()).domain(
                                SelectionDomain::explicit("brush", Channel::X, [-100.0, 100.0]),
                            ),
                        ))
                        .y("b"),
                )
                .id("points")
                .tools([Select::interval("brush", IntervalSelect::builder())]);
            let plot = Plot::cartesian([symbols])
                .id("scatter")
                .tools([Select::interval("brush", IntervalSelect::builder())]);
            let chart = Chart::builder(plot)
                .selections([Selection::new("brush", SelectionResolution::Global)
                    .initial([range_clause("scatter", range)])])
                .build()
                .unwrap();
            let group = x_group(&chart);
            explicit(source(&chart, &group, &states(&chart).unwrap()).unwrap())
        };
    assert_eq!(domain(ValueRange::new("a").low(2.0)), [2.0, 100.0]);
    assert_eq!(domain(ValueRange::new("a").low(10i64)), [10.0, 100.0]);
    assert_eq!(domain(ValueRange::new("a").low(200.0)), [200.0, 200.0]);
    assert_eq!(domain(ValueRange::new("a").high(-500.0)), [-500.0, -500.0]);
}

/// A range with one end over a data fallback pins that end of the fallback's fit, which finds the
/// other.
#[test]
fn a_range_with_one_end_pins_a_fit_of_the_data() {
    let symbols = CartesianLayer::symbol(
        points(),
        SymbolMark::builder()
            .x(
                Encoding::scaled_field("a").scale(Scale::linear(LinearScale::builder()).domain(
                    SelectionDomain::data("brush", Channel::X, DataDomain::builder()),
                )),
            )
            .y("b"),
    )
    .id("points")
    .tools([Select::interval("brush", IntervalSelect::builder())]);
    let plot = Plot::cartesian([symbols])
        .id("scatter")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    let chart = Chart::builder(plot)
        .selections([Selection::new("brush", SelectionResolution::Global)
            .initial([range_clause("scatter", ValueRange::new("a").high(2.5))])])
        .build()
        .unwrap();
    let group = x_group(&chart);
    let DomainSource::Inferred { pin, data, .. } =
        source(&chart, &group, &states(&chart).unwrap()).unwrap()
    else {
        panic!("a fit of the data");
    };
    assert_eq!(data, [true]);
    match pin {
        Some(DomainPin::High(high)) => assert_eq!(f64s(&high), [2.5]),
        other => panic!("a high pin, not {other:?}"),
    }
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
    let table = RecordBatch::try_from_iter([
        ("a", numbers(&[1.0, 2.0, 3.0, 4.0])),
        ("b", numbers(&[1.0, 1.0, 1.0, 1.0])),
    ])
    .unwrap();
    let magnitude = Widget::range_slider(
        "magnitude",
        RangeSliderWidget::new(WidgetTarget::new("brush", "a"), WidgetDataDomain::builder()),
    );
    let symbols =
        CartesianLayer::symbol(table, SymbolMark::builder().x("a").y("b")).widgets([magnitude]);
    let plot = Plot::cartesian([symbols])
        .id("scatter")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    let widget_clause =
        range_clause("", ValueRange::new("a").low(1.0).high(2.5)).widget("magnitude");
    let brush_clause = range_clause("scatter", ValueRange::new("a").low(2.5).high(4.0));
    let chart = Chart::builder(plot)
        .selections([Selection::new("brush", SelectionResolution::Intersect)
            .initial([widget_clause, brush_clause])])
        .build()
        .unwrap();
    let states = states(&chart).unwrap();
    let owners: Vec<&str> = (states["brush"].clauses.iter())
        .map(|clause| clause.owner.as_str())
        .collect();
    // The canonical form orders the clauses by their owners: the plot, then the widgets of its
    // layers.
    assert_eq!(owners, ["scatter", "\0magnitude\0"]);

    let selected = Predicate::selected(Selected::new("brush").exclude_own(true))
        .build()
        .unwrap();
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
    let table = RecordBatch::try_from_iter([("a", numbers(&[1.0]))]).unwrap();
    let symbols = CartesianLayer::symbol(table, SymbolMark::builder().x("a").y("a"));
    let chart = Chart::builder(Plot::cartesian([symbols]))
        .number_format_syntax("d3")
        .build()
        .unwrap();
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
    use avenger_scales::scales::{domain_summary::Extents, linear::LinearScale as Linear};
    let summary = Linear::configured((0.0, 1.0), (0.0, 1.0))
        .summarize(&numbers(&[-5.0, 50.0]), Extents::None)
        .unwrap();
    let symbols = CartesianLayer::symbol(
        points(),
        SymbolMark::builder()
            .x(
                Encoding::scaled_field("a").scale(Scale::linear(LinearScale::builder()).domain(
                    SummaryDomain::new(Value::scalar(Arc::new(summary.to_arrow()))),
                )),
            )
            .y("b"),
    )
    .id("points")
    .tools([Select::interval("brush", IntervalSelect::builder())]);
    let plot = Plot::cartesian([symbols])
        .id("scatter")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    let chart = Chart::builder(plot)
        .selections([Selection::new("brush", SelectionResolution::Global)])
        .build()
        .unwrap();
    let group = x_group(&chart);
    let DomainSource::Inferred {
        summaries, data, ..
    } = source(&chart, &group, &BTreeMap::new()).unwrap()
    else {
        panic!("an inferred domain");
    };
    assert_eq!(summaries, [summary]);
    assert_eq!(data, [false]);
}

/// A selection domain whose selection's clauses give no range through its channel takes its
/// fallback and warns, and one whose clauses give a range does not warn.
#[test]
fn a_selection_domain_without_its_channel_warns() {
    let warnings = |channel: Option<Channel>| {
        let symbols =
            CartesianLayer::symbol(
                points(),
                SymbolMark::builder()
                    .x(Encoding::scaled_field("a").scale(Domain::selection(
                        SelectionDomain::explicit("brush", Channel::X, [-100.0, 100.0]),
                    )))
                    .y("b"),
            )
            .id("points");
        let plot = Plot::cartesian([symbols])
            .id("scatter")
            .tools([Select::interval("brush", IntervalSelect::builder())]);
        let range = ValueRange::new("a").low(2.0).high(5.0);
        let range = match channel {
            Some(channel) => range.channel(channel),
            None => range,
        };
        let chart = Chart::builder(plot)
            .selections([Selection::new("brush", SelectionResolution::Global)
                .initial([Clause::new([range]).owner("scatter")])])
            .build()
            .unwrap();
        let resolved = chart.resolve();
        super::domain::selection_warnings(&resolved.scales, &states(&chart).unwrap())
    };
    let missed = warnings(None);
    assert_eq!(missed.len(), 1);
    let EvalWarning::Domain { reason, .. } = &missed[0] else {
        panic!("a domain warning, got {missed:?}")
    };
    assert!(
        reason.contains("give no range or set through `x`"),
        "{reason}"
    );
    assert!(warnings(Some(Channel::X)).is_empty());
}
