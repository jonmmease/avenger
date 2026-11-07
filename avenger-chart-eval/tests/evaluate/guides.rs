//! Titles, attributions, axes, legends, and colorbars, and the styles that hide and paint them.

use std::sync::Arc;

use arrow::array::{RecordBatch, StringArray};
use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, Registry};
use avenger_color::ColorOrGradient;
use avenger_style::HostInputs;

use crate::support::*;

#[test]
fn titles_and_attributions() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    );
    let plot = Plot::cartesian([symbols])
        .title(
            Title::new("Fuel economy")
                .subtitle("By weight and origin")
                .align(TitleAlign::Start),
        )
        .attribution(Attribution::builder().credits(["Source: StatLib"]));
    let chart = Chart::builder(plot).build().unwrap();
    render_both(&chart, "title-subtitle-attribution");

    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    );
    let plot = Plot::cartesian([symbols]).title(
        Title::new("Figure 1: heavier cars travel fewer miles on a gallon.")
            .side(TitleSide::Bottom),
    );
    let caption = Chart::builder(plot).build().unwrap();
    render_both(&caption, "caption");
}

#[test]
fn legends() {
    // A threshold scale of the fuel economies, at 20 and 30.
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color(
            Encoding::scaled_field("mpg")
                .scale(Scale::threshold().domain(Domain::explicit([20.0, 30.0]))),
        ),
    );
    let threshold = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    render_both(&threshold, "legend-threshold");

    // A color and a shape of one column share one legend.
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x("weight")
            .y("mpg")
            .color("origin")
            .shape("origin"),
    );
    let merged = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    render_both(&merged, "legend-color-shape");
}

#[test]
fn an_axis_with_explicit_ticks() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight")
                .scale(Axis::builder().ticks(Ticks::values([2000.0, 3000.0, 4000.0]))))
            .y("mpg"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    render_both(&chart, "axis-explicit-ticks");
}

/// A continuous color scale whose fit pins its middle is diverging, so the base stylesheet gives
/// it a diverging scheme, centered on the pinned value.
#[test]
fn a_color_scale_that_pins_its_middle_diverges() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x("weight")
            .y("mpg")
            .color(Encoding::scaled_field("mpg").scale(Domain::data(
                DataDomain::builder().fit(IntervalFit::builder().mid(25.0)),
            ))),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    render_both(&chart, "legend-diverging");
}

/// A legend of 40 categories ends at its 30th entry, which counts the rest.
#[test]
fn a_legend_past_its_max_entries() {
    let names: Vec<String> = (0..40).map(|index| format!("item {index}")).collect();
    let table = RecordBatch::try_from_iter([
        ("x", numbers(&(0..40).map(f64::from).collect::<Vec<_>>())),
        (
            "name",
            Arc::new(StringArray::from(names)) as arrow::array::ArrayRef,
        ),
    ])
    .unwrap();
    let symbols = CartesianLayer::symbol(table, SymbolMark::builder().x("x").y("x").color("name"));
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    render(&chart, [480.0, 640.0], "legend-max-entries");
}

#[test]
fn a_colorbar_with_a_longest_length_beside_a_tall_plot() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color(
            Encoding::scaled_field("mpg").scale(
                Legend::builder()
                    .form(LegendForm::Colorbar)
                    .max_length(120.0),
            ),
        ),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    render(&chart, [320.0, 480.0], "colorbar-longest-length");
}

#[test]
fn legends_draw_the_constant_colors_of_their_marks() {
    // A size legend of red symbols shows red symbols.
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x("weight")
            .y("mpg")
            .size("count")
            .color(Encoding::direct_value("#e45756")),
    );
    let symbols = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let red = [228.0 / 255.0, 87.0 / 255.0, 86.0 / 255.0, 1.0];
    let fills = legend_symbol_fills(&evaluate(&symbols, [320.0, 240.0]));
    assert!(!fills.is_empty());
    assert!(
        fills
            .iter()
            .all(|fill| matches!(fill, ColorOrGradient::Color(color)
            if color.iter().zip(&red).all(|(a, b)| (a - b).abs() < 1e-3))),
        "{fills:?}"
    );
    render_both(&symbols, "legend-constant-color-symbols");

    // A dash legend of green lines shows green dashes.
    let lines = CartesianLayer::line(
        cars(),
        LineMark::builder()
            .x("year")
            .y("mpg")
            .detail("origin")
            .stroke_dash("origin")
            .color(Encoding::direct_value("#54a24b")),
    );
    let lines = Chart::builder(Plot::cartesian([lines])).build().unwrap();
    render_both(&lines, "legend-constant-color-lines");
}

