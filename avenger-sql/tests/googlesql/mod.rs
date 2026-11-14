//! What the tests that compare with GoogleSQL's `execute_query` share: the text of a value as its
//! table mode prints it.

use arrow::{
    array::{Array, ArrayRef, AsArray},
    datatypes::{DataType, Float64Type, IntervalMonthDayNanoType},
    util::display::array_value_to_string,
};

/// The text of the value at `index` of `array`, as the table mode of `execute_query` prints it.
pub fn cell(array: &ArrayRef, index: usize) -> String {
    if array.is_null(index) {
        return "NULL".to_string();
    }
    match array.data_type() {
        DataType::Float64 => {
            let value = array.as_primitive::<Float64Type>().value(index);
            match value.fract() == 0.0 && value.abs() < 1e15 {
                true => format!("{}", value as i64),
                false => format!("{value}"),
            }
        }
        DataType::Decimal128(..) | DataType::Decimal256(..) => {
            let text = array_value_to_string(array, index).unwrap();
            match text.contains('.') {
                true => text.trim_end_matches('0').trim_end_matches('.').to_string(),
                false => text,
            }
        }
        DataType::Timestamp(_, None) => array_value_to_string(array, index)
            .unwrap()
            .replace('T', " "),
        DataType::Interval(_) => {
            // GoogleSQL's text of an interval: `Y-M D H:M:S[.F]`, with milliseconds or
            // microseconds.
            let value = array
                .as_primitive::<IntervalMonthDayNanoType>()
                .value(index);
            let sign = if value.months < 0 { "-" } else { "" };
            let (years, months) = (value.months.abs() / 12, value.months.abs() % 12);
            let micros = value.nanoseconds / 1_000;
            let time_sign = if micros < 0 { "-" } else { "" };
            let micros = micros.abs();
            let (hours, minutes, seconds, fraction) = (
                micros / 3_600_000_000,
                micros / 60_000_000 % 60,
                micros / 1_000_000 % 60,
                micros % 1_000_000,
            );
            let seconds = match (fraction, fraction % 1_000) {
                (0, _) => seconds.to_string(),
                (f, 0) => format!("{seconds}.{:03}", f / 1_000),
                (f, _) => format!("{seconds}.{f:06}"),
            };
            format!(
                "{sign}{years}-{months} {} {time_sign}{hours}:{minutes}:{seconds}",
                value.days
            )
        }
        DataType::List(_) | DataType::LargeList(_) => {
            let values = match array.data_type() {
                DataType::List(_) => array.as_list::<i32>().value(index),
                _ => array.as_list::<i64>().value(index),
            };
            let items: Vec<String> = (0..values.len()).map(|i| cell(&values, i)).collect();
            format!("[{}]", items.join(", "))
        }
        // The table mode prints a struct as its values, each before its field's name, as
        // `{2 x, b s}`, with no name for an anonymous field.
        DataType::Struct(fields) => {
            let array = array.as_struct();
            let values: Vec<String> = (fields.iter().enumerate())
                .map(|(field_index, field)| {
                    let value = cell(array.column(field_index), index);
                    match field.name().is_empty() || field.name().starts_with("_field_") {
                        true => value,
                        false => format!("{value} {}", field.name()),
                    }
                })
                .collect();
            format!("{{{}}}", values.join(", "))
        }
        // The table mode prints bytes as their text.
        DataType::Binary | DataType::BinaryView | DataType::LargeBinary => {
            let bytes = match array.data_type() {
                DataType::Binary => array.as_binary::<i32>().value(index),
                DataType::LargeBinary => array.as_binary::<i64>().value(index),
                _ => array.as_binary_view().value(index),
            };
            String::from_utf8_lossy(bytes).into_owned()
        }
        _ => array_value_to_string(array, index).unwrap(),
    }
}
