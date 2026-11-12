//! The function library: GoogleSQL's functions, and avenger's own, mapped onto DataFusion.
//!
//! The pass resolves the name of each call against the [`Library`]. A [`Function`] has a
//! signature of named parameters, which binds the arguments of a call, positional or named as in
//! `bin(x, maxbins => 30)`, and a mapping from the bound arguments to DataFusion:
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
//! default to UTC, as in BigQuery. Where GoogleSQL fails at run time and DataFusion returns `NaN`
//! or infinity, as for `SQRT(-1)`, the mapping returns DataFusion's value.

mod aggregate;
mod array;
mod avenger;
mod conditional;
mod datetime;
mod math;
mod string;

use std::{collections::HashMap, fmt, sync::Arc};

use datafusion::{
    arrow::datatypes::DataType,
    common::{DFSchema, ScalarValue},
    logical_expr::{AggregateUDF, Expr, ExprSchemable, WindowFunctionDefinition},
};

pub use array::Subscript;
pub use datetime::{DatePart, Weekday};

/// What a call of a function computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionKind {
    /// A value of each row.
    Scalar,
    /// A value of the rows of a group, or of a window with `OVER`.
    Aggregate,
    /// A value of each row over a window, with `OVER`.
    Window,
}

/// What a parameter takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamKind {
    /// An expression.
    Value,
    /// A date part, such as `MONTH` or `WEEK(MONDAY)`.
    DatePart,
}

/// How many arguments a parameter takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arity {
    Required,
    Optional,
    /// Any number of arguments, at the end of the parameters.
    Repeated,
}

/// A parameter of a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub kind: ParamKind,
    pub arity: Arity,
    /// Whether an argument takes this parameter only by its name, as the options of `bin` do.
    pub named_only: bool,
}

impl Param {
    fn new(name: &str, arity: Arity) -> Self {
        Self {
            name: name.to_string(),
            kind: ParamKind::Value,
            arity,
            named_only: false,
        }
    }

    pub(crate) fn required(name: &str) -> Self {
        Self::new(name, Arity::Required)
    }

    pub(crate) fn optional(name: &str) -> Self {
        Self::new(name, Arity::Optional)
    }

    pub(crate) fn repeated(name: &str) -> Self {
        Self::new(name, Arity::Repeated)
    }

    pub(crate) fn date_part(name: &str) -> Self {
        Self {
            kind: ParamKind::DatePart,
            ..Self::required(name)
        }
    }

    pub(crate) fn named(mut self) -> Self {
        self.named_only = true;
        self
    }
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
        /// The type of GoogleSQL's result, when it differs from the aggregate's, as `STRING_AGG`
        /// returns a `STRING`. The pass casts the aggregate's column to it after the aggregate.
        result: Option<DataType>,
    },
    Window {
        func: WindowFunctionDefinition,
        args: Vec<Expr>,
        /// The type of GoogleSQL's result, when it differs from the function's, as `ROW_NUMBER`
        /// returns an `INT64`. The pass casts the window's column to it after the window.
        result: Option<DataType>,
    },
}

/// What a mapping knows of the place of its call.
#[derive(Clone, Copy, Debug)]
pub struct PlanContext<'a> {
    /// The schema of the input of the call, for the types of its arguments.
    pub schema: &'a DFSchema,
    /// The default time zone of the functions that take one.
    pub time_zone: &'a str,
}

/// The arguments of a call, bound to the parameters of its function.
pub struct Call<'a> {
    /// The argument of each parameter, other than a repeated one, and the index at which the
    /// call writes it.
    slots: Vec<Option<(usize, Arg)>>,
    /// The arguments of the repeated parameter, with their indices.
    rest: Vec<(usize, Arg)>,
    /// Whether the call writes `SAFE.` before the function's name.
    pub safe: bool,
    /// Whether the call's only argument is `*`, as in `COUNT(*)`.
    pub star: bool,
    pub context: PlanContext<'a>,
}

impl<'a> Call<'a> {
    /// The value of the parameter at `index`, when the call gives one.
    pub(crate) fn value(&self, index: usize) -> Option<&Expr> {
        match self.slots.get(index)? {
            Some((_, Arg::Value(expr))) => Some(expr),
            _ => None,
        }
    }

