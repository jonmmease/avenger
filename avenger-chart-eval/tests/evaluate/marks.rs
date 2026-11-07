//! The 10 Cartesian marks, their texts and images, and their conditional values.

use std::sync::Arc;

use arrow::array::{RecordBatch, StringArray};
use avenger_chart::{
    build::{self},
    proto::{self, cartesian_layer},
    Chart,
};
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_scenegraph::marks::{mark::SceneMark, text::text_origin};

use crate::support::*;

#[test]
fn symbols() {
    let chart = cars_symbols(|_, mark| mark.color = Some(col("origin")));
    render_both(&chart, "symbols");
}

#[test]
fn symbols_with_a_size_and_a_shape() {
    let chart = cars_symbols(|_, mark| {
        mark.y = Some(scaled(
            "mpg",
            fitted(proto::IntervalFit {
                nice: Some(5.0),
                ..Default::default()
            }),
        ));
        mark.shape = Some(col("cylinders"));
        mark.size = Some(col("count"));
    });
    render_both(&chart, "symbols-size-shape");
}

#[test]
fn lines() {
    let chart = one(series(), |_| {
        cartesian_layer::Mark::Line(proto::LineMark {
            x: Some(col("date")),
            y: Some(col("value")),
            color: Some(col("symbol")),
            ..Default::default()
        })
    });
    render_both(&chart, "lines");
}

#[test]
fn trails() {
    let chart = one(series(), |_| {
        cartesian_layer::Mark::Trail(proto::TrailMark {
            x: Some(col("date")),
            y: Some(col("value")),
            color: Some(col("symbol")),
            size: Some(col("value")),
            ..Default::default()
        })
    });
    render_both(&chart, "trails");
}

#[test]
fn areas() {
    let chart = one(series(), |parts| {
        cartesian_layer::Mark::Area(proto::AreaMark {
            x: Some(col("date")),
            y: Some(zeroed("value")),
            color: Some(col("symbol")),
            opacity: Some(parts.number(0.6)),
            ..Default::default()
        })
    });
    render_both(&chart, "areas");
}

#[test]
fn bars() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Bar(proto::BarMark {
            x: Some(col("category")),
            y: Some(zeroed("amount")),
            ..Default::default()
        })
    });
    render_both(&chart, "bars");
}

#[test]
fn horizontal_bars() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Bar(proto::BarMark {
            x: Some(zeroed("amount")),
            y: Some(col("category")),
            color: Some(col("category")),
            ..Default::default()
        })
    });
    render_both(&chart, "bars-horizontal");
}

/// A histogram: rects from the start to the end of each bin, with an inset between bins.
#[test]
fn rects() {
    let starts: Vec<f64> = (0..8).map(|bin| bin as f64 * 5.0).collect();
    let ends: Vec<f64> = starts.iter().map(|start| start + 5.0).collect();
    let table = RecordBatch::try_from_iter([
        ("start", numbers(&starts)),
        ("end", numbers(&ends)),
        (
            "count",
            numbers(&[2.0, 5.0, 9.0, 14.0, 11.0, 7.0, 3.0, 1.0]),
        ),
    ])
    .unwrap();
    let chart = one(table, |parts| {
        cartesian_layer::Mark::Rect(proto::RectMark {
            inset: Some(1.0),
            x: Some(col("start")),
            x2: Some(raw("end")),
            y: Some(zeroed("count")),
            y2: Some(parts.scaled_number(0.0)),
            ..Default::default()
        })
    });
    render_both(&chart, "rects-histogram");
}

#[test]
fn a_rect_without_y_spans_the_height_of_its_band() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Rect(proto::RectMark {
            x: Some(col("category")),
            color: Some(col("amount")),
            ..Default::default()
        })
    });
    render_both(&chart, "rects-bands");
}

#[test]
fn rules() {
    let chart = one(cars(), |parts| {
        cartesian_layer::Mark::Rule(proto::RuleMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            y2: Some(parts.scaled_number(15.0)),
            color: Some(col("origin")),
            stroke_dash: Some(parts.text("4 2")),
            ..Default::default()
        })
    });
    render_both(&chart, "rules");
}

