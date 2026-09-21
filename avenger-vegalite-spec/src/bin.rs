use std::fmt;

use serde::de::{self, value::MapAccessDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::presence::present;

/// Authored binning settings. Encoding null and omission use `MissingNullOrValue`.
#[derive(Debug, Clone, PartialEq)]
pub enum Bin {
    Bool(bool),
    Params(BinParams),
    Binned,
}

/// Vega-Lite bin parameters, with all defaults left unresolved.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinParams {
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub maxbins: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub step: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub steps: Option<Vec<f64>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub minstep: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub base: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub divide: Option<Vec<f64>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub nice: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub anchor: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub extent: Option<[f64; 2]>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub binned: Option<bool>,
}

/// A bin-start alias or explicit start/end aliases. The string form stays unexpanded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BinOutput {
    Name(String),
    Pair([String; 2]),
}

impl Serialize for Bin {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Bool(value) => value.serialize(serializer),
            Self::Params(params) => params.serialize(serializer),
            Self::Binned => serializer.serialize_str("binned"),
        }
    }
}

impl<'de> Deserialize<'de> for Bin {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BinVisitor;
        impl<'de> Visitor<'de> for BinVisitor {
            type Value = Bin;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a boolean, bin parameter object, or \"binned\"")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Bin, E> {
                Ok(Bin::Bool(value))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Bin, E> {
                match value {
                    "binned" => Ok(Bin::Binned),
                    _ => Err(E::unknown_variant(value, &["binned"])),
                }
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Bin, M::Error> {
                BinParams::deserialize(MapAccessDeserializer::new(map)).map(Bin::Params)
            }
        }
        deserializer.deserialize_any(BinVisitor)
    }
}
