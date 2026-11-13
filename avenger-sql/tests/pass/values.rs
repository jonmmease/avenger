//! The values of the tests' rows as GoogleSQL writes them: `json` in the JSON form of
//! `execute_query --output_mode=json`, and `literal` as a literal of the docs' examples.

use std::sync::Arc;

use avenger_sql::types;
use datafusion::arrow::{
    array::{Array, ArrayRef, AsArray},
    datatypes::{
        DataType, Date32Type, Float64Type, Int32Type, Int64Type, IntervalMonthDayNanoType,
        TimestampMicrosecondType, UInt64Type,
    },
};

/// The value at `index` of `array`, in GoogleSQL's JSON form.
pub fn json(array: &ArrayRef, index: usize) -> String {
    if array.is_null(index) {
        return "null".to_string();
    }
    match array.data_type() {
        DataType::Int64 => format!("\"{}\"", array.as_primitive::<Int64Type>().value(index)),
        DataType::Int32 => format!("\"{}\"", array.as_primitive::<Int32Type>().value(index)),
        DataType::UInt64 => format!("\"{}\"", array.as_primitive::<UInt64Type>().value(index)),
        DataType::Float64 => {
            let value = array.as_primitive::<Float64Type>().value(index);
            match value.fract() == 0.0 && value.abs() < 1e15 {
                true => format!("{}", value as i64),
                false => format!("{value}"),
            }
        }
        DataType::Boolean => array.as_boolean().value(index).to_string(),
        DataType::Utf8 => string(array.as_string::<i32>().value(index)),
        DataType::LargeUtf8 => string(array.as_string::<i64>().value(index)),
        DataType::Utf8View => string(array.as_string_view().value(index)),
        DataType::Date32 => array.as_primitive::<Date32Type>().value(index).to_string(),
        DataType::Timestamp(_, Some(_)) => {
            let micros =
                datafusion::arrow::compute::cast(&array.slice(index, 1), &types::timestamp())
                    .expect("microseconds");
            format!(
                "\"{}\"",
                micros.as_primitive::<TimestampMicrosecondType>().value(0)
            )
        }
        DataType::Timestamp(_, None) => {
            let micros =
                datafusion::arrow::compute::cast(&array.slice(index, 1), &types::datetime())
                    .expect("microseconds");
            let micros = micros.as_primitive::<TimestampMicrosecondType>().value(0);
            format!("\"{}\"", packed_datetime(micros))
        }
        DataType::List(_) | DataType::LargeList(_) => {
            let list = datafusion::arrow::compute::cast(
                &array.slice(index, 1),
                &DataType::List(Arc::new(datafusion::arrow::datatypes::Field::new(
                    "item",
                    match array.data_type() {
                        DataType::List(field) | DataType::LargeList(field) => {
                            field.data_type().clone()
                        }
                        _ => unreachable!(),
                    },
                    true,
                ))),
            )
            .expect("a list");
            let values = list.as_list::<i32>().value(0);
            let items: Vec<String> = (0..values.len()).map(|i| json(&values, i)).collect();
            format!("[{}]", items.join(", "))
        }
        DataType::Interval(_) => {
            let value = array
                .as_primitive::<IntervalMonthDayNanoType>()
                .value(index);
            let (years, months) = (value.months / 12, value.months % 12);
            let micros = value.nanoseconds / 1_000;
            let sign = if micros < 0 { "-" } else { "" };
            let micros = micros.abs();
            let (hours, minutes, seconds, fraction) = (
                micros / 3_600_000_000,
                micros / 60_000_000 % 60,
                micros / 1_000_000 % 60,
                micros % 1_000_000,
            );
            let seconds = match fraction {
                0 => seconds.to_string(),
                f => format!("{seconds}.{}", format!("{f:06}").trim_end_matches('0')),
            };
            format!(
                "\"{years}-{months} {} {sign}{hours}:{minutes}:{seconds}\"",
                value.days
            )
        }
        DataType::Decimal128(..) | DataType::Decimal256(..) => {
            let text = datafusion::arrow::util::display::array_value_to_string(array, index)
                .expect("a value");
            let text = match text.contains('.') {
                true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
                false => text,
            };
            format!("\"{text}\"")
        }
        DataType::Binary => format!("\"{}\"", base64(array.as_binary::<i32>().value(index))),
        other => panic!("the tests show no {other}"),
    }
}

