//! The bytes of the bound calls of marks and compositions, which the codec of a plan writes with
//! a function's name, so a plan that a host sends builds the same charts where it runs.

use arrow::datatypes::{FieldRef, Schema};
use avenger_chart::functions::{composition, MarkTemplate};
use datafusion_common::{exec_datafusion_err, Result, ScalarValue};
use datafusion_proto_common as protobuf;
use prost::Message;

use super::{composition::CompositionInstance, mark::MarkInstance};

#[derive(Clone, PartialEq, Message)]
struct MarkBytes {
    #[prost(bytes = "vec", tag = "1")]
    template: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    fields: Option<protobuf::Schema>,
    #[prost(message, repeated, tag = "3")]
    order: Vec<OrderBytes>,
}

#[derive(Clone, PartialEq, Message)]
struct OrderBytes {
    #[prost(bool, tag = "1")]
    asc: bool,
    #[prost(bool, tag = "2")]
    nulls_first: bool,
}

#[derive(Clone, PartialEq, Message)]
struct CompositionBytes {
    #[prost(string, tag = "1")]
    spec: String,
    #[prost(string, optional, tag = "2")]
    column: Option<String>,
    #[prost(message, repeated, tag = "3")]
    options: Vec<OptionBytes>,
    #[prost(message, repeated, tag = "4")]
    order: Vec<OrderBytes>,
}

#[derive(Clone, PartialEq, Message)]
struct OptionBytes {
    #[prost(message, optional, tag = "1")]
    value: Option<protobuf::ScalarValue>,
}

fn invalid(error: impl std::fmt::Display) -> datafusion_common::DataFusionError {
    exec_datafusion_err!("the bytes of a chart's function: {error}")
}

fn orders(order: &[(bool, bool)]) -> Vec<OrderBytes> {
    (order.iter())
        .map(|(asc, nulls_first)| OrderBytes {
            asc: *asc,
            nulls_first: *nulls_first,
        })
        .collect()
}

fn from_orders(order: &[OrderBytes]) -> Vec<(bool, bool)> {
    (order.iter())
        .map(|order| (order.asc, order.nulls_first))
        .collect()
}

impl MarkInstance {
    /// The bound call as bytes, which [`MarkInstance::from_bytes`] reads.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let (template, fields, order) = self.parts();
        let schema = Schema::new(fields.to_vec());
        Ok(MarkBytes {
            template: template.to_bytes(),
            fields: Some(protobuf::Schema::try_from(&schema).map_err(invalid)?),
            order: orders(order),
        }
        .encode_to_vec())
    }

    /// The bound call of the bytes `bytes`, as [`MarkInstance::to_bytes`] writes them.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let decoded = MarkBytes::decode(bytes).map_err(invalid)?;
        let template = MarkTemplate::from_bytes(&decoded.template).map_err(invalid)?;
        let schema = decoded
            .fields
            .as_ref()
            .ok_or_else(|| invalid("no fields"))?;
        let schema = Schema::try_from(schema).map_err(invalid)?;
        let fields: Vec<FieldRef> = schema.fields().iter().cloned().collect();
        Ok(Self::with_order(
            template,
            fields,
            from_orders(&decoded.order),
        ))
    }
}

impl CompositionInstance {
    /// The bound call as bytes, which [`CompositionInstance::from_bytes`] reads.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let (spec, column, options, order) = self.parts();
        let options = (options.iter())
            .map(|value| {
                let value = value
                    .as_ref()
                    .map(|value| protobuf::ScalarValue::try_from(value).map_err(invalid))
                    .transpose()?;
                Ok(OptionBytes { value })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(CompositionBytes {
            spec: spec.to_string(),
            column: column.map(str::to_string),
            options,
            order: orders(order),
        }
        .encode_to_vec())
    }

    /// The bound call of the bytes `bytes`, as [`CompositionInstance::to_bytes`] writes them.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let decoded = CompositionBytes::decode(bytes).map_err(invalid)?;
        let spec = composition(&decoded.spec)
            .ok_or_else(|| invalid(format!("no composition `{}`", decoded.spec)))?;
        let options = (decoded.options.iter())
            .map(|option| {
                option
                    .value
                    .as_ref()
                    .map(|value| ScalarValue::try_from(value).map_err(invalid))
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::with_order(
            spec,
            decoded.column,
            options,
            from_orders(&decoded.order),
        ))
    }
}
