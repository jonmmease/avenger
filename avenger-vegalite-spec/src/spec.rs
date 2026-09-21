use std::{collections::BTreeMap, fmt};

use serde::de::{self, value::MapAccessDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::presence::present;
use crate::{AggregateOp, Bin, Data, InlineDataset, MissingNullOrValue, SpecError, Transform};

/// A top-level single-view specification in the supported Vega-Lite subset.
///
/// Parsing preserves authored options and performs no data access. Direct Serde
/// deserialization checks structure only. Call `validate` after construction or editing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitSpec {
    pub data: Data,
    pub mark: Mark,
    #[serde(
        rename = "$schema",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub schema: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub title: Option<Text>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub width: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub height: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub encoding: Option<Encoding>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub transform: Option<Vec<Transform>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub params: Option<Vec<Parameter>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub datasets: Option<BTreeMap<String, InlineDataset>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub usermeta: Option<Map<String, Value>>,
}

impl UnitSpec {
    /// Parses one JSON document and validates rules that do not require source data.
    ///
    /// Returns a path-aware error for unsupported properties, malformed JSON, or
    /// invalid values. Data sources and `$schema` URLs are never accessed.
    pub fn from_json(json: &str) -> Result<Self, SpecError> {
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let spec: Self = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            let path = error.path().to_string();
            SpecError::new(
                if path.is_empty() || path == "." {
                    "$".to_string()
                } else {
                    path
                },
                error.inner().to_string(),
            )
        })?;
        deserializer
            .end()
            .map_err(|error| SpecError::new("$", error.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }
}

/// A single line or multiple title lines, preserving the JSON shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    String(String),
    Lines(Vec<String>),
}

/// A bar mark, either its short string form or an object with explicit options.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    Bar,
    Def(BarMark),
}

/// Supported bar options. Orientation and styling defaults remain unresolved.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BarMark {
    #[serde(rename = "type")]
    pub mark_type: MarkType,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub orient: Option<Orient>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub color: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub opacity: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub size: Option<f64>,
}

/// The supported mark object discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkType {
    #[default]
    Bar,
}

/// Explicit bar orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orient {
    Horizontal,
    Vertical,
}

/// Positional encodings for ordinary and ranged bars.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Encoding {
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub x: Option<PositionFieldDef>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub y: Option<PositionFieldDef>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub x2: Option<SecondaryFieldDef>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub y2: Option<SecondaryFieldDef>,
}

/// A primary positional field, optionally binned or aggregated.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionFieldDef {
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub field: Option<String>,
    #[serde(
        rename = "type",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub field_type: Option<FieldType>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub aggregate: Option<AggregateOp>,
    #[serde(default, skip_serializing_if = "MissingNullOrValue::is_missing")]
    pub bin: MissingNullOrValue<Bin>,
    #[serde(default, skip_serializing_if = "MissingNullOrValue::is_missing")]
    pub title: MissingNullOrValue<Text>,
    #[serde(default, skip_serializing_if = "MissingNullOrValue::is_missing")]
    pub sort: MissingNullOrValue<SortOrder>,
    #[serde(default, skip_serializing_if = "MissingNullOrValue::is_missing")]
    pub axis: MissingNullOrValue<Axis>,
    #[serde(default, skip_serializing_if = "MissingNullOrValue::is_missing")]
    pub scale: MissingNullOrValue<Scale>,
    #[serde(default, skip_serializing_if = "MissingNullOrValue::is_missing")]
    pub stack: MissingNullOrValue<StackOffset>,
}

/// A secondary boundary field. Primary-channel options do not apply here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecondaryFieldDef {
    pub field: String,
}

/// Authored measurement type, before data-driven inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Quantitative,
    Ordinal,
    Nominal,
    Temporal,
}

/// Supported field sort directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Axis options for the initial bar-chart subset.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Axis {
    #[serde(default, skip_serializing_if = "MissingNullOrValue::is_missing")]
    pub title: MissingNullOrValue<Text>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub format: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub label_angle: Option<f64>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub grid: Option<bool>,
}

/// Basic scale options, with omission distinct from an explicitly disabled scale.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scale {
    #[serde(
        rename = "type",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub scale_type: Option<ScaleType>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub zero: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub nice: Option<bool>,
}

/// Supported positional scale types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScaleType {
    Linear,
    Band,
    Point,
}

impl Serialize for Mark {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Bar => serializer.serialize_str("bar"),
            Self::Def(mark) => mark.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Mark {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MarkVisitor;
        impl<'de> Visitor<'de> for MarkVisitor {
            type Value = Mark;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("\"bar\" or a bar mark object")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Mark, E> {
                match value {
                    "bar" => Ok(Mark::Bar),
                    _ => Err(E::unknown_variant(value, &["bar"])),
                }
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Mark, M::Error> {
                BarMark::deserialize(MapAccessDeserializer::new(map)).map(Mark::Def)
            }
        }
        deserializer.deserialize_any(MarkVisitor)
    }
}

/// An initialized numeric variable parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub value: f64,
}

/// The supported stack offset. Explicit null disables stacking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StackOffset {
    Zero,
}
