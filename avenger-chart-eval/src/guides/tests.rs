//! The guides of a chart of one plot: the entries of legends, merged legends, the sides of axes,
//! and the fit of a position scale with room for the reach of its marks.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{Array, ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::{
    build::{self, ChartParts},
    list_value,
    proto::{self, cartesian_layer, domain, scale_spec, Channel},
    Chart,
};
use avenger_common::types::SymbolShape;
use avenger_guides::GuideSide;
use avenger_scales::scales::{fitting::FittingScale, ConfiguredScale};
use avenger_style::{
    Cascade, ComputedStyles, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable,
};
use avenger_typst_label::LabelEngine;

use super::{entries, guides, GuideInputs, LegendGuide};
use crate::{fit, style::tree, Registry};

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine().with_number_formatting(Arc::new(
        avenger_format_number_d3::D3NumberFormatProvider::new(),
    ))
}

fn styles_of(tree: &avenger_style::StyleTree) -> ComputedStyles {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Cascade::new(&StyleTable::new().with_author(sheet, Env::new()))
        .compute(tree, &HostInputs::sized(400.0, 300.0))
}

fn penguins() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "flipper",
            Arc::new(Float64Array::from(vec![181.0, 186.0, 195.0, 210.0])) as ArrayRef,
        ),
        (
            "mass",
            Arc::new(Float64Array::from(vec![3750.0, 3800.0, 3250.0, 4500.0])),
        ),
        (
            "species",
            Arc::new(StringArray::from(vec![
                "Adelie", "Adelie", "Gentoo", "Gentoo",
            ])),
        ),
    ])
    .unwrap()
}

/// A chart of `layers` over the penguins.
fn chart(layers: impl FnOnce(proto::TableRef) -> Vec<proto::CartesianLayer>) -> Chart {
    let mut parts = ChartParts::new();
    let table = parts.table(penguins());
    parts
        .finish(build::message(build::cartesian(layers(table))))
        .unwrap()
}

/// The values of each member of `group` from its layer's column.
fn member_values(
    chart: &Chart,
    resolved: &avenger_chart::resolve::Resolved,
    index: usize,
) -> Vec<Option<ArrayRef>> {
    let _ = chart;
    let group = &resolved.scales[index];
    (group.members.iter())
        .map(|member| {
            let layer = &resolved.tree.layers[member.layer];
            match layer.mark.encoding(member.channel)?.value.as_ref()? {
                proto::encoding::Value::Column(name) => layer.table.column_by_name(name).cloned(),
                _ => None,
            }
        })
        .collect()
}

/// The guides of `chart`, with each scale fitted from its members' columns.
fn with_guides<T>(
    chart: &Chart,
    check: impl FnOnce(&super::Guides, &[Option<FittingScale>]) -> T,
) -> T {
    let resolved = chart.resolve();
    let (style_tree, nodes, _) = tree::build(chart, &resolved);
    let styles = styles_of(&style_tree);
    let registry = Registry::new();
    let states = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut positions: Vec<Option<FittingScale>> = Vec::new();
    let mut scales: Vec<Option<ConfiguredScale>> = Vec::new();
    for (index, group) in resolved.scales.iter().enumerate() {
        let values = member_values(chart, &resolved, index);
        let style = &styles[nodes.scales[index]];
        if group.is_position() {
            let values: Vec<_> = values
                .into_iter()
                .map(|values| values.map(|values| (values, None)))
                .collect();
            let inputs = fit::FitInputs {
                chart,
                registry: &registry,
                states: &states,
            };
            let position = fit::position(&inputs, group, style, &values).unwrap();
            positions.push(Some(position.fitting));
            scales.push(None);
        } else {
            let inputs = fit::FitInputs {
                chart,
                registry: &registry,
                states: &states,
            };
            let scale =
                fit::non_position(&inputs, group, style, &styles, &values, &mut warnings).unwrap();
            positions.push(None);
            scales.push(Some(scale));
        }
    }
    let engine = engine();
    let inputs = GuideInputs {
        chart,
        resolved: &resolved,
        nodes: &nodes,
        styles: &styles,
        engine: &engine,
        positions: &positions,
        scales: &scales,
    };
    let guides = guides(&inputs).unwrap();
    check(&guides, &positions)
}

