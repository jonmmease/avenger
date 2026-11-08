//! What the tests of the app share: the label engine, the cascades of the base stylesheet, the
//! cars, apps on the test's runtime that record their changes, and window events.

#![allow(dead_code)]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray},
    compute::{filter_record_batch, kernels::cmp::eq},
};
use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartApp, ChartAppState, ChartChange, ChartUpdates, Reports};
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, MouseScrollDelta, WindowCursorMoved, WindowEvent, WindowMouseInput,
    WindowMouseWheel,
};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_style::{Cascade, Env, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_typst_label::LabelEngine;

pub(crate) const SIZE: [f32; 2] = [800.0, 400.0];

pub(crate) fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cascade of a style table of `css` after the base stylesheet.
pub(crate) fn cascade_with(css: &str) -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let text = format!("@import 'std:base.css';\n{css}");
    let (sheet, diagnostics) = Sheet::load(&text, None, SourceId(1), options, &mut no_files);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

pub(crate) fn cascade() -> Arc<Cascade> {
    cascade_with("")
}

/// A table of 24 cars: a weight, a fuel economy, and an origin.
pub(crate) fn cars() -> RecordBatch {
    let rows = 24;
    let weight: Vec<f64> = (0..rows)
        .map(|row| 1800.0 + ((row * 37) % 24) as f64 * 130.0)
        .collect();
    let mpg: Vec<f64> = (weight.iter().enumerate())
        .map(|(row, weight)| 48.0 - weight / 140.0 + ((row * 7) % 5) as f64)
        .collect();
    let origins = ["USA", "Europe", "Japan"];
    RecordBatch::try_from_iter([
        ("weight", Arc::new(Float64Array::from(weight)) as ArrayRef),
        ("mpg", Arc::new(Float64Array::from(mpg))),
        (
            "origin",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| origins[row % 3]),
            )),
        ),
    ])
    .unwrap()
}

/// The cars of `origins`.
pub(crate) fn cars_of(origins: &[&str]) -> RecordBatch {
    let cars = cars();
    let column = cars.column_by_name("origin").unwrap();
    let keep = (origins.iter())
        .map(|origin| eq(column, &StringArray::new_scalar(*origin)).unwrap())
        .reduce(|a, b| arrow::compute::or(&a, &b).unwrap())
        .unwrap();
    filter_record_batch(&cars, &keep).unwrap()
}

/// A navigation of `x`.
pub(crate) fn navigate_x() -> Vec<ToolBuilder> {
    vec![Navigate::builder().channels([Channel::X]).into()]
}

/// A navigation of every navigated channel.
pub(crate) fn navigate() -> Vec<ToolBuilder> {
    vec![Navigate::builder().into()]
}

/// The changes that an app reports.
pub(crate) type Changes = Arc<Mutex<Vec<ChartChange>>>;

/// A chart app of `chart` with a handle, on the test's runtime, that records the changes that it
/// reports by `reports`.
pub(crate) async fn app_reporting(
    chart: Chart,
    reports: Reports,
) -> (AvengerApp<ChartAppState>, ChartUpdates, Changes) {
    let updates = ChartUpdates::new();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let heard = changes.clone();
    let app = ChartApp::new(chart, cascade(), engine())
        .size(SIZE)
        .updates(&updates)
        .reports(reports)
        .on_change(move |change| heard.lock().unwrap().push(change.clone()))
        .build()
        .await
        .unwrap()
        .with_runtime(tokio::runtime::Handle::current())
        .unwrap();
    (app, updates, changes)
}

pub(crate) async fn app(chart: Chart) -> (AvengerApp<ChartAppState>, ChartUpdates, Changes) {
    app_reporting(chart, Reports::Every).await
}

/// Sends events 10 ms apart, so the 8 ms throttle of a pan takes each move.
pub(crate) async fn send(
    app: &mut AvengerApp<ChartAppState>,
    events: impl IntoIterator<Item = WindowEvent>,
) {
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
}

/// Refreshes the app until `done` holds, at most for 5 seconds.
pub(crate) async fn refresh_until(
    app: &mut AvengerApp<ChartAppState>,
    done: impl Fn(&AvengerApp<ChartAppState>) -> bool,
) {
    for _ in 0..500 {
        app.refresh().await;
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the app did not apply its update");
}

pub(crate) fn moved(position: [f32; 2]) -> WindowEvent {
    WindowEvent::CursorMoved(WindowCursorMoved { position })
}

pub(crate) fn mouse(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput(WindowMouseInput {
        state,
        button: MouseButton::Left,
    })
}

pub(crate) fn drag(from: [f32; 2], to: [f32; 2]) -> [WindowEvent; 5] {
    let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    [
        moved(from),
        mouse(ElementState::Pressed),
        moved(middle),
        moved(to),
        mouse(ElementState::Released),
    ]
}

pub(crate) fn double_click(at: [f32; 2]) -> [WindowEvent; 5] {
    [
        moved(at),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
        mouse(ElementState::Pressed),
        mouse(ElementState::Released),
    ]
}

pub(crate) fn wheel(lines: f32) -> WindowEvent {
    WindowEvent::MouseWheel(WindowMouseWheel {
        delta: MouseScrollDelta::LineDelta(0.0, lines),
    })
}

/// The center of the plot at `plot`.
pub(crate) fn center(app: &AvengerApp<ChartAppState>, plot: usize) -> [f32; 2] {
    let [x, y, width, height] = app.state().plots()[plot].rect;
    [x + width / 2.0, y + height / 2.0]
}

/// The domains of the `x` and `y` scales of the plot at `plot`.
pub(crate) fn domains(app: &AvengerApp<ChartAppState>, plot: usize) -> [(f64, f64); 2] {
    let coords = &app.state().plots()[plot].coords;
    [0, 1].map(|dimension| {
        coords
            .scale(dimension)
            .unwrap()
            .numeric_interval_domain()
            .unwrap()
    })
}
