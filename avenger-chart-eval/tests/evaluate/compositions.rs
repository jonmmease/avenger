//! Charts of several plots: concats, wraps, empty cells, facets and their titles and labels, tracks
//! and plot sizes, the chart's padding, part rules, merged legends, shared scales across plots, and
//! the titles of compositions, on the cars dataset.

use std::sync::Arc;

use arrow::array::{ArrayRef, Int64Array, RecordBatch};
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
        cars_dataset(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    );
    let powers = CartesianLayer::symbol(
        cars_dataset(),
        SymbolMark::builder()
            .x("horsepower")
            .y("mpg")
            .color("origin"),
    );
    let row = |parts: PartRulesBuilder| {
        Chart::builder(Concat::new(Direction::Row).parts(parts).children([
            Plot::cartesian([weights.clone()]),
            Plot::cartesian([powers.clone()]),
        ]))
        .build()
        .unwrap()
    };
    let chart = row(PartRules::builder());
    render(&chart, [640.0, 300.0], "concat-row");

    // The color scale is shared, and the concat merges its legend into one.
    assert_eq!(count(&chart, [640.0, 300.0], "origin"), 1);
    // A rule that keeps legends draws the legend beside each plot.
    let kept = row(PartRules::builder().legend(PartRuleKind::Keep));
    assert_eq!(count(&kept, [640.0, 300.0], "origin"), 2);
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
        let symbols =
            CartesianLayer::symbol(cars_dataset(), SymbolMark::builder().x("weight").y(y));
        Plot::cartesian([symbols])
    };
    let column = |rules: PartRulesBuilder| {
        Chart::builder(
            Concat::new(Direction::Column)
                .resolve(Resolve::builder().x(Sharing::Shared))
                .parts(rules)
                .children([plot("mpg"), plot("horsepower")]),
        )
        .build()
        .unwrap()
    };
    let collapsed = column(PartRules::builder());
    render(&collapsed, [360.0, 420.0], "concat-column-shared-x");
    // The axes of the shared `x` collapse into one, below the column, with one title.
    assert_eq!(count(&collapsed, [360.0, 420.0], "weight"), 1);
    let evaluated = evaluate(&collapsed, [360.0, 420.0]);
    let [top, bottom] = [evaluated.plots[0].rect, evaluated.plots[1].rect];
    assert_eq!(top[2], bottom[2], "the plots of a shared x have one width");

    // A rule that keeps the ticks and the labels draws them beside each plot.
    let kept = column(
        PartRules::builder()
            .axis_labels(PartRuleKind::Keep)
            .axis_ticks(PartRuleKind::Keep)
            .axis_title(PartRuleKind::Keep),
    );
    render(&kept, [360.0, 420.0], "concat-column-kept-axes");
    assert_eq!(count(&kept, [360.0, 420.0], "weight"), 2);
}

#[test]
fn a_facet_grid_of_nested_partitions_labels_its_rows_and_columns() {
    // A column that facets the origins, of rows that facet the cylinder counts, with an empty cell
    // where an origin has no cars of a count, so the columns line up.
    let counts = [3_i64, 4, 5, 6, 8];
    let rows = Node::partitions(
        &cars_dataset(),
        "origin",
        PartitionOrder::Ascending,
        |origin| {
            let cells = counts.map(|count| {
                let cell = rows_where(&origin, "cylinders", count);
                let node: NodeBuilder = match cell.num_rows() {
                    0 => Empty::builder().into(),
                    _ => Plot::cartesian([CartesianLayer::symbol(
                        cell,
                        SymbolMark::builder().x("weight").y("mpg"),
                    )])
                    .into(),
                };
                node.partition([PartitionValue::new(
                    "cylinders",
                    Value::scalar(Arc::new(Int64Array::from(vec![count])) as ArrayRef),
                )])
            });
            Ok(Concat::new(Direction::Row)
                .facet("cylinders")
                .children(cells))
        },
    )
    .unwrap();
    let chart = Chart::builder(
        Concat::new(Direction::Column)
            .facet("origin")
            .children(rows),
    )
    .build()
    .unwrap();
    render(&chart, [720.0, 520.0], "facet-grid");

    // Each origin labels its row once, and the columns' labels collapse into one row of labels.
    for origin in ["Europe", "Japan", "USA"] {
        assert_eq!(count(&chart, [720.0, 520.0], origin), 1, "{origin}");
    }
    for cylinders in ["3", "4", "5", "6", "8"] {
        assert_eq!(count(&chart, [720.0, 520.0], cylinders), 1, "{cylinders}");
    }
    assert_eq!(count(&chart, [720.0, 520.0], "cylinders"), 1);
    assert_eq!(count(&chart, [720.0, 520.0], "origin"), 1);
    // Six of the fifteen cells are empty.
    let evaluated = evaluate(&chart, [720.0, 520.0]);
    assert_eq!(evaluated.plots.len(), 9);
    // The axes stand on the edges of the grid, past its empty cells: one `x` axis below each
    // column and one `y` axis beside each row, with one title each.
    assert_eq!(count(&chart, [720.0, 520.0], "weight"), 1);
    assert_eq!(count(&chart, [720.0, 520.0], "mpg"), 1);
    let rects = (evaluated.plots.iter()).map(|plot| plot.rect);
    let [left, bottom] = rects.fold([f32::MAX, 0.0], |[left, bottom], rect| {
        [left.min(rect[0]), bottom.max(rect[1] + rect[3])]
    });
    let texts = scene_text_positions(&evaluated.scene.marks);
    let at = |label: &str| -> Vec<[f32; 2]> {
        (texts.iter())
            .filter(|(text, _)| text == label)
            .map(|(_, position)| *position)
            .collect()
    };
    let (thousands, forties) = (at("4,000"), at("40"));
    assert_eq!(thousands.len(), 5);
    assert!(
        thousands.iter().all(|[_, y]| *y > bottom),
        "{thousands:?} {bottom}"
    );
    assert_eq!(forties.len(), 3);
    assert!(forties.iter().all(|[x, _]| *x < left), "{forties:?} {left}");
}

