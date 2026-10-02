use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{new_empty_array, Array, ArrayRef, AsArray, Float64Array, StructArray},
        compute::cast,
        datatypes::{DataType, Field, FieldRef, Float64Type},
    },
    common::{exec_err, plan_err, Result, ScalarValue},
    logical_expr::{
        ColumnarValue, Expr, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl,
        Signature, Volatility,
    },
};

/// Semantic version used by the dataflow function manifest and transform codec.
pub const TRANSFORM_FUNCTION_VERSION: &str = "avenger-transform/1";

/// Function-version requirements for a dataflow containing transform UDFs.
pub fn function_versions() -> BTreeMap<String, String> {
    Function::ALL
        .into_iter()
        .map(|f| {
            (
                format!("scalar:{}", f.name()),
                TRANSFORM_FUNCTION_VERSION.into(),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Function {
    Truthy,
    Valid,
    Missing,
    Numeric,
    Clean,
    Extent,
    BinParameters,
    BinBounds,
}

impl Function {
    pub(crate) const ALL: [Self; 8] = [
        Self::Truthy,
        Self::Valid,
        Self::Missing,
        Self::Numeric,
        Self::Clean,
        Self::Extent,
        Self::BinParameters,
        Self::BinBounds,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Truthy => "avenger_truthy",
            Self::Valid => "avenger_valid",
            Self::Missing => "avenger_missing",
            Self::Numeric => "avenger_numeric",
            Self::Clean => "avenger_clean",
            Self::Extent => "avenger_extent",
            Self::BinParameters => "avenger_bin_parameters",
            Self::BinBounds => "avenger_bin_bounds",
        }
    }

    pub(crate) fn udf(self) -> ScalarUDF {
        ScalarUDF::new_from_impl(TransformUdf {
            function: self,
            signature: Signature::user_defined(Volatility::Immutable),
        })
    }

    pub(crate) fn call(self, args: Vec<Expr>) -> Expr {
        self.udf().call(args)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TransformUdf {
    pub(crate) function: Function,
    signature: Signature,
}

impl ScalarUDFImpl for TransformUdf {
    fn name(&self) -> &str {
        self.function.name()
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        use Function::*;
        let arity = if matches!(self.function, Extent | BinParameters | BinBounds) {
            2
        } else {
            1
        };
        if types.len() != arity {
            return plan_err!("{} expects {arity} arguments", self.name());
        }
        let mut result = types.to_vec();
        match self.function {
            BinParameters => {
                crate::bin::validate_struct(&types[0], &["min", "max"])?;
                crate::bin::validate_options(&types[1])?;
            }
            BinBounds => {
                if !crate::values::numeric(&types[0]) {
                    return plan_err!("Bin values must be numeric, received {}", types[0]);
                }
                result[0] = DataType::Float64;
                crate::bin::validate_struct(&types[1], &["start", "stop", "step", "upper_bound"])?;
            }
            Extent => {
                if !types.iter().all(crate::values::numeric) {
                    return plan_err!("Extent endpoints must be numeric");
                }
                result.fill(DataType::Float64);
            }
            f => {
                crate::values::output_type(f, &types[0])?;
                if f == Numeric {
                    result[0] = DataType::Float64;
                }
            }
        }
        Ok(result)
    }

    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        self.coerce_types(types)?;
        Ok(match self.function {
            Function::Extent => DataType::Struct(crate::bin::fields(&["min", "max"], true)),
            Function::BinParameters => crate::bin::parameter_type(),
            Function::BinBounds => DataType::Struct(crate::bin::fields(&["start", "end"], true)),
            f => crate::values::output_type(f, &types[0])?,
        })
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let types = args
            .arg_fields
            .iter()
            .map(|f| f.data_type().clone())
            .collect::<Vec<_>>();
        let nullable = matches!(
            self.function,
            Function::Clean | Function::Numeric | Function::BinParameters
        );
        Ok(Arc::new(Field::new(
            self.name(),
            self.return_type(&types)?,
            nullable,
        )))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let types = args
            .args
            .iter()
            .map(ColumnarValue::data_type)
            .collect::<Vec<_>>();
        let coerced = self.coerce_types(&types)?;
        let output_type = self.return_type(&types)?;
        let scalar = args
            .args
            .iter()
            .all(|a| matches!(a, ColumnarValue::Scalar(_)));
        let rows = if scalar { 1 } else { args.number_rows };
        for a in &args.args {
            if let ColumnarValue::Array(a) = a {
                if a.len() != rows {
                    return exec_err!(
                        "{} argument length {} does not match {rows}",
                        self.name(),
                        a.len()
                    );
                }
            }
        }
        if rows == 0 {
            return Ok(ColumnarValue::Array(new_empty_array(&output_type)));
        }
        if self.function == Function::BinParameters {
            let result = crate::bin::calculate(&args.args[0], &args.args[1])?;
            return if scalar {
                Ok(ColumnarValue::Scalar(result))
            } else {
                Ok(ColumnarValue::Array(result.to_array_of_size(rows)?))
            };
        }
        let first = cast(&args.args[0].clone().into_array(rows)?, &coerced[0])?;
        let result = match self.function {
            Function::BinBounds => crate::bin::apply(&first, &args.args[1])?,
            Function::Extent => {
                let second = cast(&args.args[1].clone().into_array(rows)?, &DataType::Float64)?;
                let pairs = first
                    .as_primitive::<Float64Type>()
                    .iter()
                    .zip(second.as_primitive::<Float64Type>().iter())
                    .map(|(a, b)| match (a, b) {
                        (Some(a), Some(b)) if a.is_finite() && b.is_finite() => (Some(a), Some(b)),
                        _ => (None, None),
                    })
                    .collect::<Vec<_>>();
                Arc::new(StructArray::new(
                    crate::bin::fields(&["min", "max"], true),
                    vec![
                        Arc::new(Float64Array::from_iter(pairs.iter().map(|p| p.0))),
                        Arc::new(Float64Array::from_iter(pairs.iter().map(|p| p.1))),
                    ],
                    None,
                )) as ArrayRef
            }
            f => crate::values::evaluate(f, &first)?,
        };
        if scalar {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &result, 0,
            )?))
        } else {
            Ok(ColumnarValue::Array(result))
        }
    }
}

pub(crate) fn uniform(value: &ColumnarValue, name: &str) -> Result<ScalarValue> {
    match value {
        ColumnarValue::Scalar(value) => Ok(value.clone()),
        ColumnarValue::Array(array) => {
            if array.is_empty() {
                return exec_err!("Cannot read bin {name} from an empty array");
            }
            let first = array.slice(0, 1).to_data();
            if (1..array.len()).any(|i| array.slice(i, 1).to_data() != first) {
                return exec_err!("Bin {name} must be constant within a record batch");
            }
            ScalarValue::try_from_array(array, 0)
        }
    }
}
