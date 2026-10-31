//! Updates, which install a new chart in an evaluator and carry the state of the old chart by
//! their options: the fitted domains of its scales, the views of pans and zooms, and the
//! contributions of its selections.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, RecordBatch, StringArray},
    compute::{filter_record_batch, kernels::cmp::eq},
};
use avenger_chart::{
    proto::{self, Channel, SelectionResolution},
    resolve::Identity,
    Chart, Contribution, Encode, Scale, Selection, TupleTest,
};
use avenger_chart_eval::{
    Carry, ChartEvaluator, EvaluatedChart, Fit, PlotCoord, PreparedChart, ScaleIdentity, Theme,
    UpdateOptions, Views,
};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use avenger_typst_label::{EngineOptions, LabelEngine};

const SIZE: [f32; 2] = [400.0, 300.0];

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// A table of 24 cars: a weight, a fuel economy, and an origin.
fn cars() -> RecordBatch {
    let rows = 24;
    let weight: Vec<f64> = (0..rows)
        .map(|row| 1800.0 + ((row * 37) % 24) as f64 * 130.0)
        .collect();
    let mpg: Vec<f64> = weight
        .iter()
        .enumerate()
        .map(|(row, weight)| 48.0 - weight / 140.0 + ((row * 7) % 5) as f64)
        .collect();
    let origins = ["USA", "Europe", "Japan"];
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("weight", Arc::new(Float64Array::from(weight))),
        ("mpg", Arc::new(Float64Array::from(mpg))),
        (
            "origin",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| origins[row % 3]),
            )),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

/// The cars of `origin`.
fn cars_of(origin: &str) -> RecordBatch {
    let cars = cars();
    let keep = eq(
        cars.column_by_name("origin").unwrap(),
        &StringArray::new_scalar(origin),
    )
    .unwrap();
    filter_record_batch(&cars, &keep).unwrap()
}

/// A scatter plot of `cars` with the id `plot`.
fn scatter(cars: RecordBatch) -> Chart {
    Chart::unit(cars, proto::SymbolMark::default())
        .id("plot")
        .x("weight")
        .y("mpg")
        .build()
        .unwrap()
}

/// A facet row of the cars of each origin in `origins`, whose plots share the id `panel`, with an
/// `x` for each plot in the share family `weight` and the scale type `x_type`, and the union
/// selection `brush` and the global selection `pick`.
fn facets(origins: &[&str], x_type: &str) -> Chart {
    let panels: Vec<Chart> = origins
        .iter()
        .map(|origin| {
            Chart::unit(cars_of(origin), proto::SymbolMark::default())
                .id("panel")
                .partition("origin", *origin)
                .x(Encode::column("weight").scale(Scale::of(x_type).family("weight")))
                .y("mpg")
                .selection(Selection::new("brush", SelectionResolution::Union))
                .selection(Selection::new("pick", SelectionResolution::Global))
                .build()
                .unwrap()
        })
        .collect();
    Chart::row(panels)
        .facet("origin")
        .independent(Channel::X)
        .build()
        .unwrap()
}

/// A bar chart of a count for each of `origins`, in the order of `origins`.
fn bars(origins: &[&str]) -> Chart {
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("origin", Arc::new(StringArray::from(origins.to_vec()))),
        (
            "count",
            Arc::new(Float64Array::from_iter_values(
                (0..origins.len()).map(|index| index as f64 + 1.0),
            )),
        ),
    ];
    Chart::unit(
        RecordBatch::try_from_iter(columns).unwrap(),
        proto::BarMark::default(),
    )
    .id("bars")
    .x("origin")
    .y("count")
    .build()
    .unwrap()
}

fn prepare(chart: &Chart) -> ChartEvaluator {
    ChartEvaluator::new(chart, &engine(), &Theme::default()).unwrap()
}

