//! Expressions: each construct of the typed tree, planned through its form.
//!
//! A name resolves by GoogleSQL's rules: a range variable first, as `t` in `t.x`, then a column of
//! the input, then the fields of a struct. An operator resolves among the forms of the library's
//! operators, and a call among its function's forms, so every cast of the plan comes from
//! GoogleSQL's coercion. The values of `CASE` and of an array take a common type by GoogleSQL's
//! supertype rule.
//!
//! Each planning function returns `None` after an error, which it reports once at its text, and
//! visits its children first, so the model resolves every name of an expression with an error.

use crate::library::search::{closest_name, did_you_mean};
use std::sync::Arc;

use avenger_sql_syntax::{
    ast::{self, AstNode, BinaryOp, LiteralKind, PrefixOp},
    TextRange,
};
use datafusion::{
    arrow::datatypes::{DataType, Field},
    common::{DFSchema, ExprSchema, ScalarValue},
    functions::core::expr_fn as core,
    functions_nested::expr_fn::make_array,
    logical_expr::{
        expr::{
            AggregateFunction, Case, Exists, InSubquery, NullTreatment, Placeholder, WindowFunction,
        },
        Expr, ExprSchemable, LogicalPlanBuilder, SortExpr, Subquery, TryCast, WindowFrame,
        WindowFrameBound, WindowFrameUnits, WindowFunctionDefinition,
    },
};

use super::{
    build::{self, cast, int, subquery},
    lifting,
    model::{self, CallForm, Definition, EnumValue, FieldBase, PlaceKind},
    rows::{Col, RangeVar, Rows},
    Pass,
};
use crate::{
    library::{
        self, context, form::supertype, operator, Arg, CallError, DatePart, DefaultWindow, ErrorAt,
        Function, Need, Over, PlanContext, Planned, Subscript, Ty,
    },
    types,
};

/// What an expression can read, and the calls it can make.
#[derive(Clone, Copy)]
pub(crate) struct Scope<'r> {
    pub columns: &'r [Col],
    pub vars: &'r [RangeVar],
    /// The schema of the input, or `None` after an error, when the pass resolves names only.
    pub schema: Option<&'r DFSchema>,
    /// The clause, as an error names it, such as `pipe WHERE`.
    pub clause: &'static str,
    pub aggregates: bool,
    pub windows: bool,
    /// The group expressions of the aggregate whose items the scope plans, over which `HAVING MAX`
    /// finds the greatest value, or `None` for grouping sets.
    pub groups: Option<&'r [Expr]>,
    /// Whether the scope is in the channels of a mark call in an item of a pipe `AGGREGATE`,
    /// where windows run on the rows, even in the arguments of an aggregate or a window.
    pub marked: bool,
}

impl<'r> Scope<'r> {
    pub(crate) fn of(rows: &'r Rows, clause: &'static str) -> Self {
        Self {
            columns: &rows.columns,
            vars: &rows.vars,
            schema: rows.schema(),
            clause,
            aggregates: false,
            windows: false,
            groups: Some(&[]),
            marked: false,
        }
    }

    /// The scope of a constant, such as the count of a `LIMIT`, which reads no columns, over
    /// `empty`, an empty schema.
    pub(crate) fn constant(empty: &'r DFSchema, clause: &'static str) -> Self {
        Self {
            columns: &[],
            vars: &[],
            schema: Some(empty),
            clause,
            aggregates: false,
            windows: false,
            groups: Some(&[]),
            marked: false,
        }
    }

    pub(crate) fn aggregates(self) -> Self {
        Self {
            aggregates: true,
            ..self
        }
    }

    /// The scope of the items of an aggregate of the group expressions `groups`.
    pub(crate) fn grouped(self, groups: Option<&'r [Expr]>) -> Self {
        Self { groups, ..self }
    }

    pub(crate) fn windows(self) -> Self {
        Self {
            windows: true,
            ..self
        }
    }

    /// The scope of the arguments of an aggregate or a window function, which call neither, but
    /// for the windows on the rows of a mark's channels.
    pub(crate) fn arguments(self) -> Self {
        Self {
            clause: ARGUMENT,
            aggregates: false,
            windows: self.marked,
            ..self
        }
    }
}

/// The clause of the arguments of an aggregate or a window function.
pub(crate) const ARGUMENT: &str = "an aggregate's argument";

/// The clause of the items of a pipe `AGGREGATE`.
pub(crate) const AGGREGATE_ITEMS: &str = "pipe AGGREGATE";

/// The clause of the channels of a mark in an item of a pipe `AGGREGATE`, which take aggregates.
pub(crate) const MARK_CHANNEL: &str = "a mark's channel";

/// The range of the error `error` of a call at `range` whose arguments are at `ranges`.
fn error_range(error: &CallError, ranges: &[TextRange], range: TextRange) -> TextRange {
    match error.at {
        ErrorAt::Arg(index) => ranges.get(index).copied().unwrap_or(range),
        ErrorAt::Call => range,
    }
}

fn is_null_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(ScalarValue::Null, _))
}

