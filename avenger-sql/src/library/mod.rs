//! The function library: GoogleSQL's functions, and avenger's own, mapped onto DataFusion.
//!
//! The pass resolves the name of each call against the [`Library`]. A [`Function`] documents
//! itself and each of its parameters, and declares its valid combinations of arguments as
//! [`Form`]s, typed as GoogleSQL types them, as in `SUBSTR(value: STRING, position: INT64[,
//! length: INT64]) -> STRING`. A call takes the form that its arguments fit at the least cost of
//! coercion, which [`form`] describes. The pass, the docs, and completion read the same forms.
//!
//! Each form has a mapping from its arguments, cast to the types of the form, to DataFusion:
//!
//! - a scalar function maps to an [`Expr`];
//! - an aggregate function maps to an [`AggregateUDF`] and its arguments, to which the pass adds
//!   the clauses of the call, such as `DISTINCT` and `ORDER BY`, or an `OVER` clause;
//! - a window function maps to a window function and its arguments, to which the pass adds the
//!   `OVER` clause.
//!
//! A parameter can take a date part, such as `MONTH` in `DATE_TRUNC(d, MONTH)`, which the pass
//! reads from the syntax as a [`DatePart`], not as an expression. A function that a call writes
//! as `SAFE.f(...)` returns `NULL` where `f` fails, when its mapping can tell.
//!
//! Each mapping follows GoogleSQL's result where DataFusion's function differs, as in `LOG(x, b)`,
//! the logarithm of `x` in the base `b`, and `WEEK`, whose weeks start on Sunday. Time zones
//! default to UTC, as in BigQuery. A function's notes name where avenger-sql differs from
//! GoogleSQL, as `SQRT(-1)` does, which returns `NaN` where GoogleSQL fails.

mod aggregate;
mod array;
mod avenger;
mod bytes;
mod cast_string;
mod conditional;
mod datetime;
mod datetime_format;
mod encoding;
pub(crate) mod error;
pub mod form;
pub(crate) mod interval;
mod like;
pub(crate) mod match_recognize;
mod math;
pub(crate) mod operator;
pub(crate) mod random;
mod reference;
mod string;

use std::{collections::HashMap, fmt, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, FieldRef},
    common::{DFSchema, ScalarValue},
    logical_expr::{
        function::WindowUDFFieldArgs, AggregateUDF, Expr, ExprSchemable, WindowFunctionDefinition,
    },
};

pub use array::Subscript;
pub(crate) use cast_string::cast_to_string;
pub(crate) use datetime::extract;
pub use datetime::{DatePart, Weekday};
pub use form::{Accept, Arity, FormArg, Label, PartSet, Signature, Ty};
pub use reference::{entry, reference};

use avenger_sql_syntax::FunctionRole;

use crate::types::TypeOptions;
use form::{Observed, Resolution};

/// What a call of a function computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionKind {
    /// A value of each row.
    Scalar,
    /// A value of the rows of a group, or of a window with `OVER`.
    Aggregate,
    /// A value of each row over a window, with `OVER`.
    Window,
    /// The rows of a table, of a pipe `CALL` or of `FROM`.
    Table,
}

/// Where a function comes from: GoogleSQL, or avenger's marks, constructors and transforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    GoogleSql,
    /// A mark of avenger-chart, an aggregate that builds a chart of the rows of its group.
    Mark,
    /// A constructor of avenger-chart, or `scaled` or `direct`, which returns a fragment of a
    /// chart's message.
    Constructor,
    /// A transform of avenger-datafusion-transforms: `bin`, `stack`, `timeunit`, `lump`, or
    /// `impute`.
    Transform,
    /// A composition of avenger-chart: an aggregate that composes the charts of a group, such as
    /// `facet_col`.
    Composition,
}

impl FunctionKind {
    /// The role of a function of this kind, for highlighting.
    pub fn role(self) -> FunctionRole {
        match self {
            FunctionKind::Scalar => FunctionRole::Scalar,
            FunctionKind::Aggregate => FunctionRole::Aggregate,
            FunctionKind::Window => FunctionRole::Window,
            FunctionKind::Table => FunctionRole::Table,
        }
    }
}

/// A parameter of a function, which the forms of the function take by its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    /// What the parameter takes, what its values mean, and what omitting it does.
    pub doc: String,
    /// What a parameter of a chart function that takes constructors takes, which its form types
    /// as `BYTES` or `ANY`. Empty for the other parameters.
    pub takes: Takes,
}

/// What a parameter of a chart function takes besides the type of its form: the strings of a
/// set, the types of its other values, and the constructors whose fragments it takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Takes {
    /// The strings of the set that the parameter takes, such as the types of a scale.
    pub strings: Vec<String>,
    /// The types of the other values that the parameter takes, such as `STRING` for the text of
    /// a title.
    pub values: Vec<Ty>,
    /// The constructors, each once: those of the target itself, then those of its parts in the
    /// order of the parameters of the target's constructor.
    pub constructors: Vec<Builds>,
}

/// A constructor that a parameter takes, by its name in SQL, with the part of the parameter's
/// target that its fragment sets: the parameter of the target's constructor that sets the
/// outermost field of its chain, such as `type` for `linear_scale` and `legend` for `legend` in
/// the `scale` of `scaled`, or `None` for the target itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Builds {
    pub constructor: String,
    pub part: Option<String>,
    /// Whether the parameter takes an array of the constructor's fragments, as `tools` does.
    pub array: bool,
    /// The form of the constructor without its result, such as
    /// `domain_pin(value: ANY, position: 'low' | 'mid' | 'high')`.
    pub form: String,
}

/// The longest form of a function that a hover writes, in characters, and of a constructor in
/// the forms of a parameter. A longer form writes `name(...)`.
pub const FORM: usize = 100;

