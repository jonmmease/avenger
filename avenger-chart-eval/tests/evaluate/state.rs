//! The state of an evaluation: views of scales, identities, new clauses of selections, and updates
//! that carry fitted domains, views, clauses, and computed styles to a new chart.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, Float64Array, RecordBatch};
use avenger_chart::{prelude::*, resolve::Identity};
use avenger_chart_eval::{
    Carry, ChartEvaluator, EvaluatedChart, Fit, PreparedChart, Registry, ScaleIdentity,
    UpdateOptions, Views,
};
use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::group::Clip;
use avenger_selection::{
    Clause as SelectionClause, Condition as SelectionCondition, Resolution, SelectionState,
};

use crate::support::*;

/// The cars in a plot `cars`, with a layer `points`, colored gray outside the selection `brush`.
fn cars_chart(table: RecordBatch) -> Chart {
    let color = Encoding::scaled_field("origin").conditional([ConditionalValue::direct_value(
        Predicate::negate(Predicate::selected("brush")),
        "lightgray",
    )]);
    let symbols = CartesianLayer::symbol(
        table,
        SymbolMark::builder().x("weight").y("mpg").color(color),
    )
    .id("points");
    let plot = Plot::cartesian([symbols])
        .id("cars")
        .tools([Select::interval("brush", IntervalSelect::builder())]);
    Chart::builder(plot)
        .selections([Selection::new("brush", SelectionResolution::Union)])
        .build()
        .unwrap()
}

/// The clip of the group of the first plot of an evaluation.
fn clip(evaluated: &EvaluatedChart) -> &Clip {
    &plot_group(evaluated).clip
}

/// A state of the brush with one clause of `cars` over the weights from `low` to `high`.
fn brushed(low: f64, high: f64) -> BTreeMap<String, SelectionState> {
    let mut clause = SelectionClause::new("cars");
    clause.conditions.push(SelectionCondition::Range {
        column: "weight".into(),
        channel: Some("x".into()),
        ends: Arc::new(Float64Array::from(vec![low, high])) as ArrayRef,
        exclude_end: false,
        cells: None,
    });
    BTreeMap::from([(
        "brush".to_string(),
        SelectionState::new(Resolution::Union, vec![clause]),
    )])
}

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
fn a_view_keeps_its_domain_at_every_size_and_shows_on_its_axis() {
    let chart = cars_chart(cars());
    let mut evaluator = evaluator(&chart);
    let fitted = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert_eq!(*clip(&fitted), Clip::None);
    let fitted_texts = scene_texts(&fitted.scene.marks);

    // A zoom into the weights from 2000 to 3000.
    let view = evaluator.scale_view_mut(0, Channel::X).unwrap();
    *view = view.clone().with_domain_interval((2000.0, 3000.0));
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluator.evaluate(&host(size)).unwrap();
        assert_eq!(domain(&evaluated, 0), (2000.0, 3000.0), "{size:?}");
        assert!(matches!(clip(&evaluated), Clip::Rect { .. }));
        assert_image(&evaluated, &format!("state-view-{}x{}", size[0], size[1]));
    }
    let viewed = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert_ne!(scene_texts(&viewed.scene.marks), fitted_texts);

    // Without the view, the scale fits again.
    evaluator.clear_scale_view(0, Channel::X);
    let cleared = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert_eq!(domain(&cleared, 0), domain(&fitted, 0));
    assert_eq!(scene_texts(&cleared.scene.marks), fitted_texts);
}

#[test]
fn a_view_that_does_not_move_drops() {
    let chart = cars_chart(cars());
    let mut evaluator = evaluator(&chart);
    evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    // A press reads the view and moves nothing.
    assert!(evaluator.scale_view_mut(0, Channel::X).is_some());
    evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert!(evaluator.scale_view(0, Channel::X).is_none());
    // A scale of categories has no view.
    let bars = CartesianLayer::bar(
        categories(),
        BarMark::builder()
            .x(Encoding::scaled_field("category").scale(Scale::band(BandScale::builder())))
            .y("amount"),
    );
    let bars = Chart::builder(Plot::cartesian([bars])).build().unwrap();
    let mut evaluator = crate::support::evaluator(&bars);
    evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert!(evaluator.scale_view_mut(0, Channel::X).is_none());
    assert!(evaluator.scale_view_mut(0, Channel::Y).is_some());
}

