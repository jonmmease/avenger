//! The constructor functions: a scalar function for each constructor of avenger-chart, which
//! returns the fragment of its message, built from its arguments. A function has the unique name of
//! its constructor, such as `log_scale`, and gives its errors under the name that its call writes,
//! such as `log`, the name that the parameter of its argument gives it.

use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, Field, FieldRef},
    common::{exec_err, plan_err, Result, ScalarValue},
    logical_expr::{
        simplify::{ExprSimplifyResult, SimplifyContext},
        ColumnarValue, DocSection, Documentation, DocumentationBuilder, Expr, ReturnFieldArgs,
        ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
    },
};
use prost::Message;

use avenger_chart::functions::{Constructor, FRAGMENT_EXTENSION_NAME};

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

/// The scalar function of a constructor, with the name that its call writes.
#[derive(Debug)]
pub(crate) struct ConstructorFunction {
    spec: &'static Constructor,
    /// The name that the call writes, which its errors give.
    written: String,
    signature: Signature,
    documentation: Documentation,
}

impl ConstructorFunction {
    pub(crate) fn new(spec: &'static Constructor, written: &str) -> Self {
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
            format!("{written}({})", syntax.join(", ")),
        );
        for param in spec.params {
            documentation = documentation.with_argument(param.name, param.doc);
        }
        Self {
            spec,
            written: written.to_string(),
            signature,
            documentation: documentation.build(),
        }
    }

    /// The fragment of the arguments `values`, one for each parameter, or the error of the call.
    fn build(&self, values: Vec<Option<ScalarValue>>) -> std::result::Result<Vec<u8>, String> {
        let args = (values.iter())
            .map(|value| match value {
                Some(value) => super::arg(value).map_err(|error| error.to_string()),
                None => Ok(None),
            })
            .collect::<std::result::Result<Vec<_>, String>>()?;
        let fragment = self.spec.build(&args).map_err(|error| error.to_string())?;
        Ok(fragment.encode_to_vec())
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
        let (name, count) = (&self.written, self.spec.params.len());
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
                return plan_err!("{}(): {error}", self.written);
            }
        }
        let message = match self.spec.oneof {
            Some(oneof) => format!("{}.{oneof}", self.spec.message),
            None => self.spec.message.to_string(),
        };
        Ok(Arc::new(fragment_field(self.spec.name, &message)))
    }

    fn simplify(&self, args: Vec<Expr>, _info: &SimplifyContext) -> Result<ExprSimplifyResult> {
        // A call whose arguments became constants as the plan folded, such as a call of another
        // constructor, fails here with its own error. The constant evaluator, which folds the
        // call, keeps a call whose evaluation fails, and the error would be lost.
        let constants: Option<Vec<Option<ScalarValue>>> = (args.iter())
            .map(|arg| match arg {
                Expr::Literal(value, _) => Some(Some(value.clone())),
                _ => None,
            })
            .collect();
        if let Some(values) = constants {
            if let Err(error) = self.build(values) {
                return plan_err!("{}(): {error}", self.written);
            }
        }
        Ok(ExprSimplifyResult::Original(args))
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let mut values = Vec::with_capacity(args.args.len());
        for arg in args.args {
            match arg {
                ColumnarValue::Scalar(value) => values.push(Some(value)),
                ColumnarValue::Array(_) => {
                    return exec_err!("{}() takes constants, not columns", self.written)
                }
            }
        }
        match self.build(values) {
            Ok(bytes) => Ok(ColumnarValue::Scalar(ScalarValue::BinaryView(Some(bytes)))),
            Err(error) => exec_err!("{}(): {error}", self.written),
        }
    }

    fn documentation(&self) -> Option<&Documentation> {
        Some(&self.documentation)
    }
}
