//! Views of scales, which replace the fitted domains of continuous scales of `x` and `y`, as a pan
//! or a zoom does, and the identities that carry them to a new chart.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray},
    compute::{filter_record_batch, kernels::cmp::eq},
};
use avenger_chart::{
    proto::{self, Channel},
    resolve::Identity,
    Chart, Encode, Scale, Selection,
};
use avenger_chart_eval::{
    ChartEvaluator, EvaluatedChart, PlotCoord, ScaleIdentity, ScaleNavigation, Theme,
};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use avenger_svg::SvgRenderer;
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

/// A scatter plot of the cars with the id `plot`.
fn scatter() -> Chart {
    Chart::unit(cars(), proto::SymbolMark::default())
        .id("plot")
        .x("weight")
        .y("mpg")
        .build()
        .unwrap()
}

/// A facet row of the cars of each origin in `origins`, whose plots share the id `panel`, with an
/// `x` for each plot in the share family `weight`.
fn origins(origins: &[&str]) -> Chart {
    facets(origins, Scale::new().family("weight"))
}

/// A facet row of the cars of each origin in `origins`, whose plots share the id `panel`, with an
/// `x` for each plot of the spec `x`.
fn facets(origins: &[&str], x: Scale) -> Chart {
    let panels: Vec<Chart> = origins
        .iter()
        .map(|origin| {
            Chart::unit(cars_of(origin), proto::SymbolMark::default())
                .id("panel")
                .partition("origin", *origin)
                .x(Encode::column("weight").scale(x.clone()))
                .y("mpg")
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

fn prepare(chart: &Chart) -> ChartEvaluator {
    ChartEvaluator::new(chart, &engine(), &Theme::default()).unwrap()
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

/// Moves the domain of the view of `channel` of the plot at `plot` to `domain`.
fn view(evaluator: &mut ChartEvaluator, plot: usize, channel: Channel, domain: (f64, f64)) {
    let view = evaluator.scale_view_mut(plot, channel).expect("a view");
    *view = view.clone().with_domain_interval(domain);
}

/// The group of the plot named `name` in a scene.
fn plot_group<'a>(
    evaluated: &'a EvaluatedChart,
    name: &str,
) -> &'a avenger_scenegraph::marks::group::SceneGroup {
    evaluated
        .scene
        .marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Group(group) if group.name == name => Some(group),
            _ => None,
        })
        .expect("the group of the plot")
}

/// The text of every text mark of a scene, its groups included.
fn texts(marks: &[SceneMark]) -> Vec<String> {
    let mut texts = Vec::new();
    for mark in marks {
        match mark {
            SceneMark::Group(group) => texts.extend(self::texts(&group.marks)),
            SceneMark::Text(text) => texts.extend(text.text.as_vec(text.len as usize, None)),
            _ => {}
        }
    }
    texts
}

#[test]
fn a_view_renders_its_domain_and_clips_its_plot() {
    let engine = engine();
    let mut evaluator = prepare(&scatter());
    let fitted = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(plot_group(&fitted, "plot").clip, Clip::None);
    assert!(texts(&fitted.scene.marks).contains(&"4,000".to_string()));

    // The symbol at 3,490 stands on the right edge, and the clip halves it.
    view(&mut evaluator, 0, Channel::X, (2500.0, 3490.0));
    view(&mut evaluator, 0, Channel::Y, (20.0, 30.0));
    let evaluated = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(domains(&evaluated, 0), [(2500.0, 3490.0), (20.0, 30.0)]);
    assert!(matches!(
        plot_group(&evaluated, "plot").clip,
        Clip::Rect { .. }
    ));
    let labels = texts(&evaluated.scene.marks);
    assert!(labels.contains(&"3,000".to_string()), "{labels:?}");
    assert!(!labels.contains(&"4,000".to_string()), "{labels:?}");

    let svg = SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine)
        .unwrap();
    assert_baseline(
        env!("CARGO_MANIFEST_DIR"),
        &svg_to_png(&svg, 2.0),
        "view-zoomed-400x300",
    );
}

#[test]
fn a_view_keeps_its_domain_at_another_size() {
    let mut evaluator = prepare(&scatter());
    evaluator.evaluate(SIZE).unwrap();
    view(&mut evaluator, 0, Channel::X, (2500.0, 3500.0));
    let evaluated = evaluator.evaluate([600.0, 300.0]).unwrap();
    assert_eq!(domains(&evaluated, 0)[0], (2500.0, 3500.0));
    // The view takes the length of the plot, so a pan moves the domain by the pixels of the
    // new length.
    let width = f64::from(evaluated.plots[0].rect[2]);
    let view = evaluator.scale_view(0, Channel::X).unwrap();
    assert_eq!(view.numeric_interval_range().unwrap(), (0.0, width));
    assert!(evaluator.scale_view(0, Channel::Y).is_none());
}

