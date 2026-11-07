//! Conditional values and selection domains from the initial clauses of selections.

use avenger_chart::prelude::*;
use avenger_chart_eval::EvaluatedChart;
use avenger_color::ColorOrGradient;

use crate::support::*;

/// Whether each symbol of the first plot is gray.
fn grays(evaluated: &EvaluatedChart) -> Vec<bool> {
    let symbol = plot_symbols(evaluated);
    (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
        .map(|color| {
            matches!(color, ColorOrGradient::Color(color)
                if (color[0] - color[1]).abs() < 1e-3 && (color[1] - color[2]).abs() < 1e-3)
        })
        .collect()
}

#[test]
fn a_condition_grays_the_rows_outside_a_brush() {
    // The cars, gray outside the brush, in a plot `plot` with a box of the brush.
    let brushed = |initial: Vec<ClauseBuilder>| {
        let color = Encoding::scaled_field("origin").conditional([ConditionalValue::direct_value(
            Predicate::negate(Predicate::selected("brush")),
            "lightgray",
        )]);
        let symbols = CartesianLayer::symbol(
            cars(),
            SymbolMark::builder().x("weight").y("mpg").color(color),
        )
        .id("points")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
        let plot = Plot::cartesian([symbols])
            .id("plot")
            .tools([Select::interval("brush", IntervalSelect::builder())]);
        let brush = Selection::new("brush", SelectionResolution::Union).initial(initial);
        Chart::builder(plot).selections([brush]).build().unwrap()
    };

    let without = brushed(Vec::new());
    let evaluated = render(&without, [320.0, 240.0], "condition-without-brush");
    assert!(grays(&evaluated).iter().all(|gray| !gray));

    let with = brushed(vec![Clause::new([ValueRange::new("weight")
        .channel(Channel::X)
        .low(2500.0)
        .high(3500.0)])
    .owner("plot")]);
    let evaluated = render(&with, [320.0, 240.0], "condition-with-brush");
    let grays = grays(&evaluated);
    assert!(grays.iter().any(|gray| *gray) && grays.iter().any(|gray| !gray));
}

#[test]
fn a_cyclic_range_selects_across_its_wrap() {
    // The gray rows of the cars, gray outside the brush, whose plot `plot` has a clause of a
    // cyclic range of the weights.
    let rows = |start: f64, end: f64, period: f64| {
        let color = Encoding::scaled_field("origin").conditional([ConditionalValue::direct_value(
            Predicate::negate(Predicate::selected("brush")),
            "lightgray",
        )]);
        let symbols = CartesianLayer::symbol(
            cars(),
            SymbolMark::builder().x("weight").y("mpg").color(color),
        )
        .id("points")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
        let plot = Plot::cartesian([symbols])
            .id("plot")
            .tools([Select::interval("brush", IntervalSelect::builder())]);
        let brush = Selection::new("brush", SelectionResolution::Union).initial([Clause::new([
            CyclicRange::new("weight")
                .start(start)
                .end(end)
                .period(period),
        ])
        .owner("plot")]);
        let chart = Chart::builder(plot).selections([brush]).build().unwrap();
        grays(&evaluate(&chart, [320.0, 240.0]))
    };

    // With a period of 3000, a range from 4500 to 2300 holds 4500 to 5300 and 1500 to 2300.
    let wrapped = rows(4500.0, 2300.0, 3000.0);
    for (index, weight) in weights().iter().enumerate() {
        let inside = (4500.0..=5300.0).contains(weight) || (1500.0..=2300.0).contains(weight);
        assert_eq!(wrapped[index], !inside, "{weight}");
    }
    assert!(wrapped.iter().any(|gray| *gray) && wrapped.iter().any(|gray| !gray));
    // The condition has no origin of the wrap, so the same range a period earlier selects alike.
    assert_eq!(rows(1500.0, 2300.0, 3000.0), wrapped);
    // A range a whole period wide holds every row.
    assert!(rows(-180.0, 180.0, 360.0).iter().all(|gray| !gray));
}

#[test]
fn selected_predicates_follow_their_empty_behavior_owners_and_resolutions() {
    // The gray rows of the cars, gray where `predicate` does not select them, in a plot `plot`
    // whose layer `points` also has a box of the brush, so each owns clauses.
    let rows = |resolution: SelectionResolution,
                predicate: SelectedBuilder,
                initial: Vec<ClauseBuilder>| {
        let color = Encoding::scaled_field("origin").conditional([ConditionalValue::direct_value(
            Predicate::negate(Predicate::selected(predicate)),
            "lightgray",
        )]);
        let symbols = CartesianLayer::symbol(
            cars(),
            SymbolMark::builder().x("weight").y("mpg").color(color),
        )
        .id("points")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
        let plot = Plot::cartesian([symbols])
            .id("plot")
            .tools([Select::interval("brush", IntervalSelect::builder())]);
        let brush = Selection::new("brush", resolution).initial(initial);
        let chart = Chart::builder(plot).selections([brush]).build().unwrap();
        grays(&evaluate(&chart, [320.0, 240.0]))
    };
    let predicate = |empty_selects_none: bool, exclude_own: bool| {
        Selected::new("brush")
            .empty_selects_none(empty_selects_none)
            .exclude_own(exclude_own)
    };
    let weights_between = |owner: &str, low: f64, high: f64| {
        Clause::new([ValueRange::new("weight")
            .channel(Channel::X)
            .low(low)
            .high(high)])
        .owner(owner)
    };

    // Without clauses, a predicate selects every row, or none when it says so.
    let gray = rows(
        SelectionResolution::Union,
        predicate(true, false),
        Vec::new(),
    );
    assert!(gray.iter().all(|gray| *gray));
    let gray = rows(
        SelectionResolution::Union,
        predicate(false, false),
        Vec::new(),
    );
    assert!(gray.iter().all(|gray| !gray));

    // A predicate that skips its own plot's clause selects every row.
    let own = vec![weights_between("plot", 2500.0, 3000.0)];
    let gray = rows(SelectionResolution::Union, predicate(false, true), own);
    assert!(gray.iter().all(|gray| !gray));

    // Union selects the rows of either owner, and intersect only the rows of both.
    let both = || {
        vec![
            weights_between("plot", 2000.0, 3000.0),
            weights_between("points", 2800.0, 4000.0),
        ]
    };
    let union = rows(SelectionResolution::Union, predicate(false, false), both());
    let intersect = rows(
        SelectionResolution::Intersect,
        predicate(false, false),
        both(),
    );
    for (index, weight) in weights().iter().enumerate() {
        assert_eq!(
            union[index],
            !(2000.0..=4000.0).contains(weight),
            "{weight}"
        );
        assert_eq!(
            intersect[index],
            !(2800.0..=3000.0).contains(weight),
            "{weight}"
        );
    }
}

#[test]
fn a_test_of_a_missing_column_selects_no_row_and_warns() {
    // A clause of the layer `points` that tests a column `price`, which the table lacks.
    let color = Encoding::scaled_field("origin").conditional([ConditionalValue::direct_value(
        Predicate::negate(Predicate::selected("brush")),
        "lightgray",
    )]);
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color(color),
    )
    .id("points")
    .tools([Select::interval("brush", IntervalSelect::builder())]);
    let plot = Plot::cartesian([symbols])
        .id("plot")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    let brush = Selection::new("brush", SelectionResolution::Union)
        .initial([Clause::new([Condition::set([SetColumn::new("price", [1.0])])]).owner("points")]);
    let chart = Chart::builder(plot).selections([brush]).build().unwrap();

    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(grays(&evaluated).iter().all(|gray| *gray));
    assert!(warns(&evaluated, "`price`"), "{:?}", evaluated.warnings);
}

