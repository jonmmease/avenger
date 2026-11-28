//! The library's functions in a plan that travels, such as to a page: each travels as its name and
//! the bytes of its state, so a reader that knows the library builds the function that the planner
//! built. The state is a `LibraryState` of package `avenger.sql.v1`, which
//! [`proto`](crate::proto) holds. A function without state, such as `$format`, travels as its name
//! alone. avenger-sql's host, [`bake::host`](crate::bake::host), calls these for its functions.

use std::{any::Any, sync::Arc};

use avenger_format_datetime_googlesql::{Kind, Locale, Zone};
use avenger_format_locales_d3::DateTimeLocaleSpec;
use datafusion::{
    common::{exec_datafusion_err, Result},
    logical_expr::ScalarUDF,
};
use prost::Message;

use super::{
    bytes, cast_string, datetime_format, encoding, error, interval, like, re2, string,
    string_format,
};
use crate::proto::{self, LibraryState};

fn invalid(error: impl std::fmt::Display) -> datafusion::common::DataFusionError {
    exec_datafusion_err!("the bytes of a function of avenger-sql's library: {error}")
}

/// `locale` as a state holds it: the d3 time locale of its names and composites.
fn locale_state(locale: &Locale) -> proto::Locale {
    let spec = locale.spec();
    proto::Locale {
        date_time: spec.date_time,
        date: spec.date,
        time: spec.time,
        periods: spec.periods.to_vec(),
        days: spec.days.to_vec(),
        short_days: spec.short_days.to_vec(),
        months: spec.months.to_vec(),
        short_months: spec.short_months.to_vec(),
    }
}

/// The locale that `state` holds.
fn locale_of(state: proto::Locale) -> Result<Locale> {
    fn names<const N: usize>(names: Vec<String>) -> Result<[String; N]> {
        names
            .try_into()
            .map_err(|names: Vec<String>| invalid(format!("{} names", names.len())))
    }
    let spec = DateTimeLocaleSpec {
        date_time: state.date_time,
        date: state.date,
        time: state.time,
        periods: names(state.periods)?,
        days: names(state.days)?,
        short_days: names(state.short_days)?,
        months: names(state.months)?,
        short_months: names(state.short_months)?,
    };
    Locale::new(&spec).map_err(invalid)
}

/// The bytes of the state of a function: its time zone by its name, its locale, and whether it
/// gives `NULL` where it fails.
fn state_bytes(zone: Zone, locale: Option<&Locale>, safe: bool) -> Vec<u8> {
    let state = LibraryState {
        zone: zone.name(),
        locale: locale.map(locale_state),
        safe,
    };
    state.encode_to_vec()
}

/// The time zone that `state` names.
fn zone_of(state: &LibraryState) -> Result<Zone> {
    Zone::parse(&state.zone).ok_or_else(|| invalid(format!("the time zone {}", state.zone)))
}

/// The locale of `state`, or the default locale when it holds none.
fn state_locale(state: &LibraryState) -> Result<Locale> {
    match state.locale.clone() {
        Some(locale) => locale_of(locale),
        None => Ok(Locale::default()),
    }
}

/// The library's functions without state.
fn stateless() -> [Arc<ScalarUDF>; 11] {
    [
        Arc::new(string_format::FORMAT.clone()),
        string::INITCAP.clone(),
        bytes::BYTES_BITWISE.clone(),
        like::LIKE_UNNEST.clone(),
        error::ERROR_WHEN.clone(),
        interval::MAKE.clone(),
        interval::MULTIPLY.clone(),
        interval::DIVIDE.clone(),
        re2::CONTAINS.clone(),
        re2::EXTRACT.clone(),
        re2::REPLACE.clone(),
    ]
}

