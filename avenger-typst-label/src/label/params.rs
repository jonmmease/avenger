//! A label's parameters: the values its source can refer to by name.

use std::hash::{Hash, Hasher};

use indexmap::{IndexMap, IndexSet};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::error::{LabelError, source_error};
use crate::typst_eval::parse_label;
use crate::typst_library::Library;
use crate::typst_library::foundations::{Binding, Datetime, Scope, Scopes, Str, Value};
use typst_syntax::{SyntaxKind, SyntaxNode};

/// The values a label's source can refer to by name, as `#name` in markup and code, or `name`
/// in math. Parameters shadow the label library's definitions.
pub type LabelParams = IndexMap<String, LabelParamValue>;

/// A parameter's value.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LabelParamValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Date(chrono::NaiveDate),
    /// A date and time without a timezone.
    NaiveDateTime(chrono::NaiveDateTime),
    /// An instant, which formatters show in their timezone.
    ZonedDateTime(chrono::DateTime<chrono::Utc>),
    Array(Vec<LabelParamValue>),
    Dict(IndexMap<String, LabelParamValue>),
}

impl Hash for LabelParamValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::None => {}
            Self::Bool(value) => value.hash(state),
            Self::Int(value) => value.hash(state),
            Self::Float(value) => value.to_bits().hash(state),
            Self::Str(value) => value.hash(state),
            Self::Date(value) => value.hash(state),
            Self::NaiveDateTime(value) => value.hash(state),
            Self::ZonedDateTime(value) => value.hash(state),
            Self::Array(values) => values.hash(state),
            Self::Dict(values) => {
                values.len().hash(state);
                for (key, value) in values {
                    key.hash(state);
                    value.hash(state);
                }
            }
        }
    }
}

impl LabelParamValue {
    /// The parameter as a value of the evaluator.
    fn to_value(&self) -> Value {
        match self {
            Self::None => Value::None,
            Self::Bool(value) => Value::Bool(*value),
            Self::Int(value) => Value::Int(*value),
            Self::Float(value) => Value::Float(*value),
            Self::Str(value) => Value::Str(value.as_str().into()),
            Self::Date(value) => Value::Datetime(Datetime::Date(*value)),
            Self::NaiveDateTime(value) => Value::Datetime(Datetime::Naive(*value)),
            Self::ZonedDateTime(value) => Value::Datetime(Datetime::Utc(*value)),
            Self::Array(values) => {
                Value::Array(values.iter().map(Self::to_value).collect())
            }
            Self::Dict(values) => Value::Dict(
                values
                    .iter()
                    .map(|(name, value)| (Str::from(name.as_str()), value.to_value()))
                    .collect(),
            ),
        }
    }
}

/// The scope that binds a label's parameters.
pub(crate) fn scope(params: &LabelParams) -> Scope {
    let mut scope = Scope::new();
    for (name, value) in params {
        scope.bind(name.as_str().into(), Binding::detached(value.to_value()));
    }
    scope
}

/// The names a label's source refers to that the label library doesn't define, in source
/// order: the parameters it needs.
pub fn referenced_params(source: &str) -> Result<Vec<String>, LabelError> {
    let root = parse_label(source);
    let (errors, _) = root.errors_and_warnings();
    if let Some(error) = errors.into_iter().next() {
        return Err(source_error(source, &error.into()));
    }
    let scopes = Scopes::new(Some(Library::get()));
    let mut names = IndexSet::new();
    collect(&root, &scopes, &mut names);
    Ok(names.into_iter().collect())
}

/// Collects the names a node refers to that the scopes don't define. Names of fields and named
/// arguments aren't references. The walk keeps its own stack, so that deep sources can't
/// overflow the thread's.
fn collect(root: &SyntaxNode, scopes: &Scopes, names: &mut IndexSet<String>) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let free = match node.kind() {
            SyntaxKind::Ident => scopes.get(node.leaf_text()).is_err(),
            SyntaxKind::MathIdent => scopes.get_in_math(node.leaf_text()).is_err(),
            _ => false,
        };
        if free {
            names.insert(node.leaf_text().to_string());
        }
        let children = node.children().as_slice();
        let children = match node.kind() {
            // The target of a field access, not the field.
            SyntaxKind::FieldAccess | SyntaxKind::MathFieldAccess => {
                children.get(..1).unwrap_or_default()
            }
            // The value of a named argument, not its name.
            SyntaxKind::Named => children.get(1..).unwrap_or_default(),
            _ => children,
        };
        // In source order.
        stack.extend(children.iter().rev());
    }
}
