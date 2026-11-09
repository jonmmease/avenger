//! `scaled(value, <options>)` and `direct(value)`: a channel value of the kind that each names.

use datafusion::{
    arrow::datatypes::{DataType, FieldRef},
    common::Result,
    logical_expr::{
        ColumnarValue, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
    },
};

/// The name of `scaled`, whose argument a mark draws through the channel's scale: a
/// `scaled_field` or a `scaled_value`.
pub const SCALED: &str = "scaled";

/// The name of `direct`, whose argument a mark draws as it is: a `direct_field` or a
/// `direct_value`.
pub const DIRECT: &str = "direct";

/// The kind of a channel value that a call names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Scaled,
    Direct,
}

impl Kind {
    /// The kind that the function `name` gives.
    pub(crate) fn of(name: &str) -> Option<Self> {
        match name {
            SCALED => Some(Self::Scaled),
            DIRECT => Some(Self::Direct),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Scaled => SCALED,
            Self::Direct => DIRECT,
        }
    }
}

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
                let options = super::constructor::generated::PARAMS_ENCODING.iter();
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
