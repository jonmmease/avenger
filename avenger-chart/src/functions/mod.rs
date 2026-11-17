//! The functions of the format, behind the `functions` feature: a constructor for each message of
//! the options, which builds the fragment of its message, a constructor for each oneof of them,
//! which builds the oneof, and a mark function for each mark of each layer, which builds the chart
//! of one layer. A binding, such as the SQL functions of avenger-sql, exposes them in its
//! interface.
//!
//! The module has three parts:
//!
//! - Tables that describe the functions: [`constructors`], [`marks`], with the options of the
//!   plot and the layer of each mark function, and [`channel_functions`], a function for each
//!   channel with a scale, which takes the channel's value and the options of its encoding. Each
//!   parameter lists every input that it accepts, with the chain of conversions that the input
//!   takes.
//! - [`Constructor::build`], which builds the fragment of a constructor from [`Arg`]s, the
//!   constant arguments that a binding converts its values to.
//! - [`Mark::call`], which checks the arguments of a mark function and gives a
//!   [`MarkTemplate`], which builds the chart of one layer from a table.
//!
//! The names of the mark functions and of the parameters are the same in every interface, and a
//! binding escapes the names that its language reserves, such as the parameter `lambda` in Python.
//! A constructor's [`Constructor::name`] is unique, such as `log_scale` and `scale.type`, and a
//! binding that knows the parameter of each argument names a constructor after the place that it
//! fills, which [`Param::constructor`] gives: `type => log(base => 2)` in SQL. A parameter takes a
//! plain value, such as a shorthand of its message, or a fragment of its own constructor, and SQL
//! also takes a constructor of one of that constructor's parameters, which it wraps in the
//! parameter's own constructor.

mod argument;
mod call;
mod compose;
mod error;

use std::fmt::Write;

pub use argument::{Arg, Class};
pub use call::{ChannelArg, MarkCall, MarkTemplate};
pub use compose::{composition, compositions, Child, ChildOrder, Composition, CompositionKind};
pub use error::{ArgError, Expected, Reason};

use crate::{
    proto,
    validate::{is_position, scales_strings, takes_scale},
    CoordKind,
};

/// The extension name of an Arrow column of fragments, whose metadata is the full name of their
/// message.
pub const FRAGMENT_EXTENSION_NAME: &str = "avenger.chart.fragment";

/// The name of `datum`, whose constant argument a mark draws through the channel's scale: a
/// `scaled_value`, as Vega-Lite's `datum` is.
pub const DATUM: &str = "datum";

/// The name of `value`, whose argument a mark draws as it is: a `direct_field` or a
/// `direct_value`, as Vega-Lite's `value` is.
pub const VALUE: &str = "value";

/// A constructor: its unique name, the full name of its message, the oneof that it builds, its
/// documentation, and its parameters. A constructor of a message builds a fragment of the message.
/// A constructor of a oneof builds a fragment of its message with only the oneof set, whose type is
/// the message's full name and the oneof's name, such as `avenger.chart.v1.Scale.type`, which only
/// the oneof's parameter takes.
///
/// The first parameter of a constructor of a message with shorthands is
/// [`positional`](Param::positional) and takes exactly the shorthands: `title => 'Mass'` and
/// `title => title('Mass')` build the same title.
pub struct Constructor {
    pub name: &'static str,
    pub message: &'static str,
    pub oneof: Option<&'static str>,
    pub doc: &'static str,
    pub params: &'static [Param],
    /// The parameters that the first parameter fills when it is a parameter of its own, `value`,
    /// whose argument a call then need not give, as `type` of `projection('mercator')`.
    pub fills: &'static [&'static str],
    build: fn(&[Option<Arg>]) -> Result<proto::Fragment, ArgError>,
    convert: fn(&Arg) -> Result<proto::Fragment, ArgError>,
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

    /// The fragment of `arg`, a plain value or a fragment of the constructor, as a parameter whose
    /// own constructor this is converts it: `foo => v` for the value `v`.
    pub fn convert(&self, arg: &Arg) -> Result<proto::Fragment, ArgError> {
        (self.convert)(arg)
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
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub doc: &'static str,
    pub required: bool,
    pub accepts: &'static [Accept],
    /// The name of the constructor of the parameter's own form, which a binding gives the
    /// fragment that the parameter takes: the field's name for a message field, such as `scale`,
    /// the oneof's name for a oneof, such as `type`, and the `element` option of a repeated
    /// message field, such as `level` for `levels`. `None` for a parameter of plain values, and
    /// for a repeated field whose element holds only a choice, such as `tools`, whose elements
    /// take the choice's members.
    pub constructor: Option<&'static str>,
    /// Whether a call gives the argument by its position: only the first parameter of a
    /// constructor with shorthands, which takes them.
    pub positional: bool,
}