/// The forms of a parameter that takes constructors, from [`Param::forms`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forms {
    /// The lines of what the parameter takes, such as
    /// `pin => domain_pin(value: ANY, position: 'low' | 'mid' | 'high')`: its strings, its other
    /// values, and the constructors of its target itself.
    pub lines: Vec<String>,
    /// The constructors of the parts of its target, by part, such as
    /// `("Its type", ["band_scale(...)", ...])`.
    pub groups: Vec<(String, Vec<String>)>,
}

impl Forms {
    /// The forms as Markdown: the lines in a code block, then a list of the groups.
    pub fn markdown(&self) -> String {
        let lines = format!("```googlesql\n{}\n```", self.lines.join("\n"));
        match self.groups() {
            Some(groups) => format!("{lines}\n\n{groups}"),
            None => lines,
        }
    }

    /// The groups as a Markdown list, such as `- Its type: `band_scale(...)`, ...`, or
    /// `None` without groups.
    pub fn groups(&self) -> Option<String> {
        let items: Vec<String> = (self.groups.iter())
            .map(|(label, constructors)| {
                let names: Vec<String> = constructors.iter().map(|c| format!("`{c}`")).collect();
                format!("- {label}: {}", names.join(", "))
            })
            .collect();
        (!items.is_empty()).then(|| items.join("\n"))
    }
}

impl Param {
    /// The forms of the parameter when it takes constructors: a line of its strings, at most
    /// `strings` of them and then `…` when `strings` is given, a line of its other values, and a
    /// line for each constructor of its target itself, with the full form of a constructor when it
    /// is the only one and no longer than [`FORM`], else `name(...)`. The constructors of the
    /// parts of its target follow in groups.
    pub fn forms(&self, strings: Option<usize>) -> Option<Forms> {
        let takes = &self.takes;
        if takes.constructors.is_empty() {
            return None;
        }
        let name = &self.name;
        let mut lines = Vec::new();
        if !takes.strings.is_empty() {
            let shown = strings.unwrap_or(usize::MAX);
            let mut values: Vec<String> = (takes.strings.iter().take(shown))
                .map(|value| format!("'{value}'"))
                .collect();
            if takes.strings.len() > shown {
                values.push("…".to_string());
            }
            lines.push(format!("{name} => {}", values.join(" | ")));
        }
        if !takes.values.is_empty() {
            let values: Vec<String> = takes.values.iter().map(Ty::to_string).collect();
            lines.push(format!("{name} => {}", values.join(" | ")));
        }
        let direct: Vec<&Builds> = (takes.constructors.iter())
            .filter(|builds| builds.part.is_none())
            .collect();
        let only = direct.len() == 1;
        for builds in direct {
            let form = match only && builds.form.chars().count() <= FORM {
                true => builds.form.clone(),
                false => format!("{}(...)", builds.constructor),
            };
            lines.push(match builds.array {
                true => format!("{name} => [{form}, ...]"),
                false => format!("{name} => {form}"),
            });
        }
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for builds in &takes.constructors {
            let Some(part) = &builds.part else {
                continue;
            };
            let label = format!("Its {}", part.replace('_', " "));
            let form = format!("{}(...)", builds.constructor);
            match groups.iter_mut().find(|(other, _)| *other == label) {
                Some((_, forms)) => forms.push(form),
                None => groups.push((label, vec![form])),
            }
        }
        Some(Forms { lines, groups })
    }
}

/// An example of a call: a query or an expression, and its result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Example {
    /// An expression, for a scalar function, or a pipe query.
    pub query: String,
    /// The result as GoogleSQL's literals write it, such as `'pple'` or `DATE '2024-05-17'`, or
    /// the rows of a query as a table.
    pub result: String,
}

/// Whether a clause of `OVER` is required, allowed, or forbidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Need {
    Required,
    Allowed,
    Forbidden,
}

/// The `OVER` clause that a call takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Over {
    /// No `OVER`: a scalar function, or an aggregate that only groups.
    Forbidden,
    /// An aggregate, which a call can also compute over a window.
    Optional,
    /// A window function, whose `OVER` takes an `ORDER BY` as `order_by` says, and a frame when
    /// `frame` holds.
    Required { order_by: Need, frame: bool },
}

/// The clauses that a call of a function takes in its parentheses, and its `OVER`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Clauses {
    /// `*` in place of the arguments, as `COUNT(*)` takes.
    pub star: bool,
    pub distinct: bool,
    /// `IGNORE NULLS` and `RESPECT NULLS`.
    pub null_treatment: bool,
    pub order_by: bool,
    /// `HAVING MAX` and `HAVING MIN`, which compute an aggregate over the rows of the greatest or
    /// the least value of an expression.
    pub having: bool,
    /// `LIMIT n`, which keeps the first values of an aggregate in its order.
    pub limit: bool,
    pub over: Over,
}

impl Clauses {
    const NONE: Clauses = Clauses {
        star: false,
        distinct: false,
        null_treatment: false,
        order_by: false,
        having: false,
        limit: false,
        over: Over::Forbidden,
    };
}

/// An argument of a call, which the pass plans by the kind of its parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Value(Expr),
    DatePart(DatePart),
}

/// Where an error of a call is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorAt {
    /// The call, or the function's name.
    Call,
    /// The argument at this index, in the order that the call writes its arguments.
    Arg(usize),
}

/// An error of a call of a function, at the call or at one of its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallError {
    pub at: ErrorAt,
    pub message: String,
    /// Whether the error is that no form of the function takes the types of the arguments.
    pub no_form: bool,
}

impl CallError {
    pub(crate) fn call(message: impl Into<String>) -> Self {
        Self {
            at: ErrorAt::Call,
            message: message.into(),
            no_form: false,
        }
    }

