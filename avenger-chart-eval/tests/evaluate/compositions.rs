//! Charts of several plots: concats, wraps, empty cells, facets and their headers, tracks and plot
//! sizes, the chart's padding, guide rules, merged legends, shared scales across plots, and the
//! titles of compositions.

use arrow::array::RecordBatch;
use avenger_chart::prelude::*;

use crate::support::*;

/// The number of the scene's texts that are `text`.
fn count(chart: &Chart, size: [f32; 2], text: &str) -> usize {
    let evaluated = evaluate(chart, size);
    (scene_texts(&evaluated.scene.marks).iter())
        .filter(|known| *known == text)
        .count()
}

#[test]
fn a_row_of_plots_shares_its_color_legend() {
    let weights = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column("weight"))
            .y(Encoding::scaled_column("mpg"))
            .color(Encoding::scaled_column("origin")),
    )
    .table(cars());
    let counts = CartesianLayer::symbol(
        SymbolMark::builder()
            .x(Encoding::scaled_column("count"))
            .y(Encoding::scaled_column("mpg"))
            .color(Encoding::scaled_column("origin")),
    )
    .table(cars());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Concat::builder().direction(Direction::Row).children([
            Plot::cartesian(CartesianPlot::builder().layers([weights])),
            Plot::cartesian(CartesianPlot::builder().layers([counts])),
        ]))
        .infer_scales()
        .build()
        .unwrap();
    render(&chart, [640.0, 300.0], "concat-row");

    // The color scale is shared, and the concat merges its legend into one.
    assert_eq!(count(&chart, [640.0, 300.0], "origin"), 1);
    let evaluated = evaluate(&chart, [640.0, 300.0]);
    assert_eq!(evaluated.plots.len(), 2);
    let [left, right] = [evaluated.plots[0].rect, evaluated.plots[1].rect];
    assert!(left[0] + left[2] < right[0], "{left:?} {right:?}");
    // Each plot fits its own `x`.
    assert_ne!(domain(&evaluated, 0), {
        let x = evaluated.plots[1].coords.scale(0).unwrap();
        x.numeric_interval_domain().unwrap()
    });
}

#[test]
fn a_column_with_a_shared_x_draws_one_x_axis() {
    let plot = |y: &str| {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_column("weight"))
                .y(Encoding::scaled_column(y)),
        )
        .table(cars());
        Plot::cartesian(CartesianPlot::builder().layers([symbols]))
    };
    let column = |rules: Vec<GuideRuleBuilder>| {
        Chart::builder()
            .number_format_syntax("d3")
            .datetime_format_syntax("d3")
            .root(
                Concat::builder()
                    .direction(Direction::Column)
                    .resolve([Resolve::channel(Channel::X).sharing(Sharing::Shared)])
                    .layout(ConcatLayout::builder().guide_rules(rules))
                    .children([plot("mpg"), plot("count")]),
            )
            .infer_scales()
            .build()
            .unwrap()
    };
    let collapsed = column(Vec::new());
    render(&collapsed, [360.0, 420.0], "concat-column-shared-x");
    // The axes of the shared `x` collapse into one, below the column, with one title.
    assert_eq!(count(&collapsed, [360.0, 420.0], "weight"), 1);
    let evaluated = evaluate(&collapsed, [360.0, 420.0]);
    let [top, bottom] = [evaluated.plots[0].rect, evaluated.plots[1].rect];
    assert_eq!(top[2], bottom[2], "the plots of a shared x have one width");

    // A rule that keeps the ticks and the labels draws them beside each plot.
    let kept = column(vec![
        GuideRule::builder()
            .part(GuidePart::AxisLabels)
            .rule(GuideRuleKind::Keep),
        GuideRule::builder()
            .part(GuidePart::AxisTicks)
            .rule(GuideRuleKind::Keep),
        GuideRule::builder()
            .part(GuidePart::AxisTitle)
            .rule(GuideRuleKind::Keep),
    ]);
    render(&kept, [360.0, 420.0], "concat-column-kept-axes");
    assert_eq!(count(&kept, [360.0, 420.0], "weight"), 2);
}

