//! The forms of a function: its valid combinations of arguments, with their types, and the
//! resolution of a call's form.
//!
//! A form is declared as text in the shape of GoogleSQL's listing of signatures, with names:
//!
//! ```text
//! (value: T, position: INT64[, length: INT64]) -> T where T: STRING | BYTES
//! ```
//!
//! An argument in brackets is optional, `name: TYPE...` repeats, and `name => TYPE` takes its
//! argument only by name. A type is a GoogleSQL type, `ARRAY<...>`, `STRUCT<name TYPE, ...>`,
//! `ANY`, or a type variable that the `where` clause gives a set of types. An argument can also
//! take `PART(DATE)`, `PART(DATETIME)` or `PART(FIXED)`, a date part of the set; `CONST TYPE`, a
//! literal of the type; or `ONEOF('a', 'b')`, one of the strings.
//!
//! Resolution follows GoogleSQL's coercion. A literal coerces as its value can, as a string
//! literal to a `DATE`; `NULL` fits any type; `INT64` widens to `FLOAT64`, `NUMERIC` and
//! `BIGNUMERIC`, in that order of preference; `NUMERIC` to `BIGNUMERIC` and `FLOAT64`; and `DATE`
//! to `DATETIME`. A type variable takes the type of its set that its arguments coerce to at the
//! least cost, and a call takes the form of the least cost.

use std::{collections::BTreeMap, fmt, sync::Arc};

use datafusion::arrow::datatypes::{DataType, Field, Fields, IntervalUnit, TimeUnit};

use super::DatePart;
use crate::types::{self, TypeOptions};

/// A type of a form.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Int64,
    Float64,
    Numeric,
    BigNumeric,
    Bool,
    String,
    Bytes,
    Date,
    Time,
    Datetime,
    Timestamp,
    Interval,
    Array(Box<Ty>),
    Struct(Vec<(String, Ty)>),
    /// A type variable of the form.
    Var(String),
    /// Any type.
    Any,
    /// The type of `NULL`, of an argument.
    Null,
    /// A table, the result of a table function.
    Table,
}

impl Ty {
    /// The type of a value of the Arrow type `ty`, or `None` for a type that GoogleSQL lacks.
    /// Integers and floats of any width are `INT64` and `FLOAT64`, and every string type is a
    /// `STRING`.
    pub fn of_arrow(ty: &DataType) -> Option<Ty> {
        let ty = match ty {
            DataType::Null => Ty::Null,
            DataType::Boolean => Ty::Bool,
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64 => Ty::Int64,
            DataType::Float16 | DataType::Float32 | DataType::Float64 => Ty::Float64,
            DataType::Decimal32(..) | DataType::Decimal64(..) | DataType::Decimal128(..) => {
                Ty::Numeric
            }
            DataType::Decimal256(..) => Ty::BigNumeric,
            DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8 => Ty::String,
            DataType::Binary
            | DataType::BinaryView
            | DataType::LargeBinary
            | DataType::FixedSizeBinary(_) => Ty::Bytes,
            DataType::Date32 | DataType::Date64 => Ty::Date,
            DataType::Time32(_) | DataType::Time64(_) => Ty::Time,
            DataType::Timestamp(_, None) => Ty::Datetime,
            DataType::Timestamp(_, Some(_)) => Ty::Timestamp,
            DataType::Interval(_) | DataType::Duration(_) => Ty::Interval,
            DataType::List(field)
            | DataType::LargeList(field)
            | DataType::ListView(field)
            | DataType::FixedSizeList(field, _) => {
                Ty::Array(Box::new(Ty::of_arrow(field.data_type())?))
            }
            DataType::Struct(fields) => Ty::Struct(
                fields
                    .iter()
                    .map(|field| Some((field.name().clone(), Ty::of_arrow(field.data_type())?)))
                    .collect::<Option<_>>()?,
            ),
            DataType::Dictionary(_, value) => Ty::of_arrow(value)?,
            _ => return None,
        };
        Some(ty)
    }