    fn arg(index: usize, message: impl Into<String>) -> Self {
        Self {
            at: ErrorAt::Arg(index),
            message: message.into(),
            no_form: false,
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<datafusion::error::DataFusionError> for CallError {
    fn from(error: datafusion::error::DataFusionError) -> Self {
        CallError::call(error.to_string())
    }
}

/// What a mapping plans a call to.
#[derive(Clone, Debug)]
pub enum Planned {
    Scalar(Expr),
    Aggregate {
        func: Arc<AggregateUDF>,
        args: Vec<Expr>,
        /// The rows of the group that the aggregate reads, as `COUNTIF` reads the rows of its
        /// condition.
        filter: Option<Expr>,
        /// The type of the form's result, when it differs from the aggregate's, as `COUNT`
        /// returns an `INT64`. A mapping leaves it `None`, and [`Function::plan`] sets it from
        /// the form. The pass casts the aggregate's column to it after the aggregate.
        result: Option<DataType>,
    },
    Window {
        func: WindowFunctionDefinition,
        args: Vec<Expr>,
        /// The type of the form's result, when it differs from the function's, as `ROW_NUMBER`
        /// returns an `INT64`. A mapping leaves it `None`, and [`Function::plan`] sets it from
        /// the form. The pass casts the window's column to it after the window.
        result: Option<DataType>,
    },
}

impl From<Expr> for Planned {
    fn from(expr: Expr) -> Self {
        Planned::Scalar(expr)
    }
}

/// What a mapping knows of the place of its call.
#[derive(Clone, Copy, Debug)]
pub struct PlanContext<'a> {
    /// The schema of the input of the call, for the types of its arguments.
    pub schema: &'a DFSchema,
    /// The default time zone of the functions that take one.
    pub time_zone: &'a str,
    /// The default locale of the format and parse functions.
    pub locale: Option<&'a str>,
    /// How GoogleSQL's types map to Arrow types.
    pub type_options: TypeOptions,
}

/// The arguments of a call, bound to the arguments of its form and cast to their types.
pub struct Call<'a> {
    /// The argument of each argument of the form, other than a repeated one, and the index at
    /// which the call writes it.
    slots: Vec<Option<(usize, Arg)>>,
    /// The arguments of the repeated argument of the form, with their indices.
    rest: Vec<(usize, Arg)>,
    resolution: Resolution,
    /// Whether the call writes `SAFE.` before the function's name.
    pub safe: bool,
    /// Whether the call's only argument is `*`, as in `COUNT(*)`.
    pub star: bool,
    pub context: PlanContext<'a>,
}

impl<'a> Call<'a> {
    /// The value of the form's argument at `index`, when the call gives one.
    pub(crate) fn value(&self, index: usize) -> Option<&Expr> {
        match self.slots.get(index)? {
            Some((_, Arg::Value(expr))) => Some(expr),
            _ => None,
        }
    }

    /// The value of the required argument at `index`, which binding guarantees.
    pub(crate) fn arg(&self, index: usize) -> Expr {
        self.value(index)
            .cloned()
            .expect("binding gives each required argument a value")
    }

    /// The date part of the form's argument at `index`.
    pub(crate) fn date_part(&self, index: usize) -> DatePart {
        match self.slots.get(index) {
            Some(Some((_, Arg::DatePart(part)))) => *part,
            _ => panic!("binding gives each date part argument a date part"),
        }
    }

    /// The date part of the optional argument at `index`, when the call gives it.
    pub(crate) fn optional_date_part(&self, index: usize) -> Option<DatePart> {
        match self.slots.get(index) {
            Some(Some((_, Arg::DatePart(part)))) => Some(*part),
            _ => None,
        }
    }

    /// The values of the repeated argument.
    pub(crate) fn rest(&self) -> Vec<Expr> {
        self.rest
            .iter()
            .filter_map(|(_, arg)| match arg {
                Arg::Value(expr) => Some(expr.clone()),
                Arg::DatePart(_) => None,
            })
            .collect()
    }

    /// The values of all the arguments of the form, in their order, with `None` for an omitted
    /// one.
    pub(crate) fn values(&self) -> Vec<Option<Expr>> {
        (0..self.slots.len())
            .map(|index| self.value(index).cloned())
            .collect()
    }

    /// The type that the form's type variable `var` takes for the call.
    pub(crate) fn var(&self, var: &str) -> &Ty {
        self.resolution
            .vars
            .get(var)
            .expect("resolution gives each variable a type")
    }

    /// The type of `expr` in the call's input.
    pub(crate) fn type_of(&self, expr: &Expr) -> Result<DataType, CallError> {
        Ok(expr.get_type(self.context.schema)?)
    }

    /// An error at the argument at `index` of the form, or at the call when the call gives it no
    /// argument.
    pub(crate) fn error_at(&self, index: usize, message: impl Into<String>) -> CallError {
        match self.slots.get(index).and_then(|slot| slot.as_ref()) {
            Some((at, _)) => CallError::arg(*at, message),
            None => CallError::call(message),
        }
    }
}

/// The mapping of a form.
type Mapping = Arc<dyn Fn(&Call) -> Result<Planned, CallError> + Send + Sync>;

/// A form of a function: its signature and its mapping.
#[derive(Clone)]
pub struct Form {
    pub signature: Signature,
    mapping: Mapping,
}

impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Form")
            .field("signature", &self.signature)
            .finish_non_exhaustive()
    }
}