fn symbols(
    table: proto::TableRef,
    extra: impl FnOnce(&mut proto::SymbolMark),
) -> proto::CartesianLayer {
    let mut mark = proto::SymbolMark {
        x: Some(build::scaled_column("flipper")),
        y: Some(build::scaled_column("mass")),
        ..Default::default()
    };
    extra(&mut mark);
    proto::CartesianLayer {
        table: Some(table),
        mark: Some(cartesian_layer::Mark::Symbol(mark)),
        ..Default::default()
    }
}

/// The labels of a symbol legend.
fn legend_labels(guide: &LegendGuide) -> Vec<String> {
    match guide {
        LegendGuide::Legend(legend) => (legend.marks(400.0).items.iter())
            .map(|item| item.label.clone())
            .collect(),
        LegendGuide::Colorbar(_) => panic!("a symbol legend"),
    }
}

#[test]
fn axes_stand_on_their_sides_and_a_discrete_legend_lists_its_categories() {
    let chart = chart(|table| {
        vec![symbols(table, |mark| {
            mark.color = Some(build::scaled_column("species"));
        })]
    });
    with_guides(&chart, |guides, _| {
        let sides: Vec<GuideSide> = guides.axes.iter().map(|axis| axis.side).collect();
        assert!(
            sides.contains(&GuideSide::Bottom) && sides.contains(&GuideSide::Left),
            "{sides:?}"
        );
        assert_eq!(guides.legends.len(), 1);
        assert_eq!(guides.legends[0].side, GuideSide::Right);
        assert_eq!(
            legend_labels(&guides.legends[0].guide),
            ["Adelie", "Gentoo"]
        );
    });
}

#[test]
fn legends_of_equal_categories_merge() {
    let chart = chart(|table| {
        vec![symbols(table, |mark| {
            mark.color = Some(build::scaled_column("species"));
            mark.shape = Some(build::scaled_column("species"));
        })]
    });
    with_guides(&chart, |guides, _| {
        assert_eq!(guides.legends.len(), 1);
        assert_eq!(
            legend_labels(&guides.legends[0].guide),
            ["Adelie", "Gentoo"]
        );
    });
}

#[test]
fn a_continuous_fill_draws_a_colorbar_and_a_size_a_legend_of_ticks() {
    let chart = chart(|table| {
        vec![symbols(table, |mark| {
            mark.fill = Some(build::scaled_column("mass"));
            mark.size = Some(build::scaled_column("flipper"));
        })]
    });
    with_guides(&chart, |guides, _| {
        assert_eq!(guides.legends.len(), 2);
        assert!(matches!(guides.legends[0].guide, LegendGuide::Colorbar(_)));
        let labels = legend_labels(&guides.legends[1].guide);
        assert!(labels.len() >= 2, "{labels:?}");
    });
}

#[test]
fn a_threshold_legend_has_an_entry_for_each_bin() {
    let threshold = avenger_scales::scales::threshold::ThresholdScale::configured(
        vec![10.0, 20.0],
        Arc::new(StringArray::from(vec!["a", "b", "c"])),
    );
    let chart = chart(|table| vec![symbols(table, |_| {})]);
    let (values, labels, _) = entries(
        &threshold,
        &proto::Legend::default(),
        Some(5.0),
        &chart,
        &engine(),
    )
    .unwrap();
    assert_eq!(values.len(), 3);
    assert_eq!(labels, ["< 10", "10 – 20", "≥ 20"]);
}

