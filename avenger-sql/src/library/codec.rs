//! The library's functions in the plans of a page's graph: each travels as its name and the bytes
//! of its state, so a page that knows the library builds the function that the planner built.
//! A function without state, such as `$format`, travels as its name alone.

use std::{any::Any, sync::Arc};

use avenger_format_datetime_googlesql::{Kind, Locale, Zone};
use avenger_format_locales_d3::DateTimeLocaleSpec;
use avenger_physical_graph::codec::HostCodec;
use datafusion::{
    common::{exec_datafusion_err, Result},
    logical_expr::ScalarUDF,
};
use prost::Message;

use super::{
    bytes, cast_string, datetime_format, encoding, error, interval, like, re2, string,
    string_format,
};

/// The codec of the library's functions, as the host of a graph's plans.
pub fn host() -> Arc<dyn HostCodec> {
    Arc::new(Library)
}

#[derive(Debug)]
struct Library;

/// The state of a function of dates and times: its time zone by its name, its locale, and whether
/// it gives `NULL` where it fails.
#[derive(Clone, PartialEq, Message)]
struct State {
    #[prost(string, tag = "1")]
    zone: String,
    #[prost(message, optional, tag = "2")]
    locale: Option<LocaleState>,
    #[prost(bool, tag = "3")]
    safe: bool,
}

/// A locale, by the d3 time locale of its names and composites.
#[derive(Clone, PartialEq, Message)]
struct LocaleState {
    #[prost(string, tag = "1")]
    date_time: String,
    #[prost(string, tag = "2")]
    date: String,
    #[prost(string, tag = "3")]
    time: String,
    #[prost(string, repeated, tag = "4")]
    periods: Vec<String>,
    #[prost(string, repeated, tag = "5")]
    days: Vec<String>,
    #[prost(string, repeated, tag = "6")]
    short_days: Vec<String>,
    #[prost(string, repeated, tag = "7")]
    months: Vec<String>,
    #[prost(string, repeated, tag = "8")]
    short_months: Vec<String>,
}

fn invalid(error: impl std::fmt::Display) -> datafusion::common::DataFusionError {
    exec_datafusion_err!("the bytes of a function of avenger-sql's library: {error}")
}

impl LocaleState {
    fn of(locale: &Locale) -> LocaleState {
        let spec = locale.spec();
        LocaleState {
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

    fn locale(self) -> Result<Locale> {
        fn names<const N: usize>(names: Vec<String>) -> Result<[String; N]> {
            names
                .try_into()
                .map_err(|names: Vec<String>| invalid(format!("{} names", names.len())))
        }
        let spec = DateTimeLocaleSpec {
            date_time: self.date_time,
            date: self.date,
            time: self.time,
            periods: names(self.periods)?,
            days: names(self.days)?,
            short_days: names(self.short_days)?,
            months: names(self.months)?,
            short_months: names(self.short_months)?,
        };
        Locale::new(&spec).map_err(invalid)
    }
}

impl State {
    fn bytes(zone: Zone, locale: Option<&Locale>, safe: bool) -> Vec<u8> {
        let state = State {
            zone: zone.name(),
            locale: locale.map(LocaleState::of),
            safe,
        };
        state.encode_to_vec()
    }

    fn zone(&self) -> Result<Zone> {
        Zone::parse(&self.zone).ok_or_else(|| invalid(format!("the time zone {}", self.zone)))
    }

    fn locale(&self) -> Result<Locale> {
        match self.locale.clone() {
            Some(locale) => locale.locale(),
            None => Ok(Locale::default()),
        }
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

impl HostCodec for Library {
    fn name(&self) -> &str {
        "avenger-sql"
    }

    fn encode_udf(&self, udf: &ScalarUDF) -> Option<Result<Vec<u8>>> {
        let inner = udf.inner().as_ref() as &dyn Any;
        if let Some(format) = inner.downcast_ref::<datetime_format::FormatDatetime>() {
            return Some(Ok(State::bytes(
                format.zone,
                Some(&format.locale),
                format.safe,
            )));
        }
        if let Some(parse) = inner.downcast_ref::<datetime_format::ParseDatetime>() {
            return Some(Ok(State::bytes(
                parse.zone,
                Some(&parse.locale),
                parse.safe,
            )));
        }
        if let Some(cast) = inner.downcast_ref::<cast_string::CastToString>() {
            return Some(Ok(State::bytes(cast.zone, None, false)));
        }
        if inner.downcast_ref::<encoding::Encode>().is_some() {
            return Some(Ok(Vec::new()));
        }
        if let Some(decode) = inner.downcast_ref::<encoding::Decode>() {
            return Some(Ok(State::bytes(Zone::UTC, None, decode.safe)));
        }
        (stateless().iter())
            .any(|function| function.name() == udf.name())
            .then(|| Ok(Vec::new()))
    }

    fn decode_udf(&self, name: &str, bytes: &[u8]) -> Option<Result<Arc<ScalarUDF>>> {
        let state = || State::decode(bytes).map_err(invalid);
        let parse = |kind: Kind| {
            let state = state()?;
            let udf = datetime_format::parse_udf(kind, state.zone()?, state.locale()?, state.safe);
            Ok(Arc::new(udf))
        };
        let decoded = match name {
            "$format_datetime" => (|| {
                let state = state()?;
                let udf = datetime_format::format_udf(state.zone()?, state.locale()?, state.safe);
                Ok(Arc::new(udf))
            })(),
            "$parse_date" => parse(Kind::Date),
            "$parse_time" => parse(Kind::Time),
            "$parse_datetime" => parse(Kind::Datetime),
            "$parse_timestamp" => parse(Kind::Timestamp),
            "$cast_to_string" => (|| Ok(Arc::new(cast_string::udf(state()?.zone()?))))(),
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
            let bytes = Library.encode_udf(&function).unwrap().unwrap();
            let decoded = Library
                .decode_udf(function.name(), &bytes)
                .unwrap()
                .unwrap();
            assert_eq!(decoded, function, "{}", function.name());
        }
        let other = datafusion::functions::string::upper();
        assert!(Library.encode_udf(&other).is_none());
        assert!(Library.decode_udf("upper", &[]).is_none());
    }
}
