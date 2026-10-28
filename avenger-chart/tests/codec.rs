//! Round trips of charts through bytes: equal bytes, one entry for each distinct table, the
//! alignment of each table, and tables that decode in place.

use std::{ops::Range, sync::Arc};

use arrow::{
    array::{
        Array, ArrayData, ArrayRef, BooleanArray, Date32Array, DictionaryArray, Float64Array,
        Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
    },
    datatypes::Int32Type,
};
use avenger_chart::{
    payload,
    proto::{self, domain, domain_fit, encoding, mark, node},
    Chart, ChartError, FORMAT_VERSION,
};
use avenger_scales::scalar::Scalar;
use bytes::Bytes;
use prost::Message;

fn linear() -> proto::ScaleSpec {
    proto::ScaleSpec {
        r#type: "linear".into(),
        domain: Some(proto::Domain {
            domain: Some(domain::Domain::Data(proto::DataDomain {})),
        }),
        fit: Some(proto::DomainFit {
            fit: Some(domain_fit::Fit::Interval(proto::IntervalFit::default())),
        }),
        ..Default::default()
    }
}

fn encode(channel: proto::Channel, column: &str) -> proto::Encoding {
    proto::Encoding {
        channel: channel as i32,
        value: Some(encoding::Value::Column(column.into())),
        scale: Some(linear()),
        ..Default::default()
    }
}

/// A scatter plot of the `x` and `y` columns of table `table`.
fn scatter(table: u32) -> proto::Node {
    proto::Node {
        node: Some(node::Node::Unit(proto::Unit {
            table,
            mark: Some(proto::Mark {
                mark: Some(mark::Mark::Symbol(proto::SymbolMark::default())),
            }),
            encodings: vec![
                encode(proto::Channel::X, "x"),
                encode(proto::Channel::Y, "y"),
            ],
            ..Default::default()
        })),
    }
}

fn overlay(children: Vec<proto::Node>) -> proto::Node {
    proto::Node {
        node: Some(node::Node::Overlay(proto::Overlay {
            children,
            ..Default::default()
        })),
    }
}

fn chart(root: proto::Node, tables: Vec<RecordBatch>) -> Chart {
    let message = proto::Chart {
        version: FORMAT_VERSION,
        root: Some(root),
        ..Default::default()
    };
    Chart::new(message, tables).expect("a valid chart")
}

