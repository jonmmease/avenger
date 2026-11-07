//! Titles, attributions, axes, legends, and colorbars, and the styles that hide and paint them.

use avenger_chart::{
    build::{self},
    proto::{self, cartesian_layer, guide, guide_ticks, scale_spec},
    Chart,
};
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, Registry};
use avenger_color::ColorOrGradient;

use crate::support::*;

/// A scatter of the cars colored by origin, with the plot's title and attribution.
fn titled(title: proto::Title, attribution: Option<proto::Attribution>) -> Chart {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let mut root = build::cartesian(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            color: Some(col("origin")),
            ..Default::default()
        }),
    )]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.title = Some(title);
        plot.attribution = attribution;
    }
    parts.chart(root, Vec::new())
}

#[test]
fn titles_and_attributions() {
    let chart = titled(
        proto::Title {
            text: "Fuel economy".into(),
            subtitle: "By weight and origin".into(),
            align: proto::TitleAlign::Start as i32,
            ..Default::default()
        },
        Some(proto::Attribution {
            credits: vec!["Source: StatLib".into()],
            ..Default::default()
        }),
    );
    render_both(&chart, "title-subtitle-attribution");
    let caption = titled(
        proto::Title {
            text: "Figure 1: heavier cars travel fewer miles on a gallon.".into(),
            side: proto::TitleSide::Bottom as i32,
            ..Default::default()
        },
        None,
    );
    render_both(&caption, "caption");
}

#[test]
fn legends() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let thresholds = parts.list(numbers(&[20.0, 30.0]));
    let threshold = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            color: Some(scaled(
                "mpg",
                proto::ScaleSpec {
                    r#type: Some(scale_spec::Type::Threshold(proto::ThresholdScale::default())),
                    domain: Some(explicit(thresholds)),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }),
    )]);
    render_both(&threshold, "legend-threshold");
    let merged = cars_symbols(|_, mark| {
        mark.color = Some(col("origin"));
        mark.shape = Some(col("origin"));
    });
    render_both(&merged, "legend-color-shape");
}

#[test]
fn an_axis_with_explicit_ticks() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let ticks = parts.list(numbers(&[2000.0, 3000.0, 4000.0]));
    let axis = proto::Axis {
        ticks: Some(proto::GuideTicks {
            at: Some(guide_ticks::At::Values(ticks)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let chart = parts.plot(vec![layer(
        table,
        cartesian_layer::Mark::Symbol(proto::SymbolMark {
            x: Some(scaled(
                "weight",
                proto::ScaleSpec {
                    guide: Some(proto::Guide {
                        guide: Some(guide::Guide::Axis(axis)),
                    }),
                    ..Default::default()
                },
            )),
            y: Some(col("mpg")),
            ..Default::default()
        }),
    )]);
    render_both(&chart, "axis-explicit-ticks");
}

#[test]
fn a_colorbar_with_a_longest_length_beside_a_tall_plot() {
    let chart = cars_symbols(|_, mark| {
        mark.color = Some(scaled(
            "mpg",
            proto::ScaleSpec {
                guide: Some(proto::Guide {
                    guide: Some(guide::Guide::Colorbar(proto::Colorbar {
                        max_length: Some(120.0),
                        ..Default::default()
                    })),
                }),
                ..Default::default()
            },
        ));
    });
    render(&chart, [320.0, 480.0], "colorbar-longest-length");
}

#[test]
fn legends_draw_the_constant_colors_of_their_marks() {
    // A size legend of red symbols shows red symbols.
    let symbols = cars_symbols(|parts, mark| {
        mark.size = Some(col("count"));
        mark.color = Some(parts.text("#e45756"));
    });
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
    let lines = one(cars(), |parts| {
        cartesian_layer::Mark::Line(proto::LineMark {
            x: Some(col("year")),
            y: Some(col("mpg")),
            detail: Some(raw("origin")),
            stroke_dash: Some(col("origin")),
            color: Some(parts.text("#54a24b")),
            ..Default::default()
        })
    });
    render_both(&lines, "legend-constant-color-lines");
}

/// `display: none` on the node of a guide hides the whole guide.
#[test]
fn a_hidden_guide_draws_nothing() {
    let chart = cars_symbols(|_, mark| mark.color = Some(col("origin")));
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

/// A `color-mix()` of `currentColor` tints the color of each row, in the marks and in the swatches
/// of their legend.
#[test]
fn a_mix_of_current_color_tints_each_rows_color() {
    let chart = cars_symbols(|_, mark| mark.color = Some(col("origin")));
    let css = "mark[type=symbol] { fill: color-mix(in srgb, currentColor 50%, white); }";
    let mut tinted =
        ChartEvaluator::new(chart.clone(), cascade_with(css), engine(), Registry::new()).unwrap();
    let tinted = tinted.evaluate(&host([320.0, 240.0])).unwrap();
    let plain = evaluate(&chart, [320.0, 240.0]);
    let fills = |evaluated: &EvaluatedChart| {
        let symbol = plot_symbols(evaluated);
        symbol.fill.as_vec(symbol.len as usize, None)
    };
    let tint = |color: &ColorOrGradient| match color {
        ColorOrGradient::Color([r, g, b, a]) => {
            [(r + 1.0) / 2.0, (g + 1.0) / 2.0, (b + 1.0) / 2.0, *a]
        }
        other => panic!("a color, not {other:?}"),
    };
    let close = |a: &ColorOrGradient, b: [f32; 4]| match a {
        ColorOrGradient::Color(a) => a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3),
        _ => false,
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
