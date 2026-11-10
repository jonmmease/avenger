//! The functions of the format, behind the `functions` feature: a constructor for each message of
//! the options, which builds the fragment of its message, and a mark function for each mark of
//! each layer, which builds the chart of one layer. A binding, such as the SQL functions of
//! avenger-sql, exposes them in its interface.
//!
//! The module has three parts:
//!
//! - Tables that describe the functions: [`constructors`], [`marks`], and [`encoding_options`],
//!   the options of `scaled`. Each parameter lists every input that it accepts, with the chain of
//!   conversions that the input takes.
//! - [`Constructor::build`], which builds the fragment of a constructor from [`Arg`]s, the
//!   constant arguments that a binding converts its values to.
//! - [`Mark::call`], which checks the arguments of a mark function and gives a
//!   [`MarkTemplate`], which builds the chart of one layer from a table.
//!
//! The names of the functions and their parameters are the same in every interface. A binding
//! escapes the names that its language reserves, such as the parameter `lambda` in Python.

mod argument;
mod call;
mod error;

use std::fmt::Write;

pub use argument::{Arg, Class};
pub use call::{ChannelArg, MarkCall, MarkTemplate};
pub use error::{ArgError, Expected, Reason};

use crate::{proto, CoordKind};

/// The extension name of an Arrow column of fragments, whose metadata is the full name of their
/// message.
pub const FRAGMENT_EXTENSION_NAME: &str = "avenger.chart.fragment";

/// The name of `scaled`, whose argument a mark draws through the channel's scale: a
/// `scaled_field` or a `scaled_value`.
pub const SCALED: &str = "scaled";

/// The name of `direct`, whose argument a mark draws as it is: a `direct_field` or a
/// `direct_value`.
pub const DIRECT: &str = "direct";

/// A constructor: its name, the full name of its message, its documentation, and its
/// parameters.
pub struct Constructor {
    pub name: &'static str,
    pub message: &'static str,
    pub doc: &'static str,
    pub params: &'static [Param],
    build: fn(&[Option<Arg>]) -> Result<proto::Fragment, ArgError>,
}

impl Constructor {
    /// The fragment of the constructor's message from `args`, one for each parameter, where
    /// `None` leaves a parameter unset. Missing arguments at the end are unset.
    pub fn build(&self, args: &[Option<Arg>]) -> Result<proto::Fragment, ArgError> {
        match args.len() <= self.params.len() {
            true => {
                let mut args = args.to_vec();
                args.resize_with(self.params.len(), || None);
                (self.build)(&args)
            }
            false => Err(ArgError::arity(self.params.len(), args.len())),
        }
    }
}

impl std::fmt::Debug for Constructor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Constructor")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A parameter of a function: a field outside a oneof, or a oneof, with the inputs that it
/// accepts.
#[derive(Debug)]
pub struct Param {
    pub name: &'static str,
    pub doc: &'static str,
    pub required: bool,
    pub accepts: &'static [Accept],
}

/// One input of a parameter: an argument of a class, the field and member names of the chain
/// from the parameter's target to the field that the argument sets, innermost first, and the shape
/// of that field. `interval_fit(...) -> interval -> fit -> data` of a domain is a fragment of
/// `IntervalFit` in the member `interval` of `DomainFit`, which is the `fit` of a `DataDomain`,
/// which is the member `data` of the domain.
#[derive(Debug)]
pub struct Accept {
    pub class: Class,
    pub chain: &'static [&'static str],
    pub shape: Shape,
}

/// What a field holds, as an argument sets it.
#[derive(Debug)]
pub enum Shape {
    Bool,
    Int,
    Float,
    Str,
    Bytes,
    /// An enum, by its full name, which a string names with one of its values.
    Enum {
        name: &'static str,
        values: &'static [Name],
    },
    /// Any constant, which a `ValueRef` stores.
    Value,
    /// A message, by its full name, with the name of its constructor.
    Message {
        message: &'static str,
        constructor: &'static str,
    },
    /// A member of a oneof that a string names, without content of its own.
    Names(&'static [Name]),
    Array(&'static Shape),
}

/// A name that a string argument gives, with its documentation: a value of an enum, or a member
/// of a oneof.
#[derive(Debug)]
pub struct Name {
    pub name: &'static str,
    pub doc: &'static str,
}

/// The generated code: the code of the parameter model, which holds the parameters, the
/// application of arguments, and the conversions of each message of the option tree, each mark
/// message, and `Encoding`, and the table of the constructors on top of it.
#[allow(dead_code, unreachable_patterns, clippy::all)]
pub(crate) mod generated {
    use super::{Accept, Arg, ArgError, Class, Constructor, Expected, Name, Param, Shape};
    include!(concat!(env!("OUT_DIR"), "/parameters.rs"));
    include!(concat!(env!("OUT_DIR"), "/constructors.rs"));
}

/// The constructors, in the order of the option tree.
pub fn constructors() -> &'static [Constructor] {
    generated::CONSTRUCTORS
}

/// The constructor `name`.
pub fn constructor(name: &str) -> Option<&'static Constructor> {
    constructors()
        .iter()
        .find(|constructor| constructor.name == name)
}