/// One input of a parameter: an argument of a class, the field and member names of the chain
/// from the parameter's target to the field that the argument sets, innermost first, and the shape
/// of that field. `array of number -> bounds -> box` of a projection's `fit` is an array of numbers
/// in the field `bounds` of `FitBox`, which is the member `box` of the fit. An array of a repeated
/// message field can take values that each convert to the message, and its chain is the chain of
/// each element, as `array of number -> length` of a concat's `tracks` sets the length of each
/// track.
#[derive(Clone, Copy, Debug)]
pub struct Accept {
    pub class: Class,
    pub chain: &'static [&'static str],
    pub shape: Shape,
}

/// What a field holds, as an argument sets it.
#[derive(Clone, Copy, Debug)]
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
    /// A message, by its full name, with the unique name of its constructor. A fragment of the
    /// constructor of a oneof has the message's full name and the oneof's name.
    Message {
        message: &'static str,
        constructor: &'static str,
    },
    /// A member of a oneof that a string names, without content of its own.
    Names(&'static [Name]),
    Array(&'static Shape),
    /// Named options, which a struct sets: a field of the struct for each option, whose value is
    /// a Boolean, a number, a string, or an array of numbers or of strings.
    Options,
}

/// A name that a string argument gives, with its documentation: a value of an enum, or a member
/// of a oneof.
#[derive(Clone, Copy, Debug)]
pub struct Name {
    pub name: &'static str,
    pub doc: &'static str,
}

/// The generated code: the code of the parameter model, which holds the parameters, the
/// application of arguments, and the conversions of each message of the option tree, each mark
/// message, and `Encoding`, and the table of the constructors on top of it.
#[allow(dead_code, unreachable_patterns, clippy::all)]
pub(crate) mod generated {
    use super::{
        Accept, Arg, ArgError, Class, Composition, CompositionKind, Constructor, Expected, Name,
        Param, Shape,
    };
    include!(concat!(env!("OUT_DIR"), "/parameters.rs"));
    include!(concat!(env!("OUT_DIR"), "/constructors.rs"));
    include!(concat!(env!("OUT_DIR"), "/compositions.rs"));
    include!(concat!(env!("OUT_DIR"), "/mark_options.rs"));
}

/// The constructors, in the order of the option tree.
pub fn constructors() -> &'static [Constructor] {
    generated::CONSTRUCTORS
}

/// The constructor with the unique name `name`, such as `log_scale` or `scale.type`.
pub fn constructor(name: &str) -> Option<&'static Constructor> {
    constructors()
        .iter()
        .find(|constructor| constructor.name == name)
}

/// The options of a position with a scale of its own, `x`, `y`, `theta`, and `radius`, in the
/// order of its channel function's parameters.
const POSITION_OPTIONS: &[&str] = &[
    "scale",
    "axis",
    "title",
    "band_position",
    "band_level",
    "selection_field",
];

/// The options of the second position of a pair, `x2`, `y2`, `theta2`, and `radius2`, which
/// follows the scale, the axis, and the title of the first, so it takes none.
const PARTNER_OPTIONS: &[&str] = &[];

/// The options of another channel with a scale of its own, such as `color`.
const OTHER_OPTIONS: &[&str] = &["scale", "legend", "title", "selection_field"];

