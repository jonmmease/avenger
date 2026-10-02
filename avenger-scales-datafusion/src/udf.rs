use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_scales::{
    scalar::Scalar,
    scales::{ConfiguredScale, ScaleConfig, ScaleContext, ScaleImpl},
};
use datafusion::{
    arrow::{
        array::{new_empty_array, Array, ArrayRef},
        compute::cast,
        datatypes::DataType,
    },
    common::{exec_err, plan_err, DataFusionError, Result, ScalarValue},
    logical_expr::{
        ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use crate::ScaleSpec;

pub const SCALE_FUNCTION_NAME: &str = "scale";
/// Version for the dataflow semantic manifest key `scalar:scale`.
pub const SCALE_FUNCTION_VERSION: &str = "avenger-scales-datafusion/2";

/// A scale kernel whose configuration is supplied by expressions.
#[derive(Clone, Debug)]
pub struct ScaleUDF {
    spec: Arc<dyn ScaleSpec>,
    implementation: Arc<dyn ScaleImpl>,
    signature: Signature,
    identity: Vec<u8>,
}

impl ScaleUDF {
    pub fn new(spec: Arc<dyn ScaleSpec>) -> Result<Self> {
        let mut identity = serde_json::to_value(&spec).map_err(external)?;
        identity.sort_all_objects();
        let identity = serde_json::to_vec(&identity).map_err(external)?;
        // Execute the same descriptor snapshot that the codec reconstructs.
        // This also rejects descriptors that cannot round trip through JSON.
        let spec: Box<dyn ScaleSpec> = serde_json::from_slice(&identity).map_err(external)?;
        let spec: Arc<dyn ScaleSpec> = Arc::from(spec);
        let implementation = spec.create_impl()?;
        Ok(Self {
            spec,
            implementation,
            signature: Signature::user_defined(Volatility::Immutable),
            identity,
        })
    }

    pub fn spec(&self) -> &dyn ScaleSpec {
        self.spec.as_ref()
    }

    pub(crate) fn payload(&self) -> &[u8] {
        &self.identity
    }
}

fn external(error: impl std::error::Error + Send + Sync + 'static) -> DataFusionError {
    DataFusionError::External(Box::new(error))
}

impl PartialEq for ScaleUDF {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}
impl Eq for ScaleUDF {}
impl Hash for ScaleUDF {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
    }
}

fn list_type<'a>(data_type: &'a DataType, name: &str) -> Result<&'a DataType> {
    match data_type {
        DataType::List(field) => Ok(field.data_type()),
        _ => plan_err!("Scale {name} must be a List, got {data_type}"),
    }
}

impl ScalarUDFImpl for ScaleUDF {
    fn name(&self) -> &str {
        SCALE_FUNCTION_NAME
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        if types.len() != 4 {
            return plan_err!("scale expects domain, range, options, and values");
        }
        let domain = list_type(&types[0], "domain")?;
        let range = list_type(&types[1], "range")?;
        if !matches!(types[2], DataType::Struct(_)) {
            return plan_err!("Scale options must be a Struct, got {}", types[2]);
        }
        self.spec.output_type(range)?;
        let mut types = types.to_vec();
        types[3] = self.spec.input_type(domain, &types[3])?;
        Ok(types)
    }

    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        self.coerce_types(types)?;
        self.spec.output_type(list_type(&types[1], "range")?)
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
        for value in &args.args {
            if let ColumnarValue::Array(array) = value {
                if array.len() != rows {
                    return exec_err!(
                        "Scale argument length {} does not match batch length {rows}",
                        array.len()
                    );
                }
            }
        }
        if !all_scalar && rows == 0 {
            return Ok(ColumnarValue::Array(new_empty_array(&output_type)));
        }

        let domain = uniform_scalar(&args.args[0], "domain")?;
        let range = uniform_scalar(&args.args[1], "range")?;
        let options = uniform_scalar(&args.args[2], "options")?;
        let domain = list_value(domain, "domain")?;
        let range = list_value(range, "range")?;
        let ScalarValue::Struct(options) = options else {
            return exec_err!("Scale options must be a non-null struct");
        };
        if options.len() != 1 || options.is_null(0) {
            return exec_err!("Scale options must be a non-null struct");
        }
        let mut effective_options = self.spec.default_options();
        let mut names = std::collections::HashSet::new();
        for (field, array) in options.fields().iter().zip(options.columns()) {
            if !names.insert(field.name()) {
                return exec_err!("Duplicate scale option {}", field.name());
            }
            // Keep Arrow values intact instead of narrowing i64/f64 values or
            // silently discarding unsupported option types.
            effective_options.insert(field.name().clone(), Scalar(array.clone()));
        }
        let config = ScaleConfig {
            domain,
            range,
            options: effective_options,
            context: ScaleContext::default(),
        };
        self.spec.validate_config(&config)?;
        let scale = ConfiguredScale {
            scale_impl: self.implementation.clone(),
            config,
        };
        let values = args.args[3].clone().into_array(rows)?;
        let values = cast(&values, &coerced[3])?;
        let output = scale.scale(&values).map_err(external)?;
        if output.data_type() != &output_type || output.len() != rows {
            return exec_err!(
                "Scale kernel returned {} rows of {}, expected {rows} rows of {output_type}",
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

fn uniform_scalar(value: &ColumnarValue, name: &str) -> Result<ScalarValue> {
    match value {
        ColumnarValue::Scalar(value) => Ok(value.clone()),
        ColumnarValue::Array(array) => {
            if array.is_empty() {
                return exec_err!("Cannot read scale {name} from an empty array");
            }
            let first = array.slice(0, 1).to_data();
            if (1..array.len()).any(|index| array.slice(index, 1).to_data() != first) {
                return exec_err!("Scale {name} must be constant within a record batch");
            }
            ScalarValue::try_from_array(array, 0)
        }
    }
}

fn list_value(value: ScalarValue, name: &str) -> Result<ArrayRef> {
    match value {
        ScalarValue::List(list) if list.len() == 1 && !list.is_null(0) => Ok(list.value(0)),
        _ => exec_err!("Scale {name} must be a non-null list"),
    }
}

pub fn create_scale_udf(spec: impl ScaleSpec + 'static) -> Result<ScalarUDF> {
    Ok(ScalarUDF::new_from_impl(ScaleUDF::new(Arc::new(spec))?))
}
