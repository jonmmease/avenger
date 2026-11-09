//! Columns of lumps: the kept categories by descending weight and the Other group last, on a band
//! axis and in a legend, with labels that name the Other group's categories.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, Int64Array, RecordBatch, StringArray, StructArray},
    datatypes::{DataType, Field, Fields, Int64Type},
};
use avenger_chart::prelude::*;

use crate::support::*;

/// A column of lumps of `values`, each kept, or the Other group where `kept` is false, with its
/// weight and its number of categories.
fn lumps(values: &[Option<i64>], kept: &[bool], weights: &[f64], counts: &[i64]) -> ArrayRef {
    let fields = Fields::from(vec![
        Field::new("value", DataType::Int64, true),
        Field::new("other", DataType::Utf8, true),
        Field::new("weight", DataType::Float64, true),
        Field::new("category_count", DataType::Int64, false),
    ]);
    let value: Int64Array = (values.iter().zip(kept))
        .map(|(value, kept)| if *kept { *value } else { None })
        .collect();
    let other: StringArray = kept.iter().map(|kept| (!kept).then_some("Other")).collect();
    Arc::new(StructArray::new(
        fields,
        vec![
            Arc::new(value) as ArrayRef,
            Arc::new(other),
            Arc::new(Float64Array::from(weights.to_vec())),
            Arc::new(Int64Array::from(counts.to_vec())),
        ],
        None,
    ))
}

/// The cars with their cylinders lumped to the 3 most common counts, 4, 8, and 6, and the Other
/// group of the 3 and 5 cylinder cars, as `lump(cylinders, 'n', 3) OVER ()` writes them.
fn lumped_cars() -> RecordBatch {
    let cars = cars_dataset();
    let cylinders = cars
        .column_by_name("cylinders")
        .unwrap()
        .as_primitive::<Int64Type>();
    let count = |n: i64| cylinders.iter().filter(|c| *c == Some(n)).count() as f64;
    let kept = |n: i64| matches!(n, 4 | 6 | 8);
    let other = count(3) + count(5);
    let values: Vec<Option<i64>> = cylinders.iter().collect();
    let keep: Vec<bool> = cylinders.iter().map(|c| kept(c.unwrap())).collect();
    let weights: Vec<f64> = (cylinders.iter())
        .map(|c| {
            if kept(c.unwrap()) {
                count(c.unwrap())
            } else {
                other
            }
        })
        .collect();
    let counts: Vec<i64> = cylinders
        .iter()
        .map(|c| if kept(c.unwrap()) { 1 } else { 2 })
        .collect();
    let mut columns: Vec<(&str, ArrayRef)> =
        vec![("lumped", lumps(&values, &keep, &weights, &counts))];
    for name in ["weight", "mpg", "origin"] {
        columns.push((name, cars.column_by_name(name).unwrap().clone()));
    }
    RecordBatch::try_from_iter(columns).unwrap()
}

/// The counts of the cars by their lumped cylinders: one row for each kept count and one for the
/// Other group, as a query grouped by the lump writes them.
fn counted_cars() -> RecordBatch {
    let lumped = lumps(
        &[Some(4), Some(8), Some(6), None],
        &[true, true, true, false],
        &[207.0, 108.0, 84.0, 7.0],
        &[1, 1, 1, 2],
    );
    RecordBatch::try_from_iter([
        ("lumped", lumped),
        (
            "cars",
            Arc::new(Float64Array::from(vec![207.0, 108.0, 84.0, 7.0])) as ArrayRef,
        ),
    ])
    .unwrap()
}

/// The labels of the scene's band axis, without its title, in their order along it.
fn axis_labels(evaluated: &avenger_chart_eval::EvaluatedChart) -> Vec<String> {
    let mut labels: Vec<(String, [f32; 2])> = scene_text_positions(&evaluated.scene.marks)
        .into_iter()
        .filter(|(_, at)| at[1] > evaluated.plots[0].rect[1] + evaluated.plots[0].rect[3])
        .filter(|(text, _)| text != "cylinders")
        .collect();
    labels.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
    labels.into_iter().map(|(text, _)| text).collect()
}