/// The rows of `table` whose `column` holds `value`.
fn rows_where(table: &RecordBatch, column: &str, value: i64) -> RecordBatch {
    let column = table.column_by_name(column).unwrap();
    let mask = arrow::compute::kernels::cmp::eq(column, &Int64Array::new_scalar(value)).unwrap();
    arrow::compute::filter_record_batch(table, &mask).unwrap()
}

#[test]
fn a_wrapped_facet_labels_each_panel() {
    let panels = Node::partitions(
        &cars_dataset(),
        "origin",
        PartitionOrder::Ascending,
        |rows| {
            let symbols = CartesianLayer::symbol(rows, SymbolMark::builder().x("weight").y("mpg"));
            Ok(Plot::cartesian([symbols]))
        },
    )
    .unwrap();
    let chart = Chart::builder(Wrap::builder().facet("origin").children(panels))
        .build()
        .unwrap();
    render(&chart, [560.0, 460.0], "facet-wrap");

    // Each panel has its label, and the wrap merges the facets' titles into one.
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

/// A row height that is a share with a maximum gives each row of a wrap its share of the height,
/// up to the maximum, as a track of avenger-layout with a maximum does.
#[test]
fn a_wrap_with_a_row_height_share_stops_its_rows_at_the_maximum() {
    let wrap = |row_height: Option<TrackBuilder>| {
        let panels = Node::partitions(
            &cars_dataset(),
            "origin",
            PartitionOrder::Ascending,
            |rows| {
                let symbols =
                    CartesianLayer::symbol(rows, SymbolMark::builder().x("weight").y("mpg"));
                Ok(Plot::cartesian([symbols]))
            },
        )
        .unwrap();
        let wrap = Wrap::builder().facet("origin").max_columns(1);
        let wrap = match row_height {
            Some(row_height) => wrap.row_height(row_height),
            None => wrap,
        };
        Chart::builder(wrap.children(panels)).build().unwrap()
    };
    let heights = |chart: &Chart| -> Vec<f32> {
        (evaluate(chart, [400.0, 900.0]).plots.iter())
            .map(|plot| plot.rect[3])
            .collect()
    };
    let shared = heights(&wrap(Some(Track::share(1.0))));
    let capped = heights(&wrap(Some(Track::share(1.0).max(150.0))));
    assert_eq!(shared.len(), 3);
    assert_eq!(capped.len(), 3);
    for (shared, capped) in shared.iter().zip(&capped) {
        assert!(*capped <= 150.0, "{capped}");
        assert!(capped < shared, "{capped} {shared}");
    }
    // A share of weight 1 is the height of a row without a size.
    assert_eq!(heights(&wrap(None)), shared);
}

#[test]
fn an_empty_cell_keeps_its_track() {
    let plot = || {
        let symbols =
            CartesianLayer::symbol(cars_dataset(), SymbolMark::builder().x("weight").y("mpg"));
        Plot::cartesian([symbols])
    };
    let chart = Chart::builder(Concat::new(Direction::Row).children([
        NodeBuilder::from(plot()),
        Empty::builder().into(),
        plot().into(),
    ]))
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
        let symbols =
            CartesianLayer::symbol(cars_dataset(), SymbolMark::builder().x("weight").y("mpg"));
        let plot = Plot::cartesian([symbols]);
        match width {
            Some(width) => plot.width(width),
            None => plot,
        }
    };
    let chart = |padding: f32| {
        Chart::builder(Concat::new(Direction::Row).children([plot(Some(120.0)), plot(None)]))
            .padding_top(padding)
            .padding_right(padding)
            .padding_bottom(padding)
            .padding_left(padding)
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
            cars_dataset(),
            SymbolMark::builder().x("weight").y("mpg").color("origin"),
        );
        Plot::cartesian([symbols]).title(title)
    };
    let chart = Chart::builder(
        Concat::new(Direction::Row)
            .title(
                Title::builder()
                    .text("Fuel economy")
                    .subtitle("By weight, in two panels"),
            )
            .children([plot("Left"), plot("Right")]),
    )
    .build()
    .unwrap();
    render(&chart, [640.0, 320.0], "concat-titles");
    for title in ["Fuel economy", "Left", "Right"] {
        assert_eq!(count(&chart, [640.0, 320.0], title), 1, "{title}");
    }
}