#[test]
fn a_facet_grid_of_nested_partitions_labels_its_rows_and_columns() {
    // A column that facets the origins, of rows that facet the cylinder counts.
    let rows = Node::partitions(&cars(), "origin", PartitionOrder::Ascending, |origin| {
        let cells = Node::partitions(
            &origin,
            "cylinders",
            PartitionOrder::Ascending,
            |cell: RecordBatch| {
                let symbols = CartesianLayer::symbol(
                    SymbolMark::builder()
                        .x(Encoding::scaled_column("weight"))
                        .y(Encoding::scaled_column("mpg")),
                )
                .table(cell);
                Ok(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
            },
        )?;
        Ok(Concat::builder()
            .direction(Direction::Row)
            .facet(Facet::builder().column("cylinders"))
            .children(cells))
    })
    .unwrap();
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(
            Concat::builder()
                .direction(Direction::Column)
                .facet(Facet::builder().column("origin"))
                .children(rows),
        )
        .infer_scales()
        .build()
        .unwrap();
    render(&chart, [640.0, 520.0], "facet-grid");

    // Each origin labels its row once, and the columns' labels collapse into one header.
    for origin in ["Europe", "Japan", "USA"] {
        assert_eq!(count(&chart, [640.0, 520.0], origin), 1, "{origin}");
    }
    assert_eq!(count(&chart, [640.0, 520.0], "cylinders"), 1);
    assert_eq!(count(&chart, [640.0, 520.0], "origin"), 1);
}

#[test]
fn a_wrapped_facet_labels_each_panel() {
    let panels = Node::partitions(&cars(), "origin", PartitionOrder::Ascending, |rows| {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_column("weight"))
                .y(Encoding::scaled_column("mpg")),
        )
        .table(rows);
        Ok(Plot::cartesian(CartesianPlot::builder().layers([symbols])))
    })
    .unwrap();
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(
            Wrap::builder()
                .facet(Facet::builder().column("origin"))
                .children(panels),
        )
        .infer_scales()
        .build()
        .unwrap();
    render(&chart, [560.0, 460.0], "facet-wrap");

    // Each panel has its label, and the wrap merges the headers' titles into one.
    for origin in ["Europe", "Japan", "USA"] {
        assert_eq!(count(&chart, [560.0, 460.0], origin), 1, "{origin}");
    }
    assert_eq!(count(&chart, [560.0, 460.0], "origin"), 1);
    // The wrap reflows: a narrow chart stands its panels in more rows.
    let wide = evaluate(&chart, [560.0, 460.0]);
    let narrow = evaluate(&chart, [300.0, 700.0]);
    let rows = |evaluated: &avenger_chart_eval::EvaluatedChart| {
        let mut tops: Vec<f32> = (evaluated.plots.iter()).map(|plot| plot.rect[1]).collect();
        tops.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        tops.len()
    };
    assert!(
        rows(&narrow) > rows(&wide),
        "{} {}",
        rows(&narrow),
        rows(&wide)
    );
}

#[test]
fn an_empty_cell_keeps_its_track() {
    let plot = || {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_column("weight"))
                .y(Encoding::scaled_column("mpg")),
        )
        .table(cars());
        Plot::cartesian(CartesianPlot::builder().layers([symbols]))
    };
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Concat::builder().direction(Direction::Row).children([
            NodeBuilder::from(plot()),
            Empty::builder().into(),
            plot().into(),
        ]))
        .infer_scales()
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [600.0, 240.0]);
    let [first, second] = [evaluated.plots[0].rect, evaluated.plots[1].rect];
    // The empty middle cell keeps a track as wide as a plot's.
    let gap = second[0] - (first[0] + first[2]);
    assert!(gap > first[2] * 0.5, "{first:?} {second:?}");
}

#[test]
fn a_plot_width_fixes_its_column_and_the_padding_surrounds_the_chart() {
    let plot = |width: Option<f32>| {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_column("weight"))
                .y(Encoding::scaled_column("mpg")),
        )
        .table(cars());
        let plot = Plot::cartesian(CartesianPlot::builder().layers([symbols]));
        match width {
            Some(width) => plot.width(width),
            None => plot,
        }
    };
    let chart = |padding: f32| {
        Chart::builder()
            .number_format_syntax("d3")
            .datetime_format_syntax("d3")
            .padding(
                Padding::builder()
                    .top(padding)
                    .right(padding)
                    .bottom(padding)
                    .left(padding),
            )
            .root(
                Concat::builder()
                    .direction(Direction::Row)
                    .children([plot(Some(120.0)), plot(None)]),
            )
            .infer_scales()
            .build()
            .unwrap()
    };
    let evaluated = evaluate(&chart(5.0), [640.0, 300.0]);
    assert!((evaluated.plots[0].rect[2] - 120.0).abs() < 0.5);
    assert!(evaluated.plots[1].rect[2] > 120.0);
    // A wider padding moves the plots in and leaves the scene's size alone.
    let padded = evaluate(&chart(40.0), [640.0, 300.0]);
    assert_eq!(padded.scene.width, 640.0);
    assert!(padded.plots[0].rect[1] > evaluated.plots[0].rect[1] + 30.0);
}

#[test]
fn titles_of_compositions_run_along_their_side() {
    let plot = |title: &str| {
        let symbols = CartesianLayer::symbol(
            SymbolMark::builder()
                .x(Encoding::scaled_column("weight"))
                .y(Encoding::scaled_column("mpg"))
                .color(Encoding::scaled_column("origin")),
        )
        .table(cars());
        Plot::cartesian(CartesianPlot::builder().layers([symbols]))
            .title(Title::builder().text(title))
    };
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(
            Concat::builder()
                .direction(Direction::Row)
                .title(
                    Title::builder()
                        .text("Fuel economy")
                        .subtitle("By weight, in two panels"),
                )
                .children([plot("Left"), plot("Right")]),
        )
        .infer_scales()
        .build()
        .unwrap();
    render(&chart, [640.0, 320.0], "concat-titles");
    for title in ["Fuel economy", "Left", "Right"] {
        assert_eq!(count(&chart, [640.0, 320.0], title), 1, "{title}");
    }
}