/// The bytes of the state of `udf`, when it is one of the library's functions.
pub(crate) fn encode_udf(udf: &ScalarUDF) -> Option<Result<Vec<u8>>> {
    let inner = udf.inner().as_ref() as &dyn Any;
    if let Some(format) = inner.downcast_ref::<datetime_format::FormatDatetime>() {
        return Some(Ok(state_bytes(
            format.zone,
            Some(&format.locale),
            format.safe,
        )));
    }
    if let Some(parse) = inner.downcast_ref::<datetime_format::ParseDatetime>() {
        return Some(Ok(state_bytes(parse.zone, Some(&parse.locale), parse.safe)));
    }
    if let Some(cast) = inner.downcast_ref::<cast_string::CastToString>() {
        return Some(Ok(state_bytes(cast.zone, None, false)));
    }
    if inner.downcast_ref::<encoding::Encode>().is_some() {
        return Some(Ok(Vec::new()));
    }
    if let Some(decode) = inner.downcast_ref::<encoding::Decode>() {
        return Some(Ok(state_bytes(Zone::UTC, None, decode.safe)));
    }
    (stateless().iter())
        .any(|function| function.name() == udf.name())
        .then(|| Ok(Vec::new()))
}

/// The library's function named `name` with the state `bytes`, when the library has one.
pub(crate) fn decode_udf(name: &str, bytes: &[u8]) -> Option<Result<Arc<ScalarUDF>>> {
    let state = || LibraryState::decode(bytes).map_err(invalid);
    let parse = |kind: Kind| {
        let state = state()?;
        let (zone, locale) = (zone_of(&state)?, state_locale(&state)?);
        Ok(Arc::new(datetime_format::parse_udf(
            kind, zone, locale, state.safe,
        )))
    };
    let decoded = match name {
        "$format_datetime" => (|| {
            let state = state()?;
            let (zone, locale) = (zone_of(&state)?, state_locale(&state)?);
            Ok(Arc::new(datetime_format::format_udf(
                zone, locale, state.safe,
            )))
        })(),
        "$parse_date" => parse(Kind::Date),
        "$parse_time" => parse(Kind::Time),
        "$parse_datetime" => parse(Kind::Datetime),
        "$parse_timestamp" => parse(Kind::Timestamp),
        "$cast_to_string" => (|| Ok(Arc::new(cast_string::udf(zone_of(&state()?)?))))(),
        "$to_hex" => Ok(Arc::new(encoding::encode_udf(encoding::Encoding::Hex))),
        "$to_base64" => Ok(Arc::new(encoding::encode_udf(encoding::Encoding::Base64))),
        "$from_hex" => (|| {
            let udf = encoding::decode_udf(encoding::Encoding::Hex, state()?.safe);
            Ok(Arc::new(udf))
        })(),
        "$from_base64" => (|| {
            let udf = encoding::decode_udf(encoding::Encoding::Base64, state()?.safe);
            Ok(Arc::new(udf))
        })(),
        _ => return stateless().into_iter().find(|f| f.name() == name).map(Ok),
    };
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each function of the library decodes from its name and bytes into the function that
    /// encoded them.
    #[test]
    fn each_function_decodes_from_its_bytes() {
        let zone = Zone::parse("America/New_York").unwrap();
        let us = Locale::bundled("en-US").unwrap();
        let mut functions: Vec<Arc<ScalarUDF>> = vec![
            Arc::new(datetime_format::format_udf(zone, us.clone(), true)),
            Arc::new(datetime_format::format_udf(
                Zone::UTC,
                Locale::default(),
                false,
            )),
            Arc::new(cast_string::udf(Zone::parse("+05:30").unwrap())),
            Arc::new(encoding::encode_udf(encoding::Encoding::Hex)),
            Arc::new(encoding::encode_udf(encoding::Encoding::Base64)),
            Arc::new(encoding::decode_udf(encoding::Encoding::Hex, true)),
            Arc::new(encoding::decode_udf(encoding::Encoding::Base64, false)),
        ];
        for kind in [Kind::Date, Kind::Time, Kind::Datetime, Kind::Timestamp] {
            functions.push(Arc::new(datetime_format::parse_udf(
                kind,
                zone,
                us.clone(),
                false,
            )));
        }
        functions.extend(stateless());
        for function in functions {
            let bytes = encode_udf(&function).unwrap().unwrap();
            let decoded = decode_udf(function.name(), &bytes).unwrap().unwrap();
            assert_eq!(decoded, function, "{}", function.name());
        }
        let other = datafusion::functions::string::upper();
        assert!(encode_udf(&other).is_none());
        assert!(decode_udf("upper", &[]).is_none());
    }
}
