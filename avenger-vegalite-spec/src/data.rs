use std::fmt;

use serde::de::{self, value::MapAccessDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::presence::present;

/// Inline row objects. Values retain their JSON types without Arrow conversion.
pub type InlineDataset = Vec<Map<String, Value>>;

/// An inline dataset, a source reference, or explicit null. Parsing never reads sources.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Data {
    Inline {
        values: InlineDataset,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<DataFormat>,
    },
    Url {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<DataFormat>,
    },
    Named {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        format: Option<DataFormat>,
    },
    Empty,
}

/// An optional file-format hint. Parsing directives are not supported.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataFormat {
    #[serde(
        rename = "type",
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub format_type: Option<FormatType>,
}

/// Supported source-format names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatType {
    Json,
    Csv,
    Tsv,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DataFields {
    #[serde(default, deserialize_with = "present")]
    values: Option<InlineDataset>,
    #[serde(default, deserialize_with = "present")]
    url: Option<String>,
    #[serde(default, deserialize_with = "present")]
    name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    format: Option<DataFormat>,
}

impl<'de> Deserialize<'de> for Data {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DataVisitor;
        impl<'de> Visitor<'de> for DataVisitor {
            type Value = Data;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an inline, URL, or named data object, or null")
            }

            fn visit_unit<E: de::Error>(self) -> Result<Data, E> {
                Ok(Data::Empty)
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Data, M::Error> {
                let fields = DataFields::deserialize(MapAccessDeserializer::new(map))?;
                match (fields.values, fields.url, fields.name) {
                    (Some(_), Some(_), _) => {
                        Err(de::Error::custom("data cannot contain both values and url"))
                    }
                    (Some(values), None, name) => Ok(Data::Inline {
                        values,
                        name,
                        format: fields.format,
                    }),
                    (None, Some(url), name) => Ok(Data::Url {
                        url,
                        name,
                        format: fields.format,
                    }),
                    (None, None, Some(name)) => Ok(Data::Named {
                        name,
                        format: fields.format,
                    }),
                    (None, None, None) => {
                        Err(de::Error::custom("data requires values, url, or name"))
                    }
                }
            }
        }
        deserializer.deserialize_any(DataVisitor)
    }
}