pub fn string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The microseconds since 1970-01-01T00:00:00 of a civil time in GoogleSQL's packed form of a
/// DATETIME, whose bits hold its year, month, day, hour, minute, second and microsecond.
pub fn packed_datetime(micros: i64) -> i64 {
    let days = micros.div_euclid(86_400_000_000);
    let rest = micros.rem_euclid(86_400_000_000);
    // The civil date of a day number, from Howard Hinnant's `civil_from_days`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (hour, minute, second, micro) = (
        rest / 3_600_000_000,
        rest / 60_000_000 % 60,
        rest / 1_000_000 % 60,
        rest % 1_000_000,
    );
    (year << 46)
        | (month << 42)
        | (day << 37)
        | (hour << 32)
        | (minute << 26)
        | (second << 20)
        | micro
}

/// The value at `index` of `array`, as a GoogleSQL literal writes it: `3`, `3.0`, `'pple'`,
/// `TRUE`, `NULL`, `DATE '2024-05-17'`, `[1, 2]`, or `STRUCT(0.0 AS start, 10.0 AS end)`.
pub fn literal(array: &ArrayRef, index: usize) -> String {
    use datafusion::arrow::util::display::array_value_to_string;
    if array.is_null(index) {
        return "NULL".to_string();
    }
    let display = || array_value_to_string(array, index).expect("a value");
    match array.data_type() {
        DataType::Int64 | DataType::Int32 | DataType::UInt64 => display(),
        DataType::Float64 => {
            let value = array.as_primitive::<Float64Type>().value(index);
            match value.fract() == 0.0 && value.abs() < 1e15 {
                true => format!("{value:.1}"),
                false => format!("{value}"),
            }
        }
        DataType::Decimal128(..) => format!(
            "NUMERIC '{}'",
            display().trim_end_matches('0').trim_end_matches('.')
        ),
        DataType::Boolean => display().to_ascii_uppercase(),
        DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8 => {
            format!("'{}'", display().replace('\\', "\\\\").replace('\'', "\\'"))
        }
        DataType::Binary | DataType::BinaryView | DataType::LargeBinary => {
            let bytes: Vec<u8> = match array.data_type() {
                DataType::Binary => array.as_binary::<i32>().value(index).to_vec(),
                DataType::LargeBinary => array.as_binary::<i64>().value(index).to_vec(),
                _ => array.as_binary_view().value(index).to_vec(),
            };
            format!("b'{}'", String::from_utf8_lossy(&bytes))
        }
        DataType::Date32 => format!("DATE '{}'", display()),
        DataType::Timestamp(_, None) => format!("DATETIME '{}'", display().replace('T', " ")),
        DataType::Timestamp(_, Some(_)) => {
            let utc = datafusion::arrow::compute::cast(&array.slice(index, 1), &types::datetime())
                .expect("a civil time");
            let civil = array_value_to_string(&utc, 0)
                .expect("a value")
                .replace('T', " ");
            format!("TIMESTAMP '{civil}+00'")
        }
        DataType::List(_) | DataType::LargeList(_) => {
            let values = match array.data_type() {
                DataType::List(_) => array.as_list::<i32>().value(index),
                _ => array.as_list::<i64>().value(index),
            };
            let items: Vec<String> = (0..values.len()).map(|i| literal(&values, i)).collect();
            format!("[{}]", items.join(", "))
        }
        DataType::Struct(fields) => {
            let structure = array.as_struct();
            let items: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    format!(
                        "{} AS {}",
                        literal(structure.column(i), index),
                        field.name()
                    )
                })
                .collect();
            format!("STRUCT({})", items.join(", "))
        }
        DataType::Dictionary(_, value) => {
            let values =
                datafusion::arrow::compute::cast(&array.slice(index, 1), value).expect("values");
            literal(&values, 0)
        }
        other => panic!("the tests write no literal of {other}"),
    }
}

/// Base64, as GoogleSQL's JSON writes `BYTES`.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char),
                false => out.push('='),
            }
        }
    }
    out
}