/// A function of the library.
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub kind: FunctionKind,
    pub family: Family,
    /// One line, for completion lists.
    pub summary: String,
    /// What the function computes beyond what its parameters say: its result, its `NULL`s, and
    /// the edge cases of the whole call.
    pub description: String,
    /// Where avenger-sql differs from GoogleSQL.
    pub notes: Vec<String>,
    pub params: Vec<Param>,
    pub forms: Vec<Form>,
    pub clauses: Clauses,
    /// Whether a call can write `SAFE.` before the name.
    pub safe: bool,
    pub examples: Vec<Example>,
    /// In the channels of a mark call, the window of a call without `OVER`, or `None` for a
    /// function that needs one there too.
    pub default_window: Option<DefaultWindow>,
    /// Whether a mark's channel that holds a call takes the text of the call's first argument as
    /// the name of its column, as `bin(body_mass_g)` takes `body_mass_g`.
    pub titled_by_argument: bool,
    /// The order of its window that a window function reads, which decides where the default
    /// orders of a mark call's windows apply.
    pub window_order: WindowOrder,
}

/// The order of its window that a window function reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowOrder {
    /// No order: the function reads the rows of its frame as a set.
    #[default]
    None,
    /// Only the peers of the order, the rows that tie: `RANK` gives tied rows one rank.
    Peers,
    /// The order of the rows, as `ROW_NUMBER` and `LAG` do.
    Rows,
}

/// The window of a window function without `OVER` in the channels of a mark call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultWindow {
    /// `OVER ()`: the whole input of the `AGGREGATE`.
    Whole,
    /// `OVER position`: the groups and the position keys of the call.
    Position,
}

impl Function {
    fn new(name: &str, kind: FunctionKind, summary: &str, clauses: Clauses) -> Self {
        Self {
            name: name.to_string(),
            kind,
            family: Family::GoogleSql,
            summary: summary.to_string(),
            description: String::new(),
            notes: Vec::new(),
            params: Vec::new(),
            forms: Vec::new(),
            clauses,
            safe: false,
            examples: Vec::new(),
            default_window: None,
            titled_by_argument: false,
            window_order: WindowOrder::None,
        }
    }

    /// A scalar function, with its one-line summary.
    pub(crate) fn scalar(name: &str, summary: &str) -> Self {
        Self::new(name, FunctionKind::Scalar, summary, Clauses::NONE)
    }

    /// An aggregate function, which a call can also compute over a window.
    pub(crate) fn aggregate(name: &str, summary: &str) -> Self {
        let clauses = Clauses {
            having: true,
            over: Over::Optional,
            ..Clauses::NONE
        };
        Self::new(name, FunctionKind::Aggregate, summary, clauses)
    }

    /// A window function, whose `OVER` takes an `ORDER BY` as `order_by` says and a frame when
    /// `frame` holds.
    pub(crate) fn window(name: &str, summary: &str, order_by: Need, frame: bool) -> Self {
        let clauses = Clauses {
            over: Over::Required { order_by, frame },
            ..Clauses::NONE
        };
        Self::new(name, FunctionKind::Window, summary, clauses)
    }

    /// A table function, which a pipe `CALL` and `FROM` call.
    pub(crate) fn table(name: &str, summary: &str) -> Self {
        Self::new(name, FunctionKind::Table, summary, Clauses::NONE)
    }

    pub(crate) fn family(mut self, family: Family) -> Self {
        self.family = family;
        self
    }

    /// The window of a call without `OVER` in the channels of a mark call, and whether its
    /// channel's column takes the text of its first argument.
    pub(crate) fn in_marks(mut self, window: DefaultWindow, titled_by_argument: bool) -> Self {
        self.default_window = Some(window);
        self.titled_by_argument = titled_by_argument;
        self
    }

    /// The order of its window that the function reads.
    pub(crate) fn reads_order(mut self, order: WindowOrder) -> Self {
        self.window_order = order;
        self
    }

    /// What a call of the function computes, for highlighting: its family, or for a function of
    /// GoogleSQL, its kind.
    pub fn role(&self) -> FunctionRole {
        match self.family {
            Family::Mark => FunctionRole::Mark,
            Family::Constructor => FunctionRole::Constructor,
            Family::Transform => FunctionRole::Transform,
            Family::Composition => FunctionRole::Composition,
            Family::GoogleSql => self.kind.role(),
        }
    }

    pub(crate) fn description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    pub(crate) fn note(mut self, note: &str) -> Self {
        self.notes.push(note.to_string());
        self
    }

    pub(crate) fn param(mut self, name: &str, doc: &str) -> Self {
        self.params.push(Param {
            name: name.to_string(),
            doc: doc.to_string(),
            takes: Takes::default(),
        });
        self
    }

    /// A parameter of a chart function, with what it takes besides the type of its form.
    pub(crate) fn chart_param(mut self, name: &str, doc: &str, takes: Takes) -> Self {
        self = self.param(name, doc);
        self.params.last_mut().expect("the parameter").takes = takes;
        self
    }

    /// An aggregate that takes no `OVER` clause and no `HAVING MAX` or `HAVING MIN`.
    pub(crate) fn without_window(mut self) -> Self {
        self.clauses.over = Over::Forbidden;
        self.clauses.having = false;
        self
    }

    /// Lets calls write `*` in place of the arguments, which fills the first argument.
    pub(crate) fn star(mut self) -> Self {
        self.clauses.star = true;
        self
    }

    pub(crate) fn distinct(mut self) -> Self {
        self.clauses.distinct = true;
        self
    }

    pub(crate) fn null_treatment(mut self) -> Self {
        self.clauses.null_treatment = true;
        self
    }

    pub(crate) fn order_by(mut self) -> Self {
        self.clauses.order_by = true;
        self
    }

    pub(crate) fn limit(mut self) -> Self {
        self.clauses.limit = true;
        self
    }

    /// Lets calls write `SAFE.` before the name.
    pub(crate) fn safe(mut self) -> Self {
        self.safe = true;
        self
    }

    pub(crate) fn example(mut self, query: &str, result: &str) -> Self {
        self.examples.push(Example {
            query: query.to_string(),
            result: result.to_string(),
        });
        self
    }

