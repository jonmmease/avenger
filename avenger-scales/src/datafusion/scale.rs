use arrow::array::{new_empty_array, new_null_array, Array, ArrayRef, AsArray, UInt32Array};
use arrow::compute::{cast, interleave, take};
use arrow::datatypes::DataType;
use datafusion::common::{exec_err, plan_err, Result, ScalarValue};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
};

use super::config::{config_at, config_groups, config_types};
use super::{external, PlannedScale};
use crate::scales::{ConfiguredScale, DomainKind, ScaleConfig, ScaleImpl};

/// A scale applied to values, with its configuration from an expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ScaleUdf {
    scale: PlannedScale,
    name: String,
    signature: Signature,
}

impl ScaleUdf {
    pub(crate) fn new(scale: &dyn ScaleImpl) -> Result<Self> {
        Ok(Self::from_planned(PlannedScale::new(scale)?))
    }

    pub(crate) fn from_planned(scale: PlannedScale) -> Self {
        Self {
            name: function_name(scale.scale().as_ref()),
            scale,
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }

    pub(crate) fn planned_scale(&self) -> &PlannedScale {
        &self.scale
    }

    /// The scale with `config` applied to `values`, checked against the planned output type.
    fn apply(
        &self,
        config: ScaleConfig,
        values: &ArrayRef,
        output_type: &DataType,
    ) -> Result<ArrayRef> {
        let scale = ConfiguredScale {
            scale_impl: self.scale.scale().clone(),
            config,
        };
        let output = scale.scale(values).map_err(external)?;
        if output.data_type() != output_type || output.len() != values.len() {
            return exec_err!(
                "{} returned {} values of {}, not {} values of {output_type}",
                self.name,
                output.len(),
                output.data_type(),
                values.len()
            );
        }
        Ok(output)
    }
}

/// The name of the function that applies `scale`.
pub(crate) fn function_name(scale: &dyn ScaleImpl) -> String {
    format!("{}_scale", scale.scale_type())
}

/// The type to which a scale with `domain_kind` and a domain of `domain_type` casts its values.
fn value_type(
    domain_kind: DomainKind,
    domain_type: &DataType,
    value_type: &DataType,
) -> Result<DataType> {
    match domain_kind {
        DomainKind::Numeric => {
            if !(value_type.is_numeric() || value_type == &DataType::Null) {
                return plan_err!("a numeric scale needs numeric values, not {value_type}");
            }
            Ok(DataType::Float64)
        }
        _ => {
            if value_type != domain_type && value_type != &DataType::Null {
                return plan_err!(
                    "scale values must have the domain's type {domain_type}, not {value_type}"
                );
            }
            Ok(domain_type.clone())
        }
    }
}

impl ScalarUDFImpl for ScaleUdf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        let [config, values] = types else {
            return plan_err!("{} takes a scale configuration and values", self.name);
        };
        let (domain, _, _) = config_types(config)?;
        self.return_type(types)?;
        let values = value_type(self.scale.scale().domain_kind(), &domain, values)?;
        Ok(vec![config.clone(), values])
    }

    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        let Some(config) = types.first() else {
            return plan_err!("{} takes a scale configuration and values", self.name);
        };
        let (_, range, _) = config_types(config)?;
        self.scale.scale().output_type(&range).map_err(external)
    }

    /// Rows with equal configurations are scaled together, and rows with a null configuration
    /// have null results.
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let types: Vec<_> = args.args.iter().map(ColumnarValue::data_type).collect();
        let value_type = self.coerce_types(&types)?.remove(1);
        let output_type = self.return_type(&types)?;
        let all_scalar = args
            .args
            .iter()
            .all(|value| matches!(value, ColumnarValue::Scalar(_)));
        let rows = if all_scalar { 1 } else { args.number_rows };
        if rows == 0 {
            return Ok(ColumnarValue::Array(new_empty_array(&output_type)));
        }
        let values = cast(&args.args[1].to_array(rows)?, &value_type)?;
        let output = match &args.args[0] {
            ColumnarValue::Scalar(ScalarValue::Struct(config)) if config.is_valid(0) => {
                self.apply(config_at(config, 0)?, &values, &output_type)?
            }
            ColumnarValue::Scalar(_) => new_null_array(&output_type, rows),
            ColumnarValue::Array(configs) => {
                let Some(configs) = configs.as_struct_opt() else {
                    return exec_err!("a scale configuration is a struct");
                };
                let groups = config_groups(configs)?;
                match groups.as_slice() {
                    [group] if group.len() == rows => {
                        self.apply(config_at(configs, 0)?, &values, &output_type)?
                    }
                    _ => {
                        // Each row's result is in its group's part, or null without one.
                        let mut parts = vec![new_null_array(&output_type, 1)];
                        let mut positions = vec![(0, 0); rows];
                        for group in groups {
                            let config = config_at(configs, group[0] as usize)?;
                            let indices = UInt32Array::from(group.clone());
                            let part_values = take(&values, &indices, None)?;
                            for (index, row) in group.iter().enumerate() {
                                positions[*row as usize] = (parts.len(), index);
                            }
                            parts.push(self.apply(config, &part_values, &output_type)?);
                        }
                        let parts: Vec<&dyn Array> =
                            parts.iter().map(|part| part.as_ref()).collect();
                        interleave(&parts, &positions)?
                    }
                }
            }
        };
        if all_scalar {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &output, 0,
            )?))
        } else {
            Ok(ColumnarValue::Array(output))
        }
    }
}