    /// The Arrow type of the type, which has no variables.
    pub fn arrow(&self, options: TypeOptions) -> DataType {
        match self {
            Ty::Int64 | Ty::Null | Ty::Any | Ty::Var(_) | Ty::Table => DataType::Int64,
            Ty::Float64 => DataType::Float64,
            Ty::Numeric => DataType::Decimal128(38, 9),
            Ty::BigNumeric => DataType::Decimal256(76, 38),
            Ty::Bool => DataType::Boolean,
            Ty::String if options.utf8_view => DataType::Utf8View,
            Ty::String => DataType::Utf8,
            Ty::Bytes => DataType::Binary,
            Ty::Date => DataType::Date32,
            Ty::Time => DataType::Time64(TimeUnit::Microsecond),
            Ty::Datetime => types::datetime(),
            Ty::Timestamp => types::timestamp(),
            Ty::Interval => DataType::Interval(IntervalUnit::MonthDayNano),
            Ty::Array(element) => {
                DataType::List(Arc::new(Field::new("item", element.arrow(options), true)))
            }
            Ty::Struct(fields) => DataType::Struct(Fields::from(
                fields
                    .iter()
                    .map(|(name, ty)| Field::new(name, ty.arrow(options), true))
                    .collect::<Vec<_>>(),
            )),
        }
    }

    /// The type with each variable replaced by its type in `vars`.
    fn substitute(&self, vars: &BTreeMap<String, Ty>) -> Ty {
        match self {
            Ty::Var(name) => vars.get(name).cloned().unwrap_or(Ty::Any),
            Ty::Array(element) => Ty::Array(Box::new(element.substitute(vars))),
            Ty::Struct(fields) => Ty::Struct(
                fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.substitute(vars)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int64 => f.write_str("INT64"),
            Ty::Float64 => f.write_str("FLOAT64"),
            Ty::Numeric => f.write_str("NUMERIC"),
            Ty::BigNumeric => f.write_str("BIGNUMERIC"),
            Ty::Bool => f.write_str("BOOL"),
            Ty::String => f.write_str("STRING"),
            Ty::Bytes => f.write_str("BYTES"),
            Ty::Date => f.write_str("DATE"),
            Ty::Time => f.write_str("TIME"),
            Ty::Datetime => f.write_str("DATETIME"),
            Ty::Timestamp => f.write_str("TIMESTAMP"),
            Ty::Interval => f.write_str("INTERVAL"),
            Ty::Array(element) => write!(f, "ARRAY<{element}>"),
            Ty::Struct(fields) => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(name, ty)| format!("{name} {ty}"))
                    .collect();
                write!(f, "STRUCT<{}>", fields.join(", "))
            }
            Ty::Var(name) => f.write_str(name),
            Ty::Any => f.write_str("ANY"),
            Ty::Null => f.write_str("NULL"),
            Ty::Table => f.write_str("TABLE"),
        }
    }
}

/// A set of date parts that an argument takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartSet {
    /// The parts of a date: `DAY`, `WEEK`, `WEEK(day)`, `ISOWEEK`, `MONTH`, `QUARTER`, `YEAR`
    /// and `ISOYEAR`.
    Date,
    /// The parts of a date and of a time, from `MICROSECOND` to `ISOYEAR`.
    Datetime,
    /// The parts of a fixed length, from `MICROSECOND` to `DAY`.
    Fixed,
}

impl PartSet {
    pub fn contains(self, part: DatePart) -> bool {
        let date = matches!(
            part,
            DatePart::Day
                | DatePart::Week(_)
                | DatePart::IsoWeek
                | DatePart::Month
                | DatePart::Quarter
                | DatePart::Year
                | DatePart::IsoYear
        );
        let time = matches!(
            part,
            DatePart::Microsecond
                | DatePart::Millisecond
                | DatePart::Second
                | DatePart::Minute
                | DatePart::Hour
        );
        match self {
            PartSet::Date => date,
            PartSet::Datetime => date || time,
            PartSet::Fixed => time || part == DatePart::Day,
        }
    }

