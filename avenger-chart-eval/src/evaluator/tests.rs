//! The installation of a new chart, which keeps the computed styles of an equal style tree.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use avenger_chart::prelude::*;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_typst_label::LabelEngine;

use super::{ChartEvaluator, PreparedChart};
use crate::{Registry, UpdateOptions};

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(
            avenger_format_number_d3::D3NumberFormatProvider::new(),
        ))
        .with_datetime_formatting(Arc::new(
            avenger_format_datetime_d3::D3DateTimeFormatProvider::new(),
        ))
}

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

/// A scatter plot of `x` and `y` colored by `group`.
fn scatter(x: &[f64]) -> Chart {
    let table = RecordBatch::try_from_iter([
        ("x", Arc::new(Float64Array::from(x.to_vec())) as ArrayRef),
        ("y", Arc::new(Float64Array::from(x.to_vec()))),
        (
            "group",
            Arc::new(StringArray::from_iter_values(
                (0..x.len()).map(|row| ["a", "b"][row % 2]),
            )),
        ),
    ])
    .unwrap();
    let symbols = CartesianLayer::symbol(table, SymbolMark::builder().x("x").y("y").color("group"));
    Chart::builder(Plot::cartesian([symbols])).build().unwrap()
}

#[test]
fn an_update_of_the_data_keeps_the_computed_styles_of_an_equal_tree() {
    let mut evaluator = ChartEvaluator::new(
        scatter(&[1.0, 2.0, 3.0]),
        cascade(),
        engine(),
        Registry::new(),
    )
    .unwrap();
    let host = HostInputs::sized(320.0, 240.0);
    evaluator.evaluate(&host).unwrap();
    let prepared =
        PreparedChart::new(scatter(&[4.0, 5.0, 6.0, 7.0]), &engine(), &Registry::new()).unwrap();
    evaluator.install(prepared, &UpdateOptions::new()).unwrap();
    assert!(evaluator.styles.is_some());
    assert!(evaluator.prepared.is_none());
    evaluator.evaluate(&host).unwrap();

    // A chart with another tree, here without a color legend, computes its styles again.
    let table = RecordBatch::try_from_iter([(
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0])) as ArrayRef,
    )])
    .unwrap();
    let symbols = CartesianLayer::symbol(table, SymbolMark::builder().x("x").y("x"));
    let other = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let prepared = PreparedChart::new(other, &engine(), &Registry::new()).unwrap();
    evaluator.install(prepared, &UpdateOptions::new()).unwrap();
    assert!(evaluator.styles.is_none());
    evaluator.evaluate(&host).unwrap();
}
