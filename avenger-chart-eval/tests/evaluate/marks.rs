//! The 10 Cartesian marks, their texts and images, and their conditional values.

use arrow::array::RecordBatch;
use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_scenegraph::marks::{mark::SceneMark, text::text_origin};

use crate::support::*;

#[test]
fn symbols() {
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg"))
            .color(Encoding::scaled_column("origin")),
    )
    .table(cars());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "symbols");
}

#[test]
fn symbols_with_a_size_and_a_shape() {
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg").scale(
                Scale::builder()
                    .domain(DataDomain::builder().fit(IntervalFit::builder().nice(5.0))),
            ))
            .shape(Encoding::scaled_column("cylinders"))
            .size(Encoding::scaled_column("count")),
    )
    .table(cars());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "symbols-size-shape");
}

#[test]
fn lines() {
    let lines = CartesianLayer::line(
        LineMark::builder()
            .x(Encoding::scaled_column("date"))
            .y(Encoding::scaled_column("value"))
            .color(Encoding::scaled_column("symbol")),
    )
    .table(series());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([lines])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "lines");
}

#[test]
fn trails() {
    let trails = CartesianLayer::trail(
        TrailMark::builder()
            .x(Encoding::scaled_column("date"))
            .y(Encoding::scaled_column("value"))
            .color(Encoding::scaled_column("symbol"))
            .size(Encoding::scaled_column("value")),
    )
    .table(series());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([trails])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "trails");
}

#[test]
fn areas() {
    let areas = CartesianLayer::area(
        AreaMark::builder()
            .x(Encoding::scaled_column("date"))
            .y(Encoding::scaled_column("value").scale(
                Scale::builder()
                    .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true))),
            ))
            .color(Encoding::scaled_column("symbol"))
            .opacity(Encoding::constant(0.6)),
    )
    .table(series());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([areas])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "areas");
}

#[test]
fn bars() {
    let bars = CartesianLayer::bar(BarMark::builder().x(Encoding::scaled_column("category")).y(
        Encoding::scaled_column("amount").scale(
            Scale::builder().domain(DataDomain::builder().fit(IntervalFit::builder().zero(true))),
        ),
    ))
    .table(categories());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([bars])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "bars");
}

#[test]
fn horizontal_bars() {
    let bars = CartesianLayer::bar(
        BarMark::builder()
            .x(Encoding::scaled_column("amount").scale(
                Scale::builder()
                    .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true))),
            ))
            .y(Encoding::scaled_column("category"))
            .color(Encoding::scaled_column("category")),
    )
    .table(categories());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([bars])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "bars-horizontal");
}

/// A histogram: rects from the start to the end of each bin, with an inset between bins.
#[test]
fn rects() {
    let starts: Vec<f64> = (0..8).map(|bin| bin as f64 * 5.0).collect();
    let ends: Vec<f64> = starts.iter().map(|start| start + 5.0).collect();
    let bins = RecordBatch::try_from_iter([
        ("start", numbers(&starts)),
        ("end", numbers(&ends)),
        (
            "count",
            numbers(&[2.0, 5.0, 9.0, 14.0, 11.0, 7.0, 3.0, 1.0]),
        ),
    ])
    .unwrap();
    let rects = CartesianLayer::rect(
        RectMark::builder()
            .inset(1.0)
            .x(Encoding::scaled_column("start"))
            .x2(Encoding::scaled_column("end"))
            .y(Encoding::scaled_column("count").scale(
                Scale::builder()
                    .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true))),
            ))
            .y2(Encoding::scaled_constant(0.0)),
    )
    .table(bins);
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([rects])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "rects-histogram");
}

#[test]
fn a_rect_without_y_spans_the_height_of_its_band() {
    let rects = CartesianLayer::rect(
        RectMark::builder()
            .x(Encoding::scaled_column("category"))
            .color(Encoding::scaled_column("amount")),
    )
    .table(categories());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([rects])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "rects-bands");
}

#[test]
fn rules() {
    let rules = CartesianLayer::rule(
        RuleMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg"))
            .y2(Encoding::scaled_constant(15.0))
            .color(Encoding::scaled_column("origin"))
            .stroke_dash(Encoding::constant("4 2")),
    )
    .table(cars());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([rules])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "rules");
}

#[test]
fn texts() {
    let texts = CartesianLayer::text(
        TextMark::builder()
            .x(Encoding::scaled_column("category"))
            .y(Encoding::scaled_column("amount"))
            .text(Encoding::column("amount"))
            .color(Encoding::scaled_column("category")),
    )
    .table(categories());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([texts])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "texts");
}

#[test]
fn image_marks() {
    let icons = CartesianLayer::image(
        ImageMark::builder()
            .smooth(false)
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .image(Encoding::column("image"))
            .width(Encoding::constant(24.0))
            .height(Encoding::constant(24.0)),
    )
    .table(images());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([icons])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "images");
}