#[test]
fn a_zoom_that_widens_the_labels_of_y_widens_their_slab() {
    let mut evaluator = prepare(&scatter());
    let before = evaluator.evaluate(SIZE).unwrap().plots[0].rect;
    view(&mut evaluator, 0, Channel::Y, (20.1234, 20.1238));
    let zoomed = evaluator.evaluate(SIZE).unwrap();
    let after = zoomed.plots[0].rect;
    assert!(
        after[0] > before[0] + 10.0,
        "the plot starts at {} after the zoom and at {} before it",
        after[0],
        before[0]
    );
    assert_eq!(after[2] + after[0], before[2] + before[0]);

    evaluator.clear_scale_view(0, Channel::Y);
    assert_eq!(evaluator.evaluate(SIZE).unwrap().plots[0].rect, before);
}

#[test]
fn the_navigation_of_a_scale_resolves_its_data_bounds_at_the_last_size() {
    let navigated = |navigation: proto::ScaleNavigation| {
        Chart::unit(cars(), proto::SymbolMark::default())
            .id("plot")
            .x(Encode::column("weight").scale(Scale::new().navigation(navigation)))
            .y("mpg")
            .build()
            .unwrap()
    };
    let mut evaluator = prepare(&navigated(proto::ScaleNavigation {
        bounds: Some(proto::scale_navigation::Bounds::Data(proto::DataBounds {})),
        min_span: Some(250.0),
    }));
    // Before an evaluation, the plot has no size, so the data bounds are unknown.
    assert_eq!(
        evaluator.scale_navigation(0, Channel::X),
        Some(ScaleNavigation {
            bounds: None,
            min_span: Some(250.0),
        })
    );
    let fitted = domains(&evaluator.evaluate(SIZE).unwrap(), 0)[0];
    assert_eq!(
        evaluator.scale_navigation(0, Channel::X),
        Some(ScaleNavigation {
            bounds: Some([fitted.0, fitted.1]),
            min_span: Some(250.0),
        })
    );
    // A scale without a navigation moves freely, and a channel without a scale has none.
    assert_eq!(
        evaluator.scale_navigation(0, Channel::Y),
        Some(ScaleNavigation::default())
    );
    assert_eq!(evaluator.scale_navigation(0, Channel::Color), None);

    let mut evaluator = prepare(&navigated(proto::ScaleNavigation {
        bounds: Some(proto::scale_navigation::Bounds::Interval(proto::Interval {
            low: 1000.0,
            high: 6000.0,
        })),
        min_span: None,
    }));
    evaluator.evaluate(SIZE).unwrap();
    assert_eq!(
        evaluator.scale_navigation(0, Channel::X).unwrap().bounds,
        Some([1000.0, 6000.0])
    );
}

#[test]
fn home_scales_are_the_fitted_scales_at_the_last_size() {
    let mut evaluator = prepare(&scatter());
    let fitted = evaluator.evaluate(SIZE).unwrap();
    // Pans that keep the labels, and so the size of the plot.
    let [x, y] = domains(&fitted, 0);
    view(&mut evaluator, 0, Channel::X, (x.0 + 50.0, x.1 + 50.0));
    view(&mut evaluator, 0, Channel::Y, (y.0 + 1.0, y.1 + 1.0));
    let panned = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(panned.plots[0].rect, fitted.plots[0].rect);

    let [x, y] = [Channel::X, Channel::Y].map(|channel| evaluator.home_scale(0, channel));
    assert_eq!([interval(&x), interval(&y)], domains(&fitted, 0));
    let rect = fitted.plots[0].rect;
    assert_eq!(
        x.unwrap().numeric_interval_range().unwrap(),
        (0.0, f64::from(rect[2]))
    );
    assert_eq!(
        y.unwrap().numeric_interval_range().unwrap(),
        (f64::from(rect[3]), 0.0)
    );
}

#[test]
fn a_view_of_a_shared_scale_moves_every_plot_of_the_scale() {
    // The facet shares `y`, and each plot has its own `x`.
    let mut evaluator = prepare(&facets(&["USA", "Japan"], Scale::new()));
    let fitted = evaluator.evaluate(SIZE).unwrap();
    view(&mut evaluator, 1, Channel::Y, (20.0, 30.0));
    view(&mut evaluator, 1, Channel::X, (2500.0, 3500.0));
    let evaluated = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(domains(&evaluated, 0)[1], (20.0, 30.0));
    assert_eq!(domains(&evaluated, 1)[1], (20.0, 30.0));
    assert_eq!(domains(&evaluated, 0)[0], domains(&fitted, 0)[0]);
    assert_eq!(domains(&evaluated, 1)[0], (2500.0, 3500.0));

    // A share family joins the scales of `x`.
    let mut evaluator = prepare(&origins(&["USA", "Japan"]));
    evaluator.evaluate(SIZE).unwrap();
    view(&mut evaluator, 1, Channel::X, (2500.0, 3500.0));
    let evaluated = evaluator.evaluate(SIZE).unwrap();
    assert_eq!(domains(&evaluated, 0)[0], (2500.0, 3500.0));
}