/// A channel function: a function for a channel with a scale, named after the channel, which takes
/// the channel's value and then, by name, the options of its encoding that the channel has, as
/// `x(body_mass, axis => axis(grid => FALSE))`. A position with a scale of its own takes its
/// scale, its axis, its title, its place in a band, and its selection field, the second position
/// of a pair no options, and another channel its scale, its legend, its title, and its
/// selection field.
///
/// A channel function keeps the kind of its value: a bare value takes the default kind of the
/// channel, and [`DATUM`] or [`VALUE`] around the value gives its kind, so `fill('red')` draws red
/// as `fill => 'red'` does.
#[derive(Debug)]
pub struct ChannelFunction {
    pub name: &'static str,
    pub channel: proto::Channel,
    /// The documentation of the channel's values.
    pub doc: &'static str,
    /// The options of the channel's encoding, which follow its value, by name.
    pub options: Vec<&'static Param>,
}

/// The channel functions, one for each channel with a scale, in the order of the channels.
pub fn channel_functions() -> &'static [ChannelFunction] {
    static FUNCTIONS: std::sync::OnceLock<Vec<ChannelFunction>> = std::sync::OnceLock::new();
    FUNCTIONS.get_or_init(|| {
        let encoding = generated::PARAMS_ENCODING;
        (generated::ENUM_CHANNEL.iter())
            .filter_map(|name| {
                let upper = format!("CHANNEL_{}", name.name.to_ascii_uppercase());
                let channel = proto::Channel::from_str_name(&upper)?;
                let options = match (scales_strings(channel), takes_scale(channel)) {
                    (false, _) => return None,
                    (true, true) if is_position(channel) => POSITION_OPTIONS,
                    (true, true) => OTHER_OPTIONS,
                    (true, false) => PARTNER_OPTIONS,
                };
                let options = (options.iter())
                    .map(|option| {
                        (encoding.iter())
                            .find(|param| param.name == *option)
                            .expect("an option of a channel function is an option of an encoding")
                    })
                    .collect();
                Some(ChannelFunction {
                    name: name.name,
                    channel,
                    doc: name.doc,
                    options,
                })
            })
            .collect()
    })
}

/// The channel function named `name`, such as `x` or `fill_opacity`.
pub fn channel_function(name: &str) -> Option<&'static ChannelFunction> {
    (channel_functions().iter()).find(|function| function.name.eq_ignore_ascii_case(name))
}

/// The fields and the oneofs that the `lift` and `lift_oneof` options mark, the shorthands of
/// their messages, as `Message.field`, in the order of their names.
pub fn lifts() -> &'static [&'static str] {
    generated::LIFTS
}

/// The text of every constructor's and every composition's parameters and the inputs that each
/// accepts, and of the lifted fields, which the snapshot test of the constructors compares. A
/// constructor is under its unique name, and a fragment that a parameter takes under the name of
/// the parameter's own constructor.
pub fn surface() -> String {
    let mut text = String::new();
    let mut function = |name: &str, leading: &[&str], params: &[Param]| {
        let names: Vec<&str> = (leading.iter().copied())
            .chain(params.iter().map(|param| param.name))
            .collect();
        writeln!(text, "{name}({})", names.join(", ")).unwrap();
        for param in params {
            let notes: Vec<&str> = [
                (param.positional, "positional"),
                (param.required, "required"),
            ]
            .into_iter()
            .filter(|(holds, _)| *holds)
            .map(|(_, note)| note)
            .collect();
            let notes = match notes.is_empty() {
                true => String::new(),
                false => format!(" ({})", notes.join(", ")),
            };
            let accepts = describe_accepts(param);
            writeln!(text, "  {}{notes}: {accepts}", param.name).unwrap();
        }
    };
    for constructor in constructors() {
        function(constructor.name, &[], constructor.params);
    }
    for composition in compositions() {
        let leading: &[&str] = match composition.kind.takes_key() {
            true => &["chart", "key"],
            false => &["chart"],
        };
        function(composition.name, leading, composition.params);
    }
    writeln!(text, "lifts").unwrap();
    for lift in lifts() {
        writeln!(text, "  {lift}").unwrap();
    }
    text
}