#[test]
fn images_with_sizes_from_columns() {
    let icons = CartesianLayer::image(
        ImageMark::builder()
            .smooth(false)
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .image(Encoding::column("image"))
            .width(Encoding::column("size"))
            .height(Encoding::column("size")),
    )
    .table(images());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([icons])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "images-sizes");
}

#[test]
fn texts_with_weights_and_styles_from_columns() {
    let texts = CartesianLayer::text(
        TextMark::builder()
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .text(Encoding::column("name"))
            .font_weight(Encoding::column("weight"))
            .font_style(Encoding::column("style")),
    )
    .table(labels());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([texts])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "texts-fonts");
}

#[test]
fn texts_wrap_at_their_widths() {
    let texts = CartesianLayer::text(
        TextMark::builder()
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .text(Encoding::column("name"))
            .width(Encoding::column("width")),
    )
    .table(labels());
    let wrapped = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([texts])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&wrapped, "texts-wrapped");

    // A fixed width, and one line with an ellipsis where the width cuts it.
    let texts = CartesianLayer::text(
        TextMark::builder()
            .width_mode(TextWidthMode::Fixed)
            .max_lines(1)
            .ellipsis(true)
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .text(Encoding::column("name"))
            .width(Encoding::constant(70.0)),
    )
    .table(labels());
    let cut = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([texts])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&cut, "texts-cut");
}

#[test]
fn a_wrapped_label_at_the_edge_stays_inside_its_plot() {
    let texts = CartesianLayer::text(
        TextMark::builder()
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .text(Encoding::column("name"))
            .width(Encoding::constant(90.0)),
    )
    .table(labels());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([texts])))
        .infer_scales()
        .build()
        .unwrap();

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
    let paths = CartesianLayer::path(
        PathMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg"))
            .path(Encoding::constant(
                "M -8 -4 L 4 -4 L 4 -8 L 12 0 L 4 8 L 4 4 L -8 4 Z",
            ))
            .angle(Encoding::column("count"))
            .color(Encoding::scaled_column("origin")),
    )
    .table(cars());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([paths])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "paths");
}

/// The path of a row is the first conditional value that selects it.
#[test]
fn a_conditional_path_draws_for_the_rows_that_it_selects() {
    // A line, and an arrow for the rows that the brush selects.
    let path = Encoding::constant("M 0 0 L 8 0")
        .conditional([ConditionalValue::constant("M 0 0 L 8 0 L 8 8 L 0 8 Z")
            .predicate(Predicate::selected(Selected::builder().selection("brush")))]);
    let paths = CartesianLayer::path(
        PathMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg"))
            .path(path),
    )
    .table(cars());
    let plot = Plot::cartesian(CartesianPlot::builder().layers([paths]))
        .id("plot")
        .tools([Select::interval(IntervalSelect::builder()).selection("brush")]);
    // The brush starts with the weights from 2500 to 3500.
    let brush = Selection::builder()
        .name("brush")
        .resolution(SelectionResolution::Union)
        .initial([Clause::builder()
            .owner("plot")
            .conditions([ValueRange::builder()
                .column("weight")
                .channel(Channel::X)
                .low(2500.0)
                .high(3500.0)])]);
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(plot)
        .selections([brush])
        .infer_scales()
        .build()
        .unwrap();

    let evaluated = evaluate(&chart, [320.0, 240.0]);
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
    let texts = CartesianLayer::text(
        TextMark::builder()
            .x(Encoding::scaled_column("category"))
            .y(Encoding::scaled_column("amount"))
            .text(Encoding::column("amount"))
            .format("$,.2f")
            .syntax(TextSyntax::Typst),
    )
    .table(categories());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([texts])))
        .infer_scales()
        .build()
        .unwrap();

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
    let styled = |chart: &Chart, css: &str| {
        let mut evaluator =
            ChartEvaluator::new(chart.clone(), cascade_with(css), engine(), Registry::new())
                .unwrap();
        evaluator.evaluate(&host([320.0, 240.0])).unwrap()
    };

    let icons = CartesianLayer::image(
        ImageMark::builder()
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .image(Encoding::column("image")),
    )
    .table(images());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([icons])))
        .infer_scales()
        .build()
        .unwrap();
    let own = styled(&chart, "mark[type=image] { width: auto; height: auto; }");
    assert!(image_sizes(&own).iter().all(|size| *size == (4.0, 4.0)));
    let wide = styled(&chart, "mark[type=image] { width: 20; height: auto; }");
    assert!(image_sizes(&wide).iter().all(|size| *size == (20.0, 20.0)));

    let urls = CartesianLayer::image(
        ImageMark::builder()
            .x(Encoding::scaled_column("x"))
            .y(Encoding::scaled_column("y"))
            .image(Encoding::constant("https://example.com/icon.png")),
    )
    .table(images());
    let urls = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::cartesian(CartesianPlot::builder().layers([urls])))
        .infer_scales()
        .build()
        .unwrap();
    let unknown = styled(&urls, "mark[type=image] { width: auto; }");
    assert!(
        warns(&unknown, "`auto` size of a URL image"),
        "{:?}",
        unknown.warnings
    );
}