/// Bars of the counted cars, by their lumped cylinders through `scale`, with an axis titled
/// `cylinders` and the labels `labels`.
fn bars(scale: ScaleBuilder, labels: Option<LabelsBuilder>) -> Chart {
    let mut axis = Axis::builder().title("cylinders");
    if let Some(labels) = labels {
        axis = axis.labels(labels);
    }
    let x = Encoding::scaled_field("lumped").scale(scale.axis(axis));
    let bars = CartesianLayer::bar(counted_cars(), BarMark::builder().x(x).y("cars"));
    Chart::builder(Plot::cartesian([bars]))
        .number_format_syntax("d3")
        .build()
        .unwrap()
}

#[test]
fn a_band_axis_of_lumps_places_the_kept_categories_by_weight_and_other_last() {
    let chart = bars(Scale::builder(), None);
    let evaluated = render(&chart, [360.0, 240.0], "lump-bars");
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let labels = axis_labels(&evaluated);
    assert_eq!(labels[..4], ["4", "8", "6", "Other"], "{labels:?}");

    // An explicit sort orders the kept categories, and Other stays last.
    let sorted = bars(
        Scale::builder().domain(Domain::data(DataDomain::builder().fit(
            DomainFit::categories(CategoryFit::builder().sort(CategorySort::Ascending)),
        ))),
        None,
    );
    let labels = axis_labels(&evaluate(&sorted, [360.0, 240.0]));
    assert_eq!(labels[..4], ["4", "6", "8", "Other"], "{labels:?}");
}

#[test]
fn labels_of_lumps_name_the_count_and_the_weight_of_the_other_group() {
    let chart = bars(
        Scale::builder(),
        Some(Labels::builder().markup("#value (#count, #weight)")),
    );
    let evaluated = render(&chart, [360.0, 240.0], "lump-bars-counted");
    let labels = axis_labels(&evaluated);
    let other = labels.last().unwrap();
    assert!(
        other.contains("Other") && other.contains("#(2)"),
        "{labels:?}"
    );
    assert!(other.contains("\"7\""), "{labels:?}");
    assert!(labels[0].contains("207"), "{labels:?}");
}

#[test]
fn a_legend_of_lumps_ends_with_other() {
    let symbols = CartesianLayer::symbol(
        lumped_cars(),
        SymbolMark::builder().x("weight").y("mpg").color("lumped"),
    );
    let chart = Chart::builder(Plot::cartesian([symbols]))
        .number_format_syntax("d3")
        .build()
        .unwrap();
    let evaluated = render(&chart, [480.0, 320.0], "lump-legend");
    assert!(evaluated.warnings.is_empty(), "{:#?}", evaluated.warnings);
    let texts = scene_texts(&evaluated.scene.marks);
    let at = |text: &str| texts.iter().position(|known| known == text);
    let (four, eight, six, other) = (at("4"), at("8"), at("6"), at("Other"));
    assert!(four < eight && eight < six && six < other, "{texts:?}");
}

#[test]
fn a_label_shared_by_partitions_with_other_counts_names_no_count() {
    // Two regions lump their categories apart, so their Other groups hold 2 and 3 categories.
    let lumped = lumps(
        &[Some(1), None, Some(1), None],
        &[true, false, true, false],
        &[5.0, 3.0, 4.0, 6.0],
        &[1, 2, 1, 3],
    );
    let table = RecordBatch::try_from_iter([
        ("lumped", lumped),
        (
            "x",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as ArrayRef,
        ),
        (
            "region",
            Arc::new(StringArray::from(vec!["a", "a", "b", "b"])) as ArrayRef,
        ),
    ])
    .unwrap();
    let color = Encoding::scaled_field("lumped")
        .scale(Legend::builder().labels(Labels::builder().markup("#value (#count)")));
    let symbols = CartesianLayer::symbol(table, SymbolMark::builder().x("x").y("x").color(color));
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let texts = scene_texts(&evaluate(&chart, [320.0, 240.0]).scene.marks);
    assert!(
        texts.iter().any(|text| text.contains("(#(\"?\"))")),
        "{texts:?}"
    );
}
