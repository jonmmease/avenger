//! The widgets of a chart in its app: they draw in their own scene, a click or a drag writes the
//! clause of the widget to its selection, and the app reports the selection.

mod support;

use std::sync::Arc;

use arrow::array::RecordBatch;
use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartAppState, ChartChange, ChartUpdate, WidgetValue};
use avenger_eventstream::window::ElementState;
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};

use support::*;

/// The cars, gray outside the selection `filter`, with `widgets`.
fn cars_with(cars: RecordBatch, widgets: Vec<WidgetBuilder>) -> Chart {
    let color =
        Encoding::scaled_column("origin").conditional([ConditionalValue::constant("lightgray")
            .predicate(Predicate::negate(Predicate::selected(
                Selected::builder().selection("filter"),
            )))]);
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg"))
            .color(color),
    )
    .table(cars)
    .widgets(widgets);
    Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .selections([Selection::builder()
            .name("filter")
            .resolution(SelectionResolution::Intersect)])
        .infer_scales()
        .build()
        .unwrap()
}

fn radio() -> WidgetBuilder {
    Widget::radio(
        RadioWidget::builder()
            .target(WidgetTarget::builder().selection("filter").column("origin"))
            .domain(WidgetDomain::builder().data(WidgetDataDomain::builder()))
            .clear_label("All"),
    )
    .key("origin")
}

fn slider() -> WidgetBuilder {
    Widget::slider(
        SliderWidget::builder()
            .target(WidgetTarget::builder().selection("filter").column("weight"))
            .domain(WidgetDomain::builder().data(WidgetDataDomain::builder()))
            .comparison(SliderComparison::AtLeast)
            .step(100.0),
    )
    .key("weight")
}

/// The scene of the widgets, the highest of the app's scenes, or with `chart`, the scene of the
/// chart, the lowest.
fn scene(app: &AvengerApp<ChartAppState>, chart: bool) -> Arc<SceneGraph> {
    let scenes = app.scenes();
    match chart {
        true => scenes[0].clone(),
        false => scenes[scenes.len() - 1].clone(),
    }
}

/// Each mark of `marks` with the origin of the groups that hold it.
fn flatten(marks: &[SceneMark], origin: [f32; 2], out: &mut Vec<(SceneMark, [f32; 2])>) {
    for mark in marks {
        match mark {
            SceneMark::Group(group) => flatten(
                &group.marks,
                [origin[0] + group.origin[0], origin[1] + group.origin[1]],
                out,
            ),
            mark => out.push((mark.clone(), origin)),
        }
    }
}

/// The marks of a scene, with their origins.
fn marks(scene: &SceneGraph) -> Vec<(SceneMark, [f32; 2])> {
    let mut out = Vec::new();
    flatten(&scene.marks, scene.origin, &mut out);
    out
}

/// The position of the text `text` in the scene.
fn text_at(scene: &SceneGraph, text: &str) -> Option<[f32; 2]> {
    marks(scene)
        .into_iter()
        .find_map(|(mark, origin)| match mark {
            SceneMark::Text(mark) if mark.text.as_vec(1, None)[0] == text => {
                let [x, y] = [mark.x.as_vec(1, None)[0], mark.y.as_vec(1, None)[0]];
                Some([origin[0] + x, origin[1] + y])
            }
            _ => None,
        })
}

/// The center of the rectangle named `name` in the scene.
fn rect_center(scene: &SceneGraph, name: &str) -> Option<[f32; 2]> {
    marks(scene)
        .into_iter()
        .find_map(|(mark, origin)| match mark {
            SceneMark::Rect(mark) if mark.name == name => {
                let x = mark.x.as_vec(1, None)[0];
                let y = mark.y.as_vec(1, None)[0];
                let width = mark
                    .width
                    .as_ref()
                    .map_or(0.0, |width| width.as_vec(1, None)[0]);
                let height = mark
                    .height
                    .as_ref()
                    .map_or(0.0, |height| height.as_vec(1, None)[0]);
                Some([origin[0] + x + width / 2.0, origin[1] + y + height / 2.0])
            }
            _ => None,
        })
}

