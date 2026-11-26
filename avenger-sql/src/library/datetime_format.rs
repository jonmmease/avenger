//! `FORMAT_DATE` and its siblings, and `PARSE_DATE` and its siblings, by avenger-format-datetime-
//! googlesql: GoogleSQL's elements, text, and errors.
//!
//! A format reads the type of its value, as GoogleSQL's does, whatever the function: a `DATE`
//! prints the elements of a time as written, a `DATETIME` its zone, and a `TIMESTAMP` formats in a
//! time zone, so `FORMAT_DATE('%H', d)` of a `DATETIME` prints its hour. A parse gives the type of
//! its function.

use std::sync::Arc;

use avenger_format_datetime_googlesql::{
    parse_date, parse_datetime, parse_time, parse_timestamp, Error, Format, Kind, Locale, Zone,
};
use chrono::{NaiveDate, Timelike};
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, AsArray, Date32Builder, StringArray, StringBuilder,
            Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
        },
        datatypes::{
            DataType, Date32Type, Time64MicrosecondType, TimeUnit, TimestampMicrosecondType,
        },
    },
    common::{exec_err, Result},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use super::columnar;
use crate::types;

/// The text of `value`, a `DATE`, a `TIME`, a `DATETIME`, or a `TIMESTAMP` in the time zone
/// `zone`, in `format` with the names of `locale`, or `NULL` where it fails when `safe`.
pub(super) fn format(zone: Zone, locale: Locale, safe: bool, format: Expr, value: Expr) -> Expr {
    format_udf(zone, locale, safe).call(vec![format, value])
}

/// The function of [`format`].
pub(super) fn format_udf(zone: Zone, locale: Locale, safe: bool) -> ScalarUDF {
    ScalarUDF::new_from_impl(FormatDatetime {
        zone,
        locale,
        safe,
        signature: Signature::any(2, Volatility::Immutable),
    })
}

/// The value of `kind` that the string `value` holds in `format`, with the time zone `zone` for a
/// time without one and the names of `locale`, or `NULL` where it fails when `safe`.
pub(super) fn parse(
    kind: Kind,
    zone: Zone,
    locale: Locale,
    safe: bool,
    format: Expr,
    value: Expr,
) -> Expr {
    parse_udf(kind, zone, locale, safe).call(vec![format, value])
}

