use crate::error::AvengerChartError;
use datafusion::{
    arrow::{array::Array, datatypes::DataType},
    error::DataFusionError,
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, TypeSignature,
        Volatility,
    },
};
use std::sync::Arc;

/// Convert DataFusion ScalarValue to avenger_scales Scalar
fn scalar_value_to_avenger_scalar(
    value: &datafusion_common::ScalarValue,
) -> Option<avenger_scales::scalar::Scalar> {
    use avenger_scales::scalar::Scalar;
    use datafusion_common::ScalarValue;

    match value {
        ScalarValue::Float64(Some(v)) => Some(Scalar::from_f32(*v as f32)),
        ScalarValue::Float32(Some(v)) => Some(Scalar::from_f32(*v)),
        ScalarValue::Int64(Some(v)) => Some(Scalar::from_i32(*v as i32)),
        ScalarValue::Int32(Some(v)) => Some(Scalar::from_i32(*v)),
        ScalarValue::Boolean(Some(v)) => Some(Scalar::from_bool(*v)),
        ScalarValue::Utf8(Some(v)) => Some(Scalar::from_string(v)),
        _ => None,
    }
}

/// DataFusion UDF that applies scale transformation with dynamic domain/range/options
#[derive(Debug, Clone)]
pub struct ScaleUDF {
    signature: Signature,
    scale_impl: Arc<dyn avenger_scales::scales::ScaleImpl>,
    range_type: DataType,
}

impl ScaleUDF {
    pub fn new(
        scale_impl: Arc<dyn avenger_scales::scales::ScaleImpl>,
        domain_type: DataType,
        range_type: DataType,
        options_type: DataType,
    ) -> Result<Self, AvengerChartError> {
        let signature = Signature::new(
            TypeSignature::Exact(vec![
                DataType::new_list(domain_type.clone(), true), // Domain array
                DataType::new_list(range_type.clone(), true),  // Range array
                options_type,                                  // Options struct
                domain_type,                                   // Values to scale
            ]),
            Volatility::Immutable,
        );

        Ok(Self {
            signature,
            scale_impl,
            range_type,
        })
    }
}

impl ScalarUDFImpl for ScaleUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        "scale"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(self.range_type.clone())
    }

    fn invoke_with_args(
        &self,
        args: ScalarFunctionArgs,
    ) -> datafusion::error::Result<ColumnarValue> {
        use avenger_scales::scales::{ConfiguredScale, ScaleConfig, ScaleContext};
        use datafusion::arrow::array::AsArray;
        use datafusion_common::ScalarValue;
        use std::collections::HashMap;

        // Extract domain array from first argument
        let domain = match &args.args[0] {
            ColumnarValue::Scalar(ScalarValue::List(domain_arg)) => domain_arg.value(0),
            ColumnarValue::Array(array) => {
                let list_array = array.as_list_opt::<i32>().ok_or_else(|| {
                    DataFusionError::Execution(format!("Expected domain array, got {:?}", array))
                })?;
                if list_array.is_empty() {
                    return Ok(ColumnarValue::Array(
                        ScalarValue::try_from(&self.range_type)?.to_array_of_size(0)?,
                    ));
                }
                list_array.value(0)
            }
            _ => {
                return Err(DataFusionError::Execution(format!(
                    "Unexpected domain value: {:?}",
                    args.args[0]
                )));
            }
        };

        // Extract range array from second argument
        let ColumnarValue::Scalar(ScalarValue::List(range_arg)) = &args.args[1] else {
            return Err(DataFusionError::Execution(format!(
                "Expected range scalar, got {:?}",
                args.args[1]
            )));
        };
        let range = range_arg.value(0);

        // Extract options struct from third argument
        let ColumnarValue::Scalar(ScalarValue::Struct(options_arg)) = &args.args[2] else {
            return Err(DataFusionError::Execution(format!(
                "Expected options struct, got {:?}",
                args.args[2]
            )));
        };

        // Convert options to HashMap<String, Scalar>
        let mut options = HashMap::new();
        if !options_arg.fields().is_empty() && options_arg.len() > 0 {
            for (i, field_name) in options_arg.fields().iter().enumerate() {
                let field_value = ScalarValue::try_from_array(options_arg.column(i), 0)?;
                // Convert DataFusion ScalarValue to avenger_scales Scalar
                if let Some(scalar) = scalar_value_to_avenger_scalar(&field_value) {
                    options.insert(field_name.name().clone(), scalar);
                }
            }
        }

        // Create configured scale
        let config = ScaleConfig {
            domain,
            range,
            options,
            context: ScaleContext::default(),
        };

        let scale = ConfiguredScale {
            scale_impl: self.scale_impl.clone(),
            config,
        };

        // Apply scale to values (fourth argument)
        let scaled = match &args.args[3] {
            ColumnarValue::Array(values) => {
                let scaled_array = scale
                    .scale(values)
                    .map_err(|e| DataFusionError::Execution(e.to_string()))?;
                ColumnarValue::Array(scaled_array)
            }
            ColumnarValue::Scalar(value) => {
                // Convert scalar to array, scale it, then convert back
                let array = value.to_array()?;
                let scaled_array = scale
                    .scale(&array)
                    .map_err(|e| DataFusionError::Execution(e.to_string()))?;

                // Convert back to scalar
                let scalar_value = ScalarValue::try_from_array(&scaled_array, 0)?;
                ColumnarValue::Scalar(scalar_value)
            }
        };

        Ok(scaled)
    }
}

/// Create a scale UDF from scale implementation and types
pub fn create_scale_udf(
    scale_impl: Arc<dyn avenger_scales::scales::ScaleImpl>,
    domain_type: DataType,
    range_type: DataType,
    options_type: DataType,
) -> Result<ScalarUDF, AvengerChartError> {
    let scale_udf = ScaleUDF::new(scale_impl, domain_type, range_type, options_type)?;
    Ok(ScalarUDF::new_from_impl(scale_udf))
}
