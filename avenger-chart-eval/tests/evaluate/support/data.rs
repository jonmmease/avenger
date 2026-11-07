//! The tables that the charts draw.

use std::sync::Arc;

use arrow::{
    array::{
        Array, ArrayRef, Date32Array, Float64Array, Int64Array, ListArray, RecordBatch,
        StringArray, StructArray, UInt32Array, UInt8Array,
    },
    buffer::OffsetBuffer,
    datatypes::{DataType, Field},
};

use super::*;

/// A table of 24 cars: a weight, a fuel economy, a count, an origin, a cylinder count, and a
/// model year.
pub(crate) fn cars() -> RecordBatch {
    let rows = 24;
    let weight: Vec<f64> = (0..rows)
        .map(|row| 1800.0 + ((row * 37) % 24) as f64 * 130.0)
        .collect();
    let mpg: Vec<f64> = (weight.iter().enumerate())
        .map(|(row, weight)| 48.0 - weight / 140.0 + ((row * 7) % 5) as f64)
        .collect();
    let origins = ["USA", "Europe", "Japan"];
    RecordBatch::try_from_iter([
        ("weight", Arc::new(Float64Array::from(weight)) as ArrayRef),
        ("mpg", Arc::new(Float64Array::from(mpg))),
        (
            "count",
            Arc::new(Int64Array::from_iter_values(
                (0..rows).map(|row| ((row * 5) % 9) as i64 + 1),
            )),
        ),
        (
            "origin",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| origins[row % 3]),
            )),
        ),
        (
            "cylinders",
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| ["4", "6", "8"][(row / 3) % 3]),
            )),
        ),
        (
            "year",
            Arc::new(Date32Array::from_iter_values(
                (0..rows as i32).map(|row| 3650 + row * 120),
            )),
        ),
    ])
    .unwrap()
}

pub(crate) fn weights() -> Vec<f64> {
    let cars = cars();
    let weights = cars.column_by_name("weight").unwrap();
    (weights.as_any().downcast_ref::<Float64Array>())
        .unwrap()
        .values()
        .to_vec()
}

/// Three series of twelve months each.
pub(crate) fn series() -> RecordBatch {
    let names = ["A", "B", "C"];
    let mut dates = Vec::new();
    let mut values = Vec::new();
    let mut symbols = Vec::new();
    for (index, name) in names.iter().enumerate() {
        for month in 0..12 {
            dates.push(18262 + month * 30);
            let wave = ((month as f64 + index as f64 * 2.0) * 0.6).sin();
            values.push(20.0 + index as f64 * 8.0 + wave * 6.0);
            symbols.push(*name);
        }
    }
    RecordBatch::try_from_iter([
        ("date", Arc::new(Date32Array::from(dates)) as ArrayRef),
        ("value", Arc::new(Float64Array::from(values))),
        ("symbol", Arc::new(StringArray::from(symbols))),
    ])
    .unwrap()
}

/// Six categories with an amount each.
pub(crate) fn categories() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "category",
            Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F"])) as ArrayRef,
        ),
        ("amount", numbers(&[28.0, 55.0, 43.0, 91.0, 81.0, 53.0])),
    ])
    .unwrap()
}

/// A table of eight images of 4 by 4 pixels, each one color, at points.
pub(crate) fn images() -> RecordBatch {
    let colors = [
        [230u8, 85, 13, 255],
        [49, 130, 189, 255],
        [49, 163, 84, 255],
        [117, 107, 177, 255],
    ];
    let rows = 8;
    let bytes: Vec<u8> = (0..rows)
        .flat_map(|row| (0..16).flat_map(move |_| colors[row % colors.len()]))
        .collect();
    let pixels = ListArray::new(
        Arc::new(Field::new_list_field(DataType::UInt8, false)),
        OffsetBuffer::from_lengths(vec![64; rows]),
        Arc::new(UInt8Array::from(bytes)),
        None,
    );
    let images = StructArray::from(vec![
        (
            Arc::new(Field::new("width", DataType::UInt32, false)),
            Arc::new(UInt32Array::from(vec![4; rows])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("height", DataType::UInt32, false)),
            Arc::new(UInt32Array::from(vec![4; rows])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("data", pixels.data_type().clone(), true)),
            Arc::new(pixels) as ArrayRef,
        ),
    ]);
    RecordBatch::try_from_iter([
        (
            "x",
            Arc::new(Float64Array::from_iter_values(
                (0..rows).map(|row| row as f64),
            )) as ArrayRef,
        ),
        (
            "y",
            Arc::new(Float64Array::from_iter_values(
                (0..rows).map(|row| ((row * 3) % 5) as f64),
            )),
        ),
        ("image", Arc::new(images)),
        (
            "size",
            Arc::new(Float64Array::from_iter_values(
                (0..rows).map(|row| 12.0 + 4.0 * (row % 4) as f64),
            )),
        ),
    ])
    .unwrap()
}

/// Four labels at points, with a weight, a style, and a width for each.
pub(crate) fn labels() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("x", numbers(&[1.0, 2.0, 3.0, 4.0])),
        ("y", numbers(&[1.0, 3.0, 2.0, 4.0])),
        (
            "name",
            Arc::new(StringArray::from(vec![
                "A short label",
                "A label of several words that wraps",
                "Long names wrap at their width",
                "The last label",
            ])) as ArrayRef,
        ),
        ("weight", numbers(&[300.0, 400.0, 700.0, 900.0])),
        (
            "style",
            Arc::new(StringArray::from(vec![
                "normal", "italic", "oblique", "normal",
            ])),
        ),
        ("width", numbers(&[60.0, 90.0, 120.0, 60.0])),
    ])
    .unwrap()
}

/// The months of a year with their precipitation, in millimeters, and their mean temperature,
/// in degrees Celsius.
pub(crate) fn weather() -> RecordBatch {
    RecordBatch::try_from_iter([
        ("month", Arc::new(months()) as ArrayRef),
        (
            "precipitation",
            numbers(&[
                49.0, 36.0, 43.0, 61.0, 74.0, 80.0, 91.0, 84.0, 66.0, 58.0, 51.0, 47.0,
            ]),
        ),
        (
            "temperature",
            numbers(&[
                -2.0, 0.0, 5.0, 11.0, 17.0, 22.0, 25.0, 24.0, 19.0, 13.0, 6.0, 1.0,
            ]),
        ),
    ])
    .unwrap()
}

pub(crate) fn months() -> StringArray {
    StringArray::from(vec![
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ])
}

/// Paths of a region and a channel, with a null channel for a row of a whole region.
pub(crate) fn region_paths(regions: &[&str], channels: &[Option<&str>]) -> ArrayRef {
    let field = |name| Arc::new(Field::new(name, DataType::Utf8, true));
    Arc::new(StructArray::from(vec![
        (
            field("region"),
            Arc::new(StringArray::from(regions.to_vec())) as ArrayRef,
        ),
        (
            field("channel"),
            Arc::new(StringArray::from(channels.to_vec())) as ArrayRef,
        ),
    ]))
}