impl Pass<'_> {
    fn context<'s>(&'s self, schema: &'s DFSchema) -> PlanContext<'s> {
        PlanContext {
            schema,
            time_zone: &self.options.time_zone,
            locale: self.options.locale.as_deref(),
            type_options: self.type_options,
        }
    }

    /// The type of `expr` in `scope`.
    pub(crate) fn type_of(&self, expr: &Expr, scope: Scope) -> Option<DataType> {
        expr.get_type(scope.schema?).ok()
    }

    /// The GoogleSQL name of the type of `expr`.
    fn type_name(&self, expr: &Expr, scope: Scope) -> String {
        match self.type_of(expr, scope) {
            Some(ty) => types::googlesql_name(&ty),
            None => "an unknown type".into(),
        }
    }

    /// What resolution knows of `expr`: its type, and whether it is a literal.
    fn observed(&self, expr: &Expr, scope: Scope) -> Option<(Ty, bool)> {
        let ty = library::observed_type(expr, &self.type_of(expr, scope)?)?;
        Some((ty, matches!(expr, Expr::Literal(..))))
    }

    /// `expr` cast to `ty`, when its type is another.
    pub(crate) fn coerce_to(&self, expr: Expr, ty: &Ty, scope: Scope) -> Option<Expr> {
        let current = self.type_of(&expr, scope)?;
        if library::exact(&current, ty, self.type_options) {
            return Some(expr);
        }
        // A struct takes the names of the struct type field by field, as an anonymous field of
        // `STRUCT(2, 'q')` takes the name of its place.
        if let (DataType::Struct(fields), Ty::Struct(want)) = (&current, ty) {
            if fields.len() == want.len() {
                let args = (fields.iter().zip(want))
                    .flat_map(|(field, (name, ty))| {
                        let value = core::get_field(expr.clone(), field.name().as_str());
                        let value = cast(value, ty.arrow(self.type_options));
                        [self.string_literal(name), value]
                    })
                    .collect();
                return Some(core::named_struct(args));
            }
        }
        Some(cast(expr, ty.arrow(self.type_options)))
    }

    /// `values` cast to their common type, or `None` with an error at `range` when they have
    /// none, as `what` names them.
    fn common(
        &mut self,
        values: Vec<Expr>,
        range: TextRange,
        what: &str,
        scope: Scope,
    ) -> Option<(Vec<Expr>, Ty)> {
        let observed: Vec<(Ty, bool)> = (values.iter())
            .map(|value| self.observed(value, scope))
            .collect::<Option<_>>()?;
        let Some(ty) = supertype(&observed) else {
            let types: Vec<String> = observed.iter().map(|(ty, _)| ty.to_string()).collect();
            self.error(
                range,
                format!("{what} have no common supertype: {}", types.join(", ")),
            );
            return None;
        };
        let values = (values.into_iter())
            .map(|value| self.coerce_to(value, &ty, scope))
            .collect::<Option<_>>()?;
        Some((values, ty))
    }

    /// The plan of the condition `expr` in `scope`, a `BOOL`, or `None` with an error, as `what`
    /// names it, such as `JOIN ON clause`.
    pub(crate) fn condition(&mut self, expr: &ast::Expr, scope: Scope, what: &str) -> Option<Expr> {
        let at = expr.syntax().text_range();
        let planned = self.expr(expr, scope)?;
        self.boolean(planned, at, what, scope)
    }

    /// `expr` as a `BOOL`, for a place that takes one, such as `WHERE`, or `None` with an error at
    /// `range`. `NULL` is a `BOOL`.
    pub(crate) fn boolean(
        &mut self,
        expr: Expr,
        range: TextRange,
        what: &str,
        scope: Scope,
    ) -> Option<Expr> {
        let ty = Ty::of_arrow(&self.type_of(&expr, scope)?);
        match ty {
            Some(Ty::Bool) => Some(expr),
            Some(Ty::Null) => Some(cast(expr, DataType::Boolean)),
            _ => {
                let name = self.type_name(&expr, scope);
                self.error(
                    range,
                    format!("{what} should return type BOOL, but returns {name}"),
                );
                None
            }
        }
    }

    /// The plan of `expr` in `scope`.
    pub(crate) fn expr(&mut self, expr: &ast::Expr, scope: Scope) -> Option<Expr> {
        let range = expr.syntax().text_range();
        match expr {
            ast::Expr::Literal(literal) => self.literal(literal),
            ast::Expr::TypedLiteral(literal) => self.typed_literal(literal),
            ast::Expr::IntervalExpr(interval) => self.interval(interval, scope),
            ast::Expr::ParamExpr(param) => self.param(param),
            ast::Expr::PathExpr(path) => self.path(&path.path()?, scope),
            ast::Expr::FieldExpr(field) => {
                let base = self.child(field.expr(), scope);
                // The type before the `.`, whose fields an editor offers after it.
                if let (Some(planned), Some(syntax)) = (&base, field.expr()) {
                    if let Some(ty) = self.type_of(planned, scope) {
                        self.model.field_bases.push(FieldBase {
                            range: syntax.syntax().text_range(),
                            ty,
                        });
                    }
                }
                let name = field.name_ref()?;
                let text = name.text()?;
                self.field(base?, &text, name.syntax().text_range(), scope)
            }
            ast::Expr::SubscriptExpr(subscript) => self.subscript(subscript, scope),
            ast::Expr::PrefixExpr(prefix) => self.prefix(prefix, scope),
            ast::Expr::BinExpr(bin) => self.binary(bin, scope),
            ast::Expr::IsExpr(is) => {
                let value = self.child(is.expr(), scope);
                let operand = is.expr()?.syntax().text_range();
                let value = value?;
                let tested = if is.null_kw().is_some() {
                    self.operator("$is_null", vec![(value, operand)], range, scope)
                } else if is.true_kw().is_some() {
                    self.operator("$is_true", vec![(value, operand)], range, scope)
                } else if is.false_kw().is_some() {
                    self.operator("$is_false", vec![(value, operand)], range, scope)
                } else {
                    let value = self.boolean(value, operand, "The operand of IS UNKNOWN", scope)?;
                    self.operator("$is_null", vec![(value, operand)], range, scope)
                }?;
                match is.not_kw() {
                    Some(_) => self.operator("$not", vec![(tested, range)], range, scope),
                    None => Some(tested),
                }
            }
            ast::Expr::IsDistinctExpr(is) => {
                let operands = self.operands([is.lhs(), is.rhs()], scope)?;
                let name = match is.not_kw() {
                    Some(_) => "$is_not_distinct_from",
                    None => "$is_distinct_from",
                };
                self.operator(name, operands, range, scope)
            }
            ast::Expr::BetweenExpr(between) => {
                let operands =
                    self.operands([between.value(), between.low(), between.high()], scope)?;
                let value = self.operator("$between", operands, range, scope)?;
                self.negated(value, between.not_kw().is_some(), range, scope)
            }
            ast::Expr::InExpr(in_expr) => self.in_expr(in_expr, scope),
            ast::Expr::LikeExpr(like) => self.like(like, scope),
            ast::Expr::CaseExpr(case) => self.case(case, scope),
            ast::Expr::CastExpr(cast) => self.cast(cast, scope),
            ast::Expr::ExtractExpr(extract) => self.extract(extract, scope),
            ast::Expr::ArrayExpr(array) => self.array(array, scope),
            ast::Expr::StructExpr(structure) => self.structure(structure, scope),
            ast::Expr::TupleExpr(tuple) => {
                let values: Vec<Option<Expr>> = tuple
                    .exprs()
                    .map(|value| self.expr(&value, scope))
                    .collect();
                let values: Vec<Expr> = values.into_iter().collect::<Option<_>>()?;
                let args = (values.into_iter().enumerate())
                    .flat_map(|(index, value)| {
                        [self.string_literal(&format!("_field_{}", index + 1)), value]
                    })
                    .collect();
                Some(core::named_struct(args))
            }
            ast::Expr::ParenExpr(paren) => self.child(paren.expr(), scope),
            ast::Expr::CallExpr(call) => self.call(call, scope),
            ast::Expr::SubqueryExpr(subquery) => {
                let rows = self.subquery_rows(subquery.paren_query()?, scope)?;
                if rows.columns.len() != 1 {
                    self.error(
                        range,
                        "Scalar subquery cannot have more than one column unless using SELECT AS STRUCT to build STRUCT values",
                    );
                    return None;
                }
                let subquery = self.subquery_of(rows, range, false)?;
                Some(Expr::ScalarSubquery(subquery))
            }
            ast::Expr::ExistsExpr(exists) => {
                let rows = self.subquery_rows(exists.paren_query()?, scope)?;
                let subquery = self.subquery_of(rows, range, true)?;
                Some(Expr::Exists(Exists::new(subquery, false)))
            }
            ast::Expr::ArraySubqueryExpr(array) => {
                let rows = self.subquery_rows(array.paren_query()?, scope)?;
                self.array_subquery(rows, range)
            }
        }
    }

    /// The plan of an optional child, which the tree lacks after a syntax error.
    fn child(&mut self, expr: Option<ast::Expr>, scope: Scope) -> Option<Expr> {
        self.expr(&expr?, scope)
    }

    /// The plans of the operands `exprs`, each with its range, after planning all of them.
    fn operands<const N: usize>(
        &mut self,
        exprs: [Option<ast::Expr>; N],
        scope: Scope,
    ) -> Option<Vec<(Expr, TextRange)>> {
        let planned: Vec<Option<(Expr, TextRange)>> = exprs
            .into_iter()
            .map(|expr| {
                let expr = expr?;
                let range = expr.syntax().text_range();
                Some((self.expr(&expr, scope)?, range))
            })
            .collect();
        planned.into_iter().collect()
    }

    fn negated(&mut self, value: Expr, not: bool, range: TextRange, scope: Scope) -> Option<Expr> {
        match not {
            true => self.operator("$not", vec![(value, range)], range, scope),
            false => Some(value),
        }
    }

    /// The plan of the operator `name` of `operands`, each with its range, at `range`, through
    /// the operator's forms.
    pub(crate) fn operator(
        &mut self,
        name: &str,
        operands: Vec<(Expr, TextRange)>,
        range: TextRange,
        scope: Scope,
    ) -> Option<Expr> {
        if let Some(text) = operator::rejects_null(name) {
            if operands.iter().any(|(expr, _)| is_null_literal(expr)) {
                self.error(range, format!("Operands of {text} cannot be literal NULL"));
                return None;
            }
        }
        let library = self.library;
        let function = library.operator(name).expect("an operator of the library");
        let types: Vec<String> = (operands.iter())
            .map(|(expr, _)| self.type_name(expr, scope))
            .collect();
        self.resolve(function, operands, range, scope, |error| {
            match error.no_form {
                true => format!(
                    "No matching signature for operator {} for argument types: {}",
                    operator::text(name),
                    types.join(", ")
                ),
                false => error.message,
            }
        })
    }

    fn literal(&mut self, literal: &ast::Literal) -> Option<Expr> {
        let range = literal.syntax().text_range();
        let value = match literal.kind()? {
            LiteralKind::Int => match literal.int_value().map(i64::try_from) {
                Some(Ok(value)) => ScalarValue::Int64(Some(value)),
                _ => {
                    self.error(
                        range,
                        format!("Invalid integer literal: {}", literal.syntax()),
                    );
                    return None;
                }
            },
            LiteralKind::Float => ScalarValue::Float64(Some(literal.float_value()?)),
            LiteralKind::String => return Some(self.string_literal(&literal.string_value()?)),
            LiteralKind::Bytes => ScalarValue::Binary(Some(literal.bytes_value()?)),
            LiteralKind::Null => ScalarValue::Null,
            LiteralKind::True => ScalarValue::Boolean(Some(true)),
            LiteralKind::False => ScalarValue::Boolean(Some(false)),
        };
        Some(Expr::Literal(value, None))
    }

    /// A `STRING` literal, of the Arrow type of the session's strings.
    pub(crate) fn string_literal(&self, value: &str) -> Expr {
        let value = match self.type_options.utf8_view {
            true => ScalarValue::Utf8View(Some(value.to_string())),
            false => ScalarValue::Utf8(Some(value.to_string())),
        };
        Expr::Literal(value, None)
    }

    /// A typed literal, such as `DATE '2024-01-01'`, whose text the pass reads when it plans it.
    fn typed_literal(&mut self, literal: &ast::TypedLiteral) -> Option<Expr> {
        let range = literal.syntax().text_range();
        let (name, ty) = if literal.date_kw().is_some() {
            ("DATE", DataType::Date32)
        } else if literal.time_kw().is_some() {
            ("TIME", Ty::Time.arrow(self.type_options))
        } else if literal.datetime_kw().is_some() {
            ("DATETIME", types::datetime())
        } else if literal.timestamp_kw().is_some() {
            ("TIMESTAMP", types::timestamp())
        } else if literal.numeric_kw().is_some() || literal.decimal_kw().is_some() {
            ("NUMERIC", Ty::Numeric.arrow(self.type_options))
        } else if literal.bignumeric_kw().is_some() || literal.bigdecimal_kw().is_some() {
            ("BIGNUMERIC", Ty::BigNumeric.arrow(self.type_options))
        } else {
            self.error(range, "avenger-sql has no type for this literal yet");
            return None;
        };
        let text = literal.value()?;
        let value = match name {
            "TIMESTAMP" => {
                let (text, zoned) = timestamp_text(&text);
                match zoned {
                    true => ScalarValue::Utf8(Some(text)).cast_to(&ty),
                    // A civil time is in the default time zone.
                    false => ScalarValue::Utf8(Some(text))
                        .cast_to(&DataType::Timestamp(
                            datafusion::arrow::datatypes::TimeUnit::Microsecond,
                            Some(Arc::from(self.options.time_zone.as_str())),
                        ))
                        .and_then(|value| value.cast_to(&ty)),
                }
            }
            "DATETIME" => ScalarValue::Utf8(Some(text.trim().replacen(' ', "T", 1))).cast_to(&ty),
            _ => ScalarValue::Utf8(Some(text.trim().to_string())).cast_to(&ty),
        };
        match value {
            Ok(value) if !value.is_null() => Some(Expr::Literal(value, None)),
            _ => {
                self.error(
                    range,
                    format!("Invalid {name} literal: {}", literal.syntax()),
                );
                None
            }
        }
    }

    /// `INTERVAL count part`, whose count is an `INT64`.
    fn interval(&mut self, interval: &ast::IntervalExpr, scope: Scope) -> Option<Expr> {
        let range = interval.syntax().text_range();
        let count = interval.expr()?;
        let count_range = count.syntax().text_range();
        let planned = self.expr(&count, scope);
        let part_name = interval.from()?;
        let part_text = part_name.text()?;
        if let Some(to) = interval.to() {
            let to_text = to.text()?;
            let parts = (
                DatePart::parse(&part_text, None),
                DatePart::parse(&to_text, None),
            );
            let (Some(from), Some(to_part)) = parts else {
                self.error(
                    range,
                    format!("Unsupported INTERVAL datetime fields {part_text} TO {to_text}"),
                );
                return None;
            };
            let Some(Expr::Literal(value, _)) = planned else {
                if planned.is_some() {
                    self.error(count_range, "INTERVAL ... TO ... takes a string literal");
                }
                return None;
            };
            let Some(Some(text)) = value.try_as_str() else {
                self.error(count_range, "INTERVAL ... TO ... takes a string literal");
                return None;
            };
            return match library::interval::of_text(text, from, to_part) {
                Ok(interval) => Some(interval),
                Err(message) => {
                    self.error(count_range, message);
                    None
                }
            };
        }
        let part = DatePart::parse(&part_text, None).filter(|&part| library::interval::takes(part));
        let Some(part) = part else {
            self.error(
                part_name.syntax().text_range(),
                format!("A valid date part name is required but found {part_text}"),
            );
            return None;
        };
        let mut count = planned?;
        // A string literal of an integer is a count, as in `INTERVAL '5' DAY`.
        if let Expr::Literal(value, _) = &count {
            if let Some(Some(text)) = value.try_as_str() {
                match text.trim().parse::<i64>() {
                    Ok(value) => count = int(value),
                    Err(_) => {
                        self.error(count_range, format!("Invalid interval literal: '{text}'"));
                        return None;
                    }
                }
            }
        }
        match Ty::of_arrow(&self.type_of(&count, scope)?) {
            Some(Ty::Int64) => {}
            _ => {
                let name = self.type_name(&count, scope);
                self.error(
                    count_range,
                    format!("Interval value must be coercible to INT64 type, but has type {name}"),
                );
                return None;
            }
        }
        let count = self.coerce_to(count, &Ty::Int64, scope)?;
        Some(library::interval::of_part(count, part).expect("a part of an interval"))
    }

    fn param(&mut self, param: &ast::ParamExpr) -> Option<Expr> {
        let id = match param.param() {
            Some(token) => format!("${}", token.text().trim_start_matches('@')),
            None => {
                self.positional += 1;
                format!("${}", self.positional)
            }
        };
        Some(Expr::Placeholder(Placeholder::new_with_field(id, None)))
    }

    /// The rows of the query in parentheses `paren`, a subquery of an expression in `scope`, which
    /// reads the columns of `scope` as outer references.
    fn subquery_rows(&mut self, paren: ast::ParenQuery, scope: Scope) -> Option<Rows> {
        let query = paren.query()?;
        let mut outer = Outer::of(scope);
        outer.match_recognize = self.mr_correlated_clause();
        self.outers.push(outer);
        let rows = self.query(&query);
        self.outers.pop();
        Some(rows)
    }

    /// The subquery of the rows `rows`: whether they have rows when `exists`, and otherwise the
    /// value of their one column.
    fn subquery_of(&mut self, rows: Rows, range: TextRange, exists: bool) -> Option<Subquery> {
        let plan = rows.plan.clone()?;
        let built = match (exists, rows.columns.as_slice()) {
            // EXISTS reads whether the query has rows, through a projection of a constant, as
            // DataFusion's decorrelation expects.
            (true, _) => LogicalPlanBuilder::from(plan)
                .project(vec![int(1).alias("$exists")])
                .and_then(|b| b.build()),
            (false, [col]) => {
                let column = Expr::Column(col.column.clone());
                // DataFusion decorrelates a scalar subquery of one aggregate, so a correlated
                // query of one value gives any of its values, where GoogleSQL fails for more
                // than one.
                let correlated = !plan.all_out_ref_exprs().is_empty();
                let builder = LogicalPlanBuilder::from(plan);
                match correlated {
                    true => builder
                        .aggregate(
                            Vec::<Expr>::new(),
                            vec![Expr::AggregateFunction(AggregateFunction::new_udf(
                                datafusion::functions_aggregate::first_last::first_value_udaf(),
                                vec![column],
                                false,
                                None,
                                Vec::new(),
                                None,
                            ))
                            .alias(col.column.name.clone())],
                        )
                        .and_then(|b| b.build()),
                    false => builder.project(vec![column]).and_then(|b| b.build()),
                }
            }
            (false, _) => unreachable!("the callers check that a value's query has one column"),
        };
        let plan = match built {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return None;
            }
        };
        Some(subquery(plan))
    }

    /// `ARRAY(query)`: the array of the values of the query's one column, in its order, and an
    /// empty array for no rows.
    fn array_subquery(&mut self, rows: Rows, range: TextRange) -> Option<Expr> {
        if rows.columns.len() != 1 {
            self.error(
                range,
                "ARRAY subquery cannot have more than one column unless using SELECT AS STRUCT to build STRUCT values",
            );
            return None;
        }
        let element = rows.ty(&rows.columns[0]).and_then(|ty| Ty::of_arrow(&ty))?;
        if matches!(element, Ty::Array(_)) {
            self.error(range, "Cannot use array subquery with column of type ARRAY because nested arrays are not supported");
            return None;
        }
        let plan = rows.plan.clone()?;
        let values = Expr::AggregateFunction(AggregateFunction::new_udf(
            datafusion::functions_aggregate::array_agg::array_agg_udaf(),
            vec![Expr::Column(rows.columns[0].column.clone())],
            false,
            None,
            rows.order.clone(),
            None,
        ));
        let list = Ty::Array(Box::new(element)).arrow(self.type_options);
        let plan = LogicalPlanBuilder::from(plan)
            .aggregate(Vec::<Expr>::new(), vec![values.alias("$array")])
            .and_then(|b| b.build());
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return None;
            }
        };
        let empty = cast(make_array(Vec::new()), list.clone());
        Some(cast(
            core::coalesce(vec![Expr::ScalarSubquery(subquery(plan)), empty]),
            list,
        ))
    }

    /// The plan of a path: a range variable and its column, a column, a field of a value table,
    /// and the fields of a struct. A name that the scope lacks resolves in the scopes of the
    /// queries that enclose it, innermost first, as a correlated reference.
    pub(crate) fn path(&mut self, path: &ast::Path, scope: Scope) -> Option<Expr> {
        let refs: Vec<ast::NameRef> = path.name_refs().collect();
        let names: Vec<String> = refs.iter().map(|name| name.text()).collect::<Option<_>>()?;
        let first = names.first()?.clone();
        // A parameter of the body of a function that the pass inlines.
        if let Some(arg) = self.param_arg(&first) {
            let mut expr = arg;
            for (name, name_ref) in names.iter().zip(&refs).skip(1) {
                expr = self.field(expr, name, name_ref.syntax().text_range(), scope)?;
            }
            return Some(expr);
        }
        // A pattern variable of a `MATCH_RECOGNIZE`.
        if let Some(handled) =
            self.pattern_variable_path(&names, &refs, path.syntax().text_range(), scope)
        {
            return handled;
        }
        // The scopes of the enclosing queries, which the lookups borrow from the pass.
        let outers = std::mem::take(&mut self.outers);
        let levels = std::iter::once(scope).chain(outers.iter().rev().map(|outer| Scope {
            columns: &outer.columns,
            vars: &outer.vars,
            schema: outer.schema.as_ref(),
            ..scope
        }));
        let (mut found, mut depth, mut correlated) = (Lookup::Missing, 0, None);
        for (level_depth, level) in levels.enumerate() {
            found = match self.lookup(&names, &refs, level) {
                Lookup::Found(expr, rest) if level_depth > 0 => {
                    match outer_reference(expr, level.schema) {
                        Some(expr) => Lookup::Found(expr, rest),
                        None => Lookup::Failed,
                    }
                }
                lookup => lookup,
            };
            if !matches!(found, Lookup::Missing) {
                depth = level_depth;
                correlated = (level_depth > 0)
                    .then(|| outers[outers.len() - level_depth].match_recognize)
                    .flatten();
                break;
            }
        }
        self.outers = outers;
        let mut found = match found {
            Lookup::Found(expr, rest) => {
                // A column of `MATCH_RECOGNIZE`'s input in `MEASURES` is grouped or aggregated,
                // and a subquery of its clauses reads none of its columns.
                if depth == 0 {
                    if let Some(message) = self.mr_unaggregated(&expr, &names[..rest], scope) {
                        self.error(path.syntax().text_range(), message);
                        return None;
                    }
                }
                if let Some(clause) = correlated {
                    let clause = match clause {
                        super::match_recognize::Clause::Define => "DEFINE",
                        _ => "MEASURES",
                    };
                    self.error(
                        path.syntax().text_range(),
                        format!("Correlated access to MATCH_RECOGNIZE columns in the {clause} clause is not allowed"),
                    );
                    return None;
                }
                Some((expr, rest))
            }
            Lookup::Failed => return None,
            Lookup::Missing => None,
        };
        // A script variable, which a column hides, then a constant of a module.
        if found.is_none() {
            if let Some(variable) = self.variable(&first, refs[0].syntax().text_range()) {
                found = Some((variable, 1));
            }
        }
        if found.is_none() {
            for count in (1..=names.len()).rev() {
                if let Some(constant) = self.module_constant(&names[..count]) {
                    found = Some((constant, count));
                    break;
                }
            }
        }
        let Some((mut expr, rest)) = found else {
            let names = self.scope_names(scope, names.len() > 1);
            let names = names.iter().map(String::as_str);
            let message = did_you_mean(
                format!("Unrecognized name: {first}"),
                closest_name(&first, names),
            );
            self.error(refs[0].syntax().text_range(), message);
            return None;
        };
        for (name, name_ref) in names.iter().zip(&refs).skip(rest) {
            expr = self.field(expr, name, name_ref.syntax().text_range(), scope)?;
        }
        scope.schema?;
        Some(expr)
    }

    /// The names that a name in `scope` can read, for the suggestion of a name that it lacks: the
    /// columns of the scope and of the enclosing queries, and with `with_vars`, for the first name
    /// of a longer path, their range variables.
    fn scope_names(&self, scope: Scope, with_vars: bool) -> Vec<String> {
        let outers = self.outers.iter().rev();
        let levels = std::iter::once((scope.columns, scope.vars))
            .chain(outers.map(|outer| (outer.columns.as_slice(), outer.vars.as_slice())));
        let mut names = Vec::new();
        for (columns, vars) in levels {
            names.extend(columns.iter().map(|col| col.name.clone()));
            for var in vars {
                if with_vars {
                    names.push(var.name.clone());
                }
                names.extend(var.columns.iter().map(|col| col.name.clone()));
            }
        }
        names.retain(|name| !name.is_empty());
        names
    }

    /// The error of a call of `name`, which no function has, with the closest name of the
    /// library or of the script's temporary functions and its modules' functions.
    fn function_not_found(&self, name: &str) -> String {
        let others = self.function_paths();
        let others: Vec<&str> = others.iter().map(String::as_str).collect();
        let message = format!("Function not found: {name}");
        did_you_mean(message, self.library.closest(name, &others))
    }

    /// The lookup of the first names of a path in one scope: a range variable and its column, a
    /// column, or a field of a value table.
    fn lookup(&mut self, names: &[String], refs: &[ast::NameRef], scope: Scope) -> Lookup {
        let first = &names[0];
        let range = |index: usize| refs[index].syntax().text_range();
        if let Some(var) = (scope.vars.iter())
            .find(|var| !var.name.is_empty() && var.name.eq_ignore_ascii_case(first))
        {
            self.use_name(
                range(0),
                Definition::RangeVariable {
                    name: var.name.clone(),
                    definition: var.definition,
                },
            );
            let Some(name) = names.get(1) else {
                // The value of a value table, or the row of the range variable as a struct of
                // its columns.
                if let Some(value) = &var.value {
                    return Lookup::Found(Expr::Column(value.clone()), 1);
                }
                let args = (var.columns.iter())
                    .flat_map(|col| {
                        [
                            self.string_literal(&col.name),
                            Expr::Column(col.column.clone()),
                        ]
                    })
                    .collect();
                return Lookup::Found(core::named_struct(args), 1);
            };
            let found: Vec<&Col> = (var.columns.iter())
                .filter(|col| col.name.eq_ignore_ascii_case(name))
                .collect();
            return match found.as_slice() {
                [col] => {
                    let info = Self::column_info(col, scope.vars, scope.schema);
                    self.use_name(range(1), Definition::Column(info));
                    Lookup::Found(Expr::Column(col.column.clone()), 2)
                }
                [] => {
                    let message = format!("Name {name} not found inside {}", var.name);
                    self.error(range(1), message);
                    Lookup::Failed
                }
                _ => {
                    self.error(range(1), format!("Column name {name} is ambiguous"));
                    Lookup::Failed
                }
            };
        }
        let named = |col: &&Col| !col.name.is_empty() && col.name.eq_ignore_ascii_case(first);
        let mut found: Vec<&Col> = scope.columns.iter().filter(named).collect();
        if found.is_empty() {
            // A field of a value table, which a name reads without its range variable.
            found = (scope.vars.iter())
                .filter(|var| var.value.is_some())
                .flat_map(|var| var.columns.iter())
                .filter(named)
                .collect();
        }
        match found.as_slice() {
            [col] => {
                let info = Self::column_info(col, scope.vars, scope.schema);
                self.use_name(range(0), Definition::Column(info));
                Lookup::Found(Expr::Column(col.column.clone()), 1)
            }
            [] => Lookup::Missing,
            _ => {
                self.error(range(0), format!("Column name {first} is ambiguous"));
                Lookup::Failed
            }
        }
    }

    /// The field `name` of the struct `base`, without regard to case.
    pub(crate) fn field(
        &mut self,
        base: Expr,
        name: &str,
        range: TextRange,
        scope: Scope,
    ) -> Option<Expr> {
        let ty = self.type_of(&base, scope)?;
        let DataType::Struct(fields) = &ty else {
            self.error(
                range,
                format!(
                    "Cannot access field {name} on a value with type {}",
                    types::googlesql_name(&ty)
                ),
            );
            return None;
        };
        let found: Vec<&Arc<Field>> = (fields.iter())
            .filter(|field| field.name().eq_ignore_ascii_case(name))
            .collect();
        match found.as_slice() {
            [field] => {
                self.use_name(
                    range,
                    Definition::Field {
                        name: field.name().clone(),
                        ty: Some(field.data_type().clone()),
                    },
                );
                Some(core::get_field(base, field.name().as_str()))
            }
            [] => {
                self.error(
                    range,
                    format!(
                        "Field name {name} does not exist in {}",
                        types::googlesql_name(&ty)
                    ),
                );
                None
            }
            _ => {
                self.error(range, format!("Field name {name} is ambiguous"));
                None
            }
        }
    }

    /// `array[index]`, `array[OFFSET(i)]`, and the other subscripts of an array.
    fn subscript(&mut self, subscript: &ast::SubscriptExpr, scope: Scope) -> Option<Expr> {
        let base = self.child(subscript.base(), scope);
        let index = subscript.index()?;
        let (mode, index) = match &index {
            ast::Expr::CallExpr(call) => {
                let names = call.path().and_then(|path| path.names());
                let mode = match names.as_deref() {
                    Some([name]) => Subscript::parse(name),
                    _ => None,
                };
                match mode {
                    Some(mode) => {
                        let arg = call
                            .arg_list()
                            .and_then(|list| list.arguments().next())
                            .and_then(|argument| match argument {
                                ast::Argument::Arg(arg) => arg.expr(),
                                _ => None,
                            });
                        (mode, arg?)
                    }
                    None => (Subscript::Offset, index.clone()),
                }
            }
            _ => (Subscript::Offset, index.clone()),
        };
        let index_range = index.syntax().text_range();
        let planned = self.expr(&index, scope);
        let base = base?;
        let ty = self.type_of(&base, scope)?;
        if !matches!(Ty::of_arrow(&ty), Some(Ty::Array(_))) {
            self.error(
                subscript.syntax().text_range(),
                format!(
                    "Element access using [] is not supported on values of type {}",
                    types::googlesql_name(&ty)
                ),
            );
            return None;
        }
        let index = planned?;
        match Ty::of_arrow(&self.type_of(&index, scope)?) {
            Some(Ty::Int64 | Ty::Null) => {}
            _ => {
                let name = self.type_name(&index, scope);
                self.error(
                    index_range,
                    format!("An array index takes an INT64, not {name}"),
                );
                return None;
            }
        }
        Some(mode.element(base, index))
    }

    fn prefix(&mut self, prefix: &ast::PrefixExpr, scope: Scope) -> Option<Expr> {
        let range = prefix.syntax().text_range();
        let op = prefix.operator()?;
        let operand = prefix.expr()?;
        // A negated number is a literal, as `-9223372036854775808` is the least `INT64`.
        if op == PrefixOp::Neg {
            if let ast::Expr::Literal(literal) = &operand {
                match literal.kind() {
                    Some(LiteralKind::Int) => {
                        let value = literal.int_value().map(|value| -i128::from(value));
                        return match value.map(i64::try_from) {
                            Some(Ok(value)) => Some(int(value)),
                            _ => {
                                self.error(
                                    range,
                                    format!("Invalid integer literal: {}", prefix.syntax()),
                                );
                                None
                            }
                        };
                    }
                    Some(LiteralKind::Float) => {
                        let value = -literal.float_value()?;
                        return Some(Expr::Literal(ScalarValue::Float64(Some(value)), None));
                    }
                    _ => {}
                }
            }
        }
        let operand_range = operand.syntax().text_range();
        let value = self.expr(&operand, scope)?;
        match op {
            PrefixOp::Neg => {
                self.operator("$unary_minus", vec![(value, operand_range)], range, scope)
            }
            PrefixOp::Not => self.operator("$not", vec![(value, operand_range)], range, scope),
            PrefixOp::BitNot => {
                self.operator("$bitwise_not", vec![(value, operand_range)], range, scope)
            }
            PrefixOp::Plus => {
                match Ty::of_arrow(&self.type_of(&value, scope)?) {
                    Some(Ty::Int64 | Ty::Float64 | Ty::Numeric | Ty::BigNumeric | Ty::Interval) => {
                        Some(value)
                    }
                    _ => {
                        let name = self.type_name(&value, scope);
                        self.error(
                            range,
                            format!("No matching signature for operator unary + for argument types: {name}"),
                        );
                        None
                    }
                }
            }
        }
    }

    fn binary(&mut self, bin: &ast::BinExpr, scope: Scope) -> Option<Expr> {
        let range = bin.syntax().text_range();
        let op = bin.operator()?;
        let operands = self.operands([bin.lhs(), bin.rhs()], scope)?;
        let name = match op {
            BinaryOp::Or => "$or",
            BinaryOp::And => "$and",
            BinaryOp::Eq => "$equal",
            BinaryOp::NotEq => "$not_equal",
            BinaryOp::Lt => "$less",
            BinaryOp::LtEq => "$less_or_equal",
            BinaryOp::Gt => "$greater",
            BinaryOp::GtEq => "$greater_or_equal",
            BinaryOp::BitOr => "$bitwise_or",
            BinaryOp::BitXor => "$bitwise_xor",
            BinaryOp::BitAnd => "$bitwise_and",
            BinaryOp::ShiftLeft => "$bitwise_left_shift",
            BinaryOp::ShiftRight => "$bitwise_right_shift",
            BinaryOp::Add => "$add",
            BinaryOp::Sub => "$subtract",
            BinaryOp::Mul => "$multiply",
            BinaryOp::Div => "$divide",
            BinaryOp::Concat => return self.concat(operands, range, scope),
        };
        self.operator(name, operands, range, scope)
    }

    /// `x || y`, which GoogleSQL resolves to `CONCAT`, or to `ARRAY_CONCAT` for arrays.
    fn concat(
        &mut self,
        operands: Vec<(Expr, TextRange)>,
        range: TextRange,
        scope: Scope,
    ) -> Option<Expr> {
        if operands.iter().any(|(expr, _)| is_null_literal(expr)) {
            self.error(range, "Operands of || cannot be literal NULL");
            return None;
        }
        let array = matches!(
            Ty::of_arrow(&self.type_of(&operands[0].0, scope)?),
            Some(Ty::Array(_))
        );
        let function = match array {
            true => "ARRAY_CONCAT",
            false => "CONCAT",
        };
        self.plan_function(function, operands, range, scope)
    }

    /// A call of the library's scalar function `name` of `operands`, for an operator that
    /// GoogleSQL resolves to a function.
    fn plan_function(
        &mut self,
        name: &str,
        operands: Vec<(Expr, TextRange)>,
        range: TextRange,
        scope: Scope,
    ) -> Option<Expr> {
        let library = self.library;
        let function = library.get(name).expect("a function of the library");
        self.resolve(function, operands, range, scope, |error| error.message)
    }

    /// The plan of the scalar `function` of `operands`, each with its range, at `range`, which
    /// records the form that resolution picked, or reports the error that `message` writes at
    /// its argument or at `range`.
    fn resolve(
        &mut self,
        function: &Function,
        operands: Vec<(Expr, TextRange)>,
        range: TextRange,
        scope: Scope,
        message: impl FnOnce(CallError) -> String,
    ) -> Option<Expr> {
        let schema = scope.schema?;
        let ranges: Vec<TextRange> = operands.iter().map(|(_, range)| *range).collect();
        let args = (operands.into_iter())
            .map(|(expr, _)| (None, Arg::Value(expr)))
            .collect();
        match function.plan_resolved(args, false, false, self.context(schema)) {
            Ok((Planned::Scalar(expr), resolved)) => {
                self.model.calls.push(CallForm {
                    range,
                    form: resolved.render(&function.name),
                    index: Some(resolved.form),
                });
                Some(expr)
            }
            Ok(_) => unreachable!("a scalar function"),
            Err(error) => {
                let at = error_range(&error, &ranges, range);
                self.error(at, message(error));
                None
            }
        }
    }

    fn in_expr(&mut self, in_expr: &ast::InExpr, scope: Scope) -> Option<Expr> {
        let range = in_expr.syntax().text_range();
        let value = self.operands([in_expr.expr()], scope);
        let value = match in_expr.in_set()? {
            ast::InSet::InList(list) => {
                let values: Vec<Option<(Expr, TextRange)>> = list
                    .exprs()
                    .map(|expr| {
                        let range = expr.syntax().text_range();
                        Some((self.expr(&expr, scope)?, range))
                    })
                    .collect();
                let mut operands = value?;
                operands.extend(values.into_iter().collect::<Option<Vec<_>>>()?);
                self.operator("$in", operands, range, scope)?
            }
            ast::InSet::InUnnest(unnest) => {
                let array = self.operands([unnest.expr()], scope);
                let mut operands = value?;
                operands.extend(array?);
                self.operator("$in_array", operands, range, scope)?
            }
            ast::InSet::InQuery(query) => {
                let rows = self.subquery_rows(query.paren_query()?, scope);
                let (value, value_range) = value?.pop()?;
                let rows = rows?;
                if rows.columns.len() != 1 {
                    self.error(
                        range,
                        "Subquery of type IN must have only one output column",
                    );
                    return None;
                }
                // The value and the column of the query take their common type.
                let column_ty = rows.ty(&rows.columns[0]).and_then(|ty| Ty::of_arrow(&ty));
                let value_ty = self.observed(&value, scope);
                let (Some(column_ty), Some(value_ty)) = (column_ty, value_ty) else {
                    return None;
                };
                let Some(ty) = supertype(&[value_ty.clone(), (column_ty.clone(), false)]) else {
                    self.error(
                        value_range,
                        format!(
                            "Cannot execute IN subquery with uncomparable types {} and {column_ty}",
                            value_ty.0
                        ),
                    );
                    return None;
                };
                let value = self.coerce_to(value, &ty, scope)?;
                let mut subquery = self.subquery_of(rows, range, false)?;
                if column_ty != ty {
                    subquery = cast_subquery(subquery, ty.arrow(self.type_options))?;
                }
                return Some(Expr::InSubquery(InSubquery::new(
                    Box::new(value),
                    subquery,
                    in_expr.not_kw().is_some(),
                )));
            }
        };
        self.negated(value, in_expr.not_kw().is_some(), range, scope)
    }

    fn like(&mut self, like: &ast::LikeExpr, scope: Scope) -> Option<Expr> {
        let range = like.syntax().text_range();
        let value = self.operands([like.value()], scope);
        let matched = match like.quantified_patterns() {
            None => {
                let pattern = self.operands([like.pattern()], scope);
                let mut operands = value?;
                operands.extend(pattern?);
                self.operator("$like", operands, range, scope)?
            }
            Some(quantified) => {
                let name = match (like.not_kw().is_some(), quantified.all_kw().is_some()) {
                    (false, false) => "$like_any",
                    (false, true) => "$like_all",
                    (true, false) => "$not_like_any",
                    (true, true) => "$not_like_all",
                };
                let list = match quantified.in_set()? {
                    ast::InSet::InList(list) => list,
                    // The patterns of an array.
                    ast::InSet::InUnnest(unnest) => {
                        let array = unnest.expr()?;
                        let at = array.syntax().text_range();
                        let patterns = self.expr(&array, scope)?;
                        let mut operands = value?;
                        operands.push((patterns, at));
                        let name = format!("{name}_unnest");
                        return self.operator(&name, operands, range, scope);
                    }
                    ast::InSet::InQuery(_) => {
                        self.error(
                            range,
                            "LIKE ANY and LIKE ALL take a list of patterns or UNNEST of an array, not a query",
                        );
                        return None;
                    }
                };
                let patterns: Vec<Option<(Expr, TextRange)>> = list
                    .exprs()
                    .map(|expr| {
                        let range = expr.syntax().text_range();
                        Some((self.expr(&expr, scope)?, range))
                    })
                    .collect();
                let mut operands = value?;
                operands.extend(patterns.into_iter().collect::<Option<Vec<_>>>()?);
                return self.operator(name, operands, range, scope);
            }
        };
        self.negated(matched, like.not_kw().is_some(), range, scope)
    }

    fn case(&mut self, case: &ast::CaseExpr, scope: Scope) -> Option<Expr> {
        let range = case.syntax().text_range();
        let operand = case
            .operand()
            .map(|operand| (self.expr(&operand, scope), operand.syntax().text_range()));
        let whens: Vec<(Option<Expr>, TextRange, Option<Expr>)> = case
            .when_clauses()
            .map(|when| {
                let condition = when.condition();
                let at = condition
                    .as_ref()
                    .map_or(when.syntax().text_range(), |c| c.syntax().text_range());
                let condition = self.child(condition, scope);
                let result = self.child(when.result(), scope);
                (condition, at, result)
            })
            .collect();
        let otherwise = case
            .else_clause()
            .map(|clause| self.child(clause.expr(), scope));
        let mut conditions = Vec::new();
        let mut results = Vec::new();
        for (condition, at, result) in whens {
            conditions.push((condition?, at));
            results.push(result?);
        }
        let otherwise = match otherwise {
            Some(value) => Some(value?),
            None => None,
        };
        let operand = match operand {
            Some((operand, at)) => Some((operand?, at)),
            None => None,
        };
        let conditions: Vec<Expr> = match operand {
            Some((operand, _)) => {
                let mut values = vec![operand.clone()];
                values.extend(conditions.iter().map(|(condition, _)| condition.clone()));
                let (mut values, _) = self.common(values, range, "The values of CASE", scope)?;
                let operand = values.remove(0);
                (values.into_iter())
                    .map(|value| operand.clone().eq(value))
                    .collect()
            }
            None => (conditions.into_iter())
                .map(|(condition, at)| self.boolean(condition, at, "A WHEN condition", scope))
                .collect::<Option<_>>()?,
        };
        let mut values = results;
        values.extend(otherwise.clone());
        let (mut values, ty) = self.common(values, range, "The results of CASE", scope)?;
        let otherwise = match otherwise {
            Some(_) => values.pop().expect("the ELSE value"),
            None => cast(
                Expr::Literal(ScalarValue::Null, None),
                ty.arrow(self.type_options),
            ),
        };
        let when_then = (conditions.into_iter().zip(values))
            .map(|(when, then)| (Box::new(when), Box::new(then)))
            .collect();
        Some(Expr::Case(Case::new(
            None,
            when_then,
            Some(Box::new(otherwise)),
        )))
    }

    fn cast(&mut self, cast_expr: &ast::CastExpr, scope: Scope) -> Option<Expr> {
        let range = cast_expr.syntax().text_range();
        let value = self.child(cast_expr.expr(), scope);
        let target = cast_expr.ty()?;
        if let Some(clause) = cast_expr.format_clause() {
            self.error(
                clause.syntax().text_range(),
                "avenger-sql doesn't take CAST ... FORMAT: format and parse dates and times with \
                 FORMAT_DATE, FORMAT_TIME, FORMAT_DATETIME, FORMAT_TIMESTAMP, PARSE_DATE, \
                 PARSE_TIME, PARSE_DATETIME and PARSE_TIMESTAMP, and format the numbers and dates of a chart in its guides",
            );
            return None;
        }
        let target = match types::arrow_type(&target, self.type_options) {
            Ok(target) => target,
            Err(error) => {
                self.error(error.range, error.message);
                return None;
            }
        };
        let value = value?;
        let from = self.type_of(&value, scope)?;
        let castable = match (Ty::of_arrow(&from), Ty::of_arrow(&target)) {
            (Some(from), Some(to)) => castable(&from, &to),
            _ => false,
        };
        if !castable {
            self.error(
                range,
                format!(
                    "Invalid cast from {} to {}",
                    types::googlesql_name(&from),
                    types::googlesql_name(&target)
                ),
            );
            return None;
        }
        // GoogleSQL writes floats, decimals, and dates and times in its own way.
        if Ty::of_arrow(&target) == Some(Ty::String) {
            let written = (Ty::of_arrow(&from)).and_then(|from| {
                library::cast_to_string(value.clone(), &from, &self.options.time_zone)
            });
            if let Some(written) = written {
                return Some(cast(written, target));
            }
        }
        // GoogleSQL rounds a float or a decimal to an integer half away from zero, where Arrow's
        // cast truncates.
        let value = match (from.is_floating() || from.is_decimal(), target.is_integer()) {
            (true, true) => datafusion::functions::math::expr_fn::round(vec![value]),
            _ => value,
        };
        Some(match cast_expr.safe_cast_kw() {
            Some(_) => Expr::TryCast(TryCast::new(Box::new(value), target)),
            None => cast(value, target),
        })
    }

    fn extract(&mut self, extract: &ast::ExtractExpr, scope: Scope) -> Option<Expr> {
        let range = extract.syntax().text_range();
        let value = self.child(extract.value(), scope);
        let zone = match extract.time_zone() {
            Some(zone) => {
                let zone = self.child(zone.expr(), scope)?;
                match &zone {
                    Expr::Literal(value, _) => match value.try_as_str() {
                        Some(Some(zone)) => Some(zone.to_string()),
                        _ => None,
                    },
                    _ => None,
                }
                .or_else(|| {
                    self.error(
                        range,
                        "avenger-sql takes the time zone of EXTRACT as a string literal",
                    );
                    None
                })?
                .into()
            }
            None => None,
        };
        let part_expr = extract.part()?;
        let Some(part) = date_part(&part_expr) else {
            self.error(
                part_expr.syntax().text_range(),
                format!(
                    "A valid date part name is required but found {}",
                    part_expr.syntax()
                ),
            );
            return None;
        };
        let value = value?;
        let ty = Ty::of_arrow(&self.type_of(&value, scope)?);
        let zone = zone.unwrap_or_else(|| self.options.time_zone.clone());
        match ty.and_then(|ty| library::extract(part, value, &ty, &zone)) {
            Some(expr) => Some(expr),
            None => {
                self.error(
                    range,
                    format!("EXTRACT doesn't take {} from this value", part.name()),
                );
                None
            }
        }
    }

    fn array(&mut self, array: &ast::ArrayExpr, scope: Scope) -> Option<Expr> {
        let range = array.syntax().text_range();
        let elements: Vec<Option<Expr>> = array.exprs().map(|e| self.expr(&e, scope)).collect();
        let declared = match array.ty() {
            Some(ty) => match types::arrow_type(&ty, self.type_options) {
                Ok(ty) => Some(Ty::of_arrow(&ty)?),
                Err(error) => {
                    self.error(error.range, error.message);
                    return None;
                }
            },
            None => None,
        };
        let elements: Vec<Expr> = elements.into_iter().collect::<Option<_>>()?;
        scope.schema?;
        let (elements, ty) = match declared {
            Some(ty) => {
                let elements = (elements.into_iter())
                    .map(|element| self.coerce_to(element, &ty, scope))
                    .collect::<Option<_>>()?;
                (elements, ty)
            }
            // `[]` coerces to an array of any type.
            None if elements.is_empty() => return Some(library::empty_array()),
            None => self.common(elements, range, "The elements of an array", scope)?,
        };
        if matches!(ty, Ty::Array(_)) {
            self.error(
                range,
                format!("Cannot construct array with element type {ty} because nested arrays are not supported"),
            );
            return None;
        }
        let list = Ty::Array(Box::new(ty)).arrow(self.type_options);
        Some(cast(make_array(elements), list))
    }

    fn structure(&mut self, structure: &ast::StructExpr, scope: Scope) -> Option<Expr> {
        let range = structure.syntax().text_range();
        let declared: Vec<(Option<String>, Option<ast::Type>)> = structure
            .struct_fields()
            .map(|field| (field.name().and_then(|name| name.text()), field.ty()))
            .collect();
        let values: Vec<(Option<Expr>, String)> = structure
            .aliased_exprs()
            .map(|aliased| {
                let name = match aliased.alias().and_then(|alias| alias.name()) {
                    Some(name) => name.text().unwrap_or_default(),
                    None => aliased
                        .expr()
                        .map(|expr| inferred_name(&expr))
                        .unwrap_or_default(),
                };
                (self.child(aliased.expr(), scope), name)
            })
            .collect();
        if !declared.is_empty() && declared.len() != values.len() {
            self.error(
                range,
                format!(
                    "STRUCT type has {} fields but constructor call has {} fields",
                    declared.len(),
                    values.len()
                ),
            );
            return None;
        }
        let mut args = Vec::new();
        for (index, (value, name)) in values.into_iter().enumerate() {
            let mut value = value?;
            let mut name = name;
            if let Some((declared_name, ty)) = declared.get(index) {
                if let Some(declared_name) = declared_name {
                    name = declared_name.clone();
                }
                let ty = ty.as_ref()?;
                let ty = match types::arrow_type(ty, self.type_options) {
                    Ok(ty) => Ty::of_arrow(&ty)?,
                    Err(error) => {
                        self.error(error.range, error.message);
                        return None;
                    }
                };
                value = self.coerce_to(value, &ty, scope)?;
            }
            if name.is_empty() {
                name = format!("_field_{}", index + 1);
            }
            args.extend([self.string_literal(&name), value]);
        }
        Some(core::named_struct(args))
    }

    /// The sort keys of an `ORDER BY` in `scope`.
    pub(crate) fn sort_keys(
        &mut self,
        order: &ast::OrderBy,
        scope: Scope,
    ) -> Option<Vec<SortExpr>> {
        let keys: Vec<Option<SortExpr>> = order
            .order_keys()
            .map(|key| {
                let expr = self.child(key.expr(), scope)?;
                let (asc, nulls_first) = direction(&key);
                Some(SortExpr::new(expr, asc, nulls_first))
            })
            .collect();
        keys.into_iter().collect()
    }

    /// A call of the library through its function's forms, with its clauses and its `OVER`.
    fn call(&mut self, call: &ast::CallExpr, scope: Scope) -> Option<Expr> {
        let range = call.syntax().text_range();
        let path = call.path()?;
        let refs: Vec<ast::NameRef> = path.name_refs().collect();
        let names = path.names()?;
        let library = self.library;
        // A constructor of a chart's option, by the name that the parameter of its argument gives
        // it, before a temporary function and the library's functions.
        let argument = match names.as_slice() {
            [_] => context::argument(library, call.syntax()),
            _ => None,
        };
        let contextual = (argument.as_ref())
            .and_then(|argument| context::contextual(library, argument, &names[0]));
        // A temporary function, or a function of a module.
        if let (None, Some(function)) = (&contextual, self.temp_function(&names)) {
            return self.call_temp(function, call, scope, path.syntax().text_range());
        }
        let (safe, name, name_range) = match names.as_slice() {
            [name] => (false, name.clone(), refs[0].syntax().text_range()),
            [safe, name] if safe.eq_ignore_ascii_case("SAFE") => {
                (true, name.clone(), refs[1].syntax().text_range())
            }
            _ => {
                let message = self.function_not_found(&names.join("."));
                self.error(path.syntax().text_range(), message);
                return None;
            }
        };
        // The constructor of a place, under the name that the call writes, or the library's
        // function.
        let constructed: Function;
        let (function, fills) = match contextual {
            Some(called) => {
                let fills = called.fills.expect("a constructor fills a place");
                if fills.builds.array != fills.element {
                    let param = &argument.as_ref().expect("an argument").param.name;
                    let message = match fills.builds.array {
                        true => format!(
                            "`{param}` takes an array of its elements, as `{param} => [{name}(...)]`"
                        ),
                        false => format!(
                            "`{param}` takes one `{name}(...)`, not an array of them"
                        ),
                    };
                    self.error(range, message);
                    return None;
                }
                self.model.constructors.push(model::ConstructorUse {
                    range: name_range,
                    constructor: fills.builds.constructor.clone(),
                    place: fills.describe(),
                });
                let mut renamed = called.function.clone();
                renamed.name = called.name.clone();
                constructed = renamed;
                (&constructed, Some(fills))
            }
            None => {
                if let Some(message) =
                    (argument.as_ref()).and_then(|argument| self.misplaced(argument, call, &name))
                {
                    self.error(range, message);
                    return None;
                }
                match library.get(&name) {
                    Some(function) => (function, None),
                    None => {
                        let places = library.constructor_places(&name);
                        let channel = avenger_chart::functions::channel_function(&name);
                        let message = match places.is_empty() {
                            true => self.function_not_found(&name),
                            false if channel.is_some() => format!(
                                "`{name}(...)` sets the channel `{name}` of a mark, so only the \
                                 value of that channel holds it, as {}",
                                places.join(" or ")
                            ),
                            false => format!(
                                "`{name}(...)` builds an option of a chart, so only a chart \
                                 function's options hold it, as {}",
                                places.join(" or ")
                            ),
                        };
                        self.error(name_range, message);
                        return None;
                    }
                }
            }
        };
        self.use_name(
            name_range,
            Definition::Function {
                name: function.name.clone(),
                role: function.role(),
                definition: None,
            },
        );
        if function.name == "lump" {
            return self.lump(call, scope, range);
        }
        if function.kind == library::FunctionKind::Table {
            self.error(
                range,
                format!("Table-valued function {} goes in FROM", function.name),
            );
            return None;
        }
        let (fname, clauses) = (&function.name, function.clauses);
        if !self.mr_call_allowed(fname, call, scope, safe) {
            return None;
        }
        let list = call.arg_list();
        let over = call.over_clause();
        let aggregate = matches!(function.kind, library::FunctionKind::Aggregate);
        let window = over.is_some() || matches!(function.kind, library::FunctionKind::Window);
        // In the `keep` of a lump, a window runs over the categories, so its arguments and its
        // keys read the aggregates of each category's rows.
        // A window in a mark's channel reads the aggregates of the call's keys too.
        let defaulted = scope.marked && over.is_none() && function.default_window.is_some();
        let window_scope = match (self.categories.is_some() || (scope.marked && scope.aggregates))
            && (over.is_some() || defaulted)
        {
            true => scope.arguments().aggregates(),
            false => scope.arguments(),
        };
        // A mark in an item of a pipe `AGGREGATE` takes aggregates in its channels, which the
        // operator plans as a branch.
        let marked = function.family == library::Family::Mark
            && scope.aggregates
            && scope.clause == AGGREGATE_ITEMS
            && over.is_none();
        let arg_scope = match (aggregate || window, over.is_some()) {
            _ if marked => Scope {
                clause: MARK_CHANNEL,
                aggregates: true,
                windows: true,
                marked: true,
                ..scope
            },
            (true, true) => window_scope,
            (true, false) if defaulted => window_scope,
            (true, false) => scope.arguments(),
            (false, _) => scope,
        };
        if let (true, Some(list)) = (aggregate || window, &list) {
            self.place(
                list.syntax().text_range(),
                PlaceKind::Argument,
                &arg_scope,
                None,
            );
        }
        let star = list.as_ref().is_some_and(|list| list.star_arg().is_some());
        let distinct = list
            .as_ref()
            .is_some_and(|list| list.distinct_kw().is_some());
        let mut ok = true;
        if distinct && !clauses.distinct {
            self.error(range, format!("{fname} does not support DISTINCT"));
            ok = false;
        }
        let null_treatment = list
            .as_ref()
            .and_then(|list| list.null_treatment())
            .map(|nulls| match nulls.ignore_kw() {
                Some(_) => NullTreatment::IgnoreNulls,
                None => NullTreatment::RespectNulls,
            });
        if null_treatment.is_some() && !clauses.null_treatment {
            self.error(
                range,
                format!("IGNORE NULLS and RESPECT NULLS are not supported on {fname}"),
            );
            ok = false;
        }
        if let Some(list) = &list {
            if list.order_by().is_some() && !clauses.order_by {
                self.error(
                    range,
                    format!("{fname} does not support ORDER BY in arguments"),
                );
                ok = false;
            }
            if list.having_clause().is_some() && (!clauses.having || over.is_some()) {
                self.error(
                    range,
                    format!("{fname} does not support HAVING MAX or HAVING MIN"),
                );
                ok = false;
            }
            if list.call_limit().is_some() && !clauses.limit {
                self.error(
                    range,
                    format!("Aggregate function {fname} does not support LIMIT in arguments"),
                );
                ok = false;
            }
        }
        // `HAVING MAX x` or `HAVING MIN x`, and `LIMIT n`.
        let having = (list.as_ref().and_then(|list| list.having_clause())).map(|having| {
            let value = having
                .expr()
                .and_then(|expr| self.expr(&expr, scope.arguments()));
            (having.max_kw().is_some(), value)
        });
        let limit = (list.as_ref().and_then(|list| list.call_limit()))
            .map(|limit| limit.expr().and_then(|expr| self.constant(&expr)));
        match (clauses.over, &over) {
            (Over::Forbidden, Some(over)) => {
                self.error(
                    over.syntax().text_range(),
                    format!("{fname} does not support an OVER clause"),
                );
                ok = false;
            }
            (Over::Required { .. }, None)
                if !(scope.marked && function.default_window.is_some()) =>
            {
                self.error(
                    range,
                    format!("Analytic function {fname} cannot be called without an OVER clause"),
                );
                ok = false;
            }
            _ => {}
        }
        // The arguments, each by the kind of its parameter.
        let mut args = Vec::new();
        let mut ranges = Vec::new();
        // The range of each argument of `args` that is a string literal.
        let mut strings = Vec::new();
        // The channel values of a mark, as the user wrote them, and the count of its positional
        // arguments so far, which name their channels in the order of the form.
        let mut written = Vec::new();
        let mut positional = 0;
        // The count of the positional arguments before each, which names their parameters.
        let mut before = 0;
        // The string literals of the arguments that take Typst markup, and whether a `syntax`
        // argument of a constructor, such as that of `title(...)`, reads them as plain text.
        let mut markup = Vec::new();
        let mut plain = false;
        for (position, argument) in list.iter().flat_map(|list| list.arguments()).enumerate() {
            ranges.push(argument.syntax().text_range());
            let (named, value) = match &argument {
                ast::Argument::Arg(arg) => (None, arg.expr()),
                ast::Argument::NamedArg(arg) => {
                    (arg.name_ref().and_then(|name| name.text()), arg.expr())
                }
                ast::Argument::TableArg(_) | ast::Argument::NamedTableArg(_) => {
                    self.error(
                        argument.syntax().text_range(),
                        format!("{fname} takes no TABLE argument"),
                    );
                    ok = false;
                    continue;
                }
            };
            let param = match &named {
                Some(name) => Some(name.clone()),
                None => {
                    before += 1;
                    function.positional_param(before - 1)
                }
            };
            let Some(value) = value else {
                ok = false;
                continue;
            };
            let takes = param
                .as_deref()
                .and_then(|param| function.param_named(param));
            if takes.is_some_and(|param| param.takes.markup) {
                markup.extend(string_literals(&value));
            }
            let syntax = named
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("syntax"));
            if syntax && function.family == library::Family::Constructor {
                plain = !is_typst_syntax(&value);
            }
            // A name that no parameter of a chart function has, before its value, which would
            // resolve against no parameter.
            let chart = matches!(
                function.family,
                library::Family::Mark | library::Family::Composition | library::Family::Constructor
            );
            if let (true, Some(name)) = (chart, &named) {
                if function.param_named(name).is_none() {
                    let range = argument.syntax().text_range();
                    let params: Vec<String> = (function.params.iter())
                        .map(|param| format!("`{}`", param.name))
                        .collect();
                    let mut message = function.unknown_parameter(name);
                    match params.as_slice() {
                        [only] => message.push_str(&format!(", only {only}")),
                        [first, last] => {
                            message.push_str(&format!(": its parameters are {first} and {last}"))
                        }
                        [rest @ .., last] if params.len() <= 8 => message.push_str(&format!(
                            ": its parameters are {}, and {last}",
                            rest.join(", ")
                        )),
                        _ => {}
                    }
                    self.error(range, message);
                    ok = false;
                    continue;
                }
            }
            // A literal that a parameter of a chart function does not take.
            let unaccepted = (param.as_deref())
                .and_then(|param| function.param_named(param))
                .and_then(|param| self.unaccepted(param, &value));
            if let Some(message) = unaccepted {
                self.error(value.syntax().text_range(), message);
                ok = false;
                continue;
            }
            if function.takes_date_part(position, named.as_deref()) {
                match date_part(&value) {
                    Some(part) => {
                        args.push((named, Arg::DatePart(part)));
                        strings.push(None);
                    }
                    None => {
                        self.error(
                            value.syntax().text_range(),
                            format!(
                                "A valid date part name is required but found {}",
                                value.syntax()
                            ),
                        );
                        ok = false;
                    }
                }
                continue;
            }
            match self.expr(&value, arg_scope) {
                Some(expr) => {
                    if marked {
                        let channel = match &named {
                            Some(name) => name.to_ascii_lowercase(),
                            None => {
                                positional += 1;
                                (function.forms.first())
                                    .and_then(|form| {
                                        form.signature.positional().nth(positional - 1)
                                    })
                                    .and_then(|index| function.forms[0].signature.args.get(index))
                                    .map(|arg| arg.param.to_ascii_lowercase())
                                    .unwrap_or_default()
                            }
                        };
                        written.push(lifting::Written {
                            node: lifting::channel_node(&value, &channel),
                            value: lifting::channel_value(&expr).clone(),
                            range: value.syntax().text_range(),
                            text: lifting::channel_title(&value, &channel, self.library),
                            full: lifting::channel_text(&value, &channel),
                            channel,
                        });
                    }
                    args.push((named, Arg::Value(expr)));
                    strings.push(string_literal(&value));
                }
                None => ok = false,
            }
        }
        // A plain syntax holds for the strings of the constructors in the call too, such as a
        // title's subtitle, whatever errors the call has.
        match plain {
            true => (self.model.markup).retain(|literal| !range.contains_range(*literal)),
            false => self.model.markup.extend(markup),
        }
        let order_by = match list.as_ref().and_then(|list| list.order_by()) {
            Some(order) => self.sort_keys(&order, arg_scope),
            None => Some(Vec::new()),
        };
        let over = match &over {
            Some(over) => self.over(over, fname, clauses.over, window_scope).map(Some),
            None => Some(None),
        };
        let (order_by, over) = (order_by?, over?);
        if !ok {
            return None;
        }
        let schema = scope.schema?;
        let (planned, resolved) =
            match function.plan_resolved(args, star, safe, self.context(schema)) {
                Ok(planned) => planned,
                Err(error) => {
                    self.error(error_range(&error, &ranges, name_range), error.message);
                    return None;
                }
            };
        self.model.calls.push(CallForm {
            range,
            form: resolved.render(fname),
            index: Some(resolved.form),
        });
        for (at, param) in &resolved.enums {
            if let Some(range) = strings[*at] {
                self.model.enum_values.push(EnumValue {
                    range,
                    param: param.clone(),
                });
            }
        }
        let typed = |expr: Expr, result: Option<DataType>| match result {
            Some(ty) => cast(expr, ty),
            None => expr,
        };
        match planned {
            Planned::Scalar(expr) => match fills {
                // A constructor one level down, in the parameter's own constructor.
                Some(fills) => self.wrap(expr, &fills, argument.as_ref(), range, schema),
                None => Some(expr),
            },
            Planned::Aggregate {
                func,
                args,
                filter,
                result,
            } => match over {
                Some(over) => {
                    if !scope.windows {
                        self.error(
                            range,
                            format!("Analytic function {fname} not allowed in {}", scope.clause),
                        );
                        return None;
                    }
                    // DataFusion's windows take an aggregate's `DISTINCT` and its filter, as of
                    // `COUNTIF`.
                    let mut window = over.window(
                        WindowFunctionDefinition::AggregateUDF(func),
                        args,
                        null_treatment,
                    );
                    if let Expr::WindowFunction(function) = &mut window {
                        if distinct && !function.params.order_by.is_empty() {
                            self.error(
                                range,
                                "Window ORDER BY is not allowed if DISTINCT is specified",
                            );
                            return None;
                        }
                        function.params.distinct = distinct;
                        function.params.filter = filter.map(Box::new);
                    }
                    Some(typed(window, result))
                }
                None => {
                    if !scope.aggregates {
                        let message = match scope.clause {
                            ARGUMENT => "Aggregations of aggregations are not allowed".to_string(),
                            clause => format!("Aggregate function {fname} not allowed in {clause}"),
                        };
                        self.error(range, message);
                        return None;
                    }
                    let mut filter = filter;
                    if let Some((max, value)) = having {
                        let value = value?;
                        let Some(groups) = scope.groups else {
                            self.error(
                                range,
                                "The keep of lump doesn't take HAVING MAX or HAVING MIN",
                            );
                            return None;
                        };
                        // The rows of the greatest or the least value of the group.
                        let extreme = match max {
                            true => datafusion::functions_aggregate::min_max::max_udaf(),
                            false => datafusion::functions_aggregate::min_max::min_udaf(),
                        };
                        let extreme = Window {
                            partition: groups.to_vec(),
                            order: Vec::new(),
                            frame: WindowFrame::new(None),
                        }
                        .window(
                            WindowFunctionDefinition::AggregateUDF(extreme),
                            vec![value.clone()],
                            None,
                        );
                        let condition = value.eq(extreme);
                        filter = Some(match filter {
                            Some(filter) => filter.and(condition),
                            None => condition,
                        });
                    }
                    let (order_by, null_treatment) = self.mr_aggregate_order(
                        fname,
                        clauses.order_by,
                        distinct,
                        order_by,
                        null_treatment,
                    );
                    let aggregate = AggregateFunction::new_udf(
                        func,
                        args,
                        distinct,
                        filter.map(Box::new),
                        order_by,
                        null_treatment,
                    );
                    let aggregate = match limit {
                        Some(limit) => limited(fname, aggregate, limit?),
                        None => Expr::AggregateFunction(aggregate),
                    };
                    if marked {
                        let order: Vec<SortExpr> = match &aggregate {
                            Expr::AggregateFunction(call) => call.params.order_by.clone(),
                            _ => Vec::new(),
                        };
                        let order_by = list.as_ref().and_then(|list| list.order_by());
                        let order_range =
                            order_by.as_ref().map(|order| order.syntax().text_range());
                        let order_nodes: Vec<ast::Expr> = (order_by.iter())
                            .flat_map(|order| order.order_keys())
                            .filter_map(|key| key.expr())
                            .collect();
                        if !self.lift(
                            fname,
                            &aggregate,
                            &written,
                            &order,
                            &order_nodes,
                            order_range,
                            scope.groups,
                            range,
                        ) {
                            return None;
                        }
                    }
                    Some(typed(aggregate, result))
                }
            },
            Planned::Window { func, args, result } => {
                if !scope.windows {
                    self.error(
                        range,
                        format!("Analytic function {fname} not allowed in {}", scope.clause),
                    );
                    return None;
                }
                // A window without `OVER` in a mark's channel takes its function's default.
                let over = over.unwrap_or_else(|| Window {
                    partition: match function.default_window {
                        Some(DefaultWindow::Position) => vec![lifting::mark_window("position")],
                        Some(DefaultWindow::Whole) | None => Vec::new(),
                    },
                    order: Vec::new(),
                    frame: WindowFrame::new(None),
                });
                Some(typed(over.window(func, args, null_treatment), result))
            }
        }
    }

    /// `expr`, the fragment of a constructor that `fills` one level down from its parameter, in the
    /// call of the parameter's own constructor that it fills, as `scale => pin(...)` is
    /// `scale => scale(pin => pin(...))`. A constructor of the parameter itself is `expr`.
    fn wrap(
        &mut self,
        expr: Expr,
        fills: &context::Fills,
        argument: Option<&context::Argument>,
        range: TextRange,
        schema: &DFSchema,
    ) -> Option<Expr> {
        let (Some(part), Some(target)) = (&fills.builds.part, fills.target) else {
            return Some(expr);
        };
        let library = self.library;
        let mut function = library.constructor(target)?.clone();
        // The wrapper's errors name it as the parameter does.
        let own = (argument.into_iter())
            .flat_map(|argument| &argument.param.takes.constructors)
            .find(|builds| builds.part.is_none());
        if let Some(own) = own {
            function.name = own.name.clone();
        }
        let args = vec![(Some(part.clone()), Arg::Value(expr))];
        match function.plan(args, false, false, self.context(schema)) {
            Ok(Planned::Scalar(expr)) => Some(expr),
            Ok(_) => None,
            Err(error) => {
                self.error(range, error.message);
                None
            }
        }
    }

    /// The error of a call of a constructor named `name` in `argument`, whose parameter does not
    /// take it: what the parameter takes, and a parameter that takes the constructor, or where
    /// the constructor belongs. `None` for a name that no constructor has, and for a function of
    /// the library that no parameter near the argument takes as a constructor, as `LOG(x)`.
    fn misplaced(
        &self,
        argument: &context::Argument,
        call: &ast::CallExpr,
        name: &str,
    ) -> Option<String> {
        let library = self.library;
        let places = library.constructor_places(name);
        if places.is_empty() {
            return None;
        }
        // A call that the library's function of the name takes is that function's, as
        // `POW(10, 6)` in an explicit domain.
        let names: Vec<Option<String>> = (call.arg_list().iter())
            .flat_map(|list| list.arguments())
            .map(|argument| match argument {
                ast::Argument::NamedArg(named) => named.name_ref().and_then(|name| name.text()),
                _ => None,
            })
            .collect();
        if library
            .get(name)
            .is_some_and(|function| function.binds(&names))
        {
            return None;
        }
        // An argument of a function of GoogleSQL, as `CONCAT`, is outside the chart functions.
        let chart = matches!(
            argument.owner.function.family,
            library::Family::Mark | library::Family::Composition | library::Family::Constructor
        );
        if !chart {
            return None;
        }
        let suggestion = context::suggestion(argument, name, library);
        if suggestion.is_none() && library.get(name).is_some() {
            return None;
        }
        let param = &argument.param;
        let takes = match param.forms(Some(3)) {
            Some(forms) => format!("`{}` takes {}", param.name, forms.sentence()),
            None => format!("`{}` takes no constructor", param.name),
        };
        Some(match suggestion {
            Some(suggestion) => {
                format!("{takes}, not `{name}(...)`. Did you mean {suggestion}?")
            }
            None => format!(
                "{takes}, not `{name}(...)`, which belongs in {}",
                places.join(" or ")
            ),
        })
    }

    /// The error of a literal `value` of `param`, a parameter of a chart function that takes
    /// constructors, when the parameter does not take its kind of value: what the parameter takes,
    /// and a constructor one level down whose first parameter takes the value, as
    /// `axis => title('Bill length')` for `axis => 'Bill length'`.
    fn unaccepted(&self, param: &library::Param, value: &ast::Expr) -> Option<String> {
        let takes = &param.takes;
        if takes.constructors.is_empty() {
            return None;
        }
        let ast::Expr::Literal(literal) = value else {
            return None;
        };
        let text = literal.syntax().to_string();
        let string = literal.string_value();
        let kind = literal.kind()?;
        // Whether a type of a value takes the literal.
        let fits = |ty: &Ty| match kind {
            ast::LiteralKind::String => matches!(ty, Ty::String | Ty::Any),
            ast::LiteralKind::True | ast::LiteralKind::False => matches!(ty, Ty::Bool | Ty::Any),
            ast::LiteralKind::Int | ast::LiteralKind::Float => {
                matches!(ty, Ty::Int64 | Ty::Float64 | Ty::Any)
            }
            _ => true,
        };
        let found = match kind {
            ast::LiteralKind::String => text.clone(),
            ast::LiteralKind::True | ast::LiteralKind::False => "a Boolean".to_string(),
            ast::LiteralKind::Int | ast::LiteralKind::Float => "a number".to_string(),
            _ => return None,
        };
        let named = string
            .as_ref()
            .is_some_and(|string| takes.strings.iter().any(|value| value == string));
        if named || takes.values.iter().any(fits) {
            return None;
        }
        let forms = param.forms(Some(3))?;
        let mut message = format!("`{}` takes {}, not {found}", param.name, forms.sentence());
        // A constructor one level down whose first parameter takes the literal.
        let library = self.library;
        let hint = (takes.constructors.iter())
            .filter(|builds| builds.part.is_some() && !builds.array)
            .find(|builds| {
                let first = (library.constructor(&builds.constructor))
                    .and_then(|constructor| constructor.forms.first())
                    .and_then(|form| form.signature.args.first())
                    .filter(|first| !first.named_only);
                match first.map(|first| &first.accept) {
                    Some(library::Accept::OneOf(values)) => string
                        .as_ref()
                        .is_some_and(|string| values.contains(string)),
                    Some(library::Accept::Value(ty)) => fits(ty),
                    _ => false,
                }
            });
        if let Some(builds) = hint {
            message.push_str(&format!(
                ". Did you mean `{} => {}({text})`?",
                param.name, builds.name
            ));
        }
        Some(message)
    }

    /// The `OVER` clause of a call of the function `name`.
    pub(super) fn over(
        &mut self,
        over: &ast::OverClause,
        name: &str,
        clause: Over,
        scope: Scope,
    ) -> Option<Window> {
        let range = over.syntax().text_range();
        // A window's keys take no aggregate and no window function.
        let keys = Scope {
            aggregates: false,
            windows: false,
            ..scope
        };
        self.place(range, PlaceKind::OrderKey, &keys, None);
        // A window in a mark's channel can add to `panel`, `position`, or `series`, the windows of
        // the call, which the branch of the call resolves.
        let mut named: Option<String> = None;
        let (spec, ranked) = match self.categories.clone() {
            Some(ranked) => {
                let (spec, named) = self.category_window(over)?;
                (spec, named.then_some(ranked))
            }
            None if scope.marked => {
                let (spec, name) = self.mark_window(over)?;
                named = name;
                (spec, None)
            }
            None => (self.window_syntax(over)?, None),
        };
        if self.categories.is_some() && spec.partition.is_some() {
            self.error(
                range,
                "A window in the keep of lump runs over the categories of the lump's partition, \
                 so it takes no PARTITION BY",
            );
            return None;
        }
        let partition: Vec<Option<Expr>> = (spec.partition.iter())
            .flat_map(|partition| partition.exprs().collect::<Vec<_>>())
            .map(|expr| self.expr(&expr, scope))
            .collect();
        let order = match (&spec.order, ranked) {
            (_, Some(ranked)) => Some(ranked),
            (Some(order), None) => self.sort_keys(order, scope),
            (None, None) => Some(Vec::new()),
        };
        let (order_need, frame_allowed) = match clause {
            Over::Required { order_by, frame } => (order_by, frame),
            _ => (Need::Allowed, true),
        };
        let frame = match &spec.frame {
            Some(frame) if !frame_allowed => {
                self.error(
                    frame.syntax().text_range(),
                    format!("Window framing clause is not allowed for analytic function {name}"),
                );
                return None;
            }
            Some(frame) => Some(self.frame(frame, scope)?),
            None => None,
        };
        let mut partition: Vec<Expr> = partition.into_iter().collect::<Option<_>>()?;
        if let Some(name) = &named {
            partition = vec![lifting::mark_window(name)];
        }
        let order = order?;
        match order_need {
            // A window over a window of a mark call takes its default order.
            Need::Required if order.is_empty() && named.is_none() => {
                self.error(
                    range,
                    format!("Window ORDER BY is required for analytic function {name}"),
                );
                return None;
            }
            Need::Forbidden if !order.is_empty() => {
                self.error(
                    range,
                    format!("Window ORDER BY is not allowed for analytic function {name}"),
                );
                return None;
            }
            _ => {}
        }
        let frame = frame.unwrap_or_else(|| WindowFrame::new((!order.is_empty()).then_some(false)));
        Some(Window {
            partition,
            order,
            frame,
        })
    }

    /// The window of an `OVER` clause in the `keep` of a lump, and whether it is or adds to the
    /// window `ranked`, the only named window there.
    fn category_window(&mut self, over: &ast::OverClause) -> Option<(WindowSyntax, bool)> {
        let unknown = |pass: &mut Self, name: &ast::NameRef| {
            let text = name.text().unwrap_or_default();
            pass.error(
                name.syntax().text_range(),
                format!("Unrecognized window alias {text}; the keep of lump has the window ranked"),
            );
        };
        let ranked = |name: &ast::NameRef| {
            name.text()
                .is_some_and(|t| t.eq_ignore_ascii_case("ranked"))
        };
        if let Some(name) = over.name_ref() {
            if !ranked(&name) {
                unknown(self, &name);
                return None;
            }
            let none = WindowSyntax {
                partition: None,
                order: None,
                frame: None,
            };
            return Some((none, true));
        }
        let spec = over.window_spec()?;
        let own = WindowSyntax {
            partition: spec.partition_by(),
            order: spec.order_by(),
            frame: spec.frame(),
        };
        match spec.name_ref() {
            Some(name) if ranked(&name) => {
                if own.order.is_some() {
                    self.error(
                        spec.syntax().text_range(),
                        "ORDER BY cannot be specified when the named window ranked has one",
                    );
                    return None;
                }
                Some((own, true))
            }
            Some(name) => {
                unknown(self, &name);
                None
            }
            None => Some((own, false)),
        }
    }

    /// The window of an `OVER` clause in a mark's channel, and the window of the call, `panel`,
    /// `position`, or `series`, that it is or adds to.
    fn mark_window(&mut self, over: &ast::OverClause) -> Option<(WindowSyntax, Option<String>)> {
        let call_window = |name: &ast::NameRef| {
            name.text()
                .map(|text| text.to_ascii_lowercase())
                .filter(|text| matches!(text.as_str(), "panel" | "position" | "series"))
        };
        if let Some(name) = over.name_ref() {
            let Some(window) = call_window(&name) else {
                return self.named_window(&name).map(|spec| (spec, None));
            };
            let none = WindowSyntax {
                partition: None,
                order: None,
                frame: None,
            };
            return Some((none, Some(window)));
        }
        let spec = over.window_spec()?;
        match spec.name_ref().as_ref().and_then(call_window) {
            Some(window) => {
                if spec.partition_by().is_some() {
                    self.error(
                        spec.syntax().text_range(),
                        "If a window has a PARTITION BY, it cannot reference a named window",
                    );
                    return None;
                }
                let own = WindowSyntax {
                    partition: None,
                    order: spec.order_by(),
                    frame: spec.frame(),
                };
                Some((own, Some(window)))
            }
            None => {
                let range = spec.syntax().text_range();
                self.extend_window(&spec, range).map(|spec| (spec, None))
            }
        }
    }

    /// The window of an `OVER` clause: its own specification, a named window of the `WINDOW`
    /// clause, or a named window that the specification adds to.
    fn window_syntax(&mut self, over: &ast::OverClause) -> Option<WindowSyntax> {
        if let Some(name) = over.name_ref() {
            return self.named_window(&name);
        }
        let spec = over.window_spec()?;
        let range = spec.syntax().text_range();
        self.extend_window(&spec, range)
    }

    /// The window `spec`, after the named window that it names, which it adds to.
    pub(crate) fn extend_window(
        &mut self,
        spec: &ast::WindowSpec,
        range: TextRange,
    ) -> Option<WindowSyntax> {
        let own = WindowSyntax {
            partition: spec.partition_by(),
            order: spec.order_by(),
            frame: spec.frame(),
        };
        let Some(name) = spec.name_ref() else {
            return Some(own);
        };
        let base = self.named_window(&name)?;
        let text = name.text().unwrap_or_default();
        if own.partition.is_some() {
            self.error(
                range,
                "If a window has a PARTITION BY, it cannot reference a named window",
            );
            return None;
        }
        if base.frame.is_some() {
            self.error(
                range,
                format!("The window {text} has a frame, so no window can add to it"),
            );
            return None;
        }
        if base.order.is_some() && own.order.is_some() {
            self.error(
                range,
                format!("ORDER BY cannot be specified when the named window {text} has one"),
            );
            return None;
        }
        Some(WindowSyntax {
            partition: base.partition,
            order: own.order.or(base.order),
            frame: own.frame,
        })
    }

    /// The named window `name` of the `WINDOW` clause of the operator.
    pub(crate) fn named_window(&mut self, name: &ast::NameRef) -> Option<WindowSyntax> {
        let text = name.text()?;
        let found = (self.windows.iter().rev())
            .find(|(window, _)| window.eq_ignore_ascii_case(&text))
            .map(|(_, syntax)| syntax.clone());
        if found.is_none() {
            self.error(
                name.syntax().text_range(),
                format!("Unrecognized window alias {text}"),
            );
        }
        found
    }

    fn frame(&mut self, frame: &ast::Frame, scope: Scope) -> Option<WindowFrame> {
        let units = match frame.rows_kw() {
            Some(_) => WindowFrameUnits::Rows,
            None => WindowFrameUnits::Range,
        };
        let start = self.frame_bound(frame.start()?, scope)?;
        let end = match frame.end() {
            Some(end) => self.frame_bound(end, scope)?,
            None => WindowFrameBound::CurrentRow,
        };
        Some(WindowFrame::new_bounds(units, start, end))
    }

    fn frame_bound(&mut self, bound: ast::FrameBound, scope: Scope) -> Option<WindowFrameBound> {
        let preceding = bound.preceding_kw().is_some();
        if bound.current_kw().is_some() {
            return Some(WindowFrameBound::CurrentRow);
        }
        if bound.unbounded_kw().is_some() {
            return Some(match preceding {
                true => WindowFrameBound::Preceding(ScalarValue::UInt64(None)),
                false => WindowFrameBound::Following(ScalarValue::UInt64(None)),
            });
        }
        let expr = bound.expr()?;
        let range = expr.syntax().text_range();
        let value = self.expr(&expr, scope)?;
        let Expr::Literal(ScalarValue::Int64(Some(n)), _) = value else {
            self.error(
                range,
                "avenger-sql takes the bounds of a window frame as integer literals",
            );
            return None;
        };
        if n < 0 {
            self.error(range, "Window frame offset must be non-negative");
            return None;
        }
        let value = ScalarValue::UInt64(Some(n as u64));
        Some(match preceding {
            true => WindowFrameBound::Preceding(value),
            false => WindowFrameBound::Following(value),
        })
    }
}