/// The text of the inputs of `param`: the shape of a field whose every input sets it directly, or
/// each input with its chain, where a fragment is a call of the parameter's own constructor.
fn describe_accepts(param: &Param) -> String {
    let accepts = param.accepts;
    // A parameter without a constructor of its own, such as `tools`, takes the fragments of its
    // element's constructor, whose name the analysis gives the members of its choice.
    let fallback = accepts.iter().find_map(|accept| match accept.shape {
        Shape::Message { constructor, .. } => Some(constructor),
        Shape::Array(Shape::Message { constructor, .. }) => Some(*constructor),
        _ => None,
    });
    let own = match (param.constructor, fallback) {
        (Some(name), _) => format!("{name}(...)"),
        (None, Some(element)) => format!("a member of {element}(...)"),
        (None, None) => "?".to_string(),
    };
    if let [first, rest @ ..] = accepts {
        let direct = |accept: &Accept| accept.chain.is_empty() && accept.class != Class::Fragment;
        let same = (rest.iter()).all(|accept| shape_eq(&accept.shape, &first.shape));
        if direct(first) && rest.iter().all(direct) && same {
            return describe_shape(&first.shape, &own);
        }
    }
    let parts: Vec<String> = (accepts.iter())
        .map(|accept| {
            let mut steps: Vec<String> = Vec::new();
            steps.push(match (&accept.class, &accept.shape) {
                (Class::Fragment, Shape::Message { .. }) => own.clone(),
                (Class::Array, Shape::Array(Shape::Message { .. })) => format!("array of {own}"),
                (Class::Array, shape @ Shape::Array(_)) => describe_shape(shape, &own),
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
    describe_shape(a, "") == describe_shape(b, "")
}

/// The text of what a field of `shape` takes, where a message is `own`, a call of its
/// constructor.
fn describe_shape(shape: &Shape, own: &str) -> String {
    match shape {
        Shape::Bool => "Boolean".into(),
        Shape::Int => "integer".into(),
        Shape::Float => "number".into(),
        Shape::Str => "string".into(),
        Shape::Bytes => "bytes".into(),
        Shape::Enum { values, .. } => format!("one of {}", error::names(values, ", ")),
        Shape::Value => "any constant".into(),
        Shape::Message { .. } => own.to_string(),
        Shape::Names(names) => format!("one of {}", error::names(names, ", ")),
        Shape::Array(item) => format!("array of {}", describe_shape(item, own)),
        Shape::Options => "struct of named options".into(),
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
        Class::Struct => "struct",
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
    /// The options of its plot and its layer, which follow its properties and precede `coord`:
    /// the options of its plot that concern no other layer, `title`, `attribution`, `tools`,
    /// `width`, and `height`, then the options of its layer, `id` and `classes`, without an option
    /// whose name the mark's own parameters take, such as `width` of `text`. A mark call places
    /// each of its tools on the narrowest part that holds it: a navigation on its plot, and a
    /// tooltip or a selection on its layer. The id is the layer's: the plot of a mark call has
    /// none, so its path identifies it, and a `layer` composition gives it one. A `layer`
    /// composition merges the plots' options, as `PlotBuilder::overlay` does.
    pub options: &'static [Param],
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

/// The kind of a channel's encoding: a value that a mark draws through the channel's scale, which
/// [`DATUM`] gives a constant, or one that it draws as it is, which [`VALUE`] gives a constant or a
/// column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Scaled,
    Direct,
}

impl Kind {
    /// The kind that the function `name`, `datum` or `value`, gives.
    pub fn of(name: &str) -> Option<Self> {
        match name {
            DATUM => Some(Self::Scaled),
            VALUE => Some(Self::Direct),
            _ => None,
        }
    }

    /// The name of the function that gives the kind.
    pub fn name(self) -> &'static str {
        match self {
            Self::Scaled => DATUM,
            Self::Direct => VALUE,
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
