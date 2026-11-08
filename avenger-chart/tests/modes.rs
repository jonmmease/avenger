//! The modes of a chart: their groups and toggles, the tools that name them, their joins, and the
//! widgets that turn them on and off.

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

/// The modes `pan` and `box` of the group `drag`, and the toggle `tooltips`, which is on.
fn drag_modes() -> [ModeBuilder; 3] {
    [
        Mode::new("pan").label("Pan").group("drag"),
        Mode::new("box").label("Box").group("drag"),
        Mode::new("tooltips").on(true),
    ]
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

fn names(chart: &Chart) -> Vec<&str> {
    (chart.message().modes.iter())
        .map(|mode| mode.name.as_str())
        .collect()
}

#[test]
fn tools_name_the_modes_of_their_chart() {
    let chart = modal(Vec::new()).modes(drag_modes()).build().unwrap();
    assert_eq!(names(&chart), ["pan", "box", "tooltips"]);

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

    // A mode of a group is on when it is the first of its group.
    invalid(
        modal(Vec::new())
            .modes([
                Mode::new("pan").group("drag"),
                Mode::new("box").group("drag").on(true),
            ])
            .build(),
        "modes[1].on",
        "first of its group",
    );
}

#[test]
fn modes_with_one_name_join_at_the_first() {
    let chart = modal(Vec::new())
        .modes([Mode::new("box"), Mode::new("pan"), Mode::new("box")])
        .build()
        .unwrap();
    assert_eq!(names(&chart), ["box", "pan"]);

    // An embedded chart's modes join the embedding builder's, after them.
    let inner = modal(Vec::new())
        .modes([Mode::new("pan"), Mode::new("box")])
        .build()
        .unwrap();
    let outer = Chart::builder(Concat::new(Direction::Row).children([NodeBuilder::from(inner)]))
        .modes([Mode::new("box"), Mode::new("lasso")])
        .build()
        .unwrap();
    assert_eq!(names(&outer), ["box", "lasso", "pan"]);

    // Two modes of one name that differ are a conflict.
    for other in [
        Mode::new("pan").label("Drag"),
        Mode::new("pan").group("drag"),
    ] {
        invalid(
            modal(Vec::new())
                .modes([Mode::new("pan"), Mode::new("box"), other])
                .build(),
            "modes[2]",
            "they differ",
        );
    }
}

#[test]
fn a_mode_widget_turns_a_group_or_a_toggle_of_its_chart() {
    let widgets = vec![
        Widget::mode("drag", ModeWidget::group("drag")),
        Widget::mode("tooltips", ModeWidget::toggle("tooltips")),
    ];
    let chart = modal(widgets).modes(drag_modes()).build().unwrap();
    for group in &chart.resolve().widgets {
        assert!(group.widget.target().is_none());
        assert!(group.widget.domain().is_none());
    }

    // The group or the toggle is one of the chart's.
    let one = |widget: ModeWidgetBuilder| modal(vec![Widget::mode("drag", widget)]);
    let path = "root.plot.cartesian.layers[0].widgets[0].mode";
    invalid(
        one(ModeWidget::group("zoom")).modes(drag_modes()).build(),
        &format!("{path}.group"),
        "no group of modes `zoom`",
    );
    // A mode of a group is not a toggle.
    invalid(
        one(ModeWidget::toggle("pan")).modes(drag_modes()).build(),
        &format!("{path}.toggle"),
        "no toggle `pan`",
    );

    // A mode widget writes no clause.
    let clause = Clause::new([Condition::range(ValueRange::new("a").low(1.0))]).widget("drag");
    let built = one(ModeWidget::group("drag"))
        .modes(drag_modes())
        .selections([
            Selection::new("brush", SelectionResolution::Global),
            Selection::new("filter", SelectionResolution::Union).initial([clause]),
        ])
        .build();
    match built {
        Err(ChartError::Invalid { reason, .. }) => {
            assert!(reason.contains("sets the chart's modes"), "{reason}")
        }
        other => panic!("a clause of a mode widget is invalid: {other:?}"),
    }
}

#[test]
fn a_box_and_a_lasso_of_one_selection_take_modes_of_one_group_apart() {
    let plot = |lasso_modes: Vec<&str>| {
        let symbols = CartesianLayer::symbol(points(), SymbolMark::builder().x("a").y("b"));
        let tools = [
            ToolBuilder::from(Select::interval("brush", IntervalSelect::builder())).modes(["box"]),
            ToolBuilder::from(Select::lasso("brush", Vec::<String>::new())).modes(lasso_modes),
        ];
        Chart::builder(Plot::cartesian([symbols]).id("plot").tools(tools))
            .selections([Selection::new("brush", SelectionResolution::Global)])
            .modes([
                Mode::new("box").group("drag"),
                Mode::new("lasso").group("drag"),
                Mode::new("freehand"),
            ])
            .build()
    };
    plot(vec!["lasso"]).unwrap();
    // A lasso that can be active with the box is a conflict: in the box's mode, in every mode, or
    // in a toggle, which can be on in any mode of the group.
    for modes in [vec!["box", "lasso"], vec![], vec!["freehand"]] {
        invalid(plot(modes), "root.plot", "can be active at once");
    }
}
