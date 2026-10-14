use arrow::array::{new_empty_array, Array, ArrayRef};
use arrow::compute::cast;
use arrow::datatypes::DataType;
use datafusion::common::{exec_err, plan_err, Result, ScalarValue};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
};

use super::expr::options_from_struct;
use super::{external, PlannedScale};
use crate::scales::{ConfiguredScale, DomainKind, ScaleConfig, ScaleContext, ScaleImpl};

/// A scale applied to values, with its configuration from expressions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ScaleUdf {
    scale: PlannedScale,
    name: String,
    signature: Signature,
}

impl ScaleUdf {
    pub(crate) fn new(scale: &dyn ScaleImpl) -> Result<Self> {
        Self::from_planned(PlannedScale::new(scale)?)
    }

    pub(crate) fn from_planned(scale: PlannedScale) -> Result<Self> {
        Ok(Self {
            name: function_name(scale.scale().as_ref()),
            scale,
            signature: Signature::user_defined(Volatility::Immutable),
        })
    }

    pub(crate) fn planned_scale(&self) -> &PlannedScale {
        &self.scale
    }
}

/// The name of the function that applies `scale`.
pub(crate) fn function_name(scale: &dyn ScaleImpl) -> String {
    format!("{}_scale", scale.scale_type())
}

fn list_type<'a>(data_type: &'a DataType, name: &str) -> Result<&'a DataType> {
    match data_type {
        DataType::List(field) => Ok(field.data_type()),
        _ => plan_err!("a scale's {name} is a list, not {data_type}"),
    }
}

/// The type to which a scale with `domain_kind` and a domain of `domain_type` casts its values.
pub(crate) fn value_type(
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
        let [domain, _, options, values] = types else {
            return plan_err!("{} takes a domain, range, options, and values", self.name);
        };
        let domain = list_type(domain, "domain")?;
        if !matches!(options, DataType::Struct(_)) {
            return plan_err!("a scale's options are a struct, not {options}");
        }
        self.return_type(types)?;
        let values = value_type(self.scale.scale().domain_kind(), domain, values)?;
        Ok(vec![
            types[0].clone(),
            types[1].clone(),
            options.clone(),
            values,
        ])
    }

    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        let range = list_type(&types[1], "range")?;
        self.scale.scale().output_type(range).map_err(external)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let types: Vec<_> = args.args.iter().map(ColumnarValue::data_type).collect();
        let coerced = self.coerce_types(&types)?;
        let output_type = self.return_type(&types)?;
        let all_scalar = args
            .args
            .iter()
            .all(|value| matches!(value, ColumnarValue::Scalar(_)));
        let rows = if all_scalar { 1 } else { args.number_rows };
        if !all_scalar && rows == 0 {
            return Ok(ColumnarValue::Array(new_empty_array(&output_type)));
        }
        let domain = list_value(uniform_scalar(&args.args[0], "domain")?, "domain")?;
        let range = list_value(uniform_scalar(&args.args[1], "range")?, "range")?;
        let ScalarValue::Struct(options) = uniform_scalar(&args.args[2], "options")? else {
            return exec_err!("a scale's options are a struct");
        };
        let scale = ConfiguredScale {
            scale_impl: self.scale.scale().clone(),
            config: ScaleConfig {
                domain,
                range,
                options: options_from_struct(&options)?,
                context: ScaleContext::default(),
            },
        };
        let values = cast(&args.args[3].clone().into_array(rows)?, &coerced[3])?;
        let output = scale.scale(&values).map_err(external)?;
        if output.data_type() != &output_type || output.len() != rows {
            return exec_err!(
                "{} returned {} values of {}, not {rows} values of {output_type}",
                self.name,
                output.len(),
                output.data_type()
            );
        }
        if all_scalar {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &output, 0,
            )?))
        } else {
            Ok(ColumnarValue::Array(output))
        }
    }
}

/// The one value of a scale argument in a batch.
fn uniform_scalar(value: &ColumnarValue, name: &str) -> Result<ScalarValue> {
    match value {
        ColumnarValue::Scalar(value) => Ok(value.clone()),
        ColumnarValue::Array(array) => {
            let first = array.slice(0, 1).to_data();
            if (1..array.len()).any(|index| array.slice(index, 1).to_data() != first) {
                return exec_err!("a scale's {name} has one value in a batch");
            }
            ScalarValue::try_from_array(array, 0)
        }
    }
}

fn list_value(value: ScalarValue, name: &str) -> Result<ArrayRef> {
    match value {
        ScalarValue::List(list) if list.len() == 1 && !list.is_null(0) => Ok(list.value(0)),
        _ => exec_err!("a scale's {name} is a non-null list"),
    }
}
