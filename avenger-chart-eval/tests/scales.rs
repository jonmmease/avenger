//! The scales of an evaluated chart, as a host that shows them apart from the chart reads them.

use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::prelude::*;
use avenger_chart::proto::Channel;
use avenger_chart::FitKind;
use avenger_chart_eval::{ChartEvaluator, Registry, ScaleIdentity};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};

fn cascade() -> Arc<Cascade> {
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
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

#[test]
fn each_scale_of_the_chart_has_its_domain_and_its_kind_of_fit() {
    let rows = RecordBatch::try_from_iter([
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
                "Adelie",
                "Adelie",
                "Gentoo",
                "Chinstrap",
            ])),
        ),
    ])
    .unwrap();
    let layer = CartesianLayer::symbol(
        rows,
        SymbolMark::builder()
            .x("flipper")
            .y("mass")
            .color("species"),
    );
    let chart = Chart::builder(Plot::cartesian([layer])).build().unwrap();
    let engine = avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
    let mut evaluator = ChartEvaluator::new(chart, cascade(), engine, Registry::new()).unwrap();
    // Before an evaluation, the chart draws no scale.
    assert!(evaluator.scales().is_empty());
    evaluator
        .evaluate(&HostInputs::sized(400.0, 300.0))
        .unwrap();
    let scales = evaluator.scales();
    let channel = |identity: &ScaleIdentity| match identity {
        ScaleIdentity::Plot { channel, .. } | ScaleIdentity::Family { channel, .. } => *channel,
    };
    let of = |wanted: Channel| {
        (scales.iter())
            .find(|scale| channel(&scale.identity) == wanted)
            .unwrap_or_else(|| panic!("a scale of {wanted:?}"))
    };
    // The position scales fit their intervals, and the color scale has the categories of the rows.
    let x = of(Channel::X);
    assert_eq!(x.kind, FitKind::Interval);
    assert_eq!(x.scale.domain().len(), 2);
    assert_eq!(of(Channel::Y).kind, FitKind::Interval);
    let color = of(Channel::Color);
    assert_eq!(color.kind, FitKind::Categories);
    assert_eq!(color.scale.domain().len(), 3);
    // Each scale scales the chart's one plot.
    assert!(scales.iter().all(|scale| scale.plots == [0]));
}
