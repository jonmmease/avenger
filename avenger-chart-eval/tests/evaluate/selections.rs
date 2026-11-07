//! Conditional values and selection domains from the initial clauses of selections.

use std::sync::Arc;

use arrow::array::StringArray;
use avenger_chart::{
    build::{self},
    proto::{self, cartesian_layer, condition, domain, Channel},
    Chart,
};
use avenger_chart_eval::EvaluatedChart;
use avenger_color::ColorOrGradient;

use crate::support::*;

/// A scatter of the cars whose rows outside the brush are gray, with the brush's `resolution` and
/// initial clauses. The plot `plot` and its layer `points` each have a box of the brush, so each
/// owns clauses.
fn brushed(
    resolution: proto::SelectionResolution,
    predicate: proto::Selected,
    initial: Clauses,
) -> Chart {
    let mut parts = Parts::new();
    let table = parts.table(cars());
    let gray = parts.value(Arc::new(StringArray::from(vec!["lightgray"])));
    let initial = initial(&mut parts);
    let color = proto::Encoding {
        conditional: vec![proto::ConditionalValue {
            predicate: Some(not(selected(predicate))),
            value: Some(proto::conditional_value::Value::Constant(gray)),
        }],
        ..col("origin")
    };
    let symbols = proto::CartesianLayer {
        id: "points".to_string(),
        tools: vec![proto::LayerTool {
            kind: Some(proto::layer_tool::Kind::Select(select("brush"))),
        }],
        ..layer(
            table,
            cartesian_layer::Mark::Symbol(proto::SymbolMark {
                x: Some(col("weight")),
                y: Some(col("mpg")),
                color: Some(color),
                ..Default::default()
            }),
        )
    };
    let mut root = build::cartesian(vec![symbols]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.id = "plot".to_string();
        plot.tools = vec![proto::Tool {
            kind: Some(proto::tool::Kind::Select(select("brush"))),
        }];
    }
    let brush = proto::Selection {
        name: "brush".to_string(),
        resolution: resolution as i32,
        initial,
    };
    parts.chart(root, vec![brush])
}

