use std::sync::Arc;

use crate::{aggregate::merge_filtered, families::Family, state::StateType};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Float64Array, UInt64Array},
        compute::kernels::nullif::nullif,
        datatypes::{DataType, Field, FieldRef},
    },
    common::{Result, ScalarValue},
    logical_expr::{
        ColumnarValue, EmitTo, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDFImpl, Signature,
        Volatility,
    },
};

#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct Finalize {
    family: Family,
    name: String,
    aliases: Vec<String>,
    signature: Signature,
}
impl Finalize {
    pub fn new(family: Family) -> Self {
        let name = format!("{}Finalize", family.prefix());
        Self {
            aliases: vec![name.to_lowercase()],
            name,
            family,
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }
}
impl ScalarUDFImpl for Finalize {
    fn name(&self) -> &str {
        &self.name
    }
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        StateType::args_type(self.family, types)?;
        Ok(types.to_vec())
    }
    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        Ok(StateType::args_type(self.family, types)?
            .result
            .data_type()
            .clone())
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let types = args
            .arg_fields
            .iter()
            .map(|f| f.data_type().clone())
            .collect::<Vec<_>>();
        let state = StateType::args_type(self.family, &types)?;
        Ok(Arc::new(Field::new(
            &self.name,
            state.result.data_type().clone(),
            args.arg_fields[0].is_nullable() || state.result.is_nullable(),
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let scalar = matches!(args.args[0], ColumnarValue::Scalar(_));
        let array = args.args[0].to_array(if scalar { 1 } else { args.number_rows })?;
        let state = StateType::from_type(self.family, array.data_type())?;
        let len = array.len();
        let (columns, mask) = state.unpack(&[array], None)?;
        let mut result = if matches!(self.family, Family::Min | Family::Max) {
            // A single extrema state already contains its result. Reusing the
            // payload also avoids native grouped finite bounds changing infinity.
            Arc::clone(&columns[0])
        } else if state.groups_supported() {
            let mut groups = state.groups()?;
            merge_filtered(
                self.family,
                groups.as_mut(),
                &columns,
                &(0..len).collect::<Vec<_>>(),
                mask.as_ref(),
                len,
            )?;
            let mut result = groups.evaluate(EmitTo::All)?;
            // Native scalar population statistics finalize every singleton as zero.
            if matches!(self.family, Family::VarPop | Family::StddevPop) {
                let counts = columns[0]
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .expect("checked moment layout");
                if counts.values().contains(&1) {
                    let values = result
                        .as_any()
                        .downcast_ref::<Float64Array>()
                        .expect("native moment result");
                    result = Arc::new(Float64Array::from_iter(values.iter().enumerate().map(
                        |(i, v)| {
                            if counts.value(i) == 1 {
                                Some(0.0)
                            } else {
                                v
                            }
                        },
                    )));
                }
            }
            result
        } else {
            let mut results = Vec::with_capacity(len);
            for row in 0..len {
                let mut acc = state.accumulator()?;
                if mask.as_ref().is_none_or(|m| m.value(row)) {
                    acc.merge_batch(&columns.iter().map(|c| c.slice(row, 1)).collect::<Vec<_>>())?;
                }
                results.push(acc.evaluate()?);
            }
            if results.is_empty() {
                datafusion::arrow::array::new_empty_array(state.result.data_type())
            } else {
                ScalarValue::iter_to_array(results)?
            }
        };
        if let Some(mask) = mask {
            let absent = datafusion::arrow::compute::not(&mask)?;
            result = nullif(result.as_ref(), &absent)?;
        }
        if scalar {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &result, 0,
            )?))
        } else {
            Ok(ColumnarValue::Array(result as ArrayRef))
        }
    }
}