/// The syntax of a window: its `PARTITION BY`, its `ORDER BY` and its frame, which the pass plans
/// in the scope of each call that reads it.
#[derive(Clone, Debug)]
pub(crate) struct WindowSyntax {
    pub partition: Option<ast::PartitionBy>,
    pub order: Option<ast::OrderBy>,
    pub frame: Option<ast::Frame>,
}

/// The aggregate `aggregate` of `ARRAY_AGG` or `STRING_AGG` with `LIMIT n`: the first `n`
/// values in its order, as a slice of the array of its values.
fn limited(name: &str, aggregate: AggregateFunction, n: Expr) -> Expr {
    use datafusion::functions_nested::expr_fn::{array_slice, array_to_string};
    let one = int(1);
    if !name.eq_ignore_ascii_case("STRING_AGG") {
        return array_slice(Expr::AggregateFunction(aggregate), one, n, None);
    }
    // STRING_AGG leaves out `NULL` values, which the array of its values doesn't count.
    let params = aggregate.params;
    let value = params.args[0].clone();
    let delimiter = params.args[1].clone();
    let kept = value.clone().is_not_null();
    let filter = match params.filter {
        Some(filter) => filter.and(kept),
        None => kept,
    };
    let values = Expr::AggregateFunction(AggregateFunction::new_udf(
        datafusion::functions_aggregate::array_agg::array_agg_udaf(),
        vec![value],
        params.distinct,
        Some(Box::new(filter)),
        params.order_by,
        None,
    ));
    array_to_string(array_slice(values, one, n, None), delimiter)
}

