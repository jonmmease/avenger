//! `scaled(value, <options>)` and `direct(value)`: a channel value of the kind that each names.

use datafusion::{
    arrow::datatypes::{DataType, FieldRef},
    common::Result,
    logical_expr::{
        ColumnarValue, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
    },
};

use crate::functions::{encoding_options, Kind};

/// `scaled(value)` or `direct(value)`, whose result is `value`. A mark function reads a channel
/// argument that calls one of them as an encoding of its kind.
///
/// It is volatile, so the simplifier does not fold a call of a constant into the constant before
/// a mark function binds its call.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct KindFunction {
    kind: Kind,
    signature: Signature,
}

impl KindFunction {
    pub(crate) fn new(kind: Kind) -> Self {
        let signature = match kind {
            // `scaled` takes the options of `Encoding` after its value.
            Kind::Scaled => {
                let options = encoding_options().iter();
                let names: Vec<&str> = std::iter::once("value")
                    .chain(options.map(|p| p.name))
                    .collect();
                Signature::user_defined(Volatility::Volatile)
                    .with_parameter_names(names)
                    .expect("the options of an encoding have distinct names")
            }
            Kind::Direct => Signature::any(1, Volatility::Volatile),
        };
        Self { kind, signature }
    }
}

impl ScalarUDFImpl for KindFunction {
    fn name(&self) -> &str {
        self.kind.name()
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        Ok(arg_types[0].clone())
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(args.arg_fields[0].clone())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        Ok(args
            .args
            .into_iter()
            .next()
            .expect("a kind takes one argument"))
    }
}
