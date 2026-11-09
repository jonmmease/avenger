//! The NULL category of a categorical scale: the last band of a fitted domain and the last entry
//! of a legend, labeled `null`, and in an explicit domain where the domain names it.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::prelude::*;

use crate::support::*;

/// The number of cars of each origin, with the cars whose origin is not known.
fn origins() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "origin",
            Arc::new(StringArray::from(vec![
                Some("USA"),
                None,
                Some("Europe"),
                Some("Japan"),
            ])) as ArrayRef,
        ),
        (
            "cars",
            Arc::new(Float64Array::from(vec![249.0, 14.0, 70.0, 79.0])),
        ),
    ])
    .unwrap()
}

/// Bars of the cars by origin through `scale`, with an axis of the labels `labels`.
fn bars(scale: ScaleBuilder, labels: Option<LabelsBuilder>) -> Chart {
    let mut axis = Axis::builder().title("origin");
    if let Some(labels) = labels {
        axis = axis.labels(labels);
    }
    let x = Encoding::scaled_field("origin").scale(scale.axis(axis));
    let bars = CartesianLayer::bar(origins(), BarMark::builder().x(x).y("cars"));
    Chart::builder(Plot::cartesian([bars])).build().unwrap()
}

/// The labels of the scene's x axis, without its title, in their order along it.
fn axis_labels(evaluated: &avenger_chart_eval::EvaluatedChart) -> Vec<String> {
    let plot = evaluated.plots[0].rect;
    let mut labels: Vec<(String, [f32; 2])> = scene_text_positions(&evaluated.scene.marks)
        .into_iter()
        .filter(|(text, at)| at[1] > plot[1] + plot[3] && text != "origin")
        .collect();
    labels.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
    labels.into_iter().map(|(text, _)| text).collect()
}

/// The left edges of the drawn bars.
fn drawn_bars(evaluated: &avenger_chart_eval::EvaluatedChart) -> usize {
    rect_lefts(evaluated)
        .into_iter()
        .filter(|left| left.is_finite())
        .count()
}

#[test]
fn a_null_category_is_the_last_band_of_a_fitted_domain() {
    let evaluated = render(&bars(Scale::builder(), None), [360.0, 240.0], "null-bars");
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    assert_eq!(axis_labels(&evaluated), ["Europe", "Japan", "USA", "null"]);
    assert_eq!(drawn_bars(&evaluated), 4);

    // A descending sort keeps the NULL category last.
    let descending = bars(
        Scale::builder().domain(Domain::data(DataDomain::builder().fit(
            DomainFit::categories(CategoryFit::builder().sort(CategorySort::Descending)),
        ))),
        None,
    );
    let labels = axis_labels(&evaluate(&descending, [360.0, 240.0]));
    assert_eq!(labels, ["USA", "Japan", "Europe", "null"]);
}

#[test]
fn an_explicit_domain_draws_the_null_category_where_it_names_it() {
    let named = |values: Vec<Option<&str>>| {
        let values: ArrayRef = Arc::new(StringArray::from(values));
        bars(
            Scale::builder().domain(Domain::explicit(Value::list(values))),
            None,
        )
    };
    let evaluated = evaluate(
        &named(vec![Some("USA"), None, Some("Japan")]),
        [360.0, 240.0],
    );
    assert_eq!(axis_labels(&evaluated), ["USA", "null", "Japan"]);
    assert_eq!(drawn_bars(&evaluated), 3);
    // A domain that does not name NULL draws no NULL bar.
    let evaluated = evaluate(&named(vec![Some("USA"), Some("Japan")]), [360.0, 240.0]);
    assert_eq!(axis_labels(&evaluated), ["USA", "Japan"]);
    assert_eq!(drawn_bars(&evaluated), 2);
}