#[test]
fn scales_take_the_identity_of_their_family_or_their_first_plot() {
    let chart = cars_chart(cars());
    let evaluator = evaluator(&chart);
    assert_eq!(
        evaluator.scale_identity(0, Channel::X),
        Some(&ScaleIdentity::Plot {
            plot: Identity::given("cars", BTreeMap::new()),
            channel: Channel::X,
            index: 0,
        })
    );
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight").scale(Scale::builder().family("weights")))
            .y("mpg"),
    );
    let family = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let evaluator = crate::support::evaluator(&family);
    assert_eq!(
        evaluator.scale_identity(0, Channel::X),
        Some(&ScaleIdentity::Family {
            family: "weights".into(),
            channel: Channel::X,
        })
    );
    // A plot without an id gives its scales the identity of its path.
    assert_eq!(
        evaluator.scale_identity(0, Channel::Y),
        Some(&ScaleIdentity::Plot {
            plot: Identity {
                id: avenger_chart::resolve::Id::Path("root.plot".to_string()),
                partition: BTreeMap::new(),
            },
            channel: Channel::Y,
            index: 0,
        })
    );
}

#[test]
fn new_clauses_restyle_the_conditional_values() {
    let chart = cars_chart(cars());
    let mut evaluator = evaluator(&chart);
    let before = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert!(grays(&before).iter().all(|gray| !gray));
    evaluator.set_selections(brushed(2500.0, 3500.0));
    let after = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    let grays = grays(&after);
    assert!(grays.iter().any(|gray| *gray) && grays.iter().any(|gray| !gray));
    assert_eq!(evaluator.selections()["brush"].clauses.len(), 1);
}

#[test]
fn new_clauses_refit_a_selection_domain() {
    let symbols =
        CartesianLayer::symbol(
            cars(),
            SymbolMark::builder()
                .x(Encoding::scaled_field("weight").scale(Domain::selection(
                    SelectionDomain::data("brush", Channel::X, DataDomain::builder()),
                )))
                .y("mpg"),
        );
    let chart = Chart::builder(Plot::cartesian([symbols]).id("cars"))
        .selections([Selection::new("brush", SelectionResolution::Union)])
        .build()
        .unwrap();
    let mut evaluator = evaluator(&chart);
    let fallback = domain(&evaluator.evaluate(&host([320.0, 240.0])).unwrap(), 0);
    assert!(fallback.0 <= 1800.0, "{fallback:?}");
    evaluator.set_selections(brushed(2500.0, 3000.0));
    let brushed = domain(&evaluator.evaluate(&host([320.0, 240.0])).unwrap(), 0);
    assert_eq!(brushed, (2500.0, 3000.0));
    // The domain reads the selection, so the scale has no view, and its navigation moves the
    // selection.
    assert!(evaluator.scale_view_mut(0, Channel::X).is_none());
    assert_eq!(
        evaluator.domain_selection(0, Channel::X).as_deref(),
        Some("brush")
    );
    assert_eq!(
        evaluator.selection_column(0, Channel::X).as_deref(),
        Some("weight")
    );
}

/// The first `rows` cars, and the cars after them with weights twice as large.
fn more_cars(rows: usize) -> RecordBatch {
    let cars = cars();
    let heavier = arrow::compute::kernels::numeric::mul(
        cars.column_by_name("weight").unwrap(),
        &Float64Array::new_scalar(2.0),
    )
    .unwrap();
    let mut columns = cars.columns().to_vec();
    columns[0] = heavier;
    let doubled = RecordBatch::try_new(cars.schema(), columns).unwrap();
    arrow::compute::concat_batches(&cars.schema(), [&cars.slice(0, rows), &doubled]).unwrap()
}

/// The evaluator of `chart`, evaluated at 320 by 240, with `chart` updated to `next` by `options`
/// and evaluated again.
fn updated(
    chart: &Chart,
    next: &Chart,
    options: &UpdateOptions,
) -> (ChartEvaluator, EvaluatedChart) {
    let mut evaluator = evaluator(chart);
    evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    let prepared = PreparedChart::new(next.clone(), &engine(), &Registry::new()).unwrap();
    evaluator.install(prepared, options).unwrap();
    let evaluated = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    (evaluator, evaluated)
}