#[test]
fn texts() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Text(proto::TextMark {
            x: Some(col("category")),
            y: Some(col("amount")),
            text: Some(raw("amount")),
            color: Some(col("category")),
            ..Default::default()
        })
    });
    render_both(&chart, "texts");
}

#[test]
fn image_marks() {
    let chart = one(images(), |parts| {
        cartesian_layer::Mark::Image(proto::ImageMark {
            smooth: Some(false),
            x: Some(col("x")),
            y: Some(col("y")),
            image: Some(raw("image")),
            width: Some(parts.number(24.0)),
            height: Some(parts.number(24.0)),
            ..Default::default()
        })
    });
    render_both(&chart, "images");
}

#[test]
fn images_with_sizes_from_columns() {
    let chart = one(images(), |_| {
        cartesian_layer::Mark::Image(proto::ImageMark {
            smooth: Some(false),
            x: Some(col("x")),
            y: Some(col("y")),
            image: Some(raw("image")),
            width: Some(raw("size")),
            height: Some(raw("size")),
            ..Default::default()
        })
    });
    render_both(&chart, "images-sizes");
}

/// Texts of the labels' names at their points.
fn label_texts(mark: impl FnOnce(&mut Parts, &mut proto::TextMark)) -> Chart {
    one(labels(), |parts| {
        let mut text = proto::TextMark {
            x: Some(col("x")),
            y: Some(col("y")),
            text: Some(raw("name")),
            ..Default::default()
        };
        mark(parts, &mut text);
        cartesian_layer::Mark::Text(text)
    })
}

#[test]
fn texts_with_weights_and_styles_from_columns() {
    let chart = label_texts(|_, mark| {
        mark.font_weight = Some(raw("weight"));
        mark.font_style = Some(raw("style"));
    });
    render_both(&chart, "texts-fonts");
}

#[test]
fn texts_wrap_at_their_widths() {
    let wrapped = label_texts(|_, mark| mark.width = Some(raw("width")));
    render_both(&wrapped, "texts-wrapped");
    // A fixed width, and one line with an ellipsis where the width cuts it.
    let cut = label_texts(|parts, mark| {
        mark.width_mode = proto::TextWidthMode::Fixed as i32;
        mark.max_lines = Some(1);
        mark.ellipsis = Some(true);
        mark.width = Some(parts.number(70.0));
    });
    render_both(&cut, "texts-cut");
}

#[test]
fn a_wrapped_label_at_the_edge_stays_inside_its_plot() {
    let chart = label_texts(|parts, mark| mark.width = Some(parts.number(90.0)));
    let engine = engine();
    let mut evaluator = evaluator(&chart);
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluator.evaluate(&host(size)).unwrap();
        let [_, _, width, height] = evaluated.plots[0].rect;
        let text = (plot_group(&evaluated).marks.iter())
            .find_map(|mark| match mark {
                SceneMark::Text(text) => Some(text),
                _ => None,
            })
            .expect("a text mark");
        for label in text.labels() {
            let bounds = engine.bounds(&label.label).unwrap();
            let [x, y] = text_origin(&bounds, label.position, label.align, label.baseline);
            assert!(
                x >= -0.5 && x + bounds.width <= width + 0.5,
                "{x} + {} in {width}",
                bounds.width
            );
            assert!(
                y >= -0.5 && y + bounds.height <= height + 0.5,
                "{y} + {} in {height}",
                bounds.height
            );
        }
    }
}

#[test]
fn paths() {
    let chart = one(cars(), |parts| {
        cartesian_layer::Mark::Path(proto::PathMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            path: Some(parts.text("M -8 -4 L 4 -4 L 4 -8 L 12 0 L 4 8 L 4 4 L -8 4 Z")),
            angle: Some(raw("count")),
            color: Some(col("origin")),
            ..Default::default()
        })
    });
    render_both(&chart, "paths");
}

