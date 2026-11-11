//! The errors of the calls of the functions of the format.

use std::fmt;

use super::{argument::describe_fragment, Accept, Arg, Class, Name, Shape};
use crate::ChartError;

/// An error of a call: the path of the parameters from the function to the argument, such as
/// `x` and `scale` for an option of `scaled` of the channel `x`, and the reason. Its display is
/// the text of the SQL errors, such as `` `nice` takes a Boolean, an integer, or a
/// floating-point number, not a string ``.
#[derive(Debug)]
pub struct ArgError {
    pub path: Vec<&'static str>,
    pub reason: Reason,
}

/// The reason of an [`ArgError`].
#[derive(Debug)]
pub enum Reason {
    /// An argument that its parameter does not accept: what the parameter accepts, the class of
    /// the argument, and for a fragment the full name of its message.
    Rejected {
        expected: Expected,
        found: Class,
        fragment: Option<String>,
    },
    /// A required parameter without an argument.
    Required,
    /// More arguments than the function has parameters.
    Arity { max: usize, found: usize },
    /// An integer outside the range of its field.
    Range { value: i128 },
    /// A string that names no value of an enum.
    Value {
        values: &'static [Name],
        found: String,
    },
    /// A string that names no member of a oneof.
    Member {
        names: &'static [Name],
        found: String,
    },
    /// A null item of an array, which only a `ValueRef` stores.
    NullItem,
    /// A call of a mark function that breaks a rule of its mark.
    Mark(String),
    /// A composition of charts that breaks a rule of its composition, such as two charts of a facet
    /// with one key value.
    Composition(String),
    /// A message that the builders reject.
    Invalid(ChartError),
}

/// What a parameter accepts: the inputs of a parameter, or a value of the shape of a field.
#[derive(Clone, Copy, Debug)]
pub enum Expected {
    Inputs(&'static [Accept]),
    Shape(&'static Shape),
}

impl ArgError {
    fn new(reason: Reason) -> Self {
        Self {
            path: Vec::new(),
            reason,
        }
    }

    pub(crate) fn rejected(expected: Expected, arg: &Arg) -> Self {
        let fragment = match arg {
            Arg::Fragment(fragment) => Some(fragment.r#type.clone()),
            _ => None,
        };
        Self::new(Reason::Rejected {
            expected,
            found: arg.class(),
            fragment,
        })
    }

    pub(crate) fn required(param: &'static str) -> Self {
        Self::new(Reason::Required).within(param)
    }

    pub(crate) fn arity(max: usize, found: usize) -> Self {
        Self::new(Reason::Arity { max, found })
    }

    pub(crate) fn range(value: i128) -> Self {
        Self::new(Reason::Range { value })
    }

    pub(crate) fn value(values: &'static [Name], found: &str) -> Self {
        Self::new(Reason::Value {
            values,
            found: found.to_string(),
        })
    }

    pub(crate) fn member(names: &'static [Name], found: &str) -> Self {
        Self::new(Reason::Member {
            names,
            found: found.to_string(),
        })
    }

    pub(crate) fn null_item() -> Self {
        Self::new(Reason::NullItem)
    }

    pub(crate) fn mark(message: impl Into<String>) -> Self {
        Self::new(Reason::Mark(message.into()))
    }

    pub(crate) fn composition(message: impl Into<String>) -> Self {
        Self::new(Reason::Composition(message.into()))
    }

    pub(crate) fn invalid(error: ChartError) -> Self {
        Self::new(Reason::Invalid(error))
    }

    /// The error of the argument of the parameter `param`, whose argument this error is.
    pub(crate) fn within(mut self, param: &'static str) -> Self {
        self.path.insert(0, param);
        self
    }
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for param in &self.path {
            write!(f, "`{param}` ")?;
        }
        match &self.reason {
            Reason::Rejected {
                expected,
                found,
                fragment,
            } => {
                let found = match fragment {
                    Some(name) => describe_fragment(name),
                    None => found.describe().to_string(),
                };
                write!(f, "takes {}, not {found}", expected.describe())
            }
            Reason::Required => write!(f, "is required"),
            Reason::Arity { max, found } => write!(f, "takes {max} arguments at most, not {found}"),
            Reason::Range { value } => write!(f, "takes an integer in its range, not {value}"),
            Reason::Value { values, found } => {
                write!(f, "takes {}, not '{found}'", names(values, ", "))
            }
            Reason::Member {
                names: members,
                found,
            } => {
                write!(f, "takes one of {}, not `{found}`", names(members, ", "))
            }
            Reason::NullItem => write!(f, "takes an array without NULL items"),
            Reason::Mark(message) | Reason::Composition(message) => write!(f, "{message}"),
            Reason::Invalid(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ArgError {}

impl Expected {
    /// What a parameter accepts, in the words of an error.
    pub fn describe(&self) -> String {
        match self {
            Expected::Shape(shape) => match shape {
                Shape::Bool => "a Boolean".into(),
                Shape::Int => "an integer".into(),
                Shape::Float => "a number".into(),
                Shape::Str | Shape::Enum { .. } | Shape::Names(_) => "a string".into(),
                Shape::Bytes => "bytes".into(),
                Shape::Value => "a constant".into(),
                Shape::Array(_) => "an array".into(),
                Shape::Message { constructor, .. } => format!("`{constructor}(...)`"),
            },
            Expected::Inputs(accepts) => describe_inputs(accepts),
        }
    }
}

/// The inputs `accepts` in the words of an error: "any constant" for every class of constant, the
/// calls of the constructors of the fragments, the names that pick a member, and the classes of
/// the values.
fn describe_inputs(accepts: &[Accept]) -> String {
    const CONSTANTS: [Class; 6] = [
        Class::Bool,
        Class::Int,
        Class::Float,
        Class::Str,
        Class::Array,
        Class::Other,
    ];
    let any = (CONSTANTS.iter()).all(|class| accepts.iter().any(|accept| accept.class == *class));
    let mut parts: Vec<String> = Vec::new();
    if any {
        parts.push("any constant".to_string());
    }
    for accept in accepts {
        let part = match (&accept.class, &accept.shape) {
            (Class::Fragment, Shape::Message { constructor, .. }) => {
                format!("`{constructor}(...)`")
            }
            (class, _) if any && CONSTANTS.contains(class) => continue,
            (_, Shape::Names(members)) => format!("a name ({})", names(members, ", ")),
            (class, _) => class.describe().to_string(),
        };
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    match parts.as_slice() {
        [] => "nothing".to_string(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// The names of `values`, joined by `separator`.
pub(crate) fn names(values: &[Name], separator: &str) -> String {
    let names: Vec<&str> = values.iter().map(|value| value.name).collect();
    names.join(separator)
}