    /// Adds the form that `declaration` declares, as [`form`] describes, with its mapping.
    pub(crate) fn form<R: Into<Planned>>(
        self,
        declaration: &str,
        mapping: impl Fn(&Call) -> Result<R, CallError> + Send + Sync + 'static,
    ) -> Self {
        let signature =
            Signature::parse(declaration).unwrap_or_else(|error| panic!("{}: {error}", self.name));
        self.with_signature(signature, mapping)
    }

    /// Adds the form of `signature`, with its mapping.
    pub(crate) fn with_signature<R: Into<Planned>>(
        mut self,
        signature: Signature,
        mapping: impl Fn(&Call) -> Result<R, CallError> + Send + Sync + 'static,
    ) -> Self {
        self.forms.push(Form {
            signature,
            mapping: Arc::new(move |call| mapping(call).map(Into::into)),
        });
        self
    }

    /// The forms as signature help shows them, each with the ranges of its arguments.
    pub fn signatures(&self) -> Vec<Label> {
        self.forms
            .iter()
            .map(|form| form.signature.label(&self.name))
            .collect()
    }

    /// The parameter named `name`, without regard to case.
    pub fn param_named(&self, name: &str) -> Option<&Param> {
        self.params
            .iter()
            .find(|param| param.name.eq_ignore_ascii_case(name))
    }

    /// Whether the parameter `name` takes one value of a set in some form, as `null_category` of
    /// `lump` does.
    pub fn takes_one_of(&self, name: &str) -> bool {
        !self.values_of(name).is_empty()
    }

    /// The values of the sets that the parameter `name` takes in the forms, each once, in the
    /// order of the forms, and the strings of a parameter of a chart function that takes
    /// constructors.
    pub fn values_of(&self, name: &str) -> Vec<&str> {
        let mut values: Vec<&str> = (self.param_named(name).into_iter())
            .flat_map(|param| &param.takes.strings)
            .map(String::as_str)
            .collect();
        for arg in (self.forms.iter()).flat_map(|form| &form.signature.args) {
            if let (true, Accept::OneOf(set)) = (arg.param.eq_ignore_ascii_case(name), &arg.accept)
            {
                for value in set {
                    if !values.contains(&value.as_str()) {
                        values.push(value);
                    }
                }
            }
        }
        values
    }

    /// Whether the argument at `position`, or the one named `name`, takes a date part in some
    /// form, so the pass reads it as a date part, as `MONTH` in `DATE_TRUNC(d, MONTH)`.
    pub fn takes_date_part(&self, position: usize, name: Option<&str>) -> bool {
        self.forms.iter().any(|form| {
            let args = &form.signature.args;
            let arg = match name {
                Some(name) => args.iter().find(|arg| arg.param.eq_ignore_ascii_case(name)),
                None => (form.signature.positional().nth(position)).map(|index| &args[index]),
            };
            matches!(
                arg,
                Some(FormArg {
                    accept: Accept::Part(_),
                    ..
                })
            )
        })
    }

    /// Plans a call of the arguments `args`, in the order that the call writes them, each with
    /// its name when it is a named argument. `star` is a call of `*`, as in `COUNT(*)`, and
    /// `safe` a call of `SAFE.`.
    pub fn plan(
        &self,
        args: Vec<(Option<String>, Arg)>,
        star: bool,
        safe: bool,
        context: PlanContext,
    ) -> Result<Planned, CallError> {
        self.plan_resolved(args, star, safe, context)
            .map(|(planned, _)| planned)
    }

    /// Plans a call as [`Function::plan`] does, and returns the form that resolution picked,
    /// with the types to which it coerced the arguments.
    pub fn plan_resolved(
        &self,
        args: Vec<(Option<String>, Arg)>,
        star: bool,
        safe: bool,
        context: PlanContext,
    ) -> Result<(Planned, Resolved), CallError> {
        if safe && !self.safe {
            return Err(CallError::call(format!(
                "SAFE. doesn't apply to {}",
                self.name
            )));
        }
        if star && !self.clauses.star {
            return Err(CallError::call(format!("{} takes no `*`", self.name)));
        }
        self.check_names(&args)?;
        let observed = args
            .iter()
            .map(|(_, arg)| observe(arg, context.schema))
            .collect::<Result<Vec<_>, _>>()?;
        // The cheapest form that the arguments fit, or the reasons that none fits.
        let mut best: Option<(usize, Vec<usize>, Resolution)> = None;
        let mut failures = Vec::new();
        let mut unresolved = 0;
        for (index, form) in self.forms.iter().enumerate() {
            let binding = match bind(&self.name, &form.signature, &args, star) {
                Ok(binding) => binding,
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            };
            let bound: Vec<(usize, Observed)> = (binding.iter().enumerate())
                .map(|(at, &arg)| (arg, observed[at].clone()))
                .collect();
            match form.signature.resolve(&bound) {
                Some(resolution) => {
                    if best
                        .as_ref()
                        .is_none_or(|(_, _, b)| resolution.cost < b.cost)
                    {
                        best = Some((index, binding, resolution));
                    }
                }
                None => unresolved += 1,
            }
        }
        let Some((index, binding, resolution)) = best else {
            return Err(self.no_form(failures, unresolved, &observed));
        };
        let form = &self.forms[index];
        let resolved = Resolved {
            form: index,
            args: (binding.iter().zip(&observed))
                .map(|(&slot, observed)| {
                    match (form.signature.arg_type(slot, &resolution), observed) {
                        (_, Observed::Part(part)) => part.name(),
                        (Some(Ty::Any), Observed::Value { ty: Ty::Null, .. }) => "INT64".into(),
                        (Some(Ty::Any), Observed::Value { ty, .. }) => ty.to_string(),
                        (Some(ty), _) => ty.to_string(),
                        (None, _) => "ANY".into(),
                    }
                })
                .collect(),
            result: form.signature.result_type(&resolution),
            enums: (binding.iter().enumerate())
                .filter_map(|(at, &slot)| {
                    let arg = &form.signature.args[slot];
                    matches!(arg.accept, Accept::OneOf(_)).then(|| (at, arg.param.clone()))
                })
                .collect(),
        };
        let call = self.call(form, args, &binding, resolution, star, safe, context)?;
        let planned = (form.mapping)(&call)?;
        Ok((self.typed(form, &call, planned)?, resolved))
    }