/// Installs `chart` in `evaluator` by `options`.
fn install(evaluator: &mut ChartEvaluator, chart: &Chart, options: &UpdateOptions) {
    let prepared = PreparedChart::new(chart, &engine(), &Theme::default()).unwrap();
    evaluator.install(prepared, options).unwrap();
}

/// The domain of a continuous scale.
fn interval(scale: &Option<ConfiguredScale>) -> (f64, f64) {
    scale
        .as_ref()
        .expect("a scale")
        .numeric_interval_domain()
        .unwrap()
}

/// The domains of the `x` and `y` scales of the plot at `plot`.
fn domains(evaluated: &EvaluatedChart, plot: usize) -> [(f64, f64); 2] {
    let PlotCoord::Cartesian { x, y, .. } = &evaluated.plots[plot].coord else {
        panic!("a Cartesian plot");
    };
    [interval(x), interval(y)]
}

/// The categories of the `x` scale of the plot at `plot`.
fn categories(evaluated: &EvaluatedChart, plot: usize) -> Vec<String> {
    let PlotCoord::Cartesian { x: Some(x), .. } = &evaluated.plots[plot].coord else {
        panic!("a Cartesian plot with x");
    };
    x.domain()
        .as_string::<i32>()
        .iter()
        .map(|category| category.unwrap().to_string())
        .collect()
}

/// The clip of the group of the plot named `name`.
fn clip<'a>(evaluated: &'a EvaluatedChart, name: &str) -> &'a Clip {
    evaluated
        .scene
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Group(group) if group.name == name => Some(&group.clip),
            _ => None,
        })
        .expect("the group of the plot")
}

/// Moves the domain of the view of `channel` of the plot at `plot` to `domain`.
fn view(evaluator: &mut ChartEvaluator, plot: usize, channel: Channel, domain: (f64, f64)) {
    let view = evaluator.scale_view_mut(plot, channel).expect("a view");
    *view = view.clone().with_domain_interval(domain);
}

/// A contribution of the panel of `origin` to a range of weights.
fn weights_of(origin: &str) -> Contribution {
    Contribution::new("panel")
        .partition("origin", origin)
        .tuple([TupleTest::Range {
            column: "weight".into(),
            channel: Channel::X,
            ends: Arc::new(Float64Array::from(vec![2500.0, 3500.0])),
            exclude_end: false,
        }])
}

#[test]
fn a_prepared_chart_moves_between_threads() {
    fn moves<T: Send>() {}
    moves::<PreparedChart>();
}

/// By default, an update fits the scales to the new rows.
#[test]
fn a_refit_fits_the_scales_to_the_new_rows() {
    let mut evaluator = prepare(&scatter(cars()));
    let old = evaluator.evaluate(SIZE).unwrap();
    install(
        &mut evaluator,
        &scatter(cars_of("USA")),
        &UpdateOptions::new(),
    );
    let fresh = prepare(&scatter(cars_of("USA"))).evaluate(SIZE).unwrap();
    let new = evaluator.evaluate(SIZE).unwrap();
    assert_ne!(domains(&old, 0), domains(&fresh, 0));
    assert_eq!(domains(&new, 0), domains(&fresh, 0));
    assert_eq!(clip(&new, "plot"), &Clip::None);
}

/// `Keep` holds the old fitted domains, and the plot clips the marks outside them.
#[test]
fn keep_holds_the_old_domains_and_clips_the_marks() {
    let mut evaluator = prepare(&scatter(cars_of("USA")));
    let old = evaluator.evaluate(SIZE).unwrap();
    install(
        &mut evaluator,
        &scatter(cars()),
        &UpdateOptions::new().fit(Fit::Keep),
    );
    let new = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(domains(&new, 0), domains(&old, 0));
    assert!(matches!(clip(&new, "plot"), Clip::Rect { .. }));
}