/// `display: none` on the node of a guide hides the whole guide.
#[test]
fn a_hidden_guide_draws_nothing() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let texts = scene_texts(&evaluate(&chart, [320.0, 240.0]).scene.marks);
    assert!(texts.contains(&"Europe".to_string()) && texts.contains(&"weight".to_string()));
    let css = "legend, axis[channel=x] { display: none; }";
    let mut hidden =
        ChartEvaluator::new(chart, cascade_with(css), engine(), Registry::new()).unwrap();
    let evaluated = hidden.evaluate(&host([320.0, 240.0])).unwrap();
    let texts = scene_texts(&evaluated.scene.marks);
    assert!(!texts.contains(&"Europe".to_string()), "{texts:?}");
    assert!(!texts.contains(&"weight".to_string()), "{texts:?}");
    assert!(texts.contains(&"mpg".to_string()), "{texts:?}");
}

/// A symbol that fills with a mix of `currentColor` and white and strokes in `currentColor`: each
/// row's color tints its fill and outlines it, and the legend's swatches match the marks.
#[test]
fn a_symbol_fills_with_a_tint_of_its_color_and_strokes_in_it() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder()
            .x("weight")
            .y("mpg")
            .color("origin")
            .size(100.0),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let css = "mark[type=symbol] {
        fill: color-mix(in srgb, currentColor 50%, white);
        stroke: currentColor;
    }";
    let mut evaluator =
        ChartEvaluator::new(chart.clone(), cascade_with(css), engine(), Registry::new()).unwrap();
    let outlined = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert_image(&outlined, "symbol-tint-and-outline");
    let plain = evaluate(&chart, [320.0, 240.0]);

    // Each mark strokes in its row's color, the plain fill, and fills with a tint of it.
    let paints = |evaluated: &EvaluatedChart| {
        let symbol = plot_symbols(evaluated);
        let len = symbol.len as usize;
        (
            symbol.fill.as_vec(len, None),
            symbol.stroke.as_vec(len, None),
        )
    };
    let (fills, strokes) = paints(&outlined);
    let (colors, _) = paints(&plain);
    assert_eq!(strokes, colors);
    for (fill, color) in fills.iter().zip(&colors) {
        assert!(close(fill, tint(color)), "{fill:?} {color:?}");
    }
    // The swatches paint as the marks do.
    let colors = legend_symbol_fills(&plain);
    assert!(!colors.is_empty());
    assert_eq!(legend_symbol_strokes(&outlined), colors);
    for (fill, color) in legend_symbol_fills(&outlined).iter().zip(&colors) {
        assert!(close(fill, tint(color)), "{fill:?} {color:?}");
    }
}

/// The color halfway from `color` to white.
fn tint(color: &ColorOrGradient) -> [f32; 4] {
    match color {
        ColorOrGradient::Color([r, g, b, a]) => {
            [(r + 1.0) / 2.0, (g + 1.0) / 2.0, (b + 1.0) / 2.0, *a]
        }
        other => panic!("a color, not {other:?}"),
    }
}

/// Whether `a` is the color `b`, within rounding.
fn close(a: &ColorOrGradient, b: [f32; 4]) -> bool {
    match a {
        ColorOrGradient::Color(a) => a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3),
        _ => false,
    }
}

