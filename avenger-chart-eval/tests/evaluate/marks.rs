//! The 10 Cartesian marks, their texts and images, and their conditional values.

use std::sync::Arc;

use arrow::array::RecordBatch;
use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, Registry};
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
    // The text's pattern is a pattern of numbers, so the chart names its syntax.
    let chart = Chart::builder()
        .number_format_syntax("d3")
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

/// An ordinal range of dashes as Arrow lists draws those dashes, one for each series.
#[test]
fn a_range_of_dash_lists_draws_its_dashes() {
    use arrow::{
        array::{ArrayRef, ListArray},
        datatypes::Float64Type,
    };
    let dashes: ArrayRef = Arc::new(ListArray::from_iter_primitive::<Float64Type, _, _>([
        Some(vec![Some(1.0), Some(2.0)]),
        Some(vec![Some(4.0), Some(5.0)]),
        Some(vec![Some(6.0), Some(1.0)]),
    ]));
    let lines = CartesianLayer::line(
        LineMark::builder()
            .x(Encoding::scaled_column("date"))
            .y(Encoding::scaled_column("value"))
            .detail(Encoding::column("symbol"))
            .stroke_dash(
                Encoding::scaled_column("symbol")
                    .scale(Scale::builder().range(Range::values(Value::list(dashes)))),
            ),
    )
    .table(series());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([lines])))
        .infer_scales()
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let dashes: Vec<Option<Vec<f32>>> = (plot_group(&evaluated).marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Line(line) => Some(line.stroke_dash.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        dashes,
        [
            Some(vec![1.0, 2.0]),
            Some(vec![4.0, 5.0]),
            Some(vec![6.0, 1.0])
        ]
    );
}

/// A range that only reverses draws the stylesheet's palette in reverse.
#[test]
fn a_range_that_only_reverses_reverses_the_stylesheets_palette() {
    let chart = |reverse: bool| {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_column("weight"))
                .y(Encoding::scaled_column("mpg"))
                .color(
                    Encoding::scaled_column("origin")
                        .scale(Scale::builder().range(Range::builder().reverse(reverse))),
                ),
        )
        .table(cars());
        Chart::builder()
            .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
            .infer_scales()
            .build()
            .unwrap()
    };
    // The legend's colors, each once, in the order of its entries.
    let colors = |reverse| {
        let mut colors = Vec::new();
        for color in legend_symbol_fills(&evaluate(&chart(reverse), [320.0, 240.0])) {
            if !colors.contains(&color) {
                colors.push(color);
            }
        }
        colors
    };
    let mut forward = colors(false);
    let reversed = colors(true);
    assert_eq!(forward.len(), 3);
    forward.reverse();
    assert_eq!(forward, reversed);
}

/// A layer whose mark computes `display: none` draws no marks and has no rows, and its values
/// still fit the scales, so the axes stay as they are.
#[test]
fn a_hidden_layer_draws_nothing_and_keeps_its_domains() {
    let symbols = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg")),
    )
    .table(cars());
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
        .infer_scales()
        .build()
        .unwrap();
    let mut evaluator = ChartEvaluator::new(
        chart.clone(),
        cascade_with("mark { display: none; }"),
        engine(),
        Registry::new(),
    )
    .unwrap();
    let hidden = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    let shown = evaluate(&chart, [320.0, 240.0]);
    assert!(hidden.plots[0].layers[0].marks.is_empty());
    assert!(!(plot_group(&hidden).marks.iter()).any(|mark| matches!(mark, SceneMark::Symbol(_))));
    let pixels = |evaluated: &EvaluatedChart| {
        let [x, y] = evaluated.plots[0]
            .coords
            .pixels([&numbers(&[2000.0, 4000.0]), &numbers(&[20.0, 30.0])])
            .unwrap();
        (x, y)
    };
    assert_eq!(pixels(&hidden), pixels(&shown));
}

/// An image that spans its four coordinates asks for the size that it draws at, whatever its
/// width and height styles, which do not apply to it.
#[test]
fn a_spanning_image_asks_for_the_size_that_it_draws_at() {
    use avenger_scenegraph::marks::image::SceneImageSource;
    let table = RecordBatch::try_from_iter([
        ("x", numbers(&[0.0])),
        ("x2", numbers(&[10.0])),
        ("y", numbers(&[0.0])),
        ("y2", numbers(&[10.0])),
    ])
    .unwrap();
    let images = CartesianLayer::image(
        ImageMark::builder()
            .x(Encoding::scaled_column("x"))
            .x2(Encoding::scaled_column("x2"))
            .y(Encoding::scaled_column("y"))
            .y2(Encoding::scaled_column("y2"))
            .image(Encoding::constant("https://example.com/icon.png")),
    )
    .table(table);
    let chart = Chart::builder()
        .root(Plot::cartesian(CartesianPlot::builder().layers([images])))
        .infer_scales()
        .build()
        .unwrap();
    let image = |css: &str| {
        let mut evaluator =
            ChartEvaluator::new(chart.clone(), cascade_with(css), engine(), Registry::new())
                .unwrap();
        let evaluated = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
        assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
        let image = (plot_group(&evaluated).marks.iter())
            .find_map(|mark| match mark {
                SceneMark::Image(image) => Some(image.clone()),
                _ => None,
            })
            .expect("an image mark");
        let SceneImageSource::Resource(resource) = &image.image.as_vec(1, None)[0] else {
            panic!("a URL resource");
        };
        let [width, height] = [
            image.width.as_vec(1, None)[0],
            image.height.as_vec(1, None)[0],
        ];
        ([resource.width, resource.height], [width, height])
    };
    let (wide, drawn) = image("mark[type=image] { width: 100; height: 10; }");
    let (tall, _) = image("mark[type=image] { width: 10; height: 100; }");
    assert_eq!(wide, tall);
    assert_eq!(wide, drawn.map(|size| size.round() as u32));
}