    /// The value of the required parameter at `index`, which binding guarantees.
    pub(crate) fn arg(&self, index: usize) -> Expr {
        self.value(index)
            .cloned()
            .expect("binding gives each required parameter a value")
    }

    /// The date part of the parameter at `index`.
    pub(crate) fn date_part(&self, index: usize) -> Option<DatePart> {
        match self.slots.get(index)? {
            Some((_, Arg::DatePart(part))) => Some(*part),
            _ => None,
        }
    }

    /// The values of the repeated parameter, or of every parameter for a function whose only
    /// parameter repeats.
    pub(crate) fn rest(&self) -> Vec<Expr> {
        self.rest
            .iter()
            .filter_map(|(_, arg)| match arg {
                Arg::Value(expr) => Some(expr.clone()),
                Arg::DatePart(_) => None,
            })
            .collect()
    }

    /// The values of all the parameters, in their order, with `None` for an omitted one.
    pub(crate) fn values(&self) -> Vec<Option<Expr>> {
        (0..self.slots.len())
            .map(|index| self.value(index).cloned())
            .collect()
    }

    /// The type of `expr` in the call's input.
    pub(crate) fn type_of(&self, expr: &Expr) -> Result<DataType, CallError> {
        Ok(expr.get_type(self.context.schema)?)
    }

    /// The type of the value of the parameter at `index`.
    pub(crate) fn type_at(&self, index: usize) -> Result<DataType, CallError> {
        self.type_of(&self.arg(index))
    }

    /// An error at the argument of the parameter at `index`, or at the call when the call gives
    /// it no argument.
    pub(crate) fn error_at(&self, index: usize, message: impl Into<String>) -> CallError {
        match self.slots.get(index).and_then(|slot| slot.as_ref()) {
            Some((at, _)) => CallError::arg(*at, message),
            None => CallError::call(message),
        }
    }
}

/// The mapping of a function.
type Mapping = Box<dyn Fn(&Call) -> Result<Planned, CallError> + Send + Sync>;

/// A function of the library.
pub struct Function {
    pub name: String,
    pub kind: FunctionKind,
    pub params: Vec<Param>,
    /// Whether a call can write `SAFE.` before the name.
    pub safe: bool,
    /// What the function computes, for completion and hover, when the library has it.
    pub doc: Option<String>,
    pub(crate) mapping: Mapping,
}

impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("params", &self.params)
            .field("safe", &self.safe)
            .finish_non_exhaustive()
    }
}

