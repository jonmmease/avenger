//! The widgets of layers: where they stand, the values that their clauses give them, and the
//! clauses that their values write.

use arrow::array::Array;
use avenger_chart::prelude::*;
use avenger_chart_eval::{EvaluatedChart, SliderComparison as Comparison, WidgetSpec, WidgetValue};

use crate::support::*;

/// A target of the selection `filter` over `column`.
fn target(column: &str) -> WidgetTargetBuilder {
    WidgetTarget::new("filter", column)
}

/// A domain that the renderer fits to the target's column.
fn data() -> WidgetDomainBuilder {
    WidgetDomain::data(WidgetDataDomain::builder())
}

/// The cars with `widgets`, in a chart with the selection `filter` and its initial `clauses`.
fn cars_with(
    widgets: Vec<WidgetBuilder>,
    clauses: Vec<ClauseBuilder>,
    side: Option<Side>,
) -> Chart {
    let widgets = widgets.into_iter().map(|widget| match side {
        Some(side) => widget.side(side),
        None => widget,
    });
    let symbols = CartesianLayer::symbol(
        cars_dataset(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    )
    .widgets(widgets);
    Chart::builder(Plot::cartesian([symbols]))
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .selections([Selection::new("filter", SelectionResolution::Intersect).initial(clauses)])
        .build()
        .unwrap()
}

/// The rectangle of the first plot.
fn plot_rect(evaluated: &EvaluatedChart) -> [f32; 4] {
    evaluated.plots[0].rect
}

#[test]
fn a_slider_stands_above_its_plot_at_the_value_of_its_clause() {
    let slider = Widget::slider(
        "weight",
        SliderWidget::new(target("weight"), data())
            .comparison(SliderComparison::AtLeast)
            .step(100.0)
            .readout(Labels::builder().format(",.0f")),
    )
    .title("Weight at least");
    let clause =
        Clause::new([Condition::range(ValueRange::new("weight").low(2500.0))]).widget("weight");
    let chart = cars_with(vec![slider], vec![clause], None);
    let evaluated = render(&chart, [320.0, 240.0], "widget-slider");

    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let [widget] = &evaluated.widgets[..] else {
        panic!("one widget: {:#?}", evaluated.widgets);
    };
    assert_eq!(widget.title, "Weight at least");
    let WidgetSpec::Slider {
        comparison, domain, ..
    } = &widget.spec
    else {
        panic!("a slider: {:?}", widget.spec);
    };
    assert_eq!(*comparison, Comparison::AtLeast);
    assert_eq!(*domain, [1613.0, 5140.0]);
    assert_eq!(
        evaluator(&chart).widget_value(widget),
        WidgetValue::Number(2500.0)
    );
    // The widget stands above the plot.
    let [rect] = &widget.rects[..] else {
        panic!("one drawing: {:?}", widget.rects);
    };
    let plot = plot_rect(&evaluated);
    assert!(rect[1] + rect[3] <= plot[1], "{rect:?} {plot:?}");
}

#[test]
fn a_radio_group_and_a_check_box_list_list_their_items() {
    let radio = Widget::radio(
        "origin",
        RadioWidget::new(target("origin"), data()).clear_label("All"),
    );
    let checkboxes = Widget::checkboxes(
        "cylinders",
        CheckboxesWidget::new(target("cylinders"), data()),
    );
    let clause =
        Clause::new([Condition::set([SetColumn::new("origin", ["Japan"])])]).widget("origin");
    let chart = cars_with(vec![radio, checkboxes], vec![clause], Some(Side::Right));
    let evaluated = render(&chart, [480.0, 320.0], "widget-radio-checkboxes");

    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let evaluator = evaluator(&chart);
    let [radio, checkboxes] = &evaluated.widgets[..] else {
        panic!("two widgets: {:#?}", evaluated.widgets);
    };
    let WidgetSpec::Radio { labels, clear, .. } = &radio.spec else {
        panic!("a radio group: {:?}", radio.spec);
    };
    assert_eq!(labels, &["Europe", "Japan", "USA"]);
    assert_eq!(clear.as_deref(), Some("All"));
    assert_eq!(evaluator.widget_value(radio), WidgetValue::Item(Some(1)));
    // Without a clause, every box is checked.
    assert_eq!(
        evaluator.widget_value(checkboxes),
        WidgetValue::Items(vec![0, 1, 2, 3, 4])
    );
    // Both stand right of the plot, the check box list below the radio group.
    let plot = plot_rect(&evaluated);
    let (radio, checkboxes) = (radio.rects[0], checkboxes.rects[0]);
    assert!(radio[0] >= plot[0] + plot[2], "{radio:?} {plot:?}");
    assert_eq!(checkboxes[0], radio[0]);
    assert!(
        checkboxes[1] >= radio[1] + radio[3],
        "{checkboxes:?} {radio:?}"
    );
}

#[test]
fn widgets_share_a_row_until_their_side_is_full() {
    let slider = Widget::slider(
        "weight",
        SliderWidget::new(target("weight"), data())
            .comparison(SliderComparison::AtLeast)
            .step(100.0),
    )
    .title("Weight at least");
    let checkbox = |key: &str, checkbox: CheckboxWidgetBuilder, title: &str| {
        Widget::checkbox(key, checkbox).title(title)
    };
    let eight = CheckboxWidget::new(target("cylinders"), 8_i64);
    let japan = CheckboxWidget::new(target("origin"), "Japan");
    let chart = cars_with(
        vec![
            slider,
            checkbox("eight", eight, "Eight cylinders"),
            checkbox("japan", japan, "Japan only"),
        ],
        Vec::new(),
        None,
    );
    let rows = |evaluated: &EvaluatedChart| {
        (evaluated.widgets.iter())
            .map(|widget| widget.rects[0][1])
            .collect::<Vec<_>>()
    };

    // A wide plot holds the three widgets in one row above it.
    let wide = render(&chart, [720.0, 320.0], "widget-row");
    assert!(wide.warnings.is_empty(), "{:#?}", wide.warnings);
    let [first, second, third] = rows(&wide)[..] else {
        panic!("three widgets: {:#?}", wide.widgets);
    };
    assert!(first == second && second == third, "{:?}", rows(&wide));
    let [slider, eight, _] = [0, 1, 2].map(|index| wide.widgets[index].rects[0]);
    assert!(eight[0] >= slider[0] + slider[2], "{eight:?} {slider:?}");

    // A narrow one moves the widgets that pass its width to a row farther out.
    let narrow = render(&chart, [320.0, 320.0], "widget-row-wrapped");
    assert!(narrow.warnings.is_empty(), "{:#?}", narrow.warnings);
    let rows = rows(&narrow);
    let plot = plot_rect(&narrow);
    let bottom = narrow.widgets[0].rects[0][1] + narrow.widgets[0].rects[0][3];
    assert!(bottom <= plot[1], "{rows:?} {plot:?}");
    assert!(rows.iter().any(|&row| row < rows[0]), "{rows:?}");
}

#[test]
fn a_range_slider_and_a_check_box_stand_below_their_plot() {
    let range = Widget::range_slider(
        "mpg",
        RangeSliderWidget::new(target("mpg"), WidgetDomain::explicit(vec![10.0, 50.0])).step(1.0),
    )
    .title("Miles per gallon");
    let checkbox = Widget::checkbox("eight", CheckboxWidget::new(target("cylinders"), 8_i64))
        .title("Eight cylinders");
    let clauses = vec![
        Clause::new([Condition::range(
            ValueRange::new("mpg").low(20.0).high(35.0),
        )])
        .widget("mpg"),
        Clause::new([Condition::set([SetColumn::new("cylinders", [8_i64])])]).widget("eight"),
    ];
    let chart = cars_with(vec![range, checkbox], clauses, Some(Side::Bottom));
    let evaluated = render(&chart, [360.0, 320.0], "widget-range-checkbox");

    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let evaluator = evaluator(&chart);
    let [range, checkbox] = &evaluated.widgets[..] else {
        panic!("two widgets: {:#?}", evaluated.widgets);
    };
    assert_eq!(
        evaluator.widget_value(range),
        WidgetValue::Range([20.0, 35.0])
    );
    assert_eq!(evaluator.widget_value(checkbox), WidgetValue::Checked(true));
    let plot = plot_rect(&evaluated);
    assert!(range.rects[0][1] >= plot[1] + plot[3], "{range:?}");
    assert!(checkbox.rects[0][1] >= range.rects[0][1] + range.rects[0][3]);
}

#[test]
fn a_widget_writes_the_clause_that_gives_its_value() {
    let slider = Widget::slider(
        "weight",
        SliderWidget::new(target("weight"), data()).comparison(SliderComparison::AtMost),
    );
    let radio = Widget::radio("origin", RadioWidget::new(target("origin"), data()));
    let checkboxes = Widget::checkboxes(
        "cylinders",
        CheckboxesWidget::new(target("cylinders"), data()),
    );
    let range = Widget::range_slider(
        "mpg",
        RangeSliderWidget::new(target("mpg"), WidgetDomain::explicit(vec![10.0, 50.0])),
    );
    let chart = cars_with(vec![slider, radio, checkboxes, range], Vec::new(), None);
    let evaluated = evaluate(&chart, [480.0, 480.0]);
    let mut evaluator = evaluator(&chart);
    let filter = |evaluator: &avenger_chart_eval::ChartEvaluator| {
        evaluator.selections()["filter"].clauses.clone()
    };
    assert!(filter(&evaluator).is_empty());

    let values = [
        WidgetValue::Number(3000.0),
        WidgetValue::Item(Some(2)),
        WidgetValue::Items(vec![0, 2]),
        WidgetValue::Range([10.0, 30.0]),
    ];
    for (widget, value) in evaluated.widgets.iter().zip(&values) {
        // The clear state of each writes no clause.
        let clear = evaluator.widget_value(widget);
        assert!(widget.clause_for(&clear).is_none(), "{}", widget.key);
        let written = evaluator.write_widget(widget, value);
        assert_eq!(written.as_deref(), Some("filter"));
        assert_eq!(&evaluator.widget_value(widget), value, "{}", widget.key);
    }
    let clauses = filter(&evaluator);
    assert_eq!(clauses.len(), 4);
    // The range slider's end at the end of its domain is open.
    let mpg = clauses.iter().find(|clause| clause.owner.contains("mpg"));
    let Some(avenger_selection::Condition::Range { ends, .. }) =
        mpg.map(|clause| &clause.conditions[0])
    else {
        panic!("a range: {mpg:?}");
    };
    assert!(ends.is_null(0) && ends.is_valid(1));

    // A new evaluation shows the values that the clauses give.
    let evaluated_again = evaluator.evaluate(&host([480.0, 480.0])).unwrap();
    assert_eq!(
        evaluator.widget_value(&evaluated_again.widgets[1]),
        WidgetValue::Item(Some(2))
    );

    // Writing the clear state removes a clause.
    let radio = &evaluated.widgets[1];
    evaluator.write_widget(radio, &WidgetValue::Item(None));
    assert_eq!(filter(&evaluator).len(), 3);
    assert_eq!(evaluator.widget_value(radio), WidgetValue::Item(None));
}

#[test]
fn a_wrap_draws_a_shared_widget_once_or_beside_each_plot() {
    let wrapped = |rules: Vec<PartRuleBuilder>| {
        let panels = Node::partitions(
            &cars_dataset(),
            "origin",
            PartitionOrder::Ascending,
            |rows| {
                let slider = Widget::slider(
                    "weight",
                    SliderWidget::new(target("weight"), data())
                        .comparison(SliderComparison::AtLeast),
                );
                let symbols =
                    CartesianLayer::symbol(rows, SymbolMark::builder().x("weight").y("mpg"))
                        .widgets([slider]);
                Ok(Plot::cartesian([symbols]))
            },
        )
        .unwrap();
        Chart::builder(
            Wrap::builder()
                .facet("origin")
                .layout(WrapLayout::builder().part_rules(rules))
                .children(panels),
        )
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .selections([Selection::new("filter", SelectionResolution::Intersect)])
        .build()
        .unwrap()
    };

    // By default, the wrap merges the copies into one widget that it draws once.
    let merged = render(&wrapped(Vec::new()), [560.0, 460.0], "widget-wrap-merged");
    let [widget] = &merged.widgets[..] else {
        panic!("one widget: {:#?}", merged.widgets);
    };
    assert_eq!(widget.rects.len(), 1);
    // The domain is the weights of every panel.
    let WidgetSpec::Slider { domain, .. } = &widget.spec else {
        panic!("a slider");
    };
    assert_eq!(*domain, [1613.0, 5140.0]);

    // A rule that keeps widgets draws the one widget beside each plot.
    let keep = PartRule::new(Part::Widget, PartRuleKind::Keep);
    let kept = render(&wrapped(vec![keep]), [560.0, 460.0], "widget-wrap-kept");
    let [widget] = &kept.widgets[..] else {
        panic!("one widget: {:#?}", kept.widgets);
    };
    assert_eq!(widget.rects.len(), 3);
    for (rect, plot) in widget.rects.iter().zip(&kept.plots) {
        assert!(
            rect[1] + rect[3] <= plot.rect[1] + 0.5,
            "{rect:?} {:?}",
            plot.rect
        );
    }
}

#[test]
fn a_stylesheet_colors_and_places_widgets() {
    let slider = Widget::slider(
        "weight",
        SliderWidget::new(target("weight"), data()).step(500.0),
    );
    let clause = Clause::new([Condition::range(
        ValueRange::new("weight")
            .low(3000.0)
            .high(3500.0)
            .exclude_end(true),
    )])
    .widget("weight");
    let chart = cars_with(vec![slider], vec![clause], None);
    let mut evaluator = avenger_chart_eval::ChartEvaluator::new(
        chart,
        cascade_with("widget[key=weight] { accent-color: darkorange; orient: left; }"),
        engine(),
        avenger_chart_eval::Registry::new(),
    )
    .unwrap();
    let evaluated = evaluator.evaluate(&host([400.0, 260.0])).unwrap();
    assert_image(&evaluated, "widget-styled");
    let widget = &evaluated.widgets[0];
    let orange = [1.0, 140.0 / 255.0, 0.0, 1.0];
    let fill = widget.style.accent.normal.fill;
    assert!(
        (fill.iter().zip(orange)).all(|(a, b)| (a - b).abs() < 1e-3),
        "{fill:?}"
    );
    let plot = plot_rect(&evaluated);
    assert!(
        widget.rects[0][0] + widget.rects[0][2] <= plot[0],
        "{:?}",
        widget.rects
    );
}

#[test]
fn mode_widgets_turn_the_modes_that_decide_the_active_tools() {
    let symbols = CartesianLayer::symbol(
        cars_dataset(),
        SymbolMark::builder().x("weight").y("mpg").color("origin"),
    )
    .tools([LayerToolBuilder::from(Tooltip::builder()).modes(["tooltips"])])
    .id("points")
    .widgets([
        Widget::mode("drag", ModeWidget::group("drag")).side(Side::Right),
        Widget::mode("tooltips", ModeWidget::toggle("tooltips")).side(Side::Right),
    ]);
    let tools = [
        ToolBuilder::from(Navigate::builder()).modes(["pan"]),
        ToolBuilder::from(Select::interval("filter", IntervalSelect::builder())).modes(["box"]),
    ];
    let chart = Chart::builder(Plot::cartesian([symbols]).id("cars").tools(tools))
        .number_format_syntax("d3")
        .selections([Selection::new("filter", SelectionResolution::Global)])
        .modes([
            Mode::new("pan").label("Pan").group("drag"),
            Mode::new("box").label("Box").group("drag"),
            Mode::new("tooltips").label("Tooltips").on(true),
        ])
        .build()
        .unwrap();
    let mut evaluator = evaluator(&chart);
    let evaluated = evaluator.evaluate(&host([480.0, 320.0])).unwrap();
    assert_image(&evaluated, "widget-mode");
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let [group, toggle] = &evaluated.widgets[..] else {
        panic!("two widgets: {:#?}", evaluated.widgets);
    };
    assert_eq!(group.target, avenger_chart_eval::WidgetTarget::Modes);
    // A widget of a group has no title, and a widget of a toggle takes the toggle's label.
    assert_eq!(
        (group.title.as_str(), toggle.title.as_str()),
        ("", "Tooltips")
    );
    let WidgetSpec::ModeGroup { modes, labels } = &group.spec else {
        panic!("a widget of a group: {:?}", group.spec);
    };
    assert_eq!(modes, &["pan", "box"]);
    assert_eq!(labels, &["Pan", "Box"]);

    // The group starts in its first mode, and the toggle on.
    let [pan, select, tooltips] = ["pan", "box", "tooltips"].map(|mode| vec![mode.to_string()]);
    assert_eq!(evaluator.modes(), ["pan", "tooltips"]);
    assert_eq!(evaluator.widget_value(group), WidgetValue::Item(Some(0)));
    assert_eq!(evaluator.widget_value(toggle), WidgetValue::Checked(true));
    assert!(evaluator.is_active(&pan) && !evaluator.is_active(&select));
    assert!(evaluator.is_active(&tooltips) && evaluator.is_active(&[]));

    // The widgets turn the modes and write no clause.
    assert_eq!(
        evaluator.write_widget(group, &WidgetValue::Item(Some(1))),
        None
    );
    assert_eq!(
        evaluator.write_widget(toggle, &WidgetValue::Checked(false)),
        None
    );
    assert_eq!(evaluator.modes(), ["box"]);
    assert_eq!(evaluator.widget_value(group), WidgetValue::Item(Some(1)));
    assert_eq!(evaluator.widget_value(toggle), WidgetValue::Checked(false));
    assert!(!evaluator.is_active(&pan) && evaluator.is_active(&select));
    assert!(!evaluator.is_active(&tooltips));
    assert!(evaluator.selections()["filter"].clauses.is_empty());

    // A mode of a group stays on until another of its group turns on.
    evaluator.set_mode("box", false);
    assert_eq!(evaluator.modes(), ["box"]);
    evaluator.set_mode("pan", true);
    assert_eq!(evaluator.modes(), ["pan"]);
    // A mode that the chart does not declare is off.
    evaluator.set_mode("lasso", true);
    assert_eq!(evaluator.modes(), ["pan"]);
}