    /// The names of the parts of the set, for completion.
    pub fn names(self) -> Vec<&'static str> {
        let all = [
            ("MICROSECOND", DatePart::Microsecond),
            ("MILLISECOND", DatePart::Millisecond),
            ("SECOND", DatePart::Second),
            ("MINUTE", DatePart::Minute),
            ("HOUR", DatePart::Hour),
            ("DAY", DatePart::Day),
            ("WEEK", DatePart::Week(super::Weekday::Sunday)),
            ("ISOWEEK", DatePart::IsoWeek),
            ("MONTH", DatePart::Month),
            ("QUARTER", DatePart::Quarter),
            ("YEAR", DatePart::Year),
            ("ISOYEAR", DatePart::IsoYear),
        ];
        all.iter()
            .filter(|(_, part)| self.contains(*part))
            .map(|(name, _)| *name)
            .collect()
    }
}

/// What an argument of a form takes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Accept {
    /// A value of the type.
    Value(Ty),
    /// A literal of the type.
    Const(Ty),
    /// A string literal of the set.
    OneOf(Vec<String>),
    /// A date part of the set.
    Part(PartSet),
}

/// How many arguments an argument of a form takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arity {
    Required,
    Optional,
    /// Any number, as the last argument of the form.
    Repeated,
}

/// An argument of a form.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FormArg {
    /// The parameter of the function that the argument takes.
    pub param: String,
    pub accept: Accept,
    pub arity: Arity,
    /// Whether a call gives the argument only by name.
    pub named_only: bool,
}

/// The arguments and the result of a form, without its mapping.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Signature {
    pub args: Vec<FormArg>,
    pub result: Ty,
    /// The type variables and their sets of types. An empty set is any type.
    pub vars: Vec<(String, Vec<Ty>)>,
}

impl Ty {
    /// The type that `text` writes, as a form writes a type, without type variables.
    pub fn parse(text: &str) -> Result<Ty, String> {
        let mut parser = Parser::new(text);
        let ty = parser.ty()?;
        parser.skip_space();
        match parser.rest().is_empty() {
            true => Ok(ty),
            false => Err(format!("unexpected `{}` in `{text}`", parser.rest())),
        }
    }
}

impl Signature {
    /// The signature of the declaration `text`, as the module describes it.
    pub fn parse(text: &str) -> Result<Signature, String> {
        Parser::new(text).signature()
    }

    /// The signature as GoogleSQL writes it, with the names of the parameters, and a variable of
    /// one type written as its type: `SUBSTR(value: STRING, position: INT64[, length: INT64]) ->
    /// STRING`.
    pub fn render(&self, name: &str) -> String {
        let single: BTreeMap<String, Ty> = (self.vars.iter())
            .filter(|(_, set)| set.len() == 1)
            .map(|(var, set)| (var.clone(), set[0].clone()))
            .collect();
        let accept = |accept: &Accept| match accept {
            Accept::Value(ty) => ty.substitute_keep(&single).to_string(),
            Accept::Const(ty) => format!("CONST {}", ty.substitute_keep(&single)),
            Accept::OneOf(values) => {
                let values: Vec<String> = values.iter().map(|v| format!("'{v}'")).collect();
                values.join(" | ")
            }
            Accept::Part(set) => match set {
                PartSet::Date => "DATE PART".to_string(),
                PartSet::Datetime => "DATE OR TIME PART".to_string(),
                PartSet::Fixed => "TIME PART OR DAY".to_string(),
            },
        };
        let mut out = format!("{name}(");
        for (index, arg) in self.args.iter().enumerate() {
            let sep = if index == 0 { "" } else { ", " };
            let marker = if arg.named_only { " => " } else { ": " };
            let core = format!("{}{marker}{}", arg.param, accept(&arg.accept));
            match arg.arity {
                Arity::Required => out.push_str(&format!("{sep}{core}")),
                Arity::Optional => out.push_str(&format!("[{sep}{core}]")),
                Arity::Repeated => out.push_str(&format!("{sep}{core}, ...")),
            }
        }
        out.push_str(&format!(") -> {}", self.result.substitute_keep(&single)));
        let multi: Vec<String> = (self.vars.iter())
            .filter(|(_, set)| set.len() != 1)
            .map(|(var, set)| match set.is_empty() {
                true => format!("{var}: ANY"),
                false => {
                    let set: Vec<String> = set.iter().map(Ty::to_string).collect();
                    format!("{var}: {}", set.join(" | "))
                }
            })
            .collect();
        if !multi.is_empty() {
            out.push_str(&format!(" where {}", multi.join(", ")));
        }
        out
    }
}