#[test]
fn an_update_refits_keeps_or_extends_the_fitted_domains() {
    let old = cars_chart(cars());
    let new = cars_chart(more_cars(4));
    let old_domain = domain(&evaluate(&old, [320.0, 240.0]), 0);
    let new_domain = domain(&evaluate(&new, [320.0, 240.0]), 0);
    assert!(new_domain.1 > old_domain.1 * 1.5);

    let (_, refit) = updated(&old, &new, &UpdateOptions::new());
    assert_eq!(domain(&refit, 0), new_domain);
    assert_eq!(*clip(&refit), Clip::None);

    let (_, kept) = updated(&old, &new, &UpdateOptions::new().fit(Fit::Keep));
    assert_eq!(domain(&kept, 0), old_domain);
    assert!(matches!(clip(&kept), Clip::Rect { .. }));

    // An extended fit takes the union of the old fit and the new rows, here the new rows'.
    let shrunk = cars_chart(cars().slice(0, 6));
    let (_, extended) = updated(&old, &shrunk, &UpdateOptions::new().fit(Fit::Extend));
    assert_eq!(domain(&extended, 0), old_domain);
    let (_, refit) = updated(&old, &shrunk, &UpdateOptions::new());
    assert_ne!(domain(&refit, 0), old_domain);

    // A fit of one identity overrides the fit of the others.
    let identity = ScaleIdentity::Plot {
        plot: Identity::given("cars", BTreeMap::new()),
        channel: Channel::X,
        index: 0,
    };
    let options = UpdateOptions::new()
        .fit(Fit::Keep)
        .fit_of(identity, Fit::Refit);
    let (_, overridden) = updated(&old, &new, &options);
    assert_eq!(domain(&overridden, 0), new_domain);
}

#[test]
fn an_update_keeps_or_drops_the_views() {
    let old = cars_chart(cars());
    let new = cars_chart(more_cars(4));
    let zoomed = |options: &UpdateOptions| {
        let mut evaluator = evaluator(&old);
        evaluator.evaluate(&host([320.0, 240.0])).unwrap();
        let view = evaluator.scale_view_mut(0, Channel::X).unwrap();
        *view = view.clone().with_domain_interval((2000.0, 3000.0));
        evaluator.evaluate(&host([320.0, 240.0])).unwrap();
        let prepared = PreparedChart::new(new.clone(), &engine(), &Registry::new()).unwrap();
        evaluator.install(prepared, options).unwrap();
        domain(&evaluator.evaluate(&host([320.0, 240.0])).unwrap(), 0)
    };
    assert_eq!(zoomed(&UpdateOptions::new()), (2000.0, 3000.0));
    assert_ne!(
        zoomed(&UpdateOptions::new().views(Views::Drop)),
        (2000.0, 3000.0)
    );
}

#[test]
fn an_update_keeps_or_resets_the_clauses() {
    let old = cars_chart(cars());
    let new = cars_chart(more_cars(4));
    let carried = |options: &UpdateOptions| {
        let mut evaluator = evaluator(&old);
        evaluator.set_selections(brushed(2500.0, 3500.0));
        evaluator.evaluate(&host([320.0, 240.0])).unwrap();
        let prepared = PreparedChart::new(new.clone(), &engine(), &Registry::new()).unwrap();
        evaluator.install(prepared, options).unwrap();
        evaluator.selections()["brush"].clauses.len()
    };
    assert_eq!(carried(&UpdateOptions::new()), 1);
    assert_eq!(carried(&UpdateOptions::new().selections(Carry::Reset)), 0);
    assert_eq!(
        carried(&UpdateOptions::new().selection("brush", Carry::Reset)),
        0
    );

    // A clause whose owner the new chart lacks does not carry.
    let mut evaluator = evaluator(&old);
    let mut states = brushed(2500.0, 3500.0);
    states.get_mut("brush").unwrap().clauses[0].owner = "gone".into();
    evaluator.set_selections(states);
    let prepared = PreparedChart::new(new.clone(), &engine(), &Registry::new()).unwrap();
    evaluator.install(prepared, &UpdateOptions::new()).unwrap();
    assert!(evaluator.selections()["brush"].clauses.is_empty());
}

#[test]
fn a_chart_prepares_on_another_thread() {
    fn sendable<T: Send>(_: &T) {}
    let chart = cars_chart(cars());
    let prepared =
        std::thread::spawn(move || PreparedChart::new(chart, &engine(), &Registry::new()).unwrap())
            .join()
            .unwrap();
    sendable(&prepared);
    let mut evaluator = evaluator(&cars_chart(cars()));
    sendable(&evaluator);
    evaluator.install(prepared, &UpdateOptions::new()).unwrap();
    evaluator.evaluate(&host([320.0, 240.0])).unwrap();
}
