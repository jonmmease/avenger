//! The modes of a chart: the tools that name them, their joins, and the widgets that set them.

mod support;

use avenger_chart::{prelude::*, ChartError};
use support::points;

/// A plot `plot` of points, which pans in the mode `pan` and draws a box in the mode `box`, and
/// its symbols, which declare `widgets`.
fn modal(widgets: Vec<WidgetBuilder>) -> ChartBuilder {
    let symbols =
        CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b")).widgets(widgets);
    let tools = [
        ToolBuilder::from(Navigate::builder()).modes(["pan"]),
        ToolBuilder::from(Select::interval("brush", IntervalSelect::builder())).modes(["box"]),
    ];
    Chart::builder(Plot::cartesian([symbols]).id("plot").tools(tools))
        .selections([Selection::new("brush", SelectionResolution::Global)])
}

#[track_caller]
fn invalid(built: Result<Chart, ChartError>, path: &str, reason: &str) {
    match built {
        Err(ChartError::Invalid {
            path: at,
            reason: why,
        }) => {
            assert_eq!(at, path, "the error names the path ({why})");
            assert!(why.contains(reason), "`{why}` says `{reason}`");
        }
        other => panic!("expected an error at {path}, got {other:?}"),
    }
}

#[test]
fn tools_name_the_modes_of_their_chart() {
    let chart = modal(Vec::new())
        .modes([Mode::new("pan").label("Pan"), Mode::new("box")])
        .build()
        .unwrap();
    let names: Vec<&str> = (chart.message().modes.iter())
        .map(|mode| mode.name.as_str())
        .collect();
    assert_eq!(names, ["pan", "box"]);

    // A tool names a mode of the chart, once.
    invalid(
        modal(Vec::new()).modes([Mode::new("pan")]).build(),
        "root.plot.tools[1].modes[0]",
        "the chart has no mode `box`",
    );
    let twice = Chart::builder(
        Plot::cartesian([CartesianLayer::symbol(
            points(),
            SymbolMark::builder().x("a").y("b"),
        )])
        .id("plot")
        .tools([ToolBuilder::from(Navigate::builder()).modes(["pan", "pan"])]),
    )
    .modes([Mode::new("pan")])
    .build();
    invalid(
        twice,
        "root.plot.tools[0].modes[1]",
        "names the mode `pan` once",
    );
}

#[test]
fn modes_with_one_name_join_at_the_first() {
    let chart = modal(Vec::new())
        .modes([Mode::new("box"), Mode::new("pan"), Mode::new("box")])
        .build()
        .unwrap();
    let names: Vec<&str> = (chart.message().modes.iter())
        .map(|mode| mode.name.as_str())
        .collect();
    assert_eq!(names, ["box", "pan"]);

    // An embedded chart's modes join the embedding builder's, after them.
    let inner = modal(Vec::new())
        .modes([Mode::new("pan"), Mode::new("box")])
        .build()
        .unwrap();
    let outer = Chart::builder(Concat::new(Direction::Row).children([NodeBuilder::from(inner)]))
        .modes([Mode::new("box"), Mode::new("lasso")])
        .build()
        .unwrap();
    let names: Vec<&str> = (outer.message().modes.iter())
        .map(|mode| mode.name.as_str())
        .collect();
    assert_eq!(names, ["box", "lasso", "pan"]);

    // Two modes of one name with different labels are a conflict.
    invalid(
        modal(Vec::new())
            .modes([
                Mode::new("pan"),
                Mode::new("box"),
                Mode::new("pan").label("Drag"),
            ])
            .build(),
        "modes[2].label",
        "their labels differ",
    );
}

#[test]
fn a_mode_widget_sets_the_mode_of_a_chart_with_modes() {
    let chart = modal(vec![Widget::mode("drag")])
        .modes([Mode::new("pan"), Mode::new("box")])
        .build()
        .unwrap();
    let group = &chart.resolve().widgets[0];
    assert!(group.widget.target().is_none());
    assert!(group.widget.domain().is_none());

    // A chart with a mode widget has a mode or more.
    let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"))
        .widgets([Widget::mode("drag")]);
    invalid(
        Chart::builder(Plot::cartesian([symbols])).build(),
        "root.plot.cartesian.layers[0].widgets[0].mode",
        "has a mode or more",
    );

    // A mode widget writes no clause.
    let clause = Clause::new([Condition::range(ValueRange::new("a").low(1.0))]).widget("drag");
    let built = modal(vec![Widget::mode("drag")])
        .modes([Mode::new("pan"), Mode::new("box")])
        .selections([
            Selection::new("brush", SelectionResolution::Global),
            Selection::new("filter", SelectionResolution::Union).initial([clause]),
        ])
        .build();
    match built {
        Err(ChartError::Invalid { reason, .. }) => {
            assert!(reason.contains("sets the chart's mode"), "{reason}")
        }
        other => panic!("a clause of a mode widget is invalid: {other:?}"),
    }
}

#[test]
fn a_box_and_a_lasso_of_one_selection_take_modes_apart() {
    let plot = |lasso_modes: Vec<&str>| {
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
        let tools = [
            ToolBuilder::from(Select::interval("brush", IntervalSelect::builder())).modes(["box"]),
            ToolBuilder::from(Select::lasso("brush", Vec::<String>::new())).modes(lasso_modes),
        ];
        Chart::builder(Plot::cartesian([symbols]).id("plot").tools(tools))
            .selections([Selection::new("brush", SelectionResolution::Global)])
            .modes([Mode::new("box"), Mode::new("lasso")])
            .build()
    };
    plot(vec!["lasso"]).unwrap();
    // A lasso that is active in the mode of the box, or in every mode, is a conflict.
    for modes in [vec!["box", "lasso"], vec![]] {
        invalid(plot(modes), "root.plot", "in one mode, which is a conflict");
    }
}