impl Ty {
    /// The type with the variables of `vars` replaced, and other variables kept.
    fn substitute_keep(&self, vars: &BTreeMap<String, Ty>) -> Ty {
        match self {
            Ty::Var(name) => vars.get(name).cloned().unwrap_or_else(|| self.clone()),
            Ty::Array(element) => Ty::Array(Box::new(element.substitute_keep(vars))),
            Ty::Struct(fields) => Ty::Struct(
                fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.substitute_keep(vars)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// What resolution knows of an argument of a call.
#[derive(Clone, Debug, PartialEq)]
pub enum Observed {
    /// A value of the type, which is a literal when `literal` holds.
    Value { ty: Ty, literal: bool },
    /// A date part.
    Part(DatePart),
}

/// The cost of coercing a value of `from` to `to`, or `None` when it doesn't coerce.
pub fn coercion(from: &Ty, literal: bool, to: &Ty) -> Option<u32> {
    if from == to || *to == Ty::Any {
        return Some(0);
    }
    let cost = match (from, to) {
        (Ty::Null, _) => 1,
        (Ty::Int64, Ty::Float64) => 1,
        (Ty::Int64, Ty::Numeric) => 2,
        (Ty::Int64, Ty::BigNumeric) => 3,
        // A float literal coerces to a decimal beside one, as `NUMERIC '1.5' + 1.5` is a
        // `NUMERIC`.
        (Ty::Float64, Ty::Numeric | Ty::BigNumeric) if literal => 1,
        (Ty::Numeric, Ty::BigNumeric) => 1,
        (Ty::Numeric, Ty::Float64) => 2,
        (Ty::BigNumeric, Ty::Float64) => 1,
        (Ty::Date, Ty::Datetime) => 1,
        (Ty::String, Ty::Date | Ty::Datetime | Ty::Time | Ty::Timestamp) if literal => 1,
        // A struct coerces field by field, by place; an anonymous field, `_field_1` and so on,
        // takes the name of the other struct's field.
        (Ty::Struct(a), Ty::Struct(b)) if a.len() == b.len() => {
            let mut total = 0;
            for ((an, at), (bn, bt)) in a.iter().zip(b) {
                let anonymous = |name: &str| name.starts_with("_field_");
                if !an.eq_ignore_ascii_case(bn) && !anonymous(an) && !anonymous(bn) {
                    return None;
                }
                total += coercion(at, literal, bt)?;
            }
            total
        }
        (Ty::Array(a), Ty::Array(b)) if **a == Ty::Null => {
            let _ = b;
            1
        }
        _ => return None,
    };
    Some(cost)
}

/// The common supertype of values of the types `observed`, each with whether it is a literal, by
/// the rule of a type variable of any type: the type of one of the values to which the others
/// coerce at the least cost, the first of them on a tie. Values that are all `NULL` take `INT64`.
pub fn supertype(observed: &[(Ty, bool)]) -> Option<Ty> {
    let mut candidates: Vec<&Ty> = Vec::new();
    for (ty, _) in observed {
        if *ty != Ty::Null && !candidates.contains(&ty) {
            candidates.push(ty);
        }
    }
    if candidates.is_empty() {
        return Some(Ty::Int64);
    }
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let total = observed.iter().try_fold(0, |total, (ty, literal)| {
                Some(total + coercion(ty, *literal, candidate)?)
            })?;
            Some((total, candidate.clone()))
        })
        .min_by_key(|(total, _)| *total)
        .map(|(_, ty)| ty)
}

/// The resolution of a call to a form: its cost, the type of each variable, and the type to
/// which each value argument coerces.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    pub cost: u32,
    pub vars: BTreeMap<String, Ty>,
}

impl Signature {
    /// The resolution of the arguments `args`, each bound to the argument of the form at its
    /// index, or `None` when they don't fit. A repeated argument binds every argument at and
    /// after its index.
    pub fn resolve(&self, args: &[(usize, Observed)]) -> Option<Resolution> {
        // The observed types of each variable, from the arguments that take it.
        let mut by_var: BTreeMap<String, Vec<(Ty, bool)>> = BTreeMap::new();
        let mut cost = 0;
        for (index, observed) in args {
            let arg = &self.args[*index];
            match (&arg.accept, observed) {
                (Accept::Part(_), Observed::Part(_)) => {}
                (Accept::Part(_), _) | (_, Observed::Part(_)) => return None,
                (Accept::OneOf(_), Observed::Value { ty, .. }) => {
                    cost += coercion(ty, true, &Ty::String)?;
                }
                (Accept::Value(want) | Accept::Const(want), Observed::Value { ty, literal }) => {
                    collect(want, ty, *literal, &mut by_var, &mut cost)?;
                }
            }
        }
        let mut vars = BTreeMap::new();
        for (var, set) in &self.vars {
            let observed = by_var.remove(var).unwrap_or_default();
            // As in GoogleSQL, a variable takes the supertype of its arguments, when its set holds
            // it, or the type of its set to which the supertype coerces at the least cost.
            let ty = match (observed.is_empty(), set.first()) {
                (true, Some(first)) => first.clone(),
                (true, None) => Ty::Int64,
                (false, _) => {
                    let supertype = supertype(&observed)?;
                    match set.is_empty() || set.contains(&supertype) {
                        true => supertype,
                        false => set
                            .iter()
                            .filter_map(|ty| Some((coercion(&supertype, false, ty)?, ty)))
                            .min_by_key(|(cost, _)| *cost)?
                            .1
                            .clone(),
                    }
                }
            };
            cost += observed.iter().try_fold(0, |total, (observed, literal)| {
                Some(total + coercion(observed, *literal, &ty)?)
            })?;
            vars.insert(var.clone(), ty);
        }
        Some(Resolution { cost, vars })
    }

