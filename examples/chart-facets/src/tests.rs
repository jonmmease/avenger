//! The grid's round trip through its bytes, and its drawing at two sizes, by the evaluator, by the
//! app, and as SVG.

use super::*;
use avenger_scenegraph::marks::mark::SceneMark;

/// The text of every label of `marks`, at any depth.
fn texts(marks: &[SceneMark]) -> Vec<String> {
    marks
        .iter()
        .flat_map(|mark| match mark {
            SceneMark::Text(text) => text.text.as_vec(text.len as usize, None),
            SceneMark::Group(group) => texts(&group.marks),
            _ => Vec::new(),
        })
        .collect()
}

/// A chart decoded from its bytes encodes to the same bytes.
#[test]
fn the_grid_round_trips_through_its_bytes() {
    let chart = grid(&movies());
    let bytes = chart.to_bytes().unwrap();
    assert_eq!(round_trip(&chart).to_bytes().unwrap(), bytes);
}

/// At each size, the grid draws a plot for each genre in each era, with a header for each, and
/// its marks fit.
#[test]
fn the_grid_draws_each_genre_and_era_at_two_sizes() {
    let mut evaluator = evaluator(&engine());
    for size in [SIZE, [480.0, 360.0]] {
        let evaluated = evaluator.evaluate(size).unwrap();
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
        assert_eq!(evaluated.plots.len(), GENRES.len() * ERAS.len());
        let texts = texts(&evaluated.scene.marks);
        for header in GENRES.iter().chain(ERAS.iter().map(|era| &era.0)) {
            assert!(
                texts.iter().any(|text| text == header),
                "{header} at {size:?}"
            );
        }
    }
}

/// The chart app draws the chart at the window's size, and every panel declares a navigation and
/// a tooltip.
#[test]
fn the_app_draws_the_chart_at_the_window_size() {
    let chart = round_trip(&grid(&movies()));
    let app = pollster::block_on(ChartApp::new(chart, engine()).size(SIZE).build()).unwrap();
    let scene = &app.scenes()[0];
    assert_eq!([scene.width, scene.height], SIZE);
    assert!(texts(&scene.marks).iter().any(|text| text == "Drama"));
    let tree = avenger_chart::resolve::tree(app.state().chart());
    assert!(tree.plots.iter().all(|plot| plot.tools.len() == 2));
}

/// `--svg` writes the chart as an SVG document.
#[test]
fn the_chart_writes_as_svg() {
    let path = std::env::temp_dir().join("chart-facets-test.svg");
    write_svg(path.to_str().unwrap());
    let svg = std::fs::read_to_string(&path).unwrap();
    assert!(
        svg.starts_with("<svg") && svg.contains("Comedy"),
        "{}",
        &svg[..200]
    );
    std::fs::remove_file(path).unwrap();
}