/// `subquery` with its one column cast to `ty`.
fn cast_subquery(subquery: Subquery, ty: DataType) -> Option<Subquery> {
    let plan = subquery.subquery.as_ref().clone();
    let column = plan.schema().columns().into_iter().next()?;
    let name = column.name.clone();
    let plan = LogicalPlanBuilder::from(plan)
        .project(vec![cast(Expr::Column(column), ty).alias(name)])
        .and_then(|b| b.build())
        .ok()?;
    Some(build::subquery(plan))
}

/// The outcome of a lookup of a name in one scope.
enum Lookup {
    /// The expression of the names, and the number of names that it reads.
    Found(Expr, usize),
    /// The scope has no such name.
    Missing,
    /// An error, which the lookup reported.
    Failed,
}

/// The columns of an enclosing query in the scope of a nested one.
#[derive(Clone, Debug)]
pub(crate) struct Outer {
    pub columns: Vec<Col>,
    pub vars: Vec<RangeVar>,
    pub schema: Option<DFSchema>,
    /// The clause of a `MATCH_RECOGNIZE` whose expression holds the subquery, `DEFINE` or
    /// `MEASURES`, whose columns the subquery can't read.
    pub match_recognize: Option<super::match_recognize::Clause>,
}

impl Outer {
    pub(crate) fn of(scope: Scope) -> Self {
        Self {
            columns: scope.columns.to_vec(),
            vars: scope.vars.to_vec(),
            schema: scope.schema.cloned(),
            match_recognize: None,
        }
    }
}