/// Whether each symbol of the first plot is gray.
fn grays(evaluated: &EvaluatedChart) -> Vec<bool> {
    let symbol = plot_symbols(evaluated);
    (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
        .map(|color| {
            matches!(color, ColorOrGradient::Color(color)
                if (color[0] - color[1]).abs() < 1e-3 && (color[1] - color[2]).abs() < 1e-3)
        })
        .collect()
}

fn gray_rows(chart: &Chart) -> Vec<bool> {
    grays(&evaluate(chart, [320.0, 240.0]))
}

#[test]
fn a_condition_grays_the_rows_outside_a_brush() {
    use proto::SelectionResolution::Union;
    let without = brushed(Union, brush_of("brush"), no_clauses());
    let evaluated = render(&without, [320.0, 240.0], "condition-without-brush");
    assert!(grays(&evaluated).iter().all(|gray| !gray));

    let with = brushed(
        Union,
        brush_of("brush"),
        weights_between("plot", 2500.0, 3500.0),
    );
    let evaluated = render(&with, [320.0, 240.0], "condition-with-brush");
    let grays = grays(&evaluated);
    assert!(grays.iter().any(|gray| *gray) && grays.iter().any(|gray| !gray));
}

#[test]
fn a_cyclic_range_selects_across_its_wrap() {
    use proto::SelectionResolution::Union;
    let cyclic = |start: f64, end: f64, period: f64| -> Clauses {
        Box::new(move |_| {
            vec![clause(
                "plot",
                condition::Kind::CyclicRange(proto::CyclicRange {
                    column: "weight".to_string(),
                    channel: Channel::Unspecified as i32,
                    start,
                    end,
                    period,
                }),
            )]
        })
    };
    let rows = |clauses| gray_rows(&brushed(Union, brush_of("brush"), clauses));
    // With a period of 3000, a range from 4500 to 2300 holds 4500 to 5300 and 1500 to 2300.
    let wrapped = rows(cyclic(4500.0, 2300.0, 3000.0));
    for (index, weight) in weights().iter().enumerate() {
        let inside = (4500.0..=5300.0).contains(weight) || (1500.0..=2300.0).contains(weight);
        assert_eq!(wrapped[index], !inside, "{weight}");
    }
    assert!(wrapped.iter().any(|gray| *gray) && wrapped.iter().any(|gray| !gray));
    // The condition has no origin of the wrap, so the same range a period earlier selects alike.
    assert_eq!(rows(cyclic(1500.0, 2300.0, 3000.0)), wrapped);
    // A range a whole period wide holds every row.
    assert!(rows(cyclic(-180.0, 180.0, 360.0)).iter().all(|gray| !gray));
}

#[test]
fn selected_predicates_follow_their_empty_behavior_owners_and_resolutions() {
    use proto::SelectionResolution::{Intersect, Union};
    let predicate = |empty_selects_none: bool, exclude_own: bool| proto::Selected {
        selection: "brush".into(),
        empty_selects_none,
        exclude_own,
    };
    // Without clauses, a predicate selects every row, or none when it says so.
    let rows = gray_rows(&brushed(Union, predicate(true, false), no_clauses()));
    assert!(rows.iter().all(|gray| *gray));
    let rows = gray_rows(&brushed(Union, predicate(false, false), no_clauses()));
    assert!(rows.iter().all(|gray| !gray));

    // A predicate that skips its own plot's clause selects every row.
    let own = weights_between("plot", 2500.0, 3000.0);
    let rows = gray_rows(&brushed(Union, predicate(false, true), own));
    assert!(rows.iter().all(|gray| !gray));

    // Union selects the rows of either owner, and intersect only the rows of both.
    let both = || -> Clauses {
        Box::new(|parts| {
            vec![
                weight_range(parts, "plot", 2000.0, 3000.0),
                weight_range(parts, "points", 2800.0, 4000.0),
            ]
        })
    };
    let union = gray_rows(&brushed(Union, predicate(false, false), both()));
    let intersect = gray_rows(&brushed(Intersect, predicate(false, false), both()));
    for (index, weight) in weights().iter().enumerate() {
        assert_eq!(
            union[index],
            !(2000.0..=4000.0).contains(weight),
            "{weight}"
        );
        assert_eq!(
            intersect[index],
            !(2800.0..=3000.0).contains(weight),
            "{weight}"
        );
    }
}

#[test]
fn a_test_of_a_missing_column_selects_no_row_and_warns() {
    let price = Box::new(|parts: &mut Parts| {
        let values = parts.list(numbers(&[1.0]));
        vec![clause(
            "points",
            condition::Kind::Set(proto::ValueSet {
                columns: vec![proto::SetColumn {
                    column: "price".to_string(),
                    channel: Channel::Unspecified as i32,
                    values: Some(values),
                }],
            }),
        )]
    });
    let chart = brushed(proto::SelectionResolution::Union, brush_of("brush"), price);
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(grays(&evaluated).iter().all(|gray| *gray));
    assert!(warns(&evaluated, "`price`"), "{:?}", evaluated.warnings);
}

#[test]
fn a_selection_domain_takes_the_envelope_of_a_brush_and_its_fallback_without_one() {
    let chart = |initial: Clauses| {
        let mut parts = Parts::new();
        let table = parts.table(cars());
        let initial = initial(&mut parts);
        let spec = proto::ScaleSpec {
            domain: Some(proto::Domain {
                domain: Some(domain::Domain::Selection(proto::SelectionDomain {
                    selection: "brush".to_string(),
                    channel: Channel::X as i32,
                    fallback: Some(proto::selection_domain::Fallback::Data(
                        proto::DataDomain::default(),
                    )),
                })),
            }),
            ..Default::default()
        };
        let symbols = proto::CartesianLayer {
            id: "points".to_string(),
            tools: vec![proto::LayerTool {
                kind: Some(proto::layer_tool::Kind::Select(select("brush"))),
            }],
            ..layer(
                table,
                cartesian_layer::Mark::Symbol(proto::SymbolMark {
                    x: Some(scaled("weight", spec)),
                    y: Some(col("mpg")),
                    ..Default::default()
                }),
            )
        };
        let mut root = build::cartesian(vec![symbols]);
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
        parts.chart(root, vec![brush])
    };
    let fallback = domain(&evaluate(&chart(no_clauses()), [320.0, 240.0]), 0);
    assert!(fallback.0 <= 1800.0 && fallback.1 >= 4790.0, "{fallback:?}");
    let two = chart(Box::new(|parts| {
        vec![
            weight_range(parts, "plot", 2500.0, 3000.0),
            weight_range(parts, "points", 2800.0, 3600.0),
        ]
    }));
    assert_eq!(domain(&evaluate(&two, [320.0, 240.0]), 0), (2500.0, 3600.0));
}