/// A `color-mix()` of `currentColor` tints the color of each row, in the marks and in the swatches
/// of their legend.
#[test]
fn a_mix_of_current_color_tints_each_rows_color() {
    let symbols = CartesianLayer::symbol(
        cars(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let css = "mark[type=symbol] { fill: color-mix(in srgb, currentColor 50%, white); }";
    let mut tinted =
        ChartEvaluator::new(chart.clone(), cascade_with(css), engine(), Registry::new()).unwrap();
    let tinted = tinted.evaluate(&host([320.0, 240.0])).unwrap();
    let plain = evaluate(&chart, [320.0, 240.0]);
    let fills = |evaluated: &EvaluatedChart| {
        let symbol = plot_symbols(evaluated);
        symbol.fill.as_vec(symbol.len as usize, None)
    };
    for (tinted, plain) in fills(&tinted).iter().zip(fills(&plain).iter()) {
        assert!(close(tinted, tint(plain)), "{tinted:?} {plain:?}");
    }
    let (tinted, plain) = (legend_symbol_fills(&tinted), legend_symbol_fills(&plain));
    assert!(!tinted.is_empty() && tinted.len() == plain.len());
    for (tinted, plain) in tinted.iter().zip(&plain) {
        assert!(close(tinted, tint(plain)), "{tinted:?} {plain:?}");
    }
}

/// The bars of the twelve months, whose labels turn where a month is wider than its band.
fn months() -> Chart {
    let names = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let table = RecordBatch::try_from_iter([
        (
            "month",
            Arc::new(StringArray::from(names.to_vec())) as arrow::array::ArrayRef,
        ),
        (
            "rain",
            numbers(&[
                49.0, 36.0, 43.0, 61.0, 74.0, 80.0, 91.0, 84.0, 66.0, 58.0, 51.0, 47.0,
            ]),
        ),
    ])
    .unwrap();
    let bars = CartesianLayer::bar(table, BarMark::builder().x("month").y("rain"));
    Chart::builder(Plot::cartesian([bars])).build().unwrap()
}

/// The height of the first plot of `chart` at `size`.
fn plot_height(evaluator: &mut ChartEvaluator, size: [f32; 2]) -> f32 {
    let evaluated = evaluator.evaluate(&host(size)).unwrap();
    assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
    evaluated.plots[0].rect[3]
}

/// The labels of a discrete x axis read across where each fits its band, slant at 315 degrees
/// where the slant keeps them apart, and stand vertical where it does not, so the axis grows
/// thicker and the plot shorter as the width narrows, and a resize across a change measures the
/// axis again, as a new evaluation does.
#[test]
fn discrete_labels_turn_where_they_do_not_fit() {
    let fresh = |size| plot_height(&mut evaluator(&months()), size);
    let narrow = fresh([480.0, 300.0]);
    let wide = fresh([1400.0, 300.0]);
    assert!(narrow + 20.0 < wide, "{narrow} {wide}");

    let mut resized = evaluator(&months());
    assert_eq!(plot_height(&mut resized, [1400.0, 300.0]), wide);
    assert_eq!(plot_height(&mut resized, [480.0, 300.0]), narrow);
    assert_eq!(plot_height(&mut resized, [1400.0, 300.0]), wide);
    // Narrower still, the step no longer fits the slant, and the labels stand vertical, which
    // takes more room than the slant for the long names of months.
    let vertical = fresh([200.0, 300.0]);
    assert!(vertical < narrow, "{vertical} {narrow}");
    render(&months(), [480.0, 300.0], "labels-turned");
}

/// A plot that keeps its aspect in a canvas of a fixed height, at the width where its labels turn,
/// ends its solves: turned labels shorten the plot, which narrows it and keeps them turned.
#[test]
fn turned_labels_end_their_solves_in_a_plot_that_keeps_its_aspect() {
    for height in [200.0, 260.0, 320.0, 400.0, 520.0] {
        let mut evaluator = ChartEvaluator::new(
            months(),
            cascade_with("plot { aspect-ratio: 3; }"),
            engine(),
            Registry::new(),
        )
        .unwrap();
        let host = HostInputs {
            height: Some(height),
            ..Default::default()
        };
        let evaluated = evaluator.evaluate(&host).unwrap();
        assert!(
            evaluated.warnings.is_empty(),
            "{height}: {:?}",
            evaluated.warnings
        );
    }
}