/// `expr`, of the columns of an enclosing query of the schema `schema`, as the outer references of
/// a nested query.
fn outer_reference(expr: Expr, schema: Option<&DFSchema>) -> Option<Expr> {
    use datafusion::common::tree_node::{Transformed, TreeNode};
    let schema = schema?;
    expr.transform_up(|e| match e {
        Expr::Column(column) => {
            let field = schema.field_from_column(&column)?.clone();
            Ok(Transformed::yes(Expr::OuterReferenceColumn(field, column)))
        }
        other => Ok(Transformed::no(other)),
    })
    .ok()
    .map(|t| t.data)
}

/// The `OVER` clause of a window call.
pub(crate) struct Window {
    pub partition: Vec<Expr>,
    order: Vec<SortExpr>,
    frame: WindowFrame,
}

impl Window {
    fn window(
        self,
        func: WindowFunctionDefinition,
        args: Vec<Expr>,
        null_treatment: Option<NullTreatment>,
    ) -> Expr {
        let mut window = WindowFunction::new(func, args);
        window.params.partition_by = self.partition;
        window.params.order_by = self.order;
        window.params.window_frame = self.frame;
        window.params.null_treatment = null_treatment;
        Expr::from(window)
    }
}

/// The text of a `TIMESTAMP` literal as Arrow parses it, and whether it has a time zone: a `T`
/// between the date and the time, an offset `+HH` as `+HH:00`, and `UTC` as `+00:00`.
fn timestamp_text(text: &str) -> (String, bool) {
    let text = text.trim();
    let (civil, offset) = match text.strip_suffix("UTC") {
        Some(civil) => (civil.trim_end(), Some("+00:00".to_string())),
        None => {
            // An offset after the date: `+HH`, `+HH:MM`, `-HH`, `-HH:MM` or `Z`.
            let split = (text.char_indices().skip(10))
                .find(|(_, c)| matches!(c, '+' | '-' | 'Z'))
                .map(|(at, _)| at);
            match split {
                Some(at) => {
                    let offset = &text[at..];
                    let offset = match offset.len() {
                        3 => format!("{offset}:00"),
                        _ => offset.to_string(),
                    };
                    (text[..at].trim_end(), Some(offset))
                }
                None => (text, None),
            }
        }
    };
    let mut civil = civil.replacen(' ', "T", 1);
    if !civil.contains('T') {
        civil.push_str("T00:00:00");
    }
    match offset {
        Some(offset) => (format!("{civil}{offset}"), true),
        None => (civil, false),
    }
}