#[test]
fn only_continuous_scales_without_selection_domains_have_views() {
    let chart = Chart::unit(cars(), proto::SymbolMark::default())
        .id("plot")
        .x(Encode::column("weight").scale(Scale::new().selection("brush", Channel::X)))
        .y("mpg")
        .selection(Selection::new("brush", proto::SelectionResolution::Union))
        .build()
        .unwrap();
    let mut evaluator = prepare(&chart);
    assert!(
        evaluator.scale_view_mut(0, Channel::Y).is_none(),
        "a view needs the size of the plot"
    );
    evaluator.evaluate(SIZE).unwrap();
    assert!(evaluator.scale_view_mut(0, Channel::X).is_none());
    assert!(evaluator.scale_view_mut(0, Channel::Y).is_some());

    let bars = Chart::unit(cars(), proto::BarMark::default())
        .x("origin")
        .y("mpg")
        .build()
        .unwrap();
    let mut evaluator = prepare(&bars);
    evaluator.evaluate(SIZE).unwrap();
    assert!(evaluator.scale_view_mut(0, Channel::X).is_none());
    assert!(evaluator.scale_view_mut(0, Channel::Color).is_none());
    assert!(evaluator.scale_view_mut(0, Channel::Y).is_some());
}

#[test]
fn views_go_to_the_scales_of_a_new_chart_with_their_identities() {
    let mut evaluator = prepare(&origins(&["USA", "Japan"]));
    evaluator.evaluate(SIZE).unwrap();
    view(&mut evaluator, 0, Channel::X, (2500.0, 3500.0));
    view(&mut evaluator, 0, Channel::Y, (20.0, 30.0));
    let views = evaluator.views();
    assert_eq!(
        evaluator.scale_identity(1, Channel::X),
        Some(&ScaleIdentity::Family {
            family: "weight".into(),
            channel: Channel::X,
        })
    );
    let usa = Identity {
        id: "panel".into(),
        partition: BTreeMap::from([(
            "origin".to_string(),
            Arc::new(StringArray::from(vec!["USA"])) as ArrayRef,
        )]),
    };
    let identities: Vec<&ScaleIdentity> = views.iter().map(|(identity, _)| identity).collect();
    assert_eq!(
        identities,
        [
            &ScaleIdentity::Family {
                family: "weight".into(),
                channel: Channel::X,
            },
            &ScaleIdentity::Plot {
                plot: usa,
                channel: Channel::Y,
                second: false,
            },
        ]
    );

    // A new chart with the facets in another order: the family keeps its view, and the shared
    // `y`, whose first plot is now Japan, does not.
    let mut reordered = prepare(&origins(&["Europe", "Japan", "USA"]));
    reordered.set_views(&views);
    let evaluated = reordered.evaluate(SIZE).unwrap();
    for plot in 0..3 {
        assert_eq!(domains(&evaluated, plot)[0], (2500.0, 3500.0));
        assert_ne!(domains(&evaluated, plot)[1], (20.0, 30.0));
    }
    let mut same = prepare(&origins(&["USA", "Europe"]));
    same.set_views(&views);
    assert_eq!(domains(&same.evaluate(SIZE).unwrap(), 1)[1], (20.0, 30.0));

    // A view does not go to a scale of another type.
    let log = Chart::unit(cars_of("USA"), proto::SymbolMark::default())
        .id("panel")
        .partition("origin", "USA")
        .x(Encode::column("weight").scale(Scale::of("log").family("weight")))
        .y("mpg")
        .build()
        .unwrap();
    let mut log = prepare(&log);
    log.set_views(&views);
    let evaluated = log.evaluate(SIZE).unwrap();
    assert_ne!(domains(&evaluated, 0)[0], (2500.0, 3500.0));
    assert_eq!(domains(&evaluated, 0)[1], (20.0, 30.0));
}

#[test]
fn plots_that_share_an_id_have_their_own_names_and_rows() {
    let mut evaluator = prepare(&origins(&["USA", "Japan"]));
    let evaluated = evaluator.evaluate(SIZE).unwrap();
    let names: Vec<&str> = (evaluated.plots.iter())
        .map(|plot| plot.name.as_str())
        .collect();
    assert_eq!(names, ["panel[origin=USA]", "panel[origin=Japan]"]);
    for (plot, origin) in evaluated.plots.iter().zip(["USA", "Japan"]) {
        assert_eq!(plot.identity.as_ref().unwrap().id, "panel");
        let mark = format!("{}/0", plot.name);
        assert_eq!(plot.marks, [mark.as_str()]);
        let rows = evaluated.data.rows(&mark).unwrap();
        assert_eq!(rows.num_rows(), cars_of(origin).num_rows());
        plot_group(&evaluated, &plot.name);
    }
}
