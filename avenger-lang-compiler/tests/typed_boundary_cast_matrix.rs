use arrow::datatypes::DataType;
use avenger_lang_compiler::{normalize_sql_query, physical_type_to_arrow};
use avenger_lang_core::{IntervalUnit, PhysicalField, PhysicalType, TimeUnit};
use datafusion::{common::ScalarValue, prelude::SessionContext};

async fn evaluate(source: &str, target: &DataType) -> datafusion::common::Result<ScalarValue> {
    let sql = format!(
        "SELECT arrow_cast(({source}), '{}') AS value",
        target.to_string().replace('\'', "''")
    );
    let sql = normalize_sql_query(&sql).expect("normalize cast probe");
    let batches = SessionContext::new().sql(&sql).await?.collect().await?;
    ScalarValue::try_from_array(batches[0].column(0), 0)
}

fn all_v1_physical_types() -> Vec<PhysicalType> {
    vec![
        PhysicalType::Boolean,
        PhysicalType::Int8,
        PhysicalType::Int16,
        PhysicalType::Int32,
        PhysicalType::Int64,
        PhysicalType::UInt8,
        PhysicalType::UInt16,
        PhysicalType::UInt32,
        PhysicalType::UInt64,
        PhysicalType::Float16,
        PhysicalType::Float32,
        PhysicalType::Float64,
        PhysicalType::Utf8,
        PhysicalType::LargeUtf8,
        PhysicalType::Binary,
        PhysicalType::LargeBinary,
        PhysicalType::Date32,
        PhysicalType::Date64,
        PhysicalType::Time32(TimeUnit::Second),
        PhysicalType::Time32(TimeUnit::Millisecond),
        PhysicalType::Time64(TimeUnit::Microsecond),
        PhysicalType::Time64(TimeUnit::Nanosecond),
        PhysicalType::Timestamp {
            unit: TimeUnit::Microsecond,
            timezone: None,
        },
        PhysicalType::Timestamp {
            unit: TimeUnit::Nanosecond,
            timezone: Some("UTC".to_owned()),
        },
        PhysicalType::Duration(TimeUnit::Nanosecond),
        PhysicalType::Interval(IntervalUnit::YearMonth),
        PhysicalType::Interval(IntervalUnit::DayTime),
        PhysicalType::Interval(IntervalUnit::MonthDayNano),
        PhysicalType::FixedSizeBinary(4),
        PhysicalType::Decimal128 {
            precision: 38,
            scale: 10,
        },
        PhysicalType::Decimal256 {
            precision: 76,
            scale: 20,
        },
        PhysicalType::List(Box::new(PhysicalType::Int32)),
        PhysicalType::LargeList(Box::new(PhysicalType::Utf8)),
        PhysicalType::FixedSizeList {
            element: Box::new(PhysicalType::Float32),
            length: 2,
        },
        PhysicalType::Struct(vec![
            PhysicalField {
                name: "id".to_owned(),
                data_type: PhysicalType::UInt64,
                nullable: false,
            },
            PhysicalField {
                name: "label".to_owned(),
                data_type: PhysicalType::Utf8,
                nullable: true,
            },
        ]),
        PhysicalType::Map {
            key: Box::new(PhysicalType::Utf8),
            value: Box::new(PhysicalType::List(Box::new(PhysicalType::Int64))),
        },
    ]
}

#[tokio::test]
async fn null_strict_cast_reaches_every_v1_physical_destination() {
    for physical in all_v1_physical_types() {
        let target = physical_type_to_arrow(&physical);
        let value = evaluate("NULL", &target)
            .await
            .unwrap_or_else(|error| panic!("NULL -> {physical} ({target:?}) failed: {error}"));
        assert_eq!(value.data_type(), target, "destination {physical}");
        assert!(value.is_null(), "destination {physical}");
    }
}

#[tokio::test]
async fn pinned_strict_cast_characterization_covers_loss_failure_and_null() {
    let cases = [
        ("'42'", DataType::Int32, ScalarValue::Int32(Some(42))),
        ("3.9", DataType::Int32, ScalarValue::Int32(Some(3))),
        (
            "'true'",
            DataType::Boolean,
            ScalarValue::Boolean(Some(true)),
        ),
        ("'yes'", DataType::Boolean, ScalarValue::Boolean(Some(true))),
        ("'no'", DataType::Boolean, ScalarValue::Boolean(Some(false))),
        (
            "'1.235'",
            DataType::Decimal128(5, 2),
            ScalarValue::Decimal128(Some(124), 5, 2),
        ),
        (
            "'1970-01-01T00:00:01.123456789'",
            DataType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
            ScalarValue::TimestampMicrosecond(Some(1_123_456), None),
        ),
    ];
    for (source, target, expected) in cases {
        let actual = evaluate(source, &target)
            .await
            .unwrap_or_else(|error| panic!("{source} -> {target:?} failed: {error}"));
        assert_eq!(actual, expected, "{source} -> {target:?}");
    }

    assert!(evaluate("'maybe'", &DataType::Boolean).await.is_err());
    assert!(evaluate("'256'", &DataType::UInt8).await.is_err());
    assert!(
        evaluate("'not-an-integer'", &DataType::Int32)
            .await
            .is_err()
    );

    let batches = SessionContext::new()
        .sql("SELECT TRY_CAST('not-an-integer' AS INT) AS value")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        ScalarValue::try_from_array(batches[0].column(0), 0).unwrap(),
        ScalarValue::Int32(None)
    );
}

#[test]
fn exact_numeric_profile_covers_limits_scales_and_all_sql_islands() {
    let probes = [
        "9223372036854775807",
        "-9223372036854775808",
        "18446744073709551615",
        "99999999999999999999999999999999999999",
        "9999999999999999999999999999999999999999999999999999999999999999999999999999",
        "1.2300",
        "1e3",
        "1e-3",
        ".5",
        "1.",
        "0.0",
        "-0.0",
        "(-0.0)",
        "+2.50L",
    ];
    for probe in probes {
        let expression = avenger_lang_compiler::normalize_sql_expression(probe)
            .unwrap_or_else(|error| panic!("expression {probe}: {error}"));
        assert!(expression.contains("arrow_cast"), "{probe}: {expression}");

        let query = normalize_sql_query(&format!(
            "FROM (SELECT 1 AS id) AS input SELECT {probe} AS value, '{probe}' AS label"
        ))
        .unwrap_or_else(|error| panic!("query {probe}: {error}"));
        assert!(query.contains("arrow_cast"), "{probe}: {query}");
        assert!(query.contains(&format!("'{probe}' AS label")));
    }
}