/// The function of [`parse`].
pub(super) fn parse_udf(kind: Kind, zone: Zone, locale: Locale, safe: bool) -> ScalarUDF {
    ScalarUDF::new_from_impl(ParseDatetime {
        kind,
        zone,
        locale,
        safe,
        signature: Signature::any(2, Volatility::Immutable),
    })
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct FormatDatetime {
    pub(super) zone: Zone,
    pub(super) locale: Locale,
    pub(super) safe: bool,
    signature: Signature,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct ParseDatetime {
    pub(super) kind: Kind,
    pub(super) zone: Zone,
    pub(super) locale: Locale,
    pub(super) safe: bool,
    signature: Signature,
}

/// The strings of `arg`, as `Utf8`.
fn strings(arg: &ColumnarValue, rows: usize) -> Result<ArrayRef> {
    let array = arg.clone().into_array(rows)?;
    Ok(datafusion::arrow::compute::cast(&array, &DataType::Utf8)?)
}

/// The last format of a call that a row prepared, by its text, which the next rows of a constant
/// format reuse.
struct Prepared {
    last: Option<(String, Result<Format, Error>)>,
}

impl Prepared {
    fn get(&mut self, text: &str, kind: Kind, locale: &Locale) -> &Result<Format, Error> {
        if self.last.as_ref().is_none_or(|(last, _)| last != text) {
            self.last = Some((text.to_string(), Format::new(text, kind, locale)));
        }
        &self.last.as_ref().expect("a prepared format").1
    }
}

/// The text of the value at `row` of `values` in `format`, in the time zone `zone`.
fn text(format: &Format, values: &ArrayRef, row: usize, zone: Zone) -> Option<String> {
    Some(match values.data_type() {
        DataType::Date32 => {
            format.format_date(values.as_primitive::<Date32Type>().value_as_date(row)?)
        }
        DataType::Time64(TimeUnit::Microsecond) => format.format_time(
            values
                .as_primitive::<Time64MicrosecondType>()
                .value_as_time(row)?,
        ),
        DataType::Timestamp(TimeUnit::Microsecond, None) => format.format_datetime(
            values
                .as_primitive::<TimestampMicrosecondType>()
                .value_as_datetime(row)?,
        ),
        DataType::Timestamp(TimeUnit::Microsecond, Some(_)) => {
            let instant = (values.as_primitive::<TimestampMicrosecondType>())
                .value_as_datetime(row)?
                .and_utc();
            format.format_timestamp(instant, zone)
        }
        _ => return None,
    })
}

/// The kind of the values of the Arrow type `ty`.
fn kind_of(ty: &DataType) -> Option<Kind> {
    match ty {
        DataType::Date32 => Some(Kind::Date),
        DataType::Time64(TimeUnit::Microsecond) => Some(Kind::Time),
        DataType::Timestamp(TimeUnit::Microsecond, None) => Some(Kind::Datetime),
        DataType::Timestamp(TimeUnit::Microsecond, Some(_)) => Some(Kind::Timestamp),
        _ => None,
    }
}

impl ScalarUDFImpl for FormatDatetime {
    fn name(&self) -> &str {
        "$format_datetime"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let formats = strings(&args.args[0], rows)?;
        let formats = formats.as_string::<i32>();
        let values = args.args[1].clone().into_array(rows)?;
        let Some(kind) = kind_of(values.data_type()) else {
            return exec_err!("$format_datetime formats a date or a time");
        };
        let locale = &self.locale;
        let mut prepared = Prepared { last: None };
        let mut out = StringBuilder::new();
        for row in 0..values.len() {
            let at = if formats.len() == 1 { 0 } else { row };
            if formats.is_null(at) || values.is_null(row) {
                out.append_null();
                continue;
            }
            let format = prepared.get(formats.value(at), kind, locale);
            let result = match format {
                Ok(format) => text(format, &values, row, self.zone)
                    .ok_or_else(|| Error("Out-of-range datetime value".into())),
                Err(error) => Err(error.clone()),
            };
            match result {
                Ok(text) => out.append_value(text),
                Err(_) if self.safe => out.append_null(),
                Err(error) => return exec_err!("{error}"),
            }
        }
        let out: StringArray = out.finish();
        columnar(Arc::new(out), &args.args)
    }
}

impl ScalarUDFImpl for ParseDatetime {
    fn name(&self) -> &str {
        match self.kind {
            Kind::Date => "$parse_date",
            Kind::Time => "$parse_time",
            Kind::Datetime => "$parse_datetime",
            Kind::Timestamp => "$parse_timestamp",
        }
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(match self.kind {
            Kind::Date => DataType::Date32,
            Kind::Time => DataType::Time64(TimeUnit::Microsecond),
            Kind::Datetime => types::datetime(),
            Kind::Timestamp => types::timestamp(),
        })
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let formats = strings(&args.args[0], rows)?;
        let formats = formats.as_string::<i32>();
        let inputs = strings(&args.args[1], rows)?;
        let inputs = inputs.as_string::<i32>();
        let locale = &self.locale;
        // Each value as days since 1970-01-01 for a `DATE`, and as microseconds otherwise.
        let mut values: Vec<Option<i64>> = Vec::with_capacity(inputs.len());
        for row in 0..inputs.len() {
            let at = if formats.len() == 1 { 0 } else { row };
            if formats.is_null(at) || inputs.is_null(row) {
                values.push(None);
                continue;
            }
            let (format, input) = (formats.value(at), inputs.value(row));
            let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("1970-01-01");
            let value = match self.kind {
                Kind::Date => {
                    parse_date(format, input, locale).map(|date| (date - epoch).num_days())
                }
                Kind::Time => parse_time(format, input, locale).map(|time| {
                    i64::from(time.num_seconds_from_midnight()) * 1_000_000
                        + i64::from(time.nanosecond() / 1_000)
                }),
                Kind::Datetime => parse_datetime(format, input, locale)
                    .map(|datetime| datetime.and_utc().timestamp_micros()),
                Kind::Timestamp => parse_timestamp(format, input, self.zone, locale)
                    .map(|instant| instant.timestamp_micros()),
            };
            match value {
                Ok(value) => values.push(Some(value)),
                Err(_) if self.safe => values.push(None),
                Err(error) => return exec_err!("{error}"),
            }
        }
        let out: ArrayRef = match self.kind {
            Kind::Date => {
                let mut out = Date32Builder::with_capacity(values.len());
                for value in values {
                    out.append_option(value.map(|days| days as i32));
                }
                Arc::new(out.finish())
            }
            Kind::Time => {
                let mut out = Time64MicrosecondBuilder::with_capacity(values.len());
                out.extend(values);
                Arc::new(out.finish())
            }
            Kind::Datetime | Kind::Timestamp => {
                let mut out = TimestampMicrosecondBuilder::with_capacity(values.len());
                out.extend(values);
                let out = out.finish();
                match self.kind {
                    Kind::Timestamp => Arc::new(out.with_timezone(types::UTC)),
                    _ => Arc::new(out),
                }
            }
        };
        columnar(out, &args.args)
    }
}
