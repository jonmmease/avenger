//! Round trips of charts through bytes: equal bytes, one entry for each distinct table, the
//! alignment of each table and of the values, and tables and values that decode in place.

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
    Chart, ChartError, Contribution, Selection, TupleTest, FORMAT_VERSION,
};
use avenger_scales::{scalar::Scalar, scales::domain_summary::DomainSummary};
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
    Chart::new(message, tables, Vec::new()).expect("a valid chart")
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
    let error = Chart::new(message, vec![table(3)], Vec::new()).unwrap_err();
    assert!(
        matches!(&error, ChartError::Invalid { path, .. } if path == "root"),
        "{error}"
    );
}

#[test]
fn options_round_trip() {
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

/// A chart with a part of each kind: polar and map plots, facets with shown and hidden headers,
/// an empty cell, partitions, titles, attributions, interactions, selections with each kind of
/// tuple field, conditions, and each kind of domain and guide.
fn every_part() -> Chart {
    use avenger_chart::{Encode, Scale};
    use avenger_scales::scales::domain_summary::IntervalSummary;

    let numbers = |values: &[f64]| -> ArrayRef { Arc::new(Float64Array::from(values.to_vec())) };
    let selected = || proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: "brush".into(),
                empty_selects_none: true,
                exclude_own: true,
            },
        )),
    };
    let brush = Selection::new("brush", proto::SelectionResolution::Intersect).initial(
        Contribution::new("points")
            .partition("panel", "left")
            .tuple([
                TupleTest::Set {
                    column: "label".into(),
                    channel: proto::Channel::Unspecified,
                    values: Arc::new(StringArray::from(vec!["label 1"])),
                },
                TupleTest::Range {
                    column: "x".into(),
                    channel: proto::Channel::X,
                    ends: numbers(&[1.0, 4.0]),
                    exclude_end: true,
                },
                TupleTest::Polygon {
                    x_column: "x".into(),
                    y_column: "y".into(),
                    x: numbers(&[0.0, 4.0, 2.0]),
                    y: numbers(&[0.0, 0.0, 3.0]),
                },
                TupleTest::CyclicRange {
                    column: "x".into(),
                    channel: proto::Channel::Longitude,
                    start: 170.0,
                    end: -170.0,
                    period: 360.0,
                },
            ]),
    );
    let points = Chart::unit(table(12), proto::SymbolMark::default())
        .id("points")
        .title(proto::Title {
            text: "Points".into(),
            subtitle: "by count".into(),
            side: proto::TitleSide::Bottom as i32,
            ..Default::default()
        })
        .partition("panel", "left")
        .x(Encode::column("x").scale(
            Scale::new()
                .domain(numbers(&[0.0, 20.0]))
                .selection("brush", proto::Channel::X),
        ))
        .y(
            Encode::column("y").scale(Scale::new().summary(DomainSummary::Interval(
                IntervalSummary::new(&[0.0, 30.0], &[2.0, 2.0], &[2.0, 2.0]).unwrap(),
            ))),
        )
        .color(
            Encode::column("count")
                .scale(Scale::new().guide(proto::Colorbar {
                    max_length: Some(200.0),
                    ..Default::default()
                }))
                .when(
                    proto::Predicate {
                        predicate: Some(proto::predicate::Predicate::And(proto::PredicateList {
                            predicates: vec![
                                selected(),
                                proto::Predicate {
                                    predicate: Some(proto::predicate::Predicate::Not(Box::new(
                                        proto::Predicate {
                                            predicate: Some(proto::predicate::Predicate::Or(
                                                proto::PredicateList {
                                                    predicates: vec![selected()],
                                                },
                                            )),
                                        },
                                    ))),
                                },
                            ],
                        })),
                    },
                    avenger_chart::Value::Constant("gray".into()),
                ),
        )
        .size(Encode::column("x").scale(Scale::new().guide(proto::Legend {
            tick_count: Some(4.0),
            ..Default::default()
        })))
        .interaction(proto::Interaction {
            name: "acme.lens".into(),
            version: 3,
            payload: Bytes::from_static(&[1, 2, 3]),
        })
        .attribution(proto::Attribution {
            credits: vec!["Source: tests".into()],
            ..Default::default()
        })
        .selection(brush)
        .build()
        .unwrap();
    let other = Chart::unit(table(4), proto::SymbolMark::default())
        .partition("panel", "right")
        .x("x")
        .y("y")
        .build()
        .unwrap();
    let wrap = Chart::wrap([
        avenger_chart::Child::from(points),
        avenger_chart::Child::from(other),
        avenger_chart::Child::from(Chart::empty().partition("panel", "middle")),
    ])
    .facet("panel")
    .hide_facet_header()
    .independent(proto::Channel::X)
    .independent(proto::Channel::Y)
    .build()
    .unwrap();

    let polar = Chart::unit(
        table(6),
        proto::RectMark {
            inset: Some(1.0),
            route: proto::Route::Canvas as i32,
            ..Default::default()
        },
    )
    .coord(proto::Polar {
        start_angle: Some(-90.0),
        end_angle: Some(90.0),
        inner_radius: Some(0.5),
    })
    .theta(
        Encode::column("x").scale(
            Scale::new()
                .guide(proto::Axis {
                    route: proto::Route::Canvas as i32,
                    label_turn: Some(proto::axis::LabelTurn::LabelOrient(
                        proto::PolarTextOrient::Tangential as i32,
                    )),
                    ..Default::default()
                })
                .guide_values(numbers(&[0.0, 5.0])),
        ),
    )
    .theta2("y")
    .radius(
        Encode::column("count").scale(Scale::new().guide(proto::Axis {
            angle: Some(45.0),
            ..Default::default()
        })),
    )
    .radius2(Encode::column("count"))
    .fill("label")
    .build()
    .unwrap();

    let map = Chart::unit(table(3), proto::SymbolMark::default())
        .coord(proto::Geo {
            surface: Some(proto::geo::Surface::Spherical(proto::Spherical {
                projection: "conic_equal_area".into(),
                parameters: vec![proto::NamedOption {
                    name: "parallels".into(),
                    value: Some(
                        payload::encode_option(&Scalar::from_f64_list(&[29.5, 45.5])).unwrap(),
                    ),
                }],
                clip: Some(proto::SphericalClip {
                    clip: Some(proto::spherical_clip::Clip::CircleAngle(60.0)),
                }),
                ..Default::default()
            })),
            fit: Some(proto::geo::Fit::FitBox(proto::LonLatBox {
                west: -125.0,
                south: 24.0,
                east: -66.0,
                north: 50.0,
            })),
            ..Default::default()
        })
        .longitude("x")
        .latitude("y")
        .build()
        .unwrap();

    Chart::column([wrap, polar, map])
        .title("Every part")
        .layout(proto::ConcatLayout {
            gap: Some(12.0),
            ..Default::default()
        })
        .build()
        .unwrap()
}