fn click(at: [f32; 2]) -> [avenger_eventstream::window::WindowEvent; 3] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn the_widgets_draw_in_their_own_scene() {
    let (app, _, _) = app(cars_with(cars(), vec![slider()])).await;
    let [widget] = app.state().widgets() else {
        panic!("one widget");
    };
    assert_eq!(widget.rects.len(), 1);
    assert!(rect_center(&scene(&app, false), "thumb").is_some());
    assert!(rect_center(&scene(&app, true), "thumb").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_on_an_item_of_a_radio_group_writes_its_clause() {
    let (mut app, _, changes) = app(cars_with(cars(), vec![radio()])).await;
    let at = text_at(&scene(&app, false), "Japan").expect("the item's label");
    send(&mut app, click([at[0], at[1]])).await;

    let widget = &app.state().widgets()[0];
    let filter = app.state().evaluator().selections().get("filter");
    assert_eq!(widget.value(filter), WidgetValue::Item(Some(1)));
    let clauses = &filter.unwrap().clauses;
    assert_eq!(clauses.len(), 1);
    assert_eq!(clauses[0].owner, widget.owner());
    // The click is a gesture of its own, which settles.
    let last = changes.lock().unwrap().last().cloned();
    let Some(ChartChange {
        selections,
        settled: true,
        ..
    }) = last
    else {
        panic!("a settled change, and not {last:?}");
    };
    assert_eq!(selections[0].0, "filter");

    // The clearing item removes the clause.
    let at = text_at(&scene(&app, false), "All").expect("the clearing item's label");
    send(&mut app, click(at)).await;
    let filter = app.state().evaluator().selections().get("filter");
    assert!(filter.unwrap().clauses.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_of_a_slider_writes_its_clause_and_settles_at_its_end() {
    let (mut app, _, changes) = app(cars_with(cars(), vec![slider()])).await;
    let from = rect_center(&scene(&app, false), "thumb").expect("the thumb");
    let rect = app.state().widgets()[0].rects[0];
    let to = [rect[0] + rect[2] * 0.4, from[1]];
    send(&mut app, drag(from, to)).await;

    let widget = &app.state().widgets()[0];
    let filter = app.state().evaluator().selections().get("filter");
    let WidgetValue::Number(value) = widget.value(filter) else {
        panic!("a slider's value");
    };
    assert!(value > 1800.0, "{value}");
    // The drag reports as it moves and settles at its release.
    let changes = changes.lock().unwrap().clone();
    assert!(changes.len() >= 2, "{changes:?}");
    assert!(changes[..changes.len() - 1]
        .iter()
        .all(|change| !change.settled));
    assert!(changes.last().unwrap().settled);
    // The thumb moved.
    let thumb = rect_center(&scene(&app, false), "thumb").unwrap();
    assert!(thumb[0] > from[0] + 10.0, "{thumb:?} {from:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_chart_with_other_widgets_draws_them() {
    let (mut app, updates, _) = app(cars_with(cars(), vec![slider()])).await;
    updates.send(ChartUpdate::new(cars_with(cars(), vec![radio()])));
    refresh_until(&mut app, |app| {
        (app.state().widgets().first()).is_some_and(|widget| widget.key == "origin")
    })
    .await;
    let widgets = scene(&app, false);
    assert!(text_at(&widgets, "Japan").is_some());
    assert!(rect_center(&widgets, "thumb").is_none());

    // A chart without widgets draws none.
    updates.send(ChartUpdate::new(cars_with(cars(), Vec::new())));
    refresh_until(&mut app, |app| app.state().widgets().is_empty()).await;
    assert!(marks(&scene(&app, false)).is_empty());
}
