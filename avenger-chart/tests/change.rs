//! Changes of a chart: a `ChartChange` in a fragment with its values, checked as it decodes.

use std::sync::Arc;

use arrow::{
    array::{
        Array, ArrayRef, Decimal128Array, Float64Array, Int64Array, StringArray, StructArray,
        TimestampMillisecondArray,
    },
    datatypes::{DataType, Field, Fields, TimeUnit},
};
use avenger_chart::{prelude::*, proto::Fragment, ChangeMessage, FORMAT_VERSION};
use prost::Message;

/// Fails unless `result` is invalid at `path`, for a reason that contains `reason`.
#[track_caller]
fn invalid<T: std::fmt::Debug>(result: Result<T, ChartError>, path: &str, reason: &str) {
    match result {
        Err(ChartError::Invalid {
            path: at,
            reason: why,
        }) => {
            assert_eq!(at, path, "the error names the path ({why})");
            assert!(why.contains(reason), "`{why}` says `{reason}`");
        }
        other => panic!("expected an error at {path}, got {other:?}"),
    }
}

fn timestamps() -> ArrayRef {
    Arc::new(TimestampMillisecondArray::from(vec![
        1_700_000_000_000,
        1_800_000_000_000,
    ]))
}

fn decimals() -> ArrayRef {
    Arc::new(
        Decimal128Array::from(vec![1250, 9875])
            .with_precision_and_scale(10, 2)
            .unwrap(),
    )
}

fn square() -> ArrayRef {
    let fields = Fields::from(vec![
        Field::new("x", DataType::Float64, true),
        Field::new("y", DataType::Float64, true),
    ]);
    let x: ArrayRef = Arc::new(Float64Array::from(vec![0.0, 1.0, 1.0, 0.0]));
    let y: ArrayRef = Arc::new(Float64Array::from(vec![0.0, 0.0, 1.0, 1.0]));
    Arc::new(StructArray::try_new(fields, vec![x, y], None).unwrap())
}

/// A change with a domain of each kind of identity, one clause of each kind of condition, and
/// the modes.
fn change() -> ChartChangeBuilder {
    let set = ValueSet::new([
        SetColumn::new(
            "cylinders",
            Value::list(Arc::new(Int64Array::from(vec![4, 6]))),
        ),
        SetColumn::new(
            "origin",
            Value::list(Arc::new(StringArray::from(vec!["USA", "Japan"]))),
        ),
    ]);
    let open = ValueRange::new("weight")
        .channel(Channel::X)
        .low(Value::scalar(Arc::new(Float64Array::from(vec![2000.0]))));
    let lasso = Polygon::new("weight", "mpg", [Value::list(square())]);
    let wrap = CyclicRange::new("longitude")
        .start(170.0)
        .end(-170.0)
        .period(360.0);
    let clause = Clause::new([
        ConditionBuilder::from(set),
        open.into(),
        lasso.into(),
        wrap.into(),
    ])
    .owner("plot")
    .partition([PartitionValue::new("origin", "USA")]);
    ChartChange::builder()
        .domains([
            ScaleDomain::new(
                ScaleKey::family(Channel::X, "time"),
                Value::list(timestamps()),
            ),
            ScaleDomain::new(
                ScaleKey::plot(
                    Channel::Y,
                    PlotKey::given("plot").partition([PartitionValue::new("origin", "USA")]),
                )
                .position(1),
                Value::list(decimals()),
            ),
        ])
        .selections([SelectionClauses::new("brush").clauses([clause])])
        .modes(ModesOn::builder().names(["pan"]))
        .settled(true)
}

#[test]
fn a_change_round_trips_through_its_bytes_with_the_types_of_its_values() {
    let change = ChangeMessage::new(change()).unwrap();
    let read = ChangeMessage::from_bytes(change.to_bytes().unwrap()).unwrap();
    assert_eq!(read.message(), change.message());
    assert_eq!(read.values().len(), change.values().len());
    for (a, b) in read.values().iter().zip(change.values()) {
        assert_eq!(a.to_data(), b.to_data());
    }
    let domains = &read.message().domains;
    let first = read.list_elements(domains[0].domain.as_ref().unwrap());
    let second = read.list_elements(domains[1].domain.as_ref().unwrap());
    assert_eq!(
        first.data_type(),
        &DataType::Timestamp(TimeUnit::Millisecond, None)
    );
    assert_eq!(second.data_type(), &DataType::Decimal128(10, 2));
    assert_eq!(read.message().modes.as_ref().unwrap().names, ["pan"]);
}

#[test]
fn bytes_that_are_not_a_fragment_of_a_change_of_this_version_fail() {
    invalid(
        ChangeMessage::from_bytes(vec![0xff, 0xff, 0xff]),
        "",
        "not a fragment",
    );
    let modes = ModesOn::builder().names(["pan"]).to_fragment().unwrap();
    invalid(
        ChangeMessage::from_bytes(modes.encode_to_vec()),
        "type",
        "`avenger.chart.v1.ModesOn`",
    );
    let mut fragment = change().to_fragment().unwrap();
    fragment.version = FORMAT_VERSION + 1;
    invalid(
        ChangeMessage::from_bytes(Fragment::encode_to_vec(&fragment)),
        "version",
        "this reader reads",
    );
}

#[test]
fn a_change_follows_the_rules_of_the_clauses_and_the_partitions_of_a_chart() {
    let endless = ValueRange::new("weight");
    let change = ChartChange::builder().selections([
        SelectionClauses::new("brush").clauses([Clause::new([endless]).owner("plot")])
    ]);
    invalid(
        ChangeMessage::new(change),
        "selections[0].clauses[0].conditions[0].range",
        "a low end, a high end, or both",
    );
    let unnamed = ChartChange::builder().selections([SelectionClauses::new("")]);
    invalid(ChangeMessage::new(unnamed), "selections[0].name", "");
    let twice = ChartChange::builder().domains([ScaleDomain::new(
        ScaleKey::plot(
            Channel::X,
            PlotKey::given("plot").partition([
                PartitionValue::new("origin", "USA"),
                PartitionValue::new("origin", "Japan"),
            ]),
        ),
        Value::list(timestamps()),
    )]);
    invalid(
        ChangeMessage::new(twice),
        "domains[0].scale.plot.partition",
        "partition values",
    );
}
