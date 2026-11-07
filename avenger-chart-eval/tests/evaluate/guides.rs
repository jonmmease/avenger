//! Titles, attributions, axes, legends, and colorbars, and the styles that hide and paint them.

use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, Registry};
use avenger_color::ColorOrGradient;

use crate::support::*;

#[test]
fn titles_and_attributions() {
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(Encoding::scaled_field("origin")),
    )
    .table(cars());
    let plot = Plot::cartesian(CartesianPlot::builder().layers([symbols]))
        .title(
            Title::builder()
                .text("Fuel economy")
                .subtitle("By weight and origin")
                .align(TitleAlign::Start),
        )
        .attribution(Attribution::builder().credits(["Source: StatLib"]));
    let chart = Chart::builder().root(plot).infer_scales().build().unwrap();
    render_both(&chart, "title-subtitle-attribution");

    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(Encoding::scaled_field("origin")),
    )
    .table(cars());
    let plot = Plot::cartesian(CartesianPlot::builder().layers([symbols])).title(
        Title::builder()
            .text("Figure 1: heavier cars travel fewer miles on a gallon.")
            .side(TitleSide::Bottom),
    );
    let caption = Chart::builder().root(plot).infer_scales().build().unwrap();
    render_both(&caption, "caption");
}

#[test]
fn legends() {
    // A threshold scale of the fuel economies, at 20 and 30.
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(
                Encoding::scaled_field("mpg")
                    .scale(Scale::threshold().domain(Domain::explicit([20.0, 30.0]))),
            ),
    )
    .table(cars());
    let threshold = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&threshold, "legend-threshold");

    // A color and a shape of one column share one legend.
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(Encoding::scaled_field("origin"))
            .shape(Encoding::scaled_field("origin")),
    )
    .table(cars());
    let merged = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&merged, "legend-color-shape");
}

#[test]
fn an_axis_with_explicit_ticks() {
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight").scale(
                Scale::builder()
                    .guide(Axis::builder().ticks(Ticks::values([2000.0, 3000.0, 4000.0]))),
            ))
            .y(Encoding::scaled_field("mpg")),
    )
    .table(cars());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "axis-explicit-ticks");
}

#[test]
fn a_colorbar_with_a_longest_length_beside_a_tall_plot() {
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(
                Encoding::scaled_field("mpg")
                    .scale(Scale::builder().guide(Colorbar::builder().max_length(120.0))),
            ),
    )
    .table(cars());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
    render(&chart, [320.0, 480.0], "colorbar-longest-length");
}

#[test]
fn legends_draw_the_constant_colors_of_their_marks() {
    // A size legend of red symbols shows red symbols.
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .size(Encoding::scaled_field("count"))
            .color(Encoding::direct_value("#e45756")),
    )
    .table(cars());
    let symbols = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
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
        LineMark::builder()
            .x(Encoding::scaled_field("year"))
            .y(Encoding::scaled_field("mpg"))
            .detail(Encoding::direct_field("origin"))
            .stroke_dash(Encoding::scaled_field("origin"))
            .color(Encoding::direct_value("#54a24b")),
    )
    .table(cars());
    let lines = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([lines])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&lines, "legend-constant-color-lines");
}

/// `display: none` on the node of a guide hides the whole guide.
#[test]
fn a_hidden_guide_draws_nothing() {
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(Encoding::scaled_field("origin")),
    )
    .table(cars());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();

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
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(Encoding::scaled_field("origin"))
            .size(Encoding::direct_value(100.0)),
    )
    .table(cars());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
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
        SymbolMark::builder()
            .x(Encoding::scaled_field("weight"))
            .y(Encoding::scaled_field("mpg"))
            .color(Encoding::scaled_field("origin")),
    )
    .table(cars());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();

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
