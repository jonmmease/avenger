//! The states of the bound calls of marks and compositions, which the codec of a plan writes with
//! a function's name, so a plan that a host sends builds the same charts where it runs.

use arrow::datatypes::{FieldRef, Schema};
use avenger_chart::functions::{composition, MarkTemplate};
use datafusion_common::{exec_datafusion_err, Result, ScalarValue};
use datafusion_proto_common as protobuf;
use prost::Message;

use super::{composition::CompositionInstance, mark::MarkInstance};
use crate::proto::{CompositionState, MarkState, OptionValue, Order};

fn invalid(error: impl std::fmt::Display) -> datafusion_common::DataFusionError {
    exec_datafusion_err!("the bytes of a chart's function: {error}")
}

fn orders(order: &[(bool, bool)]) -> Vec<Order> {
    (order.iter())
        .map(|(asc, nulls_first)| Order {
            asc: *asc,
            nulls_first: *nulls_first,
        })
        .collect()
}

fn from_orders(order: &[Order]) -> Vec<(bool, bool)> {
    (order.iter())
        .map(|order| (order.asc, order.nulls_first))
        .collect()
}

impl MarkInstance {
    /// The bound call as its state, which [`MarkInstance::from_state`] reads.
    pub fn to_state(&self) -> Result<MarkState> {
        let (template, fields, order) = self.parts();
        let schema = Schema::new(fields.to_vec());
        let schema = protobuf::Schema::try_from(&schema).map_err(invalid)?;
        Ok(MarkState {
            template: template.to_bytes(),
            fields: schema.encode_to_vec(),
            order: orders(order),
        })
    }

    /// The bound call of the state `state`, as [`MarkInstance::to_state`] writes it.
    pub fn from_state(state: &MarkState) -> Result<Self> {
        let template = MarkTemplate::from_bytes(&state.template).map_err(invalid)?;
        let schema = protobuf::Schema::decode(state.fields.as_slice()).map_err(invalid)?;
        let schema = Schema::try_from(&schema).map_err(invalid)?;
        let fields: Vec<FieldRef> = schema.fields().iter().cloned().collect();
        Ok(Self::with_order(
            template,
            fields,
            from_orders(&state.order),
        ))
    }
}

impl CompositionInstance {
    /// The bound call as its state, which [`CompositionInstance::from_state`] reads.
    pub fn to_state(&self) -> Result<CompositionState> {
        let (spec, column, options, order) = self.parts();
        let options = (options.iter())
            .map(|value| {
                let value = value
                    .as_ref()
                    .map(|value| protobuf::ScalarValue::try_from(value).map_err(invalid))
                    .transpose()?;
                Ok(OptionValue {
                    value: value.map(|value| value.encode_to_vec()),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(CompositionState {
            spec: spec.to_string(),
            column: column.map(str::to_string),
            options,
            order: orders(order),
        })
    }

    /// The bound call of the state `state`, as [`CompositionInstance::to_state`] writes it.
    pub fn from_state(state: &CompositionState) -> Result<Self> {
        let spec = composition(&state.spec)
            .ok_or_else(|| invalid(format!("no composition `{}`", state.spec)))?;
        let options = (state.options.iter())
            .map(|option| {
                option
                    .value
                    .as_ref()
                    .map(|value| {
                        let value =
                            protobuf::ScalarValue::decode(value.as_slice()).map_err(invalid)?;
                        ScalarValue::try_from(&value).map_err(invalid)
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self::with_order(
            spec,
            state.column.clone(),
            options,
            from_orders(&state.order),
        ))
    }
}