    /// The error of an argument named `name`, which names no parameter. The error of `order` on a
    /// mark without the channel, such as a symbol, says that `ORDER BY` gives the draw order of its
    /// rows.
    fn unknown_parameter(&self, name: &str) -> String {
        let unknown = format!("{} has no parameter named `{name}`", self.name);
        match self.family == Family::Mark && name.eq_ignore_ascii_case("order") {
            true => format!(
                "{unknown}: `order` orders the path of a line or an area, and the `ORDER BY` of \
                 the call gives the draw order of the rows, as in `{}(... ORDER BY k)`",
                self.name
            ),
            false => unknown,
        }
    }

    /// Checks that every name of `args` names a parameter, and that the named arguments come
    /// after the positional ones.
    fn check_names(&self, args: &[(Option<String>, Arg)]) -> Result<(), CallError> {
        let mut named_seen = false;
        for (at, (name, _)) in args.iter().enumerate() {
            match name {
                None if named_seen => {
                    return Err(CallError::arg(
                        at,
                        "a positional argument comes before the named arguments",
                    ))
                }
                None => {}
                Some(name) => {
                    named_seen = true;
                    if self.param_named(name).is_none() {
                        return Err(CallError::arg(at, self.unknown_parameter(name)));
                    }
                }
            }
        }
        Ok(())
    }

    /// The error of a call that fits no form: the error of binding, when every form fails at it
    /// the same way, or the forms and the types of the arguments. `failures` are the errors of
    /// binding, and `unresolved` the number of forms whose types the arguments don't fit.
    fn no_form(
        &self,
        failures: Vec<CallError>,
        unresolved: usize,
        observed: &[Observed],
    ) -> CallError {
        if let (0, Some(first)) = (unresolved, failures.first()) {
            if failures.iter().all(|error| error == first) || self.forms.len() == 1 {
                return first.clone();
            }
        }
        let types: Vec<String> = observed
            .iter()
            .map(|observed| match observed {
                Observed::Value { ty, .. } => ty.to_string(),
                Observed::Part(part) => part.name(),
            })
            .collect();
        CallError {
            no_form: true,
            ..CallError::call(format!(
                "no form of {} takes ({}); its forms are {}",
                self.name,
                types.join(", "),
                (self.signatures().into_iter())
                    .map(|label| label.text)
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        }
    }

    /// The call of `args` to `form`, with each value cast to its type in the form, after the
    /// checks of the arguments that take a literal, a set of strings, or a set of date parts.
    #[allow(clippy::too_many_arguments)]
    fn call<'a>(
        &self,
        form: &Form,
        args: Vec<(Option<String>, Arg)>,
        binding: &[usize],
        resolution: Resolution,
        star: bool,
        safe: bool,
        context: PlanContext<'a>,
    ) -> Result<Call<'a>, CallError> {
        let signature = &form.signature;
        let repeated = signature.repeated();
        let mut slots: Vec<Option<(usize, Arg)>> = vec![None; signature.args.len()];
        let mut rest = Vec::new();
        for (at, ((_, arg), slot)) in args.into_iter().zip(binding).enumerate() {
            let index = *slot;
            let form_arg = &signature.args[index];
            let arg = match arg {
                Arg::DatePart(part) => {
                    if let Accept::Part(set) = form_arg.accept {
                        if !set.contains(part) {
                            return Err(CallError::arg(
                                at,
                                format!(
                                    "{} takes {} as `{}`, not {}",
                                    self.name,
                                    set.names().join(", "),
                                    form_arg.param,
                                    part.name()
                                ),
                            ));
                        }
                    }
                    Arg::DatePart(part)
                }
                Arg::Value(expr) => {
                    match &form_arg.accept {
                        Accept::Const(_) if !matches!(expr, Expr::Literal(..)) => {
                            return Err(CallError::arg(
                                at,
                                format!("`{}` takes a literal", form_arg.param),
                            ))
                        }
                        // A parameter's value is checked by the function that reads it, when
                        // the query runs.
                        Accept::OneOf(_) if is_parameter(&expr) => {}
                        Accept::OneOf(values) => {
                            let value = match &expr {
                                Expr::Literal(value, _) => value.try_as_str().flatten(),
                                _ => None,
                            };
                            if !value.is_some_and(|value| values.iter().any(|v| v == value)) {
                                let values: Vec<String> =
                                    values.iter().map(|v| format!("'{v}'")).collect();
                                return Err(CallError::arg(
                                    at,
                                    format!(
                                        "`{}` takes one of {}",
                                        form_arg.param,
                                        values.join(", ")
                                    ),
                                ));
                            }
                        }
                        _ => {}
                    }
                    let ty = signature.arg_type(index, &resolution);
                    Arg::Value(coerce(expr, ty, &context)?)
                }
            };
            match Some(index) == repeated {
                true => rest.push((at, arg)),
                false => slots[index] = Some((at, arg)),
            }
        }
        Ok(Call {
            slots,
            rest,
            resolution,
            safe,
            star,
            context,
        })
    }

