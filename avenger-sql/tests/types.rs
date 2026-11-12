//! GoogleSQL's types as Arrow types.

use std::sync::Arc;

use avenger_sql::types::{arrow_type, googlesql_name, TypeOptions};
use avenger_sql_syntax::ast::{self, AstNode};
use datafusion::arrow::datatypes::{DataType, Field, Fields, IntervalUnit, TimeUnit};

/// The type of `DECLARE v <text>`.
fn ty(text: &str) -> ast::Type {
    let parse = avenger_sql_syntax::parse(&format!("DECLARE v {text}"));
    assert!(parse.ok(), "{text}: {:?}", parse.diagnostics());
    parse
        .syntax()
        .descendants()
        .find_map(ast::DeclareStmt::cast)
        .and_then(|declare| declare.ty())
        .expect("a type")
}

fn arrow(text: &str) -> DataType {
    arrow_type(&ty(text), TypeOptions::default())
        .unwrap_or_else(|error| panic!("{text}: {error:?}"))
}

#[test]
fn each_type_maps_to_its_arrow_type() {
    let utc = DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC")));
    let cases = [
        ("INT64", DataType::Int64),
        ("int64", DataType::Int64),
        ("FLOAT64", DataType::Float64),
        ("NUMERIC", DataType::Decimal128(38, 9)),
        ("DECIMAL", DataType::Decimal128(38, 9)),
        ("BIGNUMERIC", DataType::Decimal256(76, 38)),
        ("BIGDECIMAL", DataType::Decimal256(76, 38)),
        ("BOOL", DataType::Boolean),
        ("STRING", DataType::Utf8),
        ("BYTES", DataType::Binary),
        ("DATE", DataType::Date32),
        ("TIME", DataType::Time64(TimeUnit::Microsecond)),
        ("DATETIME", DataType::Timestamp(TimeUnit::Microsecond, None)),
        ("TIMESTAMP", utc.clone()),
        ("INTERVAL", DataType::Interval(IntervalUnit::MonthDayNano)),
        (
            "ARRAY<STRING>",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
        ),
        (
            "STRUCT<a INT64, TIMESTAMP>",
            DataType::Struct(Fields::from(vec![
                Field::new("a", DataType::Int64, true),
                Field::new("", utc, true),
            ])),
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(arrow(text), expected, "{text}");
    }
    let view = arrow_type(&ty("STRING"), TypeOptions { utf8_view: true }).unwrap();
    assert_eq!(view, DataType::Utf8View);
}

#[test]
fn types_outside_avenger_sql_are_errors_at_their_text() {
    let cases = [
        ("JSON", "JSON", "doesn't support the JSON type"),
        (
            "RANGE<DATE>",
            "RANGE<DATE>",
            "doesn't support the RANGE type",
        ),
        ("ARRAY<ARRAY<INT64>>", "ARRAY<INT64>", "can't hold arrays"),
        ("STRING(10)", "(10)", "take no parameters"),
        ("INT32", "INT32", "`INT32` is not a type of avenger-sql"),
        ("STRUCT<a JSON>", "JSON", "JSON"),
    ];
    for (text, at, message) in cases {
        let declare = format!("DECLARE v {text}");
        let error = arrow_type(&ty(text), TypeOptions::default()).expect_err(text);
        assert_eq!(&declare[error.range], at, "{text}: {error:?}");
        assert!(error.message.contains(message), "{text}: {error:?}");
    }
}

#[test]
fn arrow_types_have_googlesql_names() {
    for text in [
        "INT64",
        "FLOAT64",
        "NUMERIC",
        "BIGNUMERIC",
        "BOOL",
        "STRING",
        "BYTES",
        "DATE",
        "TIME",
        "DATETIME",
        "TIMESTAMP",
        "INTERVAL",
        "ARRAY<DATE>",
        "STRUCT<a INT64, b ARRAY<STRING>>",
    ] {
        assert_eq!(googlesql_name(&arrow(text)), text);
    }
    assert_eq!(googlesql_name(&DataType::Int32), "Int32");
}