/// `Extend` covers the old domain and the new rows: the interval over both for a continuous
/// scale, and the old categories followed by the new ones for a discrete scale. An override by
/// identity applies to its scale alone.
#[test]
fn extend_covers_the_old_domain_and_the_new_rows() {
    let usa = prepare(&scatter(cars_of("USA"))).evaluate(SIZE).unwrap();
    let japan = prepare(&scatter(cars_of("Japan"))).evaluate(SIZE).unwrap();
    let mut evaluator = prepare(&scatter(cars_of("USA")));
    evaluator.evaluate(SIZE).unwrap();
    let plot = Identity {
        id: "plot".into(),
        partition: BTreeMap::new(),
    };
    let x = ScaleIdentity::Plot {
        plot,
        channel: Channel::X,
        second: false,
    };
    install(
        &mut evaluator,
        &scatter(cars_of("Japan")),
        &UpdateOptions::new().fit_of(x, Fit::Extend),
    );
    let extended = evaluator.evaluate(SIZE).unwrap();
    let [(low, high), y] = domains(&extended, 0);
    let ([usa_x, _], [japan_x, japan_y]) = (domains(&usa, 0), domains(&japan, 0));
    assert!(low <= usa_x.0.min(japan_x.0) && high >= usa_x.1.max(japan_x.1));
    assert_eq!(y, japan_y, "the override leaves y to refit");

    let mut evaluator = prepare(&bars(&["USA", "Europe"]));
    evaluator.evaluate(SIZE).unwrap();
    install(
        &mut evaluator,
        &bars(&["Japan", "USA"]),
        &UpdateOptions::new().fit(Fit::Extend),
    );
    let extended = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(categories(&extended, 0), ["Europe", "USA", "Japan"]);
}

/// The views go to the scales of the new chart with their identities and their scale types, and
/// `Views::Drop` drops them.
#[test]
fn views_go_to_the_scales_with_their_identities_and_types() {
    let zoomed = || {
        let mut evaluator = prepare(&facets(&["USA", "Japan"], "linear"));
        evaluator.evaluate(SIZE).unwrap();
        view(&mut evaluator, 0, Channel::X, (2500.0, 3500.0));
        view(&mut evaluator, 0, Channel::Y, (20.0, 30.0));
        evaluator
    };
    let usa = Identity {
        id: "panel".into(),
        partition: BTreeMap::from([(
            "origin".to_string(),
            Arc::new(StringArray::from(vec!["USA"])) as ArrayRef,
        )]),
    };
    let evaluator = zoomed();
    assert_eq!(
        evaluator.scale_identity(1, Channel::X),
        Some(&ScaleIdentity::Family {
            family: "weight".into(),
            channel: Channel::X,
        })
    );
    assert_eq!(
        evaluator.scale_identity(0, Channel::Y),
        Some(&ScaleIdentity::Plot {
            plot: usa,
            channel: Channel::Y,
            second: false,
        })
    );

    // The facets in another order: the family keeps its view, and the shared `y`, whose first
    // plot is now Japan, does not.
    let mut reordered = zoomed();
    let options = UpdateOptions::new();
    install(
        &mut reordered,
        &facets(&["Europe", "Japan", "USA"], "linear"),
        &options,
    );
    let evaluated = reordered.evaluate(SIZE).unwrap();
    for plot in 0..3 {
        assert_eq!(domains(&evaluated, plot)[0], (2500.0, 3500.0));
        assert_ne!(domains(&evaluated, plot)[1], (20.0, 30.0));
    }
    let mut same = zoomed();
    install(&mut same, &facets(&["USA", "Europe"], "linear"), &options);
    assert_eq!(domains(&same.evaluate(SIZE).unwrap(), 1)[1], (20.0, 30.0));

    // A view does not go to a scale of another type.
    let mut log = zoomed();
    install(&mut log, &facets(&["USA", "Japan"], "log"), &options);
    let evaluated = log.evaluate(SIZE).unwrap();
    assert_ne!(domains(&evaluated, 0)[0], (2500.0, 3500.0));
    assert_eq!(domains(&evaluated, 0)[1], (20.0, 30.0));

    // `Views::Drop` shows the fitted domains.
    let mut dropped = zoomed();
    install(
        &mut dropped,
        &facets(&["USA", "Japan"], "linear"),
        &UpdateOptions::new().views(Views::Drop),
    );
    let fitted = prepare(&facets(&["USA", "Japan"], "linear"))
        .evaluate(SIZE)
        .unwrap();
    assert_eq!(
        domains(&dropped.evaluate(SIZE).unwrap(), 0),
        domains(&fitted, 0)
    );
}