/// Whether the key `key` of an `ORDER BY` sorts ascending, and whether its `NULL`s come first,
/// which they do in an ascending order unless it says `NULLS LAST`.
pub(crate) fn direction(key: &ast::OrderKey) -> (bool, bool) {
    let asc = key.desc_kw().is_none();
    let nulls_first = match (key.first_kw(), key.last_kw()) {
        (Some(_), _) => true,
        (_, Some(_)) => false,
        _ => asc,
    };
    (asc, nulls_first)
}

/// The name of a select item without an alias: the last name of a path, the name of a field, or
/// the empty string.
pub(crate) fn inferred_name(expr: &ast::Expr) -> String {
    match expr {
        ast::Expr::PathExpr(path) => path
            .path()
            .and_then(|path| path.names())
            .and_then(|names| names.last().cloned())
            .unwrap_or_default(),
        ast::Expr::FieldExpr(field) => field
            .name_ref()
            .and_then(|name| name.text())
            .unwrap_or_default(),
        // As in GoogleSQL, `(x)` is named after `x`.
        ast::Expr::ParenExpr(paren) => paren.expr().map(|e| inferred_name(&e)).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The date part that `expr` writes, as in `MONTH` or `WEEK(MONDAY)`.
pub(crate) fn date_part(expr: &ast::Expr) -> Option<DatePart> {
    match expr {
        ast::Expr::PathExpr(path) => {
            let names = path.path()?.names()?;
            match names.as_slice() {
                [name] => DatePart::parse(name, None),
                _ => None,
            }
        }
        ast::Expr::CallExpr(call) => {
            let names = call.path()?.names()?;
            let [name] = names.as_slice() else {
                return None;
            };
            let mut arguments = call.arg_list()?.arguments();
            let day = arguments.next()?.syntax().text().to_string();
            if arguments.next().is_some() {
                return None;
            }
            DatePart::parse(name, Some(day.trim()))
        }
        _ => None,
    }
}

/// Whether GoogleSQL casts a value of `from` to `to`.
fn castable(from: &Ty, to: &Ty) -> bool {
    use Ty::*;
    let numeric = |ty: &Ty| matches!(ty, Int64 | Float64 | Numeric | BigNumeric);
    match (from, to) {
        (Null, _) => true,
        (a, b) if a == b => true,
        (a, b) if numeric(a) && numeric(b) => true,
        (Bool, Int64) | (Int64, Bool) => true,
        (String, Array(_) | Struct(_)) | (Array(_) | Struct(_), String) => false,
        (String, _) | (_, String) => true,
        (Date, Datetime | Timestamp) | (Datetime, Date | Time | Timestamp) => true,
        (Timestamp, Date | Datetime | Time) => true,
        (Array(a), Array(b)) => castable(a, b),
        (Struct(a), Struct(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|((_, a), (_, b))| castable(a, b))
        }
        _ => false,
    }
}

/// The ranges of the string literals of `expr`: itself, or the items of an array.
fn string_literals(expr: &ast::Expr) -> Vec<TextRange> {
    match expr {
        ast::Expr::ArrayExpr(array) => array
            .exprs()
            .filter_map(|item| string_literal(&item))
            .collect(),
        expr => string_literal(expr).into_iter().collect(),
    }
}

/// Whether `expr` is the string literal `'typst'`, the syntax of `TEXT_SYNTAX_TYPST`, which
/// reads a text as Typst markup. A syntax that is not a literal reads it as no syntax that the
/// analysis knows, so no string of its call is markup.
fn is_typst_syntax(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::Literal(literal) => {
            (literal.string_value()).is_some_and(|value| value.eq_ignore_ascii_case("typst"))
        }
        _ => false,
    }
}

/// The range of `expr` when it is a string literal, as `'normalize'` or `'a' 'b'`.
pub(super) fn string_literal(expr: &ast::Expr) -> Option<TextRange> {
    match expr {
        ast::Expr::Literal(literal) if literal.kind() == Some(ast::LiteralKind::String) => {
            Some(literal.syntax().text_range())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::timestamp_text;

    #[test]
    fn timestamp_literals_take_an_offset() {
        let text = |text: &str| timestamp_text(text);
        assert_eq!(
            text("2024-01-30 10:00:00+00"),
            ("2024-01-30T10:00:00+00:00".into(), true)
        );
        assert_eq!(
            text("2024-01-30 10:00:00-07:30"),
            ("2024-01-30T10:00:00-07:30".into(), true)
        );
        assert_eq!(
            text("2024-01-30 10:00:00 UTC"),
            ("2024-01-30T10:00:00+00:00".into(), true)
        );
        assert_eq!(
            text("2024-01-30 10:00:00"),
            ("2024-01-30T10:00:00".into(), false)
        );
        assert_eq!(text("2024-01-30"), ("2024-01-30T00:00:00".into(), false));
    }
}