/// The options of `scaled`, the options of an encoding, which follow its value.
pub fn encoding_options() -> &'static [Param] {
    generated::PARAMS_ENCODING
}

/// The text of every constructor's parameters and the inputs that each accepts, which the snapshot
/// test of the constructors compares.
pub fn surface() -> String {
    let mut text = String::new();
    for constructor in constructors() {
        let names: Vec<&str> = constructor.params.iter().map(|param| param.name).collect();
        writeln!(text, "{}({})", constructor.name, names.join(", ")).unwrap();
        for param in constructor.params {
            let required = if param.required { " (required)" } else { "" };
            writeln!(
                text,
                "  {}{required}: {}",
                param.name,
                describe_accepts(param.accepts)
            )
            .unwrap();
        }
    }
    text
}

/// The text of the inputs `accepts`: the shape of a field whose every input sets it directly, or
/// each input with its chain.
fn describe_accepts(accepts: &[Accept]) -> String {
    if let [first, rest @ ..] = accepts {
        let direct = |accept: &Accept| accept.chain.is_empty() && accept.class != Class::Fragment;
        let same = (rest.iter()).all(|accept| shape_eq(&accept.shape, &first.shape));
        if direct(first) && rest.iter().all(direct) && same {
            return describe_shape(&first.shape);
        }
    }
    let parts: Vec<String> = (accepts.iter())
        .map(|accept| {
            let mut steps: Vec<String> = Vec::new();
            steps.push(match (&accept.class, &accept.shape) {
                (Class::Fragment, Shape::Message { constructor, .. }) => {
                    format!("{constructor}(...)")
                }
                (class, _) => class_word(*class).to_string(),
            });
            if matches!(accept.shape, Shape::Names(_)) {
                steps.push("a member's name".to_string());
            }
            steps.extend(accept.chain.iter().map(|step| step.to_string()));
            steps.join(" -> ")
        })
        .collect();
    parts.join("; ")
}

/// Whether two shapes are the same shape.
fn shape_eq(a: &Shape, b: &Shape) -> bool {
    describe_shape(a) == describe_shape(b)
}

/// The text of what a field of `shape` takes.
fn describe_shape(shape: &Shape) -> String {
    match shape {
        Shape::Bool => "Boolean".into(),
        Shape::Int => "integer".into(),
        Shape::Float => "number".into(),
        Shape::Str => "string".into(),
        Shape::Bytes => "bytes".into(),
        Shape::Enum { values, .. } => format!("one of {}", error::names(values, ", ")),
        Shape::Value => "any constant".into(),
        Shape::Message { constructor, .. } => format!("{constructor}(...)"),
        Shape::Names(names) => format!("one of {}", error::names(names, ", ")),
        Shape::Array(item) => format!("array of {}", describe_shape(item)),
    }
}

/// A class in the text of the surface.
fn class_word(class: Class) -> &'static str {
    match class {
        Class::Bool => "Boolean",
        Class::Int => "integer",
        Class::Float => "float",
        Class::Str => "string",
        Class::Array => "array",
        Class::Bytes => "bytes",
        Class::Other => "other",
        Class::Fragment => "fragment",
    }
}

/// A mark function: its name, the coordinate system of its mark, its documentation, its channels,
/// which are its first parameters, and its properties, which follow them. A mark function takes
/// `coord`, the options of its plot, last.
pub struct Mark {
    pub name: &'static str,
    pub coord: CoordKind,
    pub doc: &'static str,
    pub channels: &'static [Channel],
    pub properties: &'static [Param],
    pub(crate) mark: fn() -> call::LayerMark,
    pub(crate) apply_properties: call::ApplyProperties,
}

impl std::fmt::Debug for Mark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mark")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A channel of a mark function: a parameter named after its field. [`Channel::field_kind`] and
/// [`Channel::value_kind`] give the kinds that a bare field and a bare value of it take.
#[derive(Debug)]
pub struct Channel {
    pub name: &'static str,
    pub channel: proto::Channel,
    pub required: bool,
    pub doc: &'static str,
}

/// The kind of a channel's encoding: a value that a mark draws through the channel's scale, or
/// one that it draws as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Scaled,
    Direct,
}

impl Kind {
    /// The kind that the function `name`, `scaled` or `direct`, gives.
    pub fn of(name: &str) -> Option<Self> {
        match name {
            SCALED => Some(Self::Scaled),
            DIRECT => Some(Self::Direct),
            _ => None,
        }
    }

    /// The name of the function that gives the kind.
    pub fn name(self) -> &'static str {
        match self {
            Self::Scaled => SCALED,
            Self::Direct => DIRECT,
        }
    }
}

/// The name of the parameter of a mark function that takes the options of its plot.
pub const COORD: &str = "coord";

include!(concat!(env!("OUT_DIR"), "/mark_functions.rs"));

/// The mark functions, in the order of the format's layers and their marks.
pub fn marks() -> &'static [Mark] {
    MARKS
}

/// The mark function `name`.
pub fn mark(name: &str) -> Option<&'static Mark> {
    MARKS.iter().find(|mark| mark.name == name)
}