#[test]
fn every_part_of_the_format_round_trips() {
    let chart = every_part();
    let bytes = chart.to_bytes().unwrap();
    let decoded = Chart::from_bytes(bytes.clone()).unwrap();
    assert_eq!(decoded, chart);
    assert_eq!(decoded.to_bytes().unwrap(), bytes);
}

/// Whether every column of every table and every value of `chart` lies within `range`.
fn all_within(chart: &Chart, range: &Range<usize>) -> bool {
    chart.tables().iter().all(|table| {
        table
            .columns()
            .iter()
            .all(|column| within(&column.to_data(), range))
    }) && chart
        .values()
        .iter()
        .all(|value| within(&value.to_data(), range))
}

#[test]
fn values_stand_in_one_aligned_table_and_decode_in_place() {
    let chart = every_part();
    let bytes = chart.to_bytes().unwrap();
    let message = proto::Chart::decode(bytes.clone()).unwrap();
    let values = message.values.expect("a table of values");
    let offset = values.ipc.as_ptr() as usize - bytes.as_ptr() as usize;
    assert_eq!(offset % 64, 0, "the values at {offset}");

    // One row, and a column for each type, whose values stand together in the chart.
    let batch = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(values.ipc), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(batch.num_rows(), 1);
    let types: Vec<&arrow::datatypes::DataType> = chart
        .values()
        .iter()
        .map(|value| value.data_type())
        .collect();
    let mut runs = types.clone();
    runs.dedup();
    assert_eq!(batch.num_columns(), runs.len());
    let mut distinct = runs.clone();
    distinct.sort_by_key(|data_type| data_type.to_string());
    distinct.dedup();
    assert_eq!(distinct.len(), runs.len(), "each type stands together");

    let range = bytes.as_ptr() as usize..bytes.as_ptr() as usize + bytes.len();
    let decoded = Chart::from_bytes(bytes).unwrap();
    assert_eq!(decoded.values().len(), chart.values().len());
    assert!(all_within(&decoded, &range));
}

#[test]
fn a_chart_without_values_writes_no_table_of_values() {
    let bytes = chart(scatter(0), vec![table(4)]).to_bytes().unwrap();
    assert!(proto::Chart::decode(bytes).unwrap().values.is_none());
}

#[test]
fn a_geoarrow_column_keeps_its_extension_type() {
    let point = |lon: f64, lat: f64| {
        let mut point = vec![1, 1, 0, 0, 0];
        point.extend(lon.to_le_bytes());
        point.extend(lat.to_le_bytes());
        point
    };
    let geometry: ArrayRef = Arc::new(arrow::array::BinaryArray::from_iter_values([
        point(-70.0, 40.0),
        point(10.0, 55.0),
    ]));
    let field = arrow::datatypes::Field::new("geometry", arrow::datatypes::DataType::Binary, false)
        .with_metadata(
            [(
                "ARROW:extension:name".to_string(),
                "geoarrow.wkb".to_string(),
            )]
            .into(),
        );
    let table = RecordBatch::try_new(
        Arc::new(arrow::datatypes::Schema::new(vec![field])),
        vec![geometry],
    )
    .unwrap();
    let chart = Chart::unit(table, proto::GeoshapeMark::default())
        .geometry("geometry")
        .coord(proto::Geo {
            surface: Some(proto::geo::Surface::Spherical(proto::Spherical {
                projection: "equal_earth".into(),
                ..Default::default()
            })),
            fit: Some(proto::geo::Fit::FitSphere(proto::WholeSphere {})),
            ..Default::default()
        })
        .build()
        .unwrap();
    let bytes = chart.to_bytes().unwrap();
    let decoded = Chart::from_bytes(bytes.clone()).unwrap();
    assert_eq!(decoded, chart);
    assert_eq!(decoded.to_bytes().unwrap(), bytes);
    assert_eq!(
        decoded.tables()[0].schema().field(0).extension_type_name(),
        Some("geoarrow.wkb")
    );
}
