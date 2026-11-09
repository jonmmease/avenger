//! The constructor functions: a scalar function for each message of the option tree with fields,
//! which returns the fragment of its message, built from its arguments.

use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, Field, FieldRef},
    common::{exec_err, plan_err, Result, ScalarValue},
    logical_expr::{
        ColumnarValue, DocSection, Documentation, DocumentationBuilder, ReturnFieldArgs,
        ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
    },
};
use prost::Message;

use super::argument::Arg;
use crate::proto;

/// The extension name of a column of fragments.
pub const FRAGMENT_EXTENSION_NAME: &str = "avenger.chart.fragment";

/// A constructor: its name, the full name of its message, its documentation, its parameters, and
/// the function that builds its fragment from its arguments, one for each parameter.
pub(crate) struct ConstructorSpec {
    pub(crate) name: &'static str,
    pub(crate) message: &'static str,
    pub(crate) doc: &'static str,
    pub(crate) params: &'static [ParamSpec],
    pub(crate) build: fn(&[Option<Arg>]) -> std::result::Result<proto::Fragment, String>,
}

/// A parameter of a message: a field outside a oneof, or a oneof.
pub(crate) struct ParamSpec {
    pub(crate) name: &'static str,
    pub(crate) doc: &'static str,
    pub(crate) required: bool,
}

/// The generated code: the table of the constructors, and the parameters, the application of
/// arguments, and the conversions of each message of the option tree and of `Encoding`.
#[allow(dead_code, unreachable_patterns, clippy::all)]
pub(crate) mod generated {
    use super::{super::argument::Class, Arg, ConstructorSpec, ParamSpec};
    include!(concat!(env!("OUT_DIR"), "/constructors.rs"));
}

/// The documentation section of the constructors.
const SECTION: DocSection = DocSection {
    include: true,
    label: "Chart Constructors",
    description: Some("Functions that build the options of avenger charts."),
};

/// The field of a fragment of the message `message`.
pub(crate) fn fragment_field(name: &str, message: &str) -> Field {
    Field::new(name, DataType::BinaryView, true).with_metadata(HashMap::from([
        (
            "ARROW:extension:name".to_string(),
            FRAGMENT_EXTENSION_NAME.to_string(),
        ),
        ("ARROW:extension:metadata".to_string(), message.to_string()),
    ]))
}

/// The scalar function of a constructor.
#[derive(Debug)]
pub(crate) struct ConstructorFunction {
    spec: &'static ConstructorSpec,
    signature: Signature,
    documentation: Documentation,
}

impl ConstructorFunction {
    pub(crate) fn new(spec: &'static ConstructorSpec) -> Self {
        let names: Vec<&str> = spec.params.iter().map(|param| param.name).collect();
        let signature = Signature::user_defined(Volatility::Immutable)
            .with_parameter_names(names.clone())
            .expect("the parameter names of a message are distinct");
        let syntax: Vec<String> = (names.iter())
            .map(|name| format!("{name} => value"))
            .collect();
        let mut documentation = DocumentationBuilder::new_with_details(
            SECTION,
            spec.doc,
            format!("{}({})", spec.name, syntax.join(", ")),
        );
        for param in spec.params {
            documentation = documentation.with_argument(param.name, param.doc);
        }
        Self {
            spec,
            signature,
            documentation: documentation.build(),
        }
    }

    /// The fragment of the arguments `values`, one for each parameter, or the error of the call.
    fn build(&self, values: Vec<Option<ScalarValue>>) -> std::result::Result<Vec<u8>, String> {
        let mut args: Vec<Option<Arg>> = values.into_iter().map(|v| v.and_then(Arg::new)).collect();
        args.resize_with(self.spec.params.len(), || None);
        let fragment = (self.spec.build)(&args)?;
        Ok(fragment.encode_to_vec())
    }
}

impl std::fmt::Debug for ConstructorSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConstructorSpec")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ConstructorFunction {
    fn eq(&self, other: &Self) -> bool {
        self.spec.name == other.spec.name
    }
}

impl Eq for ConstructorFunction {}

impl std::hash::Hash for ConstructorFunction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.spec.name.hash(state);
    }
}

impl ScalarUDFImpl for ConstructorFunction {
    fn name(&self) -> &str {
        self.spec.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, arg_types: &[DataType]) -> Result<Vec<DataType>> {
        let (name, count) = (self.spec.name, self.spec.params.len());
        if arg_types.len() > count {
            return plan_err!(
                "{name}() takes {count} arguments at most, not {}",
                arg_types.len()
            );
        }
        Ok(arg_types.to_vec())
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::BinaryView)
    }

    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        // A call of constants builds at planning, so its errors are planning errors.
        let constants: Option<Vec<Option<ScalarValue>>> = (args.scalar_arguments.iter())
            .map(|value| value.map(|value| Some(value.clone())))
            .collect();
        if let Some(values) = constants {
            if let Err(error) = self.build(values) {
                return plan_err!("{}(): {error}", self.spec.name);
            }
        }
        Ok(Arc::new(fragment_field(self.spec.name, self.spec.message)))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let mut values = Vec::with_capacity(args.args.len());
        for arg in args.args {
            match arg {
                ColumnarValue::Scalar(value) => values.push(Some(value)),
                ColumnarValue::Array(_) => {
                    return exec_err!("{}() takes constants, not columns", self.spec.name)
                }
            }
        }
        match self.build(values) {
            Ok(bytes) => Ok(ColumnarValue::Scalar(ScalarValue::BinaryView(Some(bytes)))),
            Err(error) => exec_err!("{}(): {error}", self.spec.name),
        }
    }

    fn documentation(&self) -> Option<&Documentation> {
        Some(&self.documentation)
    }
}