#[test]
fn a_selection_domain_takes_the_envelope_of_a_brush_and_its_fallback_without_one() {
    // The cars, whose `x` has a selection domain of the brush over a fit of the data, in a plot
    // `plot` whose layer `points` also has a box of the brush.
    let chart = |initial: Vec<ClauseBuilder>| {
        let symbols = CartesianLayer::symbol(
            cars(),
            SymbolMark::builder()
                .x(Encoding::scaled_field("weight").scale(Domain::selection(
                    SelectionDomain::data("brush", Channel::X, DataDomain::builder()),
                )))
                .y("mpg"),
        )
        .id("points")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
        let plot = Plot::cartesian([symbols])
            .id("plot")
            .tools([Select::interval("brush", IntervalSelect::builder())]);
        let brush = Selection::new("brush", SelectionResolution::Union).initial(initial);
        Chart::builder(plot).selections([brush]).build().unwrap()
    };
    let weights_between = |owner: &str, low: f64, high: f64| {
        Clause::new([ValueRange::new("weight")
            .channel(Channel::X)
            .low(low)
            .high(high)])
        .owner(owner)
    };

    let fallback = domain(&evaluate(&chart(Vec::new()), [320.0, 240.0]), 0);
    assert!(fallback.0 <= 1800.0 && fallback.1 >= 4790.0, "{fallback:?}");
    let two = chart(vec![
        weights_between("plot", 2500.0, 3000.0),
        weights_between("points", 2800.0, 3600.0),
    ]);
    assert_eq!(domain(&evaluate(&two, [320.0, 240.0]), 0), (2500.0, 3600.0));
}

