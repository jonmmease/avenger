//! Each chart of the gallery decodes from its bytes and builds a chart app, as the library does in
//! the browser.

use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_app::ChartApp;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_style::{Cascade, Env, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_typst_label::LabelEngine;

fn cascade() -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, _) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

/// The bundled fonts with the d3 formatters, which the library's engine has too.
fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

#[test]
fn each_chart_round_trips_through_its_bytes_and_builds_an_app() {
    let gallery = chart_web::gallery();
    assert_eq!(gallery.len(), 6);
    for entry in gallery {
        let bytes = entry.chart.to_bytes().unwrap();
        let chart = Chart::from_bytes(bytes).unwrap();
        let app = ChartApp::new(chart, cascade(), engine()).size([640.0, 400.0]);
        let app = pollster::block_on(app.build());
        let app = app.unwrap_or_else(|error| panic!("{}: {error}", entry.name));
        assert!(!app.scenes().is_empty(), "{}", entry.name);
    }
}