/// A table of `rows` rows with an `x` and a `y` column and a column of each other Arrow type that
/// a unit accepts.
fn table(rows: usize) -> RecordBatch {
    let x: Vec<f64> = (0..rows).map(|row| row as f64).collect();
    let y: Vec<f64> = x.iter().map(|x| x * 2.0).collect();
    let labels: Vec<String> = (0..rows).map(|row| format!("label {row}")).collect();
    let keys: Vec<&str> = (0..rows).map(|row| ["a", "b", "c"][row % 3]).collect();
    let columns: Vec<(&str, ArrayRef)> = vec![
        ("x", Arc::new(Float64Array::from(x))),
        ("y", Arc::new(Float64Array::from(y))),
        (
            "count",
            Arc::new(Int64Array::from_iter_values(0..rows as i64)),
        ),
        ("label", Arc::new(StringArray::from(labels))),
        (
            "key",
            Arc::new(keys.into_iter().collect::<DictionaryArray<Int32Type>>()),
        ),
        (
            "time",
            Arc::new(
                TimestampMillisecondArray::from_iter_values(
                    (0..rows as i64).map(|row| 1_700_000_000_000 + row * 60_000),
                )
                .with_timezone("America/New_York"),
            ),
        ),
        (
            "day",
            Arc::new(Date32Array::from_iter_values(
                (0..rows as i32).map(|row| 19_700 + row),
            )),
        ),
        (
            "flag",
            Arc::new(BooleanArray::from_iter(
                (0..rows).map(|row| Some(row % 2 == 0)),
            )),
        ),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
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

#[test]
fn tables_of_each_type_round_trip() {
    let original = chart(scatter(0), vec![table(25)]);
    let decoded = Chart::from_bytes(original.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn equal_charts_encode_to_equal_bytes() {
    let original = chart(scatter(0), vec![table(40)]);
    let bytes = original.to_bytes().unwrap();
    assert_eq!(original.to_bytes().unwrap(), bytes);
    let decoded = Chart::from_bytes(bytes.clone()).unwrap();
    assert_eq!(decoded.to_bytes().unwrap(), bytes);
    // prost writes the same bytes from the decoded message, padding and all.
    assert_eq!(
        proto::Chart::decode(bytes.clone()).unwrap().encode_to_vec(),
        bytes
    );
}

#[test]
fn units_over_equal_tables_share_one_entry() {
    let original = chart(
        overlay(vec![scatter(0), scatter(1)]),
        vec![table(10), table(10)],
    );
    let bytes = original.to_bytes().unwrap();
    assert_eq!(proto::Chart::decode(bytes.clone()).unwrap().tables.len(), 1);
    let decoded = Chart::from_bytes(bytes).unwrap();
    assert_eq!(decoded.tables().len(), 1);
    assert_eq!(decoded.tables()[0], original.tables()[0]);
}

#[test]
fn every_table_starts_at_a_multiple_of_64_bytes() {
    // Tables of many sizes move the later tables through every remainder.
    let tables: Vec<RecordBatch> = (0..40).map(|rows| table(rows * 3 + 1)).collect();
    let root = overlay((0..tables.len() as u32).map(scatter).collect());
    let bytes = chart(root, tables).to_bytes().unwrap();
    assert_eq!(bytes.as_ptr().align_offset(64), 0);
    let message = proto::Chart::decode(bytes.clone()).unwrap();
    assert_eq!(message.tables.len(), 40);
    for (index, table) in message.tables.iter().enumerate() {
        let offset = table.ipc.as_ptr() as usize - bytes.as_ptr() as usize;
        assert_eq!(offset % 64, 0, "table {index} at {offset}");
    }
}

#[test]
fn tables_decode_in_place() {
    let bytes = chart(
        overlay(vec![scatter(0), scatter(1)]),
        vec![table(30), table(31)],
    )
    .to_bytes()
    .unwrap();
    let range = bytes.as_ptr() as usize..bytes.as_ptr() as usize + bytes.len();
    let decoded = Chart::from_bytes(bytes.clone()).unwrap();
    for table in decoded.tables() {
        for column in table.columns() {
            assert!(within(&column.to_data(), &range), "{}", column.data_type());
        }
    }
}

#[test]
fn misaligned_bytes_decode_from_one_aligned_copy() {
    let original = chart(scatter(0), vec![table(20)]);
    let bytes = original.to_bytes().unwrap();
    let mut shifted = vec![0u8];
    shifted.extend_from_slice(&bytes);
    let shifted = Bytes::from(shifted).slice(1..);
    assert_ne!(shifted.as_ptr().align_offset(64), 0);
    let decoded = Chart::from_bytes(shifted).unwrap();
    assert_eq!(decoded, original);
    for column in decoded.tables()[0].columns() {
        let data = column.to_data();
        for buffer in data.buffers() {
            assert_eq!(buffer.as_ptr().align_offset(8), 0);
        }
    }
}

#[test]
fn bytes_that_are_not_a_chart_fail() {
    assert!(matches!(
        Chart::from_bytes(Bytes::new()),
        Err(ChartError::NotAChart(_))
    ));
    assert!(matches!(
        Chart::from_bytes(vec![0xff; 12]),
        Err(ChartError::NotAChart(_))
    ));
    let later = proto::Chart {
        version: 2,
        ..Default::default()
    };
    assert!(matches!(
        Chart::from_bytes(later.encode_to_vec()),
        Err(ChartError::UnsupportedVersion(2))
    ));
}

#[test]
fn a_unit_refers_to_a_table_of_the_chart() {
    let message = proto::Chart {
        version: FORMAT_VERSION,
        root: Some(scatter(1)),
        ..Default::default()
    };
    let error = Chart::new(message, vec![table(3)]).unwrap_err();
    assert!(
        matches!(&error, ChartError::Invalid { path, .. } if path == "root"),
        "{error}"
    );
}

#[test]
fn payloads_round_trip() {
    let values: ArrayRef = Arc::new(StringArray::from(vec!["a", "b"]));
    assert_eq!(
        payload::array(&payload::encode_array(&values).unwrap())
            .unwrap()
            .as_ref(),
        values.as_ref()
    );
    let value: ArrayRef = Arc::new(Int64Array::from(vec![7]));
    assert_eq!(
        payload::value(&payload::encode_array(&value).unwrap())
            .unwrap()
            .as_ref(),
        value.as_ref()
    );
    assert!(payload::value(&payload::encode_array(&values).unwrap()).is_err());

    for scalar in [
        Scalar::from_bool(true),
        Scalar::from_f64(0.25),
        Scalar(Arc::new(Int64Array::from(vec![3]))),
        Scalar::from_string("nice"),
        Scalar::from_f64_list(&[1.0, 2.5]),
        Scalar::from_string_list(&["a", "b"]),
    ] {
        let option = proto::NamedOption {
            name: "option".into(),
            value: Some(payload::encode_option(&scalar).unwrap()),
        };
        let decoded = payload::options(&[option]).unwrap();
        assert_eq!(decoded[0].1 .0.as_ref(), scalar.0.as_ref());
    }
    assert!(payload::encode_option(&Scalar(Arc::new(Date32Array::from(vec![1])))).is_err());
}
