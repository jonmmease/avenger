//! Each chart of the gallery and of its linked page decodes from its bytes and builds a chart app with the library's own
//! label engine and styles, over the bundled faces that the gallery's page adds, as the library
//! does in the browser.

use avenger_chart::Chart;
use avenger_chart_app::ChartApp;
use avenger_chart_web::{
    fonts::{label_engine, read_fonts},
    styles::cascade,
};
use avenger_style::ColorScheme;

#[test]
fn each_chart_round_trips_through_its_bytes_and_builds_an_app() {
    let faces = (avenger_fonts::FACES.iter())
        .flat_map(|(_, face)| read_fonts(&avenger_fonts::decompress(face)).unwrap())
        .collect();
    let engine = label_engine(faces);
    let gallery: Vec<_> = (chart_web::gallery().into_iter())
        .chain(chart_web::linked())
        .collect();
    assert_eq!(gallery.len(), 8);
    for scheme in [ColorScheme::Light, ColorScheme::Dark] {
        let styles = cascade(None, scheme, true);
        for entry in &gallery {
            let bytes = entry.chart.to_bytes().unwrap();
            let chart = Chart::from_bytes(bytes).unwrap();
            let app = ChartApp::new(chart, styles.clone(), engine.clone()).size([640.0, 400.0]);
            let app = pollster::block_on(app.build());
            let app = app.unwrap_or_else(|error| panic!("{}: {error}", entry.name));
            assert!(!app.scenes().is_empty(), "{}", entry.name);
        }
    }
}
