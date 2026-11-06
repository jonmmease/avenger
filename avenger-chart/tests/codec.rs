//! Decoding and encoding charts: round trips, alignment, the IPC requirements of the format, and
//! columns of charts.

mod support;

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, DictionaryArray, Float64Array, StringArray},
    datatypes::{DataType, Int32Type},
    ipc::{writer::IpcWriteOptions, CompressionType},
    record_batch::RecordBatch,
};
use avenger_chart::{
    charts_from_arrow, charts_to_arrow, list_value, proto, Chart, ChartError, Decode,
};
use support::*;

fn scatter() -> Chart {
    Chart::from_parts(chart(plot(vec![symbols(0)])), vec![points()], vec![]).unwrap()
}

#[test]
fn a_chart_round_trips_through_bytes() {
    let chart = scatter();
    let bytes = chart.to_bytes().unwrap();
    let decoded = Chart::from_bytes(bytes.clone()).unwrap();
    assert_eq!(decoded, chart);
    assert_eq!(decoded.to_bytes().unwrap(), bytes);
}

#[test]
fn the_fields_after_the_values_round_trip() {
    let mut message = chart(plot(vec![symbols(0)]));
    message.number_format_syntax = "d3".to_string();
    message.datetime_format_syntax = "chrono".to_string();
    message.width = Some(400.0);
    message.height = Some(300.0);
    message.padding = Some(proto::Padding {
        top: Some(4.0),
        left: Some(8.0),
        ..Default::default()
    });
    // An explicit domain gives the chart a values table, so the tail follows the values.
    let mut layer = symbols(0);
    if let Some(proto::cartesian_layer::Mark::Symbol(mark)) = &mut layer.mark {
        mark.x = Some(column(
            "a",
            linear(proto::domain::Domain::Explicit(proto::ValueRef {
                index: 0,
            })),
        ));
    }
    message.root = Some(plot(vec![layer]));
    let domain = list_value(Arc::new(Float64Array::from(vec![0.0, 10.0])) as ArrayRef);
    let chart = Chart::from_parts(message, vec![points()], vec![domain]).unwrap();
    let decoded = Chart::from_bytes(chart.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded, chart);
    assert_eq!(decoded.message().width, Some(400.0));
    assert_eq!(decoded.message().datetime_format_syntax, "chrono");
}

#[test]
fn bytes_that_start_off_alignment_decode_from_an_aligned_copy() {
    let bytes = scatter().to_bytes().unwrap();
    let mut shifted = vec![0u8];
    shifted.extend_from_slice(&bytes);
    let decoded = Chart::from_bytes(prost::bytes::Bytes::from(shifted).slice(1..)).unwrap();
    assert_eq!(decoded, scatter());
}

#[test]
fn a_table_without_rows_round_trips() {
    let empty = table(&[("a", vec![]), ("b", vec![])]);
    let chart = Chart::from_parts(chart(plot(vec![symbols(0)])), vec![empty], vec![]).unwrap();
    let decoded = Chart::from_bytes(chart.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded.tables()[0].num_rows(), 0);
}

/// A message whose table is the IPC stream of `batch` written with `options`.
fn message_with_table(batch: &RecordBatch, options: IpcWriteOptions) -> proto::Chart {
    let mut message = chart(plot(vec![symbols(0)]));
    message.tables = vec![proto::Table {
        padding: Default::default(),
        ipc: ipc(batch, options).into(),
    }];
    message
}

#[test]
fn a_dictionary_column_decodes_to_its_values() {
    let keys = DictionaryArray::<Int32Type>::new(
        vec![0, 1, 0].into(),
        Arc::new(StringArray::from(vec!["x", "y"])),
    );
    let batch = RecordBatch::try_from_iter([
        (
            "a",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
        ),
        (
            "b",
            Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0])) as ArrayRef,
        ),
        ("category", Arc::new(keys) as ArrayRef),
    ])
    .unwrap();
    let chart = Chart::from_message(message_with_table(&batch, Default::default())).unwrap();
    let category = chart.tables()[0].column_by_name("category").unwrap();
    assert_eq!(category.data_type(), &DataType::Utf8);
    assert_eq!(category.len(), 3);
}

#[test]
fn a_compressed_stream_fails() {
    let options = IpcWriteOptions::default()
        .try_with_compression(Some(CompressionType::LZ4_FRAME))
        .unwrap();
    let error = Chart::from_message(message_with_table(&points(), options)).unwrap_err();
    assert!(
        matches!(&error, ChartError::Invalid { path, reason }
            if path == "tables[0]" && reason.contains("does not compress")),
        "{error}"
    );
}

#[test]
fn versions_other_than_1_fail() {
    let mut message = chart(plot(vec![symbols(0)]));
    message.version = 0;
    let error = Chart::from_parts(message.clone(), vec![points()], vec![]).unwrap_err();
    assert!(matches!(error, ChartError::NotAChart(_)), "{error}");
    message.version = 2;
    let error = Chart::from_parts(message, vec![points()], vec![]).unwrap_err();
    assert!(
        matches!(error, ChartError::UnsupportedVersion(2)),
        "{error}"
    );
}

#[test]
fn bytes_that_are_not_a_chart_fail() {
    let error = Chart::from_bytes(vec![0xff, 0xff, 0xff]).unwrap_err();
    assert!(matches!(error, ChartError::NotAChart(_)), "{error}");
}

#[test]
fn a_reference_past_the_end_of_its_list_fails() {
    let error =
        Chart::from_parts(chart(plot(vec![symbols(1)])), vec![points()], vec![]).unwrap_err();
    assert!(
        matches!(&error, ChartError::Invalid { path, .. } if path == "tables"),
        "{error}"
    );
}

#[test]
fn a_column_of_charts_round_trips_with_nulls() {
    let chart = scatter();
    let column = charts_to_arrow([Some(&chart), None, Some(&chart)]).unwrap();
    for decode in [Decode::Auto, Decode::InPlace, Decode::Copy] {
        let charts = charts_from_arrow(&column, decode).unwrap();
        assert_eq!(charts, vec![Some(chart.clone()), None, Some(chart.clone())]);
    }
}
