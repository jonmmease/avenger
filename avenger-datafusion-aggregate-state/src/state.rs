use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanArray, StructArray, UInt64Array},
        datatypes::{DataType, Field, FieldRef, Fields, Schema},
    },
    common::{exec_err, plan_err, DataFusionError, Result, ScalarValue},
    logical_expr::{
        function::{AccumulatorArgs, StateFieldsArgs},
        Accumulator, AggregateUDF, GroupsAccumulator,
    },
    physical_expr::{expressions::Column, PhysicalExpr},
};
use serde::{Deserialize, Serialize};

use crate::families::Family;

const SIGNATURE: &str = "datafusion-aggregate-state:signature";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    inputs: Vec<DataType>,
    result: DataType,
    datafusion: String,
}

pub(crate) fn input_fields(types: &[DataType]) -> Vec<FieldRef> {
    types
        .iter()
        .enumerate()
        .map(|(i, t)| Arc::new(Field::new(format!("arg{i}"), t.clone(), true)))
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct StateType {
    pub family: Family,
    pub native: Arc<AggregateUDF>,
    pub inputs: Vec<FieldRef>,
    pub result: FieldRef,
    pub fields: Fields,
    pub data_type: DataType,
}

impl StateType {
    pub fn new(family: Family, types: &[DataType]) -> Result<Self> {
        let normalized = if family == Family::Count && types.is_empty() {
            vec![DataType::Int64]
        } else {
            types.to_vec()
        };
        let coerced = family.coerce(&normalized)?;
        if coerced != normalized {
            return plan_err!("{} state requires coerced argument types", family.prefix());
        }
        let inputs = input_fields(&normalized);
        let native = family.native();
        let result = native.return_field(&inputs)?;
        let fields: Fields = native
            .state_fields(StateFieldsArgs {
                name: "state",
                input_fields: &inputs,
                return_field: Arc::clone(&result),
                ordering_fields: &[],
                is_distinct: false,
            })?
            .into();
        let descriptor = Descriptor {
            inputs: normalized,
            result: result.data_type().clone(),
            datafusion: "54.1.0".into(),
        };
        let metadata = HashMap::from([(
            SIGNATURE.to_owned(),
            serde_json::to_string(&descriptor).map_err(external)?,
        )]);
        let payload = Field::new(
            format!("{}_state_v1", family.prefix()),
            DataType::Struct(fields.clone()),
            false,
        )
        .with_metadata(metadata);
        let data_type = DataType::Struct(vec![Arc::new(payload)].into());
        Ok(Self {
            family,
            native,
            inputs,
            result,
            fields,
            data_type,
        })
    }

    pub fn from_type(family: Family, data_type: &DataType) -> Result<Self> {
        let DataType::Struct(fields) = data_type else {
            return plan_err!("{} expects a state struct", family.prefix());
        };
        if fields.len() != 1 || fields[0].name() != &format!("{}_state_v1", family.prefix()) {
            return plan_err!(
                "Incompatible {} state family or encoding version",
                family.prefix()
            );
        }
        let encoded = fields[0].metadata().get(SIGNATURE).ok_or_else(|| {
            DataFusionError::Plan("Aggregate state signature metadata is missing".into())
        })?;
        let descriptor: Descriptor = serde_json::from_str(encoded).map_err(|e| {
            DataFusionError::Plan(format!("Invalid aggregate state signature: {e}"))
        })?;
        if descriptor.datafusion != "54.1.0" {
            return plan_err!("Unsupported aggregate state DataFusion version");
        }
        let state = Self::new(family, &descriptor.inputs)?;
        if state.result.data_type() != &descriptor.result || &state.data_type != data_type {
            return plan_err!(
                "Incompatible {} state signature or payload layout",
                family.prefix()
            );
        }
        Ok(state)
    }

    pub fn args_type(family: Family, types: &[DataType]) -> Result<Self> {
        if types.len() != 1 {
            return plan_err!("State consumers expect one argument");
        }
        Self::from_type(family, &types[0])
    }

    fn with_args<T>(&self, f: impl FnOnce(AccumulatorArgs<'_>) -> Result<T>) -> Result<T> {
        let schema = Schema::new(self.inputs.clone());
        let exprs: Vec<Arc<dyn PhysicalExpr>> = self
            .inputs
            .iter()
            .enumerate()
            .map(|(i, field)| Arc::new(Column::new(field.name(), i)) as _)
            .collect();
        f(AccumulatorArgs {
            return_field: Arc::clone(&self.result),
            schema: &schema,
            ignore_nulls: false,
            order_bys: &[],
            is_reversed: false,
            name: "state",
            is_distinct: false,
            exprs: &exprs,
            expr_fields: &self.inputs,
        })
    }

    pub fn accumulator(&self) -> Result<Box<dyn Accumulator>> {
        self.with_args(|a| self.native.accumulator(a))
    }
    pub fn groups_supported(&self) -> bool {
        self.with_args(|a| Ok(self.native.groups_accumulator_supported(a)))
            .unwrap_or(false)
    }
    pub fn groups(&self) -> Result<Box<dyn GroupsAccumulator>> {
        self.with_args(|a| self.native.create_groups_accumulator(a))
    }

    pub fn heap_size(&self) -> usize {
        // Count the bound schema once, including the nested signature metadata.
        // The native function implementation is shared by all bindings.
        self.inputs.capacity() * size_of::<FieldRef>()
            + self.inputs.iter().map(|f| f.size()).sum::<usize>()
            + self.result.size()
            + self.data_type.size()
            - size_of::<DataType>()
    }

    pub fn pack(&self, values: Vec<ArrayRef>) -> Result<ArrayRef> {
        let payload =
            Arc::new(StructArray::try_new(self.fields.clone(), values, None)?) as ArrayRef;
        let DataType::Struct(fields) = &self.data_type else {
            unreachable!()
        };
        Ok(Arc::new(StructArray::try_new(
            fields.clone(),
            vec![payload],
            None,
        )?))
    }

    pub fn pack_scalar(&self, values: Vec<ScalarValue>) -> Result<ScalarValue> {
        let array = self.pack(
            values
                .iter()
                .map(|v| v.to_array_of_size(1))
                .collect::<Result<_>>()?,
        )?;
        ScalarValue::try_from_array(&array, 0)
    }

    /// Borrow state columns and combine outer validity with an aggregate filter.
    pub fn unpack(
        &self,
        values: &[ArrayRef],
        filter: Option<&BooleanArray>,
    ) -> Result<(Vec<ArrayRef>, Option<BooleanArray>)> {
        if values.len() != 1 || values[0].data_type() != &self.data_type {
            return exec_err!("Incompatible aggregate state array");
        }
        let outer = values[0]
            .as_any()
            .downcast_ref::<StructArray>()
            .ok_or_else(|| DataFusionError::Execution("Expected state struct".into()))?;
        let inner = outer
            .column(0)
            .as_any()
            .downcast_ref::<StructArray>()
            .ok_or_else(|| DataFusionError::Execution("Expected state payload struct".into()))?;
        if filter.is_some_and(|f| f.len() != outer.len()) {
            return exec_err!("Aggregate filter length does not match state array");
        }
        let mask = if outer.null_count() == 0 && filter.is_none() {
            None
        } else {
            Some(BooleanArray::from_iter((0..outer.len()).map(|i| {
                Some(outer.is_valid(i) && filter.is_none_or(|f| f.is_valid(i) && f.value(i)))
            })))
        };
        let columns = inner.columns();
        let requires_present =
            |index: usize| self.family.moments() || !self.fields[index].is_nullable();
        if inner.null_count() != 0
            || columns
                .iter()
                .enumerate()
                .any(|(i, a)| requires_present(i) && a.null_count() != 0)
        {
            for row in 0..outer.len() {
                if !mask.as_ref().is_none_or(|m| m.value(row)) {
                    continue;
                }
                if inner.is_null(row)
                    || columns
                        .iter()
                        .enumerate()
                        .any(|(i, a)| requires_present(i) && a.is_null(row))
                {
                    return exec_err!("Aggregate state has a missing required payload value");
                }
            }
        }
        if self.family == Family::Avg
            && (columns[0].null_count() != 0 || columns[1].null_count() != 0)
        {
            let counts = columns[0]
                .as_any()
                .downcast_ref::<UInt64Array>()
                .expect("checked average layout");
            for row in 0..outer.len() {
                if !mask.as_ref().is_none_or(|m| m.value(row)) {
                    continue;
                }
                // Native grouped AVG assumes a valid sum has a valid count.
                // A nullable count is legitimate only for an empty state.
                if counts.is_null(row) && columns[1].is_valid(row)
                    || counts.is_valid(row) && counts.value(row) > 0 && columns[1].is_null(row)
                {
                    return exec_err!("Average state count and sum validity are inconsistent");
                }
            }
        }
        Ok((columns.to_vec(), mask))
    }
}

fn external(error: serde_json::Error) -> DataFusionError {
    DataFusionError::External(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Float64Array, UInt64Array};

    #[test]
    fn struct_pack_and_unpack_retain_child_buffers() -> Result<()> {
        let state = StateType::new(Family::Avg, &[DataType::Float64])?;
        let columns: Vec<ArrayRef> = vec![
            Arc::new(UInt64Array::from(vec![2, 3])),
            Arc::new(Float64Array::from(vec![10., 30.])),
        ];
        let packed = state.pack(columns.clone())?;
        let (unpacked, mask) = state.unpack(&[packed], None)?;
        assert!(mask.is_none());
        for (before, after) in columns.iter().zip(unpacked) {
            assert!(Arc::ptr_eq(before, &after));
        }
        Ok(())
    }
}
