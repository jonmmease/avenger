//! GoogleSQL's types as Arrow types.
//!
//! A type that a query writes, as in `CAST(x AS NUMERIC)` or `DECLARE n INT64`, maps to an Arrow
//! type with GoogleSQL's microsecond precision for time:
//!
//! | GoogleSQL | Arrow |
//! | --- | --- |
//! | `INT64` | `Int64` |
//! | `FLOAT64` | `Float64` |
//! | `NUMERIC`, `DECIMAL` | `Decimal128(38, 9)` |
//! | `BIGNUMERIC`, `BIGDECIMAL` | `Decimal256(76, 38)` |
//! | `BOOL` | `Boolean` |
//! | `STRING` | `Utf8`, or `Utf8View` when [`TypeOptions::utf8_view`] |
//! | `BYTES` | `Binary` |
//! | `DATE` | `Date32` |
//! | `TIME` | `Time64(Microsecond)` |
//! | `DATETIME` | `Timestamp(Microsecond, None)` |
//! | `TIMESTAMP` | `Timestamp(Microsecond, "UTC")` |
//! | `INTERVAL` | `Interval(MonthDayNano)` |
//! | `ARRAY<T>` | `List(T)` |
//! | `STRUCT<...>` | `Struct(...)` |
//!
//! Type names match without regard to case. `JSON`, `RANGE<T>`, an array of arrays, and type
//! parameters, as in `STRING(10)`, are errors at their text. The columns of the input tables keep
//! their own Arrow types, and [`googlesql_name`] names an Arrow type in GoogleSQL's terms.

use std::sync::Arc;

use avenger_sql_syntax::{ast, ast::AstNode, TextRange};
use datafusion::arrow::datatypes::{DataType, Field, Fields, IntervalUnit, TimeUnit};

/// How GoogleSQL's types map to Arrow types where a session can choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypeOptions {
    /// Whether `STRING` maps to `Utf8View` instead of `Utf8`.
    pub utf8_view: bool,
}

/// An error in a type that a query writes: the range of its text and its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError {
    pub range: TextRange,
    pub message: String,
}

impl TypeError {
    fn new(range: TextRange, message: impl Into<String>) -> Self {
        Self {
            range,
            message: message.into(),
        }
    }
}

/// The time zone of GoogleSQL's `TIMESTAMP`, which is an instant.
pub const UTC: &str = "UTC";

/// The Arrow type of `TIMESTAMP`.
pub fn timestamp() -> DataType {
    DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from(UTC)))
}

/// The Arrow type of `DATETIME`, a civil date and time.
pub fn datetime() -> DataType {
    DataType::Timestamp(TimeUnit::Microsecond, None)
}

/// The Arrow type of the type `ty`, or the error at its text.
pub fn arrow_type(ty: &ast::Type, options: TypeOptions) -> Result<DataType, TypeError> {
    match ty {
        ast::Type::NamedType(named) => named_type(named, options),
        ast::Type::ArrayType(array) => {
            let Some(element) = array.ty() else {
                return Err(TypeError::new(
                    array.range(),
                    "an ARRAY needs its element type",
                ));
            };
            if matches!(element, ast::Type::ArrayType(_)) {
                return Err(TypeError::new(
                    element.range(),
                    "an array can't hold arrays, as in GoogleSQL",
                ));
            }
            let element = arrow_type(&element, options)?;
            Ok(DataType::List(Arc::new(Field::new("item", element, true))))
        }
        ast::Type::StructType(structure) => {
            let fields = structure
                .struct_fields()
                .map(|field| {
                    // A field without a name is anonymous, as in GoogleSQL.
                    let name = field
                        .name()
                        .and_then(|name| name.text())
                        .unwrap_or_default();
                    let Some(ty) = field.ty() else {
                        return Err(TypeError::new(field.range(), "a field needs a type"));
                    };
                    Ok(Field::new(name, arrow_type(&ty, options)?, true))
                })
                .collect::<Result<Fields, _>>()?;
            Ok(DataType::Struct(fields))
        }
        ast::Type::RangeType(range) => Err(TypeError::new(
            range.range(),
            "avenger-sql doesn't support the RANGE type",
        )),
    }
}

fn named_type(named: &ast::NamedType, options: TypeOptions) -> Result<DataType, TypeError> {
    if let Some(params) = named.type_params() {
        return Err(TypeError::new(
            params.range(),
            "avenger-sql's types take no parameters",
        ));
    }
    if named.interval_kw().is_some() {
        return Ok(DataType::Interval(IntervalUnit::MonthDayNano));
    }
    let Some(name) = named.name_ref() else {
        return Err(TypeError::new(named.range(), "expected a type"));
    };
    let text = name.text().unwrap_or_default();
    let ty = match text.to_ascii_uppercase().as_str() {
        "INT64" => DataType::Int64,
        "FLOAT64" => DataType::Float64,
        "NUMERIC" | "DECIMAL" => DataType::Decimal128(38, 9),
        "BIGNUMERIC" | "BIGDECIMAL" => DataType::Decimal256(76, 38),
        "BOOL" => DataType::Boolean,
        "STRING" if options.utf8_view => DataType::Utf8View,
        "STRING" => DataType::Utf8,
        "BYTES" => DataType::Binary,
        "DATE" => DataType::Date32,
        "TIME" => DataType::Time64(TimeUnit::Microsecond),
        "DATETIME" => datetime(),
        "TIMESTAMP" => timestamp(),
        "JSON" => {
            return Err(TypeError::new(
                name.range(),
                "avenger-sql doesn't support the JSON type",
            ))
        }
        _ => {
            return Err(TypeError::new(
                name.range(),
                format!("`{text}` is not a type of avenger-sql"),
            ))
        }
    };
    Ok(ty)
}

/// The name of the Arrow type `ty` in GoogleSQL's terms, such as `INT64` for `Int64`, or its
/// Arrow name for a type that GoogleSQL doesn't have, such as `Int32`.
pub fn googlesql_name(ty: &DataType) -> String {
    match ty {
        DataType::Int64 => "INT64".to_string(),
        DataType::Float64 => "FLOAT64".to_string(),
        DataType::Decimal128(..) => "NUMERIC".to_string(),
        DataType::Decimal256(..) => "BIGNUMERIC".to_string(),
        DataType::Boolean => "BOOL".to_string(),
        DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8 => "STRING".to_string(),
        DataType::Binary | DataType::BinaryView | DataType::LargeBinary => "BYTES".to_string(),
        DataType::Date32 => "DATE".to_string(),
        DataType::Time64(TimeUnit::Microsecond) => "TIME".to_string(),
        DataType::Timestamp(_, None) => "DATETIME".to_string(),
        DataType::Timestamp(_, Some(_)) => "TIMESTAMP".to_string(),
        DataType::Interval(IntervalUnit::MonthDayNano) => "INTERVAL".to_string(),
        DataType::List(field) | DataType::LargeList(field) | DataType::ListView(field) => {
            format!("ARRAY<{}>", googlesql_name(field.data_type()))
        }
        DataType::Struct(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|field| match field.name().is_empty() {
                    true => googlesql_name(field.data_type()),
                    false => format!("{} {}", field.name(), googlesql_name(field.data_type())),
                })
                .collect();
            format!("STRUCT<{}>", fields.join(", "))
        }
        other => other.to_string(),
    }
}
