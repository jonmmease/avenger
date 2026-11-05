//! Columns of charts in Arrow arrays: the data buffers of the charts, and the rows that decode in
//! place or from a copy.

use std::{ops::Range, sync::Arc};

use arrow::{
    array::{
        Array, ArrayData, ArrayRef, BinaryArray, BinaryViewArray, ByteView, Float64Array,
        Int64Array, LargeBinaryArray, RecordBatch,
    },
    datatypes::DataType,
};
use avenger_chart::{
    chart_field, charts_from_arrow, charts_to_arrow, proto, Chart, ChartError, Decode,
    EXTENSION_NAME,
};

/// A scatter plot of `rows` rows.
fn scatter(rows: usize) -> Chart {
    let x: Vec<f64> = (0..rows).map(|row| row as f64).collect();
    let y: Vec<f64> = x.iter().map(|x| x * x).collect();
    let table = RecordBatch::try_from_iter([
        ("x", Arc::new(Float64Array::from(x)) as ArrayRef),
        ("y", Arc::new(Float64Array::from(y)) as ArrayRef),
    ])
    .unwrap();
    Chart::unit(table, proto::SymbolMark::default())
        .x("x")
        .y("y")
        .build()
        .unwrap()
}

/// Whether every buffer of `data`, of its validity, and of its children lies within `range`.
fn within(data: &ArrayData, range: &Range<usize>) -> bool {
    let inside = |pointer: *const u8| range.contains(&(pointer as usize));
    data.buffers().iter().all(|buffer| inside(buffer.as_ptr()))
        && data
            .nulls()
            .is_none_or(|nulls| inside(nulls.buffer().as_ptr()))
        && data.child_data().iter().all(|child| within(child, range))
}

/// Whether the columns of the chart's tables lie within the data buffer `buffer` of `array`.
fn in_buffer(chart: &Chart, array: &BinaryViewArray, buffer: usize) -> bool {
    let buffer = &array.data_buffers()[buffer];
    let range = buffer.as_ptr() as usize..buffer.as_ptr() as usize + buffer.len();
    chart.tables().iter().all(|table| {
        table
            .columns()
            .iter()
            .all(|column| within(&column.to_data(), &range))
    })
}

fn view(array: &BinaryViewArray, row: usize) -> ByteView {
    ByteView::from(array.views()[row])
}

#[test]
fn the_field_of_a_column_names_the_extension() {
    let field = chart_field("chart", true);
    assert_eq!(field.data_type(), &DataType::BinaryView);
    assert_eq!(
        field
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some(EXTENSION_NAME)
    );
}

#[test]
fn a_large_chart_takes_a_buffer_of_its_own() {
    let small = scatter(5);
    // Two columns of 5000 numbers are more than 64 KiB.
    let large = scatter(5000);
    let array = charts_to_arrow([Some(&small), Some(&large), None, Some(&small)]).unwrap();
    assert_eq!(array.len(), 4);
    assert!(array.is_null(2));
    assert_eq!(array.data_buffers().len(), 2);

    // The small charts share the first buffer, each at a multiple of 64 bytes.
    let (first, last) = (view(&array, 0), view(&array, 3));
    assert_eq!((first.buffer_index, first.offset), (0, 0));
    assert_eq!(last.buffer_index, 0);
    assert!(last.offset > 0 && last.offset % 64 == 0);

    // The large chart's buffer holds its bytes and nothing else.
    let own = view(&array, 1);
    assert_eq!((own.buffer_index, own.offset), (1, 0));
    assert_eq!(own.length as usize, array.data_buffers()[1].len());
}

#[test]
fn rows_decode_in_place_or_from_a_copy() {
    let small: Vec<Chart> = (0..10).map(|rows| scatter(rows + 1)).collect();
    let large = scatter(5000);
    let mut charts: Vec<Option<&Chart>> = small.iter().map(Some).collect();
    charts.push(Some(&large));
    let array = charts_to_arrow(charts.iter().copied()).unwrap();

    let decoded = charts_from_arrow(&array, Decode::Auto).unwrap();
    for (decoded, original) in decoded.iter().zip(&charts) {
        assert_eq!(decoded.as_ref(), *original);
    }
    // The large row is most of its buffer, so it decodes in place, and each small row is a
    // tenth of its shared buffer, so it decodes from a copy and keeps the buffer free.
    assert!(in_buffer(decoded[10].as_ref().unwrap(), &array, 1));
    assert!(!in_buffer(decoded[3].as_ref().unwrap(), &array, 0));

    let in_place = charts_from_arrow(&array, Decode::InPlace).unwrap();
    assert!(in_buffer(in_place[3].as_ref().unwrap(), &array, 0));
    let copied = charts_from_arrow(&array, Decode::Copy).unwrap();
    assert!(!in_buffer(copied[10].as_ref().unwrap(), &array, 1));
    assert_eq!(copied[10].as_ref(), Some(&large));
}

#[test]
fn binary_and_large_binary_columns_decode() {
    let chart = scatter(8);
    let bytes = chart.to_bytes().unwrap();
    let binary = BinaryArray::from_opt_vec(vec![Some(bytes.as_ref()), None]);
    let large = LargeBinaryArray::from_opt_vec(vec![None, Some(bytes.as_ref())]);
    assert_eq!(
        charts_from_arrow(&binary, Decode::Auto).unwrap(),
        [Some(chart.clone()), None]
    );
    assert_eq!(
        charts_from_arrow(&large, Decode::InPlace).unwrap(),
        [None, Some(chart)]
    );
}

#[test]
fn columns_without_charts_fail() {
    assert!(matches!(
        charts_from_arrow(&Int64Array::from(vec![1]), Decode::Auto),
        Err(ChartError::NotAChart(_))
    ));
    let short = BinaryViewArray::from(vec![b"chart".as_slice()]);
    assert!(matches!(
        charts_from_arrow(&short, Decode::Auto),
        Err(ChartError::NotAChart(_))
    ));
}