/// `Carry::Keep` drops the contribution of a union selection whose plot the new chart no longer
/// has, and keeps the one contribution of a global selection.
#[test]
fn keep_drops_the_contributions_of_plots_that_are_gone() {
    let mut evaluator = prepare(&facets(&["USA", "Europe", "Japan"], "linear"));
    evaluator
        .set_selections(BTreeMap::from([
            (
                "brush".to_string(),
                vec![weights_of("USA"), weights_of("Europe")],
            ),
            ("pick".to_string(), vec![weights_of("Europe")]),
        ]))
        .unwrap();
    install(
        &mut evaluator,
        &facets(&["USA", "Japan"], "linear"),
        &UpdateOptions::new(),
    );
    let selections = evaluator.selections();
    let owners = |name: &str| -> Vec<String> {
        selections[name]
            .iter()
            .map(|contribution| {
                let origin = &contribution.partition["origin"];
                let origin = origin.0.as_string::<i32>().value(0);
                format!("{}[{origin}]", contribution.owner)
            })
            .collect()
    };
    assert_eq!(owners("brush"), ["panel[USA]"]);
    assert_eq!(owners("pick"), ["panel[Europe]"]);
}

/// `Carry::Reset` takes the initial contributions of the new chart, where `Carry::Keep` keeps
/// the evaluator's.
#[test]
fn reset_takes_the_initial_contributions_of_the_new_chart() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .id("plot")
        .x("weight")
        .y("mpg")
        .selection(Selection::new("pick", SelectionResolution::Global).initial(
            Contribution::new("plot").tuple([TupleTest::Range {
                column: "weight".into(),
                channel: Channel::X,
                ends: Arc::new(Float64Array::from(vec![2500.0, 3500.0])),
                exclude_end: false,
            }]),
        ))
        .build()
        .unwrap();
    let cleared = || {
        let mut evaluator = prepare(&chart);
        evaluator
            .set_selections(BTreeMap::from([("pick".to_string(), Vec::new())]))
            .unwrap();
        evaluator
    };
    let mut kept = cleared();
    install(&mut kept, &chart, &UpdateOptions::new());
    assert!(kept.selections()["pick"].is_empty());
    let mut reset = cleared();
    install(
        &mut reset,
        &chart,
        &UpdateOptions::new().selection("pick", Carry::Reset),
    );
    assert_eq!(reset.selections()["pick"].len(), 1);
}

/// A scale whose domain reads a selection fits at installation from the kept contributions.
#[test]
fn a_selection_domain_fits_from_the_kept_contributions() {
    let detail = |cars: RecordBatch| {
        Chart::unit(cars, proto::SymbolMark::default())
            .id("detail")
            .x(Encode::column("weight").scale(Scale::new().selection("brush", Channel::X)))
            .y("mpg")
            .selection(Selection::new("brush", SelectionResolution::Global))
            .build()
            .unwrap()
    };
    let mut evaluator = prepare(&detail(cars()));
    let window = Contribution::new("detail").tuple([TupleTest::Range {
        column: "weight".into(),
        channel: Channel::X,
        ends: Arc::new(Float64Array::from(vec![2500.0, 3500.0])),
        exclude_end: false,
    }]);
    evaluator
        .set_selections(BTreeMap::from([("brush".to_string(), vec![window])]))
        .unwrap();
    install(
        &mut evaluator,
        &detail(cars_of("USA")),
        &UpdateOptions::new(),
    );
    let evaluated = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(domains(&evaluated, 0)[0], (2500.0, 3500.0));
}