#[test]
fn the_labels_of_categories_bind_whether_they_are_the_null_category() {
    let chart = bars(
        Scale::builder(),
        Some(Labels::builder().markup("#if null [Unknown] else [#value]")),
    );
    let labels = axis_labels(&evaluate(&chart, [360.0, 240.0]));
    assert!(labels[3].starts_with("#if (true)"), "{labels:?}");
    assert!(labels[..3]
        .iter()
        .all(|label| label.starts_with("#if (false)")));
}

#[test]
fn a_color_legend_ends_with_the_null_category() {
    let symbols = CartesianLayer::symbol(
        origins(),
        SymbolMark::builder().x("cars").y("cars").color("origin"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let evaluated = evaluate(&chart, [480.0, 320.0]);
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let texts = scene_texts(&evaluated.scene.marks);
    let at = |text: &str| texts.iter().position(|known| known == text);
    let (usa, null) = (at("USA"), at("null"));
    assert!(usa.is_some() && usa < null, "{texts:?}");
}

/// `chart` at `size` with the base stylesheet and `css`.
fn evaluate_styled(chart: Chart, css: &str, size: [f32; 2]) -> avenger_chart_eval::EvaluatedChart {
    let mut evaluator = avenger_chart_eval::ChartEvaluator::new(
        chart,
        cascade_with(css),
        engine(),
        avenger_chart_eval::Registry::new(),
    )
    .unwrap();
    evaluator.evaluate(&host(size)).unwrap()
}

fn is_color(color: &avenger_color::ColorOrGradient, expected: [f32; 4]) -> bool {
    matches!(color, avenger_color::ColorOrGradient::Color(color)
        if color.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-3))
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The fills of the bars of the plot, in the order of the rows.
fn bar_fills(
    evaluated: &avenger_chart_eval::EvaluatedChart,
) -> Vec<avenger_color::ColorOrGradient> {
    (plot_group(evaluated).marks.iter())
        .find_map(|mark| match mark {
            avenger_scenegraph::marks::mark::SceneMark::Rect(rect) => {
                Some(rect.fill.as_vec(rect.len as usize, None))
            }
            _ => None,
        })
        .expect("a rect mark")
}

#[test]
fn the_null_category_takes_the_style_of_its_null_nodes() {
    let css = "mark > null { color: black; pattern: \"diagonal-down\"; } \
               axis > label > null { font-style: italic; }";
    let evaluated = evaluate_styled(bars(Scale::builder(), None), css, [360.0, 240.0]);
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    assert_image(&evaluated, "null-styled");
    // The rows of the NULL category, the second row, are black, without an encoding of the
    // color, and the other rows keep the mark's color.
    let fills = bar_fills(&evaluated);
    let black: Vec<bool> = fills.iter().map(|fill| is_color(fill, BLACK)).collect();
    assert_eq!(black, [false, true, false, false]);
    // The NULL label is italic, and the other labels are as they were.
    let labels = axis_labels(&evaluated);
    assert!(labels[3].contains(r#"style: "italic""#), "{labels:?}");
    assert!(!labels[0].contains("#text"), "{labels:?}");
}

#[test]
fn the_null_entry_of_a_legend_takes_the_style_of_its_null_nodes() {
    let symbols = CartesianLayer::symbol(
        origins(),
        SymbolMark::builder().x("cars").y("cars").color("origin"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let css = "mark > null { color: black; } legend > label > null { font-style: italic; }";
    let evaluated = evaluate_styled(chart, css, [480.0, 320.0]);
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    // The symbol of the NULL row and the legend's last swatch are black.
    let symbols = plot_symbols(&evaluated);
    let fills = symbols.fill.as_vec(symbols.len as usize, None);
    assert!(is_color(&fills[1], BLACK), "{fills:?}");
    assert!(!is_color(&fills[0], BLACK), "{fills:?}");
    let swatches = legend_symbol_fills(&evaluated);
    assert!(is_color(swatches.last().unwrap(), BLACK), "{swatches:?}");
    let texts = scene_texts(&evaluated.scene.marks);
    assert!(
        texts
            .iter()
            .any(|text| text.contains(r#"style: "italic""#) && text.contains("null")),
        "{texts:?}"
    );
}
