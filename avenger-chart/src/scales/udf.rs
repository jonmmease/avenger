use crate::error::AvengerChartError;
use datafusion::{
    arrow::{array::Array, datatypes::DataType},
    error::DataFusionError,
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, TypeSignature,
        Volatility,
    },
};
use serde::{Deserialize, Serialize};
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

/// Metadata for serializing ScaleUDF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleUDFMetadata {
    /// Serializable scale specification
    pub scale: crate::scales::Scale<crate::scales::spec::Auto>,
    /// Base64-encoded Arrow IPC for domain type
    pub domain_type_ipc: String,
    /// Base64-encoded Arrow IPC for range type
    pub range_type_ipc: String,
    /// Base64-encoded Arrow IPC for options type
    pub options_type_ipc: String,
}

/// DataFusion UDF that applies scale transformation with dynamic domain/range/options
#[derive(Debug, Clone)]
pub struct ScaleUDF {
    signature: Signature,
    pub(crate) scale_impl: Arc<dyn avenger_scales::scales::ScaleImpl>,
    pub(crate) range_type: DataType,
    pub(crate) scale: crate::scales::Scale<crate::scales::spec::Auto>,
}

impl ScaleUDF {
    pub fn new(
        scale: crate::scales::Scale<crate::scales::spec::Auto>,
        domain_type: DataType,
        range_type: DataType,
        options_type: DataType,
    ) -> Result<Self, AvengerChartError> {
        // Get the scale implementation from the Scale<Auto>
        let scale_impl = scale.to_scale_impl()?;

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
            scale,
        })
    }

    /// Extract metadata for serialization
    pub fn metadata(&self) -> ScaleUDFMetadata {
        use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
        use datafusion::arrow::array::ArrayRef;
        use datafusion::arrow::datatypes::{Field, Schema};
        use datafusion::arrow::ipc::writer::StreamWriter;
        use datafusion_common::ScalarValue;

        // Get the types from the signature
        let sig_types = match &self.signature.type_signature {
            TypeSignature::Exact(types) => types,
            _ => panic!("Unexpected signature type for ScaleUDF"),
        };

        // Extract domain type from list type
        let domain_type = match &sig_types[0] {
            DataType::List(field) => field.data_type().clone(),
            DataType::LargeList(field) => field.data_type().clone(),
            _ => panic!("Unexpected domain type"),
        };

        // Extract options type
        let options_type = sig_types[2].clone();

        // Helper to serialize DataType as Arrow IPC
        let serialize_type = |dt: &DataType| -> String {
            // Create a null ScalarValue with the type
            let scalar = ScalarValue::try_from(dt).unwrap();
            let array: ArrayRef = scalar.to_array().unwrap();

            // Create schema and write to IPC
            let schema = Schema::new(vec![Field::new("type", dt.clone(), true)]);
            let mut buf = Vec::new();
            {
                let mut writer = StreamWriter::try_new(&mut buf, &schema).unwrap();
                writer
                    .write(
                        &datafusion::arrow::record_batch::RecordBatch::try_new(
                            Arc::new(schema.clone()),
                            vec![array],
                        )
                        .unwrap(),
                    )
                    .unwrap();
                writer.finish().unwrap();
            }
            BASE64.encode(&buf)
        };

        ScaleUDFMetadata {
            scale: self.scale.clone(),
            domain_type_ipc: serialize_type(&domain_type),
            range_type_ipc: serialize_type(&self.range_type),
            options_type_ipc: serialize_type(&options_type),
        }
    }

    /// Create from metadata
    pub fn from_metadata(metadata: ScaleUDFMetadata) -> Result<Self, AvengerChartError> {
        use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
        use datafusion::arrow::ipc::reader::StreamReader;
        use std::io::Cursor;

        // Helper to deserialize DataType from Arrow IPC
        let deserialize_type = |ipc_base64: &str| -> Result<DataType, AvengerChartError> {
            let bytes = BASE64.decode(ipc_base64).map_err(|e| {
                AvengerChartError::InternalError(format!("Failed to decode base64: {}", e))
            })?;

            let cursor = Cursor::new(bytes);
            let reader = StreamReader::try_new(cursor, None).map_err(|e| {
                AvengerChartError::InternalError(format!("Failed to create IPC reader: {}", e))
            })?;

            // Read the schema to get the data type
            let schema = reader.schema();
            if schema.fields().len() != 1 {
                return Err(AvengerChartError::InternalError(
                    "Expected single field in IPC schema".to_string(),
                ));
            }

            Ok(schema.field(0).data_type().clone())
        };

        // Deserialize types
        let domain_type = deserialize_type(&metadata.domain_type_ipc)?;
        let range_type = deserialize_type(&metadata.range_type_ipc)?;
        let options_type = deserialize_type(&metadata.options_type_ipc)?;

        // Get scale implementation from the Scale<Auto>
        let scale_impl = metadata.scale.to_scale_impl()?;

        Ok(Self {
            signature: Signature::new(
                TypeSignature::Exact(vec![
                    DataType::new_list(domain_type.clone(), true), // Domain array
                    DataType::new_list(range_type.clone(), true),  // Range array
                    options_type,                                  // Options struct
                    domain_type,                                   // Values to scale
                ]),
                Volatility::Immutable,
            ),
            scale_impl,
            range_type,
            scale: metadata.scale,
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
        use avenger_scales::scales::RangeKind;

        // All scales with discrete ranges return dictionary arrays for efficiency
        if self.scale_impl.range_kind() == RangeKind::Discrete {
            // Discrete-range scales return dictionary arrays
            Ok(DataType::Dictionary(
                Box::new(DataType::Int16),
                Box::new(self.range_type.clone()),
            ))
        } else {
            Ok(self.range_type.clone())
        }
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

/// Create a scale UDF from scale and types
pub fn create_scale_udf(
    scale: crate::scales::Scale<crate::scales::spec::Auto>,
    domain_type: DataType,
    range_type: DataType,
    options_type: DataType,
) -> Result<ScalarUDF, AvengerChartError> {
    let scale_udf = ScaleUDF::new(scale, domain_type, range_type, options_type)?;
    Ok(ScalarUDF::new_from_impl(scale_udf))
}