    /// `planned` with the type of the form's result.
    fn typed(&self, form: &Form, call: &Call, planned: Planned) -> Result<Planned, CallError> {
        let want = form.signature.result_type(&call.resolution);
        if want == Ty::Any {
            return Ok(planned);
        }
        let options = call.context.type_options;
        let differs = |ty: &DataType| !exact(ty, &want, options);
        let fields = |args: &[Expr]| -> Result<Vec<FieldRef>, CallError> {
            args.iter()
                .map(|arg| Ok(arg.to_field(call.context.schema)?.1))
                .collect()
        };
        Ok(match planned {
            Planned::Scalar(expr) => {
                let ty = call.type_of(&expr)?;
                match differs(&ty) {
                    true => Planned::Scalar(cast(expr, want.arrow(options))),
                    false => Planned::Scalar(expr),
                }
            }
            Planned::Aggregate {
                func, args, filter, ..
            } => {
                let ty = func.return_field(&fields(&args)?)?.data_type().clone();
                let result = differs(&ty).then(|| want.arrow(options));
                Planned::Aggregate {
                    func,
                    args,
                    filter,
                    result,
                }
            }
            Planned::Window { func, args, .. } => {
                let inputs = fields(&args)?;
                let ty = match &func {
                    WindowFunctionDefinition::WindowUDF(udwf) => udwf
                        .field(WindowUDFFieldArgs::new(&inputs, &self.name))?
                        .data_type()
                        .clone(),
                    WindowFunctionDefinition::AggregateUDF(udaf) => {
                        udaf.return_field(&inputs)?.data_type().clone()
                    }
                };
                let result = differs(&ty).then(|| want.arrow(options));
                Planned::Window { func, args, result }
            }
        })
    }
}

/// What resolution knows of the argument `arg`.
fn observe(arg: &Arg, schema: &DFSchema) -> Result<Observed, CallError> {
    match arg {
        Arg::DatePart(part) => Ok(Observed::Part(*part)),
        Arg::Value(expr) => {
            let ty = expr.get_type(schema)?;
            let ty = observed_type(expr, &ty)
                .ok_or_else(|| CallError::call(format!("avenger-sql has no type for {ty}")))?;
            Ok(Observed::Value {
                ty,
                literal: matches!(expr, Expr::Literal(..)),
            })
        }
    }
}

/// The result `out` of an internal function of the arguments `args`: a scalar when every argument
/// is a scalar, and otherwise the array.
pub(crate) fn columnar(
    out: datafusion::arrow::array::ArrayRef,
    args: &[datafusion::logical_expr::ColumnarValue],
) -> datafusion::common::Result<datafusion::logical_expr::ColumnarValue> {
    use datafusion::logical_expr::ColumnarValue;
    match args
        .iter()
        .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
    {
        true => Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(&out, 0)?)),
        false => Ok(ColumnarValue::Array(out)),
    }
}

/// The type of `expr`, of the Arrow type `ty`, as resolution sees it: the empty array literal `[]`
/// is an `ARRAY<NULL>`, which coerces to any array, as GoogleSQL's does, though its own type is
/// `ARRAY<INT64>`.
pub(crate) fn observed_type(expr: &Expr, ty: &DataType) -> Option<Ty> {
    if let Expr::Literal(ScalarValue::List(list), _) = expr {
        use datafusion::arrow::array::Array;
        if list.len() == 1 && list.value_length(0) == 0 && !list.is_null(0) {
            return Some(Ty::Array(Box::new(Ty::Null)));
        }
    }
    Ty::of_arrow(ty)
}

/// The empty array literal `[]`, an `ARRAY<INT64>`, which [`observed_type`] sees as an array of
/// any type.
pub(crate) fn empty_array() -> Expr {
    Expr::Literal(
        ScalarValue::List(ScalarValue::new_list_nullable(&[], &DataType::Int64)),
        None,
    )
}

/// The argument of `signature` that each argument of `args` binds to, or the error of binding.
fn bind(
    name: &str,
    signature: &Signature,
    args: &[(Option<String>, Arg)],
    star: bool,
) -> Result<Vec<usize>, CallError> {
    let form_args = &signature.args;
    let repeated = signature.repeated();
    let positional: Vec<usize> = signature.positional().collect();
    let mut filled = vec![false; form_args.len()];
    let mut binding = Vec::with_capacity(args.len());
    let mut next = 0;
    for (at, (arg_name, _)) in args.iter().enumerate() {
        let index = match arg_name {
            None => match (positional.get(next), repeated) {
                (Some(&index), _) => {
                    next += 1;
                    index
                }
                (None, Some(index)) => index,
                (None, None) => {
                    return Err(CallError::arg(
                        at,
                        format!(
                            "{name} takes {} positional argument{}",
                            positional.len(),
                            if positional.len() == 1 { "" } else { "s" }
                        ),
                    ))
                }
            },
            Some(arg_name) => {
                let Some(index) = (0..form_args.len()).find(|&index| {
                    Some(index) != repeated && form_args[index].param.eq_ignore_ascii_case(arg_name)
                }) else {
                    return Err(CallError::arg(
                        at,
                        format!("this form of {name} takes no `{arg_name}`"),
                    ));
                };
                if filled[index] {
                    return Err(CallError::arg(
                        at,
                        format!(
                            "the parameter `{}` has two arguments",
                            form_args[index].param
                        ),
                    ));
                }
                index
            }
        };
        filled[index] = true;
        binding.push(index);
    }
    let star_fills = star && repeated != Some(0) && !form_args.is_empty();
    for (index, arg) in form_args.iter().enumerate() {
        if Some(index) != repeated
            && arg.arity == Arity::Required
            && !filled[index]
            && !(star_fills && index == 0)
        {
            return Err(CallError::call(format!(
                "{name} needs its argument `{}`",
                arg.param
            )));
        }
    }
    Ok(binding)
}