/// A symbol at the largest value stays inside the plot: its domain reaches past the data by the
/// symbol's reach.
#[test]
fn a_position_fit_leaves_room_for_the_reach() {
    let chart = chart(|table| vec![symbols(table, |_| {})]);
    let resolved = chart.resolve();
    let (style_tree, nodes, _) = tree::build(&chart, &resolved);
    let styles = styles_of(&style_tree);
    let index = (resolved.scales.iter())
        .position(|group| group.channel == Channel::X)
        .unwrap();
    let group = &resolved.scales[index];
    let values = member_values(&chart, &resolved, index);
    let reach = vec![10.0; 4];
    let values: Vec<_> = (values.into_iter())
        .map(|values| values.map(|values| (values, Some((reach.clone(), reach.clone())))))
        .collect();
    let (registry, states) = (Registry::new(), BTreeMap::new());
    let inputs = fit::FitInputs {
        chart: &chart,
        registry: &registry,
        states: &states,
    };
    let position = fit::position(&inputs, group, &styles[nodes.scales[index]], &values).unwrap();
    let at = position.at((0.0, 200.0)).unwrap();
    assert!(at.marks_fit);
    let (low, high) = at.scale.numeric_interval_domain().unwrap();
    assert!(low < 181.0 && high > 210.0, "{low} {high}");
    assert!(!position.clip);

    // An explicit domain clips its plot.
    let explicit = proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Linear(proto::LinearScale::default())),
        domain: Some(proto::Domain {
            domain: Some(domain::Domain::Explicit(proto::ValueRef { index: 0 })),
        }),
        ..Default::default()
    };
    let mut parts = ChartParts::new();
    let table = parts.table(penguins());
    parts.value(list_value(Arc::new(Float64Array::from(vec![150.0, 250.0]))));
    let mut layer = symbols(table, |_| {});
    if let Some(cartesian_layer::Mark::Symbol(mark)) = layer.mark.as_mut() {
        mark.x.as_mut().unwrap().scale = Some(explicit);
    }
    let clipped = parts
        .finish(build::message(build::cartesian(vec![layer])))
        .unwrap();
    let resolved = clipped.resolve();
    let (style_tree, nodes, _) = tree::build(&clipped, &resolved);
    let styles = styles_of(&style_tree);
    let index = (resolved.scales.iter())
        .position(|group| group.channel == Channel::X)
        .unwrap();
    let inputs = fit::FitInputs {
        chart: &clipped,
        ..inputs
    };
    let position = fit::position(
        &inputs,
        &resolved.scales[index],
        &styles[nodes.scales[index]],
        &[],
    )
    .unwrap();
    assert!(position.clip);
    let at = position.at((0.0, 200.0)).unwrap();
    assert_eq!(at.scale.numeric_interval_domain().unwrap(), (150.0, 250.0));
}

/// Two scales of one channel over equal categories, such as two independent color scales, keep a
/// legend each, so each shows its own swatches.
#[test]
fn legends_of_two_scales_of_one_channel_stay_apart() {
    let mut parts = ChartParts::new();
    let table = parts.table(penguins());
    let layer = |table| {
        symbols(table, |mark| {
            mark.color = Some(build::scaled_column("species"));
        })
    };
    let mut root = build::cartesian(vec![layer(table), layer(table)]);
    if let Some(proto::node::Node::Plot(plot)) = root.node.as_mut() {
        plot.resolve = vec![proto::Resolve {
            key: Some(proto::resolve::Key::Channel(Channel::Color as i32)),
            sharing: proto::Sharing::Independent as i32,
        }];
    }
    let chart = parts.finish(build::message(root)).unwrap();
    with_guides(&chart, |guides, _| assert_eq!(guides.legends.len(), 2));
}

/// The shapes of the symbols of a scene group and its groups.
fn swatch_shapes(group: &avenger_scenegraph::marks::group::SceneGroup) -> Vec<String> {
    use avenger_scenegraph::marks::mark::SceneMark;
    let mut shapes = Vec::new();
    for mark in &group.marks {
        match mark {
            SceneMark::Group(group) => shapes.extend(swatch_shapes(group)),
            SceneMark::Symbol(symbol) => {
                shapes.extend(symbol.shapes.iter().map(|shape| format!("{shape:?}")))
            }
            _ => {}
        }
    }
    shapes
}

/// The swatches of a legend draw the constant shape of its layer's marks.
#[test]
fn a_legend_draws_the_constant_shape_of_its_marks() {
    let mut parts = ChartParts::new();
    let table = parts.table(penguins());
    let square = parts.value(Arc::new(StringArray::from(vec!["square"])));
    let layer = symbols(table, |mark| {
        mark.color = Some(build::scaled_column("species"));
        mark.shape = Some(proto::Encoding {
            value: Some(proto::encoding::Value::Constant(square)),
            ..Default::default()
        });
    });
    let chart = parts
        .finish(build::message(build::cartesian(vec![layer])))
        .unwrap();
    with_guides(&chart, |guides, _| {
        let LegendGuide::Legend(legend) = &guides.legends[0].guide else {
            panic!("a symbol legend");
        };
        let shapes = swatch_shapes(&legend.marks(400.0).group);
        let square = format!("{:?}", SymbolShape::from_vega_str("square").unwrap());
        assert!(
            !shapes.is_empty() && shapes.iter().all(|shape| *shape == square),
            "{shapes:?}"
        );
    });
}