/// The path of a row is the first conditional value that selects it.
#[test]
fn a_conditional_path_draws_for_the_rows_that_it_selects() {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let line = parts.text("M 0 0 L 8 0");
    let arrow = parts.value(Arc::new(StringArray::from(vec![
        "M 0 0 L 8 0 L 8 8 L 0 8 Z",
    ])));
    let initial = vec![weight_range(&mut parts, "plot", 2500.0, 3500.0)];
    let path = proto::Encoding {
        conditional: vec![proto::ConditionalValue {
            predicate: Some(selected(brush_of("brush"))),
            value: Some(proto::conditional_value::Value::Constant(arrow)),
        }],
        ..line
    };
    let mut root = build::cartesian(vec![layer(
        table,
        cartesian_layer::Mark::Path(proto::PathMark {
            x: Some(col("weight")),
            y: Some(col("mpg")),
            path: Some(path),
            ..Default::default()
        }),
    )]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.id = "plot".to_string();
        plot.tools = vec![proto::Tool {
            kind: Some(proto::tool::Kind::Select(select("brush"))),
        }];
    }
    let brush = proto::Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Union as i32,
        initial,
    };
    let evaluated = evaluate(&parts.chart(root, vec![brush]), [320.0, 240.0]);
    let paths = (plot_group(&evaluated).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Path(path) => Some(path.path.as_vec(path.len as usize, None)),
            _ => None,
        })
        .expect("a path mark");
    for (path, weight) in paths.iter().zip(weights()) {
        let events = path.iter().count();
        let inside = (2500.0..=3500.0).contains(&weight);
        assert_eq!(events > 3, inside, "{weight}: {events} events");
    }
}

/// Under Typst syntax, a formatted value is plain text, so its `$` does not open math.
#[test]
fn a_formatted_value_under_typst_syntax_is_escaped() {
    let chart = one(categories(), |_| {
        cartesian_layer::Mark::Text(proto::TextMark {
            x: Some(col("category")),
            y: Some(col("amount")),
            text: Some(raw("amount")),
            format: "$,.2f".to_string(),
            syntax: proto::TextSyntax::Typst as i32,
            ..Default::default()
        })
    });
    let text = (plot_group(&evaluate(&chart, [320.0, 240.0])).marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Text(text) => Some(text.text.as_vec(text.len as usize, None)),
            _ => None,
        })
        .expect("a text mark");
    assert!(text[0].contains("\\$"), "{text:?}");
}

/// An `auto` size is an inline image's own size in pixels, one `auto` dimension follows the
/// image's aspect, and a URL image, whose size is unknown before it loads, warns.
#[test]
fn an_auto_image_size_is_the_images_own_size() {
    let chart = one(images(), |_| {
        cartesian_layer::Mark::Image(proto::ImageMark {
            x: Some(col("x")),
            y: Some(col("y")),
            image: Some(raw("image")),
            ..Default::default()
        })
    });
    let styled = |chart: &Chart, css: &str| {
        let mut evaluator =
            ChartEvaluator::new(chart.clone(), cascade_with(css), engine(), Registry::new())
                .unwrap();
        evaluator.evaluate(&host([320.0, 240.0])).unwrap()
    };
    let own = styled(&chart, "mark[type=image] { width: auto; height: auto; }");
    assert!(image_sizes(&own).iter().all(|size| *size == (4.0, 4.0)));
    let wide = styled(&chart, "mark[type=image] { width: 20; height: auto; }");
    assert!(image_sizes(&wide).iter().all(|size| *size == (20.0, 20.0)));
    let urls = one(images(), |parts| {
        cartesian_layer::Mark::Image(proto::ImageMark {
            x: Some(col("x")),
            y: Some(col("y")),
            image: Some(parts.text("https://example.com/icon.png")),
            ..Default::default()
        })
    });
    let unknown = styled(&urls, "mark[type=image] { width: auto; }");
    assert!(
        warns(&unknown, "`auto` size of a URL image"),
        "{:?}",
        unknown.warnings
    );
}