/// `expr` cast to the type `ty`, when its type is another.
fn coerce(expr: Expr, ty: Option<Ty>, context: &PlanContext) -> Result<Expr, CallError> {
    let Some(ty) = ty.filter(|ty| *ty != Ty::Any) else {
        return Ok(expr);
    };
    let current = expr.get_type(context.schema)?;
    if exact(&current, &ty, context.type_options) {
        return Ok(expr);
    }
    // A `NULL` of an array or a struct takes its type from the cast below.
    Ok(cast(expr, ty.arrow(context.type_options)))
}

/// Whether a value of the Arrow type `current` has the type `ty` as the library maps it to Arrow:
/// a scalar type exactly, as an `INT32` column isn't an `INT64` until a cast, bytes and strings of
/// any layout, and an array, a struct or an interval as its GoogleSQL type.
pub(crate) fn exact(current: &DataType, ty: &Ty, options: TypeOptions) -> bool {
    match ty {
        // Bytes and strings of any layout are `BYTES` and `STRING` as they are: the charts and
        // fragments of avenger's functions keep their views and the extensions of their fields,
        // and a column keeps its layout, so an argument stays the column that an `ORDER BY` of
        // the call names.
        Ty::Bytes | Ty::Chart => matches!(
            current,
            DataType::Binary | DataType::BinaryView | DataType::LargeBinary
        ),
        Ty::String => matches!(
            current,
            DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8
        ),
        Ty::Array(_) | Ty::Struct(_) | Ty::Interval | Ty::Any | Ty::Var(_) | Ty::Null => {
            Ty::of_arrow(current).as_ref() == Some(ty)
        }
        _ => *current == ty.arrow(options),
    }
}

/// The form that resolution picked for a call: its index among the function's forms, the type to
/// which it coerced each argument, in the order of the call, and the type of its result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub form: usize,
    pub args: Vec<String>,
    pub result: Ty,
    /// The arguments that take one value of a set, as `offset => 'normalize'` of `stack`: their
    /// positions in the call and the names of their parameters.
    pub enums: Vec<(usize, String)>,
}

impl Resolved {
    /// The form as GoogleSQL's resolved tree writes it, such as `$add(DATE, INT64) -> DATE`.
    pub fn render(&self, name: &str) -> String {
        format!("{name}({}) -> {}", self.args.join(", "), self.result)
    }
}

/// GoogleSQL's functions and avenger's, by name.
pub struct Library {
    functions: Vec<Function>,
    index: HashMap<String, usize>,
    /// The operators, by their names in GoogleSQL's resolved tree, such as `$add`.
    operators: HashMap<String, Function>,
}

impl fmt::Debug for Library {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Library")
            .field("functions", &self.functions.len())
            .finish()
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

impl Library {
    /// The library of GoogleSQL's functions and avenger's: the marks, constructors, `scaled`,
    /// and `direct` of avenger-chart, and the transforms `bin`, `stack`, and `timeunit`.
    pub fn new() -> Self {
        let mut library = Self {
            functions: Vec::new(),
            index: HashMap::new(),
            operators: (operator::operators().into_iter())
                .map(|function| (function.name.clone(), function))
                .collect(),
        };
        for function in (aggregate::functions().into_iter())
            .chain(aggregate::window_functions())
            .chain(array::functions())
            .chain(conditional::functions())
            .chain(datetime::functions())
            .chain(match_recognize::functions())
            .chain(math::functions())
            .chain(string::functions())
            .chain(avenger::functions())
        {
            library.add(function);
        }
        library
    }

    fn add(&mut self, function: Function) {
        let key = function.name.to_ascii_uppercase();
        assert!(
            !self.index.contains_key(&key),
            "the library has two functions named {}",
            function.name
        );
        self.index.insert(key, self.functions.len());
        self.functions.push(function);
    }

    /// The function named `name`, without regard to case.
    pub fn get(&self, name: &str) -> Option<&Function> {
        self.index
            .get(&name.to_ascii_uppercase())
            .map(|&index| &self.functions[index])
    }

    /// The operator named `name` in GoogleSQL's resolved tree, such as `$add` for `+`.
    pub fn operator(&self, name: &str) -> Option<&Function> {
        self.operators.get(name)
    }

    /// The functions of the library, in the order of their groups.
    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.functions.iter()
    }
}

// --- Helpers of the mappings ---

pub(crate) fn lit_str(value: &str) -> Expr {
    Expr::Literal(ScalarValue::Utf8(Some(value.to_string())), None)
}

pub(crate) fn lit_i64(value: i64) -> Expr {
    Expr::Literal(ScalarValue::Int64(Some(value)), None)
}

pub(crate) fn lit_f64(value: f64) -> Expr {
    Expr::Literal(ScalarValue::Float64(Some(value)), None)
}

pub(crate) fn null() -> Expr {
    Expr::Literal(ScalarValue::Null, None)
}

pub(crate) use datafusion::logical_expr::cast;

/// `value`, or `NULL` when any of `args` is `NULL`, as GoogleSQL's functions return.
pub(crate) fn null_if_any_null(args: &[Expr], value: Expr) -> Expr {
    let any_null = (args.iter())
        .map(|arg| arg.clone().is_null())
        .reduce(|a, b| a.or(b));
    match any_null {
        Some(condition) => null_when(condition, value),
        None => value,
    }
}

/// `value`, or `NULL` when `condition` holds, for the `SAFE.` form of a function.
pub(crate) fn null_when(condition: Expr, value: Expr) -> Expr {
    datafusion::logical_expr::when(condition, null())
        .otherwise(value)
        .expect("a CASE of one branch")
}

/// Whether `expr` is a parameter of the plan: a placeholder, such as a script variable or a
/// query parameter, which the plan binds before it runs.
pub(crate) fn is_parameter(expr: &Expr) -> bool {
    matches!(expr, Expr::Placeholder(_))
}