impl Function {
    pub(crate) fn new(
        name: &str,
        kind: FunctionKind,
        params: Vec<Param>,
        mapping: impl Fn(&Call) -> Result<Planned, CallError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.to_string(),
            kind,
            params,
            safe: false,
            doc: None,
            mapping: Box::new(mapping),
        }
    }

    /// A scalar function whose mapping returns an expression.
    pub(crate) fn scalar(
        name: &str,
        params: Vec<Param>,
        mapping: impl Fn(&Call) -> Result<Expr, CallError> + Send + Sync + 'static,
    ) -> Self {
        Self::new(name, FunctionKind::Scalar, params, move |call| {
            mapping(call).map(Planned::Scalar)
        })
    }

    /// Lets calls write `SAFE.` before the name.
    pub(crate) fn safe(mut self) -> Self {
        self.safe = true;
        self
    }

    pub(crate) fn with_doc(mut self, doc: impl Into<String>) -> Self {
        self.doc = Some(doc.into());
        self
    }

    /// The kind of argument that the argument at `position`, or the one named `name`, takes,
    /// so the pass can plan it. `None` when no parameter takes it.
    pub fn param_kind(&self, position: usize, name: Option<&str>) -> Option<ParamKind> {
        let param = match name {
            Some(name) => self.param_named(name).map(|index| &self.params[index]),
            None => (self.params.iter())
                .filter(|param| !param.named_only && param.arity != Arity::Repeated)
                .nth(position)
                .or_else(|| (self.params.last()).filter(|param| param.arity == Arity::Repeated)),
        }?;
        Some(param.kind)
    }

    fn param_named(&self, name: &str) -> Option<usize> {
        self.params
            .iter()
            .position(|param| param.name.eq_ignore_ascii_case(name))
    }

    /// Binds the arguments `args`, in the order that the call writes them, each with its name
    /// when it is a named argument, and plans the call. `star` is a call of `*`, as in
    /// `COUNT(*)`, and `safe` a call of `SAFE.`.
    pub fn plan(
        &self,
        args: Vec<(Option<String>, Arg)>,
        star: bool,
        safe: bool,
        context: PlanContext,
    ) -> Result<Planned, CallError> {
        if safe && !self.safe {
            return Err(CallError::call(format!(
                "SAFE. doesn't apply to {}",
                self.name
            )));
        }
        if star && self.name != "COUNT" {
            return Err(CallError::call(format!("{} takes no `*`", self.name)));
        }
        let call = self.bind(args, star, safe, context)?;
        (self.mapping)(&call)
    }

    fn bind<'a>(
        &self,
        args: Vec<(Option<String>, Arg)>,
        star: bool,
        safe: bool,
        context: PlanContext<'a>,
    ) -> Result<Call<'a>, CallError> {
        let repeated = self
            .params
            .last()
            .is_some_and(|param| param.arity == Arity::Repeated);
        let fixed = self.params.len() - usize::from(repeated);
        let mut slots: Vec<Option<(usize, Arg)>> = vec![None; fixed];
        let mut rest = Vec::new();
        let positional: Vec<usize> = (0..fixed)
            .filter(|&index| !self.params[index].named_only)
            .collect();
        let mut next = 0;
        let mut named_seen = false;
        for (at, (name, arg)) in args.into_iter().enumerate() {
            match name {
                None => {
                    if named_seen {
                        return Err(CallError::arg(
                            at,
                            "a positional argument comes before the named arguments",
                        ));
                    }
                    if let Some(&index) = positional.get(next) {
                        slots[index] = Some((at, arg));
                        next += 1;
                    } else if repeated {
                        rest.push((at, arg));
                    } else {
                        return Err(CallError::arg(
                            at,
                            format!(
                                "{} takes {} positional argument{}",
                                self.name,
                                positional.len(),
                                if positional.len() == 1 { "" } else { "s" }
                            ),
                        ));
                    }
                }
                Some(name) => {
                    named_seen = true;
                    let Some(index) = self.param_named(&name).filter(|&index| index < fixed) else {
                        return Err(CallError::arg(
                            at,
                            format!("{} has no parameter named `{name}`", self.name),
                        ));
                    };
                    if slots[index].is_some() {
                        return Err(CallError::arg(
                            at,
                            format!(
                                "the parameter `{}` has two arguments",
                                self.params[index].name
                            ),
                        ));
                    }
                    slots[index] = Some((at, arg));
                }
            }
        }
        for (index, param) in self.params.iter().take(fixed).enumerate() {
            match (&slots[index], param.arity) {
                (None, Arity::Required) => {
                    return Err(CallError::call(format!(
                        "{} needs its argument `{}`",
                        self.name, param.name
                    )))
                }
                (Some((at, arg)), _) => {
                    let fits = matches!(
                        (arg, param.kind),
                        (Arg::Value(_), ParamKind::Value) | (Arg::DatePart(_), ParamKind::DatePart)
                    );
                    if !fits {
                        let what = match param.kind {
                            ParamKind::Value => "a value",
                            ParamKind::DatePart => "a date part, such as MONTH",
                        };
                        return Err(CallError::arg(
                            *at,
                            format!("the argument `{}` is {what}", param.name),
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(Call {
            slots,
            rest,
            safe,
            star,
            context,
        })
    }
}

/// GoogleSQL's functions and avenger's, by name.
pub struct Library {
    functions: Vec<Function>,
    index: HashMap<String, usize>,
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
        };
        for function in (aggregate::functions().into_iter())
            .chain(aggregate::window_functions())
            .chain(array::functions())
            .chain(conditional::functions())
            .chain(datetime::functions())
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

pub(crate) fn is_integer(ty: &DataType) -> bool {
    ty.is_integer()
}

/// `expr` as `FLOAT64` when it is an integer, as GoogleSQL computes math of integers.
pub(crate) fn float_of_integer(call: &Call, expr: Expr) -> Result<Expr, CallError> {
    Ok(match is_integer(&call.type_of(&expr)?) {
        true => cast(expr, DataType::Float64),
        false => expr,
    })
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