/// A gray constant whose selected rows take the scale of a column fits the scale's domain to that
/// column, so the selected rows draw in its colors and the legend lists its categories.
#[test]
fn a_conditional_value_alone_fits_its_scales_domain() {
    let color = Encoding::direct_value("lightgray").conditional([ConditionalValue::scaled_field(
        Predicate::selected("brush"),
        "origin",
    )]);
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color(color),
    );
    let chart = Chart::builder(Plot::cartesian([symbols]))
        .selections([Selection::new("brush", SelectionResolution::Global)])
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    // An empty global selection selects every row.
    assert!(grays(&evaluated).iter().all(|gray| !gray));
    let fills = {
        let symbol = plot_symbols(&evaluated);
        symbol.fill.as_vec(symbol.len as usize, None)
    };
    let mut distinct = fills.clone();
    distinct.dedup();
    assert!(distinct.len() > 1, "{fills:?}");
    let mut legend = Vec::new();
    for color in legend_symbol_fills(&evaluated) {
        if !legend.contains(&color) {
            legend.push(color);
        }
    }
    assert_eq!(legend.len(), 3, "{legend:?}");
}

/// The clauses of a union, one open above and one open below, bound neither end of a selection
/// domain, which takes its fallback for both.
#[test]
fn a_union_open_at_both_ends_takes_the_fallback_domain() {
    let symbols =
        CartesianLayer::symbol(
            cars(),
            SymbolMark::builder()
                .x(Encoding::scaled_field("weight").scale(Domain::selection(
                    SelectionDomain::data("brush", Channel::X, DataDomain::builder()),
                )))
                .y("mpg"),
        )
        .id("points")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    let plot = Plot::cartesian([symbols])
        .id("plot")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    let clause = |owner: &str, range: ValueRangeBuilder| {
        Clause::new([range.channel(Channel::X)]).owner(owner)
    };
    let chart = |initial: Vec<ClauseBuilder>| {
        Chart::builder(plot.clone())
            .selections([Selection::new("brush", SelectionResolution::Union).initial(initial)])
            .build()
            .unwrap()
    };
    let open = chart(vec![
        clause("plot", ValueRange::new("weight").high(3000.0)),
        clause("points", ValueRange::new("weight").low(2500.0)),
    ]);
    let x = |evaluated: &EvaluatedChart| {
        let [x, _] = evaluated.plots[0]
            .coords
            .pixels([&numbers(&[2000.0, 4000.0]), &numbers(&[20.0, 20.0])])
            .unwrap();
        x
    };
    let fallback = evaluate(&chart(Vec::new()), [320.0, 240.0]);
    assert_eq!(x(&evaluate(&open, [320.0, 240.0])), x(&fallback));
}