    /// The type that the argument at `index` coerces to under `resolution`.
    pub fn arg_type(&self, index: usize, resolution: &Resolution) -> Option<Ty> {
        match &self.args[index].accept {
            Accept::Value(ty) | Accept::Const(ty) => Some(ty.substitute(&resolution.vars)),
            Accept::OneOf(_) => Some(Ty::String),
            Accept::Part(_) => None,
        }
    }

    /// The type of the result under `resolution`.
    pub fn result_type(&self, resolution: &Resolution) -> Ty {
        self.result.substitute(&resolution.vars)
    }
}

/// Adds the type `ty` of an argument that the form wants as `want` to the observations of its
/// variables, or adds its cost of coercion for a type without variables.
fn collect(
    want: &Ty,
    ty: &Ty,
    literal: bool,
    by_var: &mut BTreeMap<String, Vec<(Ty, bool)>>,
    cost: &mut u32,
) -> Option<()> {
    match (want, ty) {
        (Ty::Var(var), _) => {
            by_var
                .entry(var.clone())
                .or_default()
                .push((ty.clone(), literal));
            Some(())
        }
        (Ty::Array(want), Ty::Array(element)) => collect(want, element, literal, by_var, cost),
        (Ty::Array(_), Ty::Null) => Some(()),
        _ => {
            *cost += coercion(ty, literal, want)?;
            Some(())
        }
    }
}

/// The parser of the declaration of a form.
struct Parser<'a> {
    text: &'a str,
    position: usize,
    vars: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        // The variables are the names that the `where` clause declares.
        let vars = match text.split_once(" where ") {
            Some((_, clause)) => clause
                .split(',')
                .filter_map(|part| {
                    part.split_once(':')
                        .map(|(name, _)| name.trim().to_string())
                })
                .collect(),
            None => Vec::new(),
        };
        Self {
            text,
            position: 0,
            vars,
        }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.position..]
    }

    fn skip_space(&mut self) {
        let trimmed = self.rest().trim_start();
        self.position = self.text.len() - trimmed.len();
    }

    fn eat(&mut self, token: &str) -> bool {
        self.skip_space();
        match self.rest().starts_with(token) {
            true => {
                self.position += token.len();
                true
            }
            false => false,
        }
    }

    fn expect(&mut self, token: &str) -> Result<(), String> {
        match self.eat(token) {
            true => Ok(()),
            false => Err(format!(
                "expected `{token}` at `{}` in `{}`",
                self.rest(),
                self.text
            )),
        }
    }

    fn word(&mut self) -> Result<String, String> {
        self.skip_space();
        let length = self
            .rest()
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(self.rest().len());
        if length == 0 {
            return Err(format!(
                "expected a name at `{}` in `{}`",
                self.rest(),
                self.text
            ));
        }
        let word = self.rest()[..length].to_string();
        self.position += length;
        Ok(word)
    }

    fn signature(mut self) -> Result<Signature, String> {
        self.expect("(")?;
        let mut args = Vec::new();
        if !self.eat(")") {
            loop {
                let optional = self.eat("[");
                if optional && !args.is_empty() {
                    self.expect(",")?;
                }
                let mut arg = self.arg()?;
                if optional {
                    arg.arity = Arity::Optional;
                    self.expect("]")?;
                }
                args.push(arg);
                if self.eat(")") {
                    break;
                }
                // An optional argument after the first writes its comma inside its brackets.
                self.skip_space();
                if self.rest().starts_with('[') {
                    continue;
                }
                self.expect(",")?;
            }
        }
        self.expect("->")?;
        let result = self.ty()?;
        let mut vars = Vec::new();
        if self.eat("where") {
            loop {
                let name = self.word()?;
                self.expect(":")?;
                let mut set = Vec::new();
                if !self.eat("ANY") {
                    loop {
                        set.push(self.ty()?);
                        if !self.eat("|") {
                            break;
                        }
                    }
                }
                vars.push((name, set));
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.skip_space();
        if !self.rest().is_empty() {
            return Err(format!("unexpected `{}` in `{}`", self.rest(), self.text));
        }
        Ok(Signature { args, result, vars })
    }

    fn arg(&mut self) -> Result<FormArg, String> {
        let param = self.word()?;
        let named_only = match (self.eat("=>"), self.eat(":")) {
            (true, _) => true,
            (false, true) => false,
            _ => {
                return Err(format!(
                    "expected `:` or `=>` after `{param}` in `{}`",
                    self.text
                ))
            }
        };
        let accept = if self.eat("PART(") {
            let set = match self.word()?.as_str() {
                "DATE" => PartSet::Date,
                "DATETIME" => PartSet::Datetime,
                "FIXED" => PartSet::Fixed,
                other => return Err(format!("no part set {other}")),
            };
            self.expect(")")?;
            Accept::Part(set)
        } else if self.eat("CONST ") {
            Accept::Const(self.ty()?)
        } else if self.eat("ONEOF(") {
            let mut values = Vec::new();
            loop {
                self.expect("'")?;
                let end = self.rest().find('\'').ok_or("an unclosed string")?;
                values.push(self.rest()[..end].to_string());
                self.position += end + 1;
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
            Accept::OneOf(values)
        } else {
            Accept::Value(self.ty()?)
        };
        let arity = match self.eat("...") {
            true => Arity::Repeated,
            false => Arity::Required,
        };
        Ok(FormArg {
            param,
            accept,
            arity,
            named_only,
        })
    }

    fn ty(&mut self) -> Result<Ty, String> {
        let word = self.word()?;
        let ty = match word.as_str() {
            "INT64" => Ty::Int64,
            "FLOAT64" => Ty::Float64,
            "NUMERIC" => Ty::Numeric,
            "BIGNUMERIC" => Ty::BigNumeric,
            "BOOL" => Ty::Bool,
            "STRING" => Ty::String,
            "BYTES" => Ty::Bytes,
            "DATE" => Ty::Date,
            "TIME" => Ty::Time,
            "DATETIME" => Ty::Datetime,
            "TIMESTAMP" => Ty::Timestamp,
            "INTERVAL" => Ty::Interval,
            "ANY" => Ty::Any,
            "TABLE" => Ty::Table,
            "ARRAY" => {
                self.expect("<")?;
                let element = self.ty()?;
                self.expect(">")?;
                Ty::Array(Box::new(element))
            }
            "STRUCT" => {
                self.expect("<")?;
                let mut fields = Vec::new();
                loop {
                    let name = self.word()?;
                    fields.push((name, self.ty()?));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(">")?;
                Ty::Struct(fields)
            }
            name if self.vars.iter().any(|var| var == name) => Ty::Var(name.to_string()),
            other => return Err(format!("`{other}` is not a type in `{}`", self.text)),
        };
        Ok(ty)
    }
}
