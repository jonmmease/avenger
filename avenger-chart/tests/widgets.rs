//! The widgets that layers declare: the copies of a key that sharing joins into one widget, the
//! identity of each widget, the agreement of copies, and the initial clauses of widgets.

mod support;

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array};
use avenger_chart::{
    list_value,
    proto::{self, CartesianLayer, Node, ValueRef},
    resolve::WidgetGroup,
    Chart,
};
use support::*;

/// A selection `filters`, which widgets write.
fn filters() -> proto::Selection {
    proto::Selection {
        name: "filters".to_string(),
        resolution: proto::SelectionResolution::Intersect as i32,
        ..Default::default()
    }
}

/// A range slider with the key `magnitude` over the column `a`, fit to the data.
fn range_slider() -> proto::Widget {
    proto::Widget {
        key: "magnitude".to_string(),
        kind: Some(proto::widget::Kind::RangeSlider(proto::RangeSliderWidget {
            target: Some(target("filters", "a")),
            domain: Some(proto::WidgetDomain {
                domain: Some(proto::widget_domain::Domain::Data(
                    proto::WidgetDataDomain::default(),
                )),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn target(selection: &str, column: &str) -> proto::WidgetTarget {
    proto::WidgetTarget {
        selection: selection.to_string(),
        column: column.to_string(),
    }
}

/// The target of a widget's kind, to change.
fn target_of(widget: &mut proto::Widget) -> &mut proto::WidgetTarget {
    use proto::widget::Kind;
    let target = match widget.kind.as_mut().expect("a widget has a kind") {
        Kind::Slider(slider) => &mut slider.target,
        Kind::RangeSlider(slider) => &mut slider.target,
        Kind::Radio(radio) => &mut radio.target,
        Kind::Checkboxes(checkboxes) => &mut checkboxes.target,
        Kind::Checkbox(checkbox) => &mut checkbox.target,
        Kind::Mode(_) => panic!("a mode widget has no target"),
    };
    target.as_mut().expect("the kind has a target")
}

/// Symbols that declare `widget`.
fn declaring(widget: proto::Widget) -> CartesianLayer {
    CartesianLayer {
        widgets: vec![widget],
        ..symbols(0)
    }
}

fn with_filters(root: Node, values: Vec<ArrayRef>) -> Parts {
    let mut parts = parts(root, values);
    parts.0.selections = vec![filters()];
    parts
}

fn groups(chart: &Chart) -> Vec<(String, Option<String>, usize)> {
    (chart.resolve().widgets.iter())
        .map(|group: &WidgetGroup| {
            (
                group.key.clone(),
                group.anchor.as_ref().map(|anchor| anchor.name()),
                group.members.len(),
            )
        })
        .collect()
}

fn set_resolve(node: &mut Node, sharing: proto::Sharing) {
    let rule = proto::Resolve {
        key: Some(proto::resolve::Key::Widget("magnitude".to_string())),
        sharing: sharing as i32,
    };
    match node.node.as_mut() {
        Some(proto::node::Node::Plot(plot)) => plot.resolve = vec![rule],
        Some(proto::node::Node::Concat(concat)) => concat.resolve = vec![rule],
        _ => panic!("a plot or a concat"),
    }
}

/// The copies of a key in an overlay, in a concat, and in the facets of a facet grid are shared by
/// default, so each chart has one widget, which anchors at the root.
#[test]
fn copies_of_a_key_share_one_widget_by_default() {
    let shared = |root: Node, values| {
        let chart = read(with_filters(root, values)).unwrap();
        assert_eq!(groups(&chart), [("magnitude".to_string(), None, 2)]);
        chart
    };
    let overlay = plot(vec![declaring(range_slider()), declaring(range_slider())]);
    shared(overlay, vec![]);
    let row = concat(
        proto::Direction::Row,
        vec![
            plot(vec![declaring(range_slider())]),
            plot(vec![declaring(range_slider())]),
        ],
    );
    let chart = shared(row, vec![]);
    assert_eq!(chart.resolve().widgets[0].owner, "root.concat");
    let mut grid = concat(
        proto::Direction::Row,
        vec![
            plot_with(
                "panel",
                vec![value("species", 0)],
                vec![declaring(range_slider())],
            ),
            plot_with(
                "panel",
                vec![value("species", 1)],
                vec![declaring(range_slider())],
            ),
        ],
    );
    concat_mut(&mut grid).facet = Some(proto::Facet {
        column: "species".to_string(),
        ..Default::default()
    });
    shared(grid, strings(&["Adelie", "Gentoo"]));
}

/// An independent rule splits the copies below its node: one widget for each facet, anchored at
/// each facet's plot, one for each child of a concat with an id, and one for each layer of a plot.
#[test]
fn an_independent_rule_splits_a_key_into_widgets_with_anchors() {
    let mut grid = concat(
        proto::Direction::Row,
        vec![
            plot_with(
                "panel",
                vec![value("species", 0)],
                vec![declaring(range_slider())],
            ),
            plot_with(
                "panel",
                vec![value("species", 1)],
                vec![declaring(range_slider())],
            ),
        ],
    );
    concat_mut(&mut grid).facet = Some(proto::Facet {
        column: "species".to_string(),
        ..Default::default()
    });
    set_resolve(&mut grid, proto::Sharing::Independent);
    let chart = read(with_filters(grid, strings(&["Adelie", "Gentoo"]))).unwrap();
    assert_eq!(
        groups(&chart),
        [
            (
                "magnitude".to_string(),
                Some("panel[species=Adelie]".to_string()),
                1
            ),
            (
                "magnitude".to_string(),
                Some("panel[species=Gentoo]".to_string()),
                1
            ),
        ]
    );

    let row = |left: &str, right: &str| {
        let mut row = concat(
            proto::Direction::Row,
            vec![
                plot_with(left, vec![], vec![declaring(range_slider())]),
                plot_with(right, vec![], vec![declaring(range_slider())]),
            ],
        );
        set_resolve(&mut row, proto::Sharing::Independent);
        with_filters(row, vec![])
    };
    let chart = read(row("quakes", "plates")).unwrap();
    assert_eq!(
        groups(&chart),
        [
            ("magnitude".to_string(), Some("quakes".to_string()), 1),
            ("magnitude".to_string(), Some("plates".to_string()), 1),
        ]
    );
    invalid(
        row("quakes", ""),
        "root.concat.children[1].plot",
        "anchors one of its widgets and has an id",
    );

    let mut layered = plot(vec![
        CartesianLayer {
            id: "points".to_string(),
            ..declaring(range_slider())
        },
        CartesianLayer {
            id: "means".to_string(),
            ..declaring(range_slider())
        },
    ]);
    set_resolve(&mut layered, proto::Sharing::Independent);
    let chart = read(with_filters(layered, vec![])).unwrap();
    assert_eq!(
        groups(&chart),
        [
            ("magnitude".to_string(), Some("points".to_string()), 1),
            ("magnitude".to_string(), Some("means".to_string()), 1),
        ]
    );
}

/// An inner shared rule below an outer independent rule keeps one widget for each child.
#[test]
fn an_inner_rule_wins_over_an_outer_rule() {
    let mut inner = concat(
        proto::Direction::Column,
        vec![
            plot(vec![declaring(range_slider())]),
            plot(vec![declaring(range_slider())]),
        ],
    );
    concat_mut(&mut inner).id = "left".to_string();
    let mut outer = concat(
        proto::Direction::Row,
        vec![
            inner,
            plot_with("right", vec![], vec![declaring(range_slider())]),
        ],
    );
    set_resolve(&mut outer, proto::Sharing::Independent);
    let chart = read(with_filters(outer, vec![])).unwrap();
    assert_eq!(
        groups(&chart),
        [
            ("magnitude".to_string(), Some("left".to_string()), 2),
            ("magnitude".to_string(), Some("right".to_string()), 1),
        ]
    );
}

/// Copies that share one widget agree: one selection, one kind, equal explicit domains, and
/// columns of one type, while the fits of their summary and data domains merge.
#[test]
fn the_copies_of_one_widget_agree() {
    let pair = |a: proto::Widget, b: proto::Widget, values: Vec<ArrayRef>| {
        with_filters(plot(vec![declaring(a), declaring(b)]), values)
    };
    let second = "root.plot.cartesian.layers[1].widgets[0]";
    let mut other_selection = range_slider();
    target_of(&mut other_selection).selection = "brush".to_string();
    let mut both = pair(range_slider(), other_selection, vec![]);
    both.0.selections.push(proto::Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        ..Default::default()
    });
    invalid(both, second, "selection` to different values");

    let mut slider = range_slider();
    slider.kind = Some(proto::widget::Kind::Slider(proto::SliderWidget {
        target: Some(target("filters", "a")),
        domain: Some(proto::WidgetDomain {
            domain: Some(proto::widget_domain::Domain::Data(
                proto::WidgetDataDomain::default(),
            )),
        }),
        comparison: proto::SliderComparison::AtLeast as i32,
        ..Default::default()
    }));
    invalid(
        pair(range_slider(), slider, vec![]),
        second,
        "are of two kinds",
    );

    let explicit = |index: u32| {
        let mut widget = range_slider();
        if let Some(proto::widget::Kind::RangeSlider(slider)) = widget.kind.as_mut() {
            slider.domain = Some(proto::WidgetDomain {
                domain: Some(proto::widget_domain::Domain::Explicit(ValueRef { index })),
            });
        }
        widget
    };
    let ends = |low: f64, high: f64| list_value(Arc::new(Float64Array::from(vec![low, high])));
    read(pair(
        explicit(0),
        explicit(1),
        vec![ends(0.0, 5.0), ends(0.0, 5.0)],
    ))
    .unwrap();
    invalid(
        pair(
            explicit(0),
            explicit(1),
            vec![ends(0.0, 5.0), ends(0.0, 6.0)],
        ),
        second,
        "set `domain` to different values",
    );
    invalid(
        pair(explicit(0), range_slider(), vec![ends(0.0, 5.0)]),
        second,
        "an explicit domain and a summary or a data domain",
    );

    // The fits of data domains merge field by field.
    let nice = |count: Option<f64>, zero: Option<bool>| {
        let mut widget = range_slider();
        if let Some(proto::widget::Kind::RangeSlider(slider)) = widget.kind.as_mut() {
            slider.domain = Some(proto::WidgetDomain {
                domain: Some(proto::widget_domain::Domain::Data(
                    proto::WidgetDataDomain {
                        fit: Some(proto::DomainFit {
                            fit: Some(proto::domain_fit::Fit::Interval(proto::IntervalFit {
                                nice_ends: count.map(proto::interval_fit::NiceEnds::NiceCount),
                                zero,
                                ..Default::default()
                            })),
                        }),
                    },
                )),
            });
        }
        widget
    };
    let chart = read(pair(nice(Some(6.0), None), nice(None, Some(true)), vec![])).unwrap();
    let merged = &chart.resolve().widgets[0].widget;
    let Some(proto::widget::Kind::RangeSlider(slider)) = &merged.kind else {
        panic!("a range slider");
    };
    let Some(proto::widget_domain::Domain::Data(data)) = slider
        .domain
        .as_ref()
        .and_then(|domain| domain.domain.as_ref())
    else {
        panic!("a data domain");
    };
    assert_eq!(
        data.fit,
        Some(proto::DomainFit {
            fit: Some(proto::domain_fit::Fit::Interval(proto::IntervalFit {
                nice_ends: Some(proto::interval_fit::NiceEnds::NiceCount(6.0)),
                zero: Some(true),
                ..Default::default()
            })),
        })
    );
    invalid(
        pair(nice(Some(6.0), None), nice(Some(4.0), None), vec![]),
        second,
        "different values",
    );
}

/// The columns of copies can differ by name, but not by type, and copies in different widgets
/// need not agree.
#[test]
fn copies_match_by_their_own_columns() {
    let mut renamed = range_slider();
    target_of(&mut renamed).column = "b".to_string();
    let chart = read(with_filters(
        plot(vec![declaring(range_slider()), declaring(renamed)]),
        vec![],
    ))
    .unwrap();
    let resolved = chart.resolve();
    let members: Vec<&str> = (resolved.widgets[0].members.iter())
        .map(|member| member.column.as_str())
        .collect();
    assert_eq!(members, ["a", "b"]);

    // A partition column of strings against a column of numbers.
    let mut by_species = range_slider();
    by_species.kind = Some(proto::widget::Kind::Checkboxes(proto::CheckboxesWidget {
        target: Some(target("filters", "species")),
        domain: Some(proto::WidgetDomain {
            domain: Some(proto::widget_domain::Domain::Data(
                proto::WidgetDataDomain::default(),
            )),
        }),
        ..Default::default()
    }));
    let mut numbers = by_species.clone();
    target_of(&mut numbers).column = "a".to_string();
    invalid(
        with_filters(
            plot(vec![
                CartesianLayer {
                    partition: vec![value("species", 0)],
                    ..declaring(by_species)
                },
                declaring(numbers),
            ]),
            strings(&["Adelie"]),
        ),
        "root.plot.cartesian.layers[1].widgets[0]",
        "their columns hold different types",
    );

    // Independent copies may disagree.
    let mut other_selection = range_slider();
    target_of(&mut other_selection).selection = "brush".to_string();
    let mut row = concat(
        proto::Direction::Row,
        vec![
            plot_with("quakes", vec![], vec![declaring(range_slider())]),
            plot_with("plates", vec![], vec![declaring(other_selection)]),
        ],
    );
    set_resolve(&mut row, proto::Sharing::Independent);
    let mut parts = with_filters(row, vec![]);
    parts.0.selections.push(proto::Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        ..Default::default()
    });
    read(parts).unwrap();
}

/// A widget's initial clause names its key and its anchor: an empty owner for a widget that the
/// whole chart shares.
#[test]
fn an_initial_clause_names_a_widget_by_its_key_and_anchor() {
    let clause = |widget: &str, owner: &str| proto::Clause {
        owner: owner.to_string(),
        widget: widget.to_string(),
        conditions: vec![proto::Condition {
            kind: Some(proto::condition::Kind::Range(proto::ValueRange {
                column: "a".to_string(),
                low: Some(ValueRef { index: 0 }),
                ..Default::default()
            })),
        }],
        ..Default::default()
    };
    let chart = |clauses: Vec<proto::Clause>| {
        let mut parts = with_filters(
            plot(vec![declaring(range_slider())]),
            vec![Arc::new(Float64Array::from(vec![2.0])) as ArrayRef],
        );
        parts.0.selections[0].initial = clauses;
        parts
    };
    read(chart(vec![clause("magnitude", "")])).unwrap();
    invalid(
        chart(vec![clause("depth", "")]),
        "selections[0].initial[0]",
        "no widget `depth`",
    );
    invalid(
        chart(vec![clause("magnitude", "scatter")]),
        "selections[0].initial[0]",
        "with the owner `scatter`",
    );
    // A widget writes only its own selection.
    let mut other = chart(vec![]);
    other.0.selections.push(proto::Selection {
        name: "brush".to_string(),
        resolution: proto::SelectionResolution::Global as i32,
        initial: vec![clause("magnitude", "")],
    });
    invalid(
        other,
        "selections[1].initial[0]",
        "writes the selection `filters`",
    );
}

/// The initial clauses of widgets follow the tree order of their owners, after the tools of their
/// plots.
#[test]
fn widget_clauses_follow_the_tree_order() {
    let range = |widget: &str, owner: &str| proto::Clause {
        owner: owner.to_string(),
        widget: widget.to_string(),
        conditions: vec![proto::Condition {
            kind: Some(proto::condition::Kind::Range(proto::ValueRange {
                column: "a".to_string(),
                low: Some(ValueRef { index: 0 }),
                ..Default::default()
            })),
        }],
        ..Default::default()
    };
    let mut depth = range_slider();
    depth.key = "depth".to_string();
    let mut root = selecting();
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        if let Some(proto::plot::Coord::Cartesian(cartesian)) = plot.coord.as_mut() {
            cartesian.layers[0].widgets = vec![depth, range_slider()];
        }
    }
    let mut parts = with_filters(
        root,
        vec![Arc::new(Float64Array::from(vec![2.0])) as ArrayRef],
    );
    parts.0.selections.push(proto::Selection {
        initial: vec![range("", "scatter")],
        ..brush(proto::SelectionResolution::Global)
    });
    parts.0.selections[0].initial = vec![range("magnitude", ""), range("depth", "")];
    // The tool's clause names no widget.
    parts.0.selections[1].initial[0].widget.clear();
    let chart = read(parts).unwrap();
    let order: Vec<&str> = (chart.selections().iter())
        .find(|selection| selection.name == "filters")
        .unwrap()
        .initial
        .iter()
        .map(|clause| clause.widget.as_str())
        .collect();
    assert_eq!(order, ["depth", "magnitude"]);
}

/// Resolve rules of widgets order after the rules of channels, by key.
#[test]
fn resolve_rules_of_widgets_follow_the_rules_of_channels() {
    let mut root = plot(vec![declaring(range_slider())]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.resolve = vec![
            proto::Resolve {
                key: Some(proto::resolve::Key::Widget("magnitude".to_string())),
                sharing: proto::Sharing::Shared as i32,
            },
            proto::Resolve {
                key: Some(proto::resolve::Key::Channel(proto::Channel::Color as i32)),
                sharing: proto::Sharing::Independent as i32,
            },
        ];
    }
    let chart = read(with_filters(root, vec![])).unwrap();
    let Some(proto::node::Node::Plot(plot)) = chart.root().node.as_ref() else {
        panic!("a plot");
    };
    let keys: Vec<_> = plot.resolve.iter().map(|rule| rule.key.clone()).collect();
    assert_eq!(
        keys,
        [
            Some(proto::resolve::Key::Channel(proto::Channel::Color as i32)),
            Some(proto::resolve::Key::Widget("magnitude".to_string())),
        ]
    );
}
