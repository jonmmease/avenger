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
pub(crate) mod cast_format;
mod conditional;
mod datetime;
pub(crate) mod error;
pub mod form;
pub(crate) mod interval;
mod like;
pub(crate) mod match_recognize;
mod math;
mod number_format;
mod operator;
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
pub(crate) use cast_format::{cast_format, cast_to_string};
pub(crate) use datetime::extract;
pub use datetime::{DatePart, Weekday};
pub use form::{Accept, Arity, FormArg, PartSet, Signature, Ty};
pub use reference::reference;

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
}

impl CallError {
    pub(crate) fn call(message: impl Into<String>) -> Self {
        Self {
            at: ErrorAt::Call,
            message: message.into(),
        }
    }

    fn arg(index: usize, message: impl Into<String>) -> Self {
        Self {
            at: ErrorAt::Arg(index),
            message: message.into(),
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
        /// The type of the form's result, when it differs from the aggregate's, as `STRING_AGG`
        /// returns a `STRING`. The pass casts the aggregate's column to it after the aggregate.
        result: Option<DataType>,
    },
    Window {
        func: WindowFunctionDefinition,
        args: Vec<Expr>,
        /// The type of the form's result, when it differs from the function's, as `ROW_NUMBER`
        /// returns an `INT64`. The pass casts the window's column to it after the window.
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

    /// What a call of the function computes, for highlighting: its family, or for a function of
    /// GoogleSQL, its kind.
    pub fn role(&self) -> FunctionRole {
        match self.family {
            Family::Mark => FunctionRole::Mark,
            Family::Constructor => FunctionRole::Constructor,
            Family::Transform => FunctionRole::Transform,
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
        });
        self
    }

    /// An aggregate that takes no `OVER` clause and no `HAVING MAX` or `HAVING MIN`.
    pub(crate) fn without_window(mut self) -> Self {
        self.clauses.over = Over::Forbidden;
        self.clauses.having = false;
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

    /// The forms as signature help shows them.
    pub fn signatures(&self) -> Vec<String> {
        self.forms
            .iter()
            .map(|form| form.signature.render(&self.name))
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
        (self.forms.iter())
            .flat_map(|form| &form.signature.args)
            .any(|arg| {
                arg.param.eq_ignore_ascii_case(name) && matches!(arg.accept, Accept::OneOf(_))
            })
    }

    /// Whether the argument at `position`, or the one named `name`, takes a date part in some
    /// form, so the pass reads it as a date part, as `MONTH` in `DATE_TRUNC(d, MONTH)`.
    pub fn takes_date_part(&self, position: usize, name: Option<&str>) -> bool {
        self.forms.iter().any(|form| {
            let args = &form.signature.args;
            let arg = match name {
                Some(name) => args.iter().find(|arg| arg.param.eq_ignore_ascii_case(name)),
                None => args
                    .iter()
                    .filter(|arg| !arg.named_only && arg.arity != Arity::Repeated)
                    .nth(position),
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
        if star && self.name != "COUNT" {
            return Err(CallError::call(format!("{} takes no `*`", self.name)));
        }
        self.check_names(&args)?;
        let observed = args
            .iter()
            .map(|(_, arg)| observe(arg, context.schema))
            .collect::<Result<Vec<_>, _>>()?;
        // The cheapest form that the arguments fit, or the reasons that none fits.
        let mut best: Option<(usize, Vec<Option<usize>>, Resolution)> = None;
        let mut failures = Vec::new();
        for (index, form) in self.forms.iter().enumerate() {
            let binding = match bind(&self.name, &form.signature, &args, star) {
                Ok(binding) => binding,
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            };
            let bound: Vec<(usize, Observed)> = (binding.iter().enumerate())
                .filter_map(|(at, slot)| slot.map(|arg| (arg, observed[at].clone())))
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
                None => failures.push(CallError::call(String::new())),
            }
        }
        let Some((index, binding, resolution)) = best else {
            return Err(self.no_form(failures, &observed));
        };
        let form = &self.forms[index];
        let resolved = Resolved {
            form: index,
            args: (binding.iter().zip(&observed))
                .map(|(slot, observed)| {
                    let slot = slot.expect("a bound argument");
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
                .filter_map(|(at, slot)| {
                    let arg = &form.signature.args[slot.expect("a bound argument")];
                    matches!(arg.accept, Accept::OneOf(_)).then(|| (at, arg.param.clone()))
                })
                .collect(),
        };
        let call = self.call(form, args, &binding, resolution, star, safe, context)?;
        let planned = (form.mapping)(&call)?;
        Ok((self.typed(form, &call, planned)?, resolved))
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
                        return Err(CallError::arg(
                            at,
                            format!("{} has no parameter named `{name}`", self.name),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// The error of a call that fits no form: the error of binding, when every form fails at it
    /// the same way, or the forms and the types of the arguments.
    fn no_form(&self, failures: Vec<CallError>, observed: &[Observed]) -> CallError {
        let binding: Vec<&CallError> = failures
            .iter()
            .filter(|error| !error.message.is_empty())
            .collect();
        if binding.len() == failures.len() && !binding.is_empty() {
            let first = binding[0];
            if binding.iter().all(|error| *error == first) || self.forms.len() == 1 {
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
        CallError::call(format!(
            "no form of {} takes ({}); its forms are {}",
            self.name,
            types.join(", "),
            self.signatures().join("; ")
        ))
    }

    /// The call of `args` to `form`, with each value cast to its type in the form, after the
    /// checks of the arguments that take a literal, a set of strings, or a set of date parts.
    #[allow(clippy::too_many_arguments)]
    fn call<'a>(
        &self,
        form: &Form,
        args: Vec<(Option<String>, Arg)>,
        binding: &[Option<usize>],
        resolution: Resolution,
        star: bool,
        safe: bool,
        context: PlanContext<'a>,
    ) -> Result<Call<'a>, CallError> {
        let signature = &form.signature;
        let repeated = signature
            .args
            .last()
            .is_some_and(|arg| arg.arity == Arity::Repeated);
        let fixed = signature.args.len() - usize::from(repeated);
        let mut slots: Vec<Option<(usize, Arg)>> = vec![None; fixed];
        let mut rest = Vec::new();
        for (at, ((_, arg), slot)) in args.into_iter().zip(binding).enumerate() {
            let index = slot.expect("a bound argument");
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
            match index < fixed {
                true => slots[index] = Some((at, arg)),
                false => rest.push((at, arg)),
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
                func,
                args,
                filter,
                result,
            } => {
                let ty = func.return_field(&fields(&args)?)?.data_type().clone();
                let result = result.or_else(|| differs(&ty).then(|| want.arrow(options)));
                Planned::Aggregate {
                    func,
                    args,
                    filter,
                    result,
                }
            }
            Planned::Window { func, args, result } => {
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
                let result = result.or_else(|| differs(&ty).then(|| want.arrow(options)));
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
) -> Result<Vec<Option<usize>>, CallError> {
    let form_args = &signature.args;
    let repeated = form_args
        .last()
        .is_some_and(|arg| arg.arity == Arity::Repeated);
    let fixed = form_args.len() - usize::from(repeated);
    let positional: Vec<usize> = (0..fixed)
        .filter(|&index| !form_args[index].named_only)
        .collect();
    let mut filled = vec![false; form_args.len()];
    let mut binding = Vec::with_capacity(args.len());
    let mut next = 0;
    for (at, (arg_name, _)) in args.iter().enumerate() {
        let index = match arg_name {
            None => match positional.get(next) {
                Some(&index) => {
                    next += 1;
                    index
                }
                None if repeated => fixed,
                None => {
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
                let Some(index) =
                    (0..fixed).find(|&index| form_args[index].param.eq_ignore_ascii_case(arg_name))
                else {
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
        binding.push(Some(index));
    }
    let star_fills = star && fixed > 0;
    for (index, arg) in form_args.iter().enumerate().take(fixed) {
        if arg.arity == Arity::Required && !filled[index] && !(star_fills && index == 0) {
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
        Ty::Bytes => matches!(
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

    /// The text of the operator `name`, when GoogleSQL rejects a literal `NULL` as its operand,
    /// as it does for `+` and `=`.
    pub fn rejects_null(name: &str) -> Option<&'static str> {
        operator::rejects_null(name)
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

pub(crate) fn cast(expr: Expr, ty: DataType) -> Expr {
    Expr::Cast(datafusion::logical_expr::Cast::new(Box::new(expr), ty))
}

/// `value`, or `NULL` when any of `args` is `NULL`, as GoogleSQL's functions return.
pub(crate) fn null_if_any_null(args: &[Expr], value: Expr) -> Expr {
    let any_null = args
        .iter()
        .map(|arg| arg.clone().is_null())
        .reduce(|a, b| a.or(b));
    match any_null {
        Some(condition) => datafusion::logical_expr::when(condition, null())
            .otherwise(value)
            .expect("a CASE of one branch"),
        None => value,
    }
}

/// `value`, or `NULL` when `condition` holds, for the `SAFE.` form of a function.
pub(crate) fn null_when(condition: Expr, value: Expr) -> Expr {
    datafusion::logical_expr::when(condition, null())
        .otherwise(value)
        .expect("a CASE of one branch")
}
