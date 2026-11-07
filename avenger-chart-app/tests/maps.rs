//! A chart app of maps, with the `geo` feature: navigations of projections and boxes on maps.

mod support;

use std::{sync::Arc, time::Duration};

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_app::app::AvengerApp;
use avenger_chart::prelude::*;
use avenger_chart_app::ChartAppState;

use support::*;

/// Cities with their longitudes and latitudes.
fn cities() -> RecordBatch {
    let rows = [
        ("London", -0.13, 51.51),
        ("New York", -74.01, 40.71),
        ("Lagos", 3.38, 6.52),
        ("Mumbai", 72.88, 19.08),
        ("Tokyo", 139.69, 35.69),
        ("Sydney", 151.21, -33.87),
        ("Lima", -77.04, -12.05),
    ];
    RecordBatch::try_from_iter([
        (
            "name",
            Arc::new(StringArray::from_iter_values(rows.iter().map(|row| row.0))) as ArrayRef,
        ),
        (
            "lon",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|row| row.1))),
        ),
        (
            "lat",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|row| row.2))),
        ),
    ])
    .unwrap()
}

/// A world map `map` of the cities, with `tools`.
fn world(tools: Vec<ToolBuilder>) -> Chart {
    let places = GeoLayer::symbol(
        GeoSymbolMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat")),
    )
    .table(cities());
    Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(
            Plot::geo(
                GeoPlot::builder()
                    .options(Geo::spherical(Spherical::equal_earth().fit_sphere()))
                    .layers([places]),
            )
            .id("map")
            .tools(tools),
        )
        .selections([Selection::builder()
            .name("brush")
            .resolution(SelectionResolution::Union)])
        .infer_scales()
        .build()
        .unwrap()
}

/// The pixel of longitude 0 and latitude 0 in the map.
fn origin(app: &AvengerApp<ChartAppState>) -> [f32; 2] {
    let [x, y] = (app.state().plots()[0].coords)
        .pixels([
            &(Arc::new(Float64Array::from(vec![0.0])) as ArrayRef),
            &(Arc::new(Float64Array::from(vec![0.0])) as ArrayRef),
        ])
        .unwrap();
    [x.value(0), y.value(0)]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drag_pans_the_projection_of_a_map_and_a_double_click_returns_it() {
    let (mut app, _, changes) = app(world(navigate())).await;
    let fitted = origin(&app);
    let at = center(&app, 0);
    send(&mut app, drag(at, [at[0] + 40.0, at[1]])).await;
    assert_ne!(origin(&app), fitted);
    assert!(app.state().evaluator().projection_view(0).is_some());
    assert!((changes.lock().unwrap().iter()).any(|change| !change.projections.is_empty()));

    std::thread::sleep(Duration::from_millis(600));
    send(&mut app, double_click(at)).await;
    assert_eq!(origin(&app), fitted);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_on_a_map_selects_ranges_of_longitude_and_latitude() {
    let select = Select::interval(IntervalSelect::builder()).selection("brush");
    let (mut app, _, _) = app(world(vec![select.into()])).await;
    let [x, y, width, height] = app.state().plots()[0].rect;
    send(
        &mut app,
        drag(
            [x + width * 0.4, y + height * 0.3],
            [x + width * 0.6, y + height * 0.6],
        ),
    )
    .await;
    let clauses = app.state().evaluator().selections()["brush"]
        .clauses
        .clone();
    assert_eq!(clauses.len(), 1);
    let columns: Vec<String> = (clauses[0].conditions.iter())
        .flat_map(|condition| condition.columns().into_iter().map(str::to_string))
        .collect();
    assert_eq!(columns, ["lon", "lat"]);
}
