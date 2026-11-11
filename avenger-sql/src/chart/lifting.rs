//! The lifting of a chart call: the pipe operators that compute each statistic of its channels in
//! its own stage and end in a call of the mark aggregate over column references.
//!
//! - Stage 1: one `EXTEND` for each depth of the discretizers, `bin`, `timeunit`, and `lump`.
//! - Stage 2: in a grouping call, an `AGGREGATE` of the aggregates of the channels and of the draw
//!   order, grouped by the partition columns and the keys.
//! - Stage 3: with the `impute` option, the pipe `CALL` of the table function `impute`, then an
//!   `EXTEND` of the keys that are functions of the imputed key ([`impute`]).
//! - Stage 4: an `EXTEND` of the stacks and the window functions of the channels and of the draw
//!   order ([`stage4`]).
//! - Stage 5: an `AGGREGATE` of the mark aggregate with the draw order, grouped by the partition
//!   columns.
//!
//! The generated operators hold clones of the user's subexpressions, which keep their spans, and
//! generated identifiers, which take the spans of their source constructs.

use std::{collections::HashSet, ops::ControlFlow};

use avenger_chart::functions::Kind;
use datafusion::{
    arrow::datatypes::{DataType, Field},
    common::{DFSchema, Result},
    sql::sqlparser::{
        ast::{
            Expr, ExprWithAlias, ExprWithAliasAndOrderBy, Function, FunctionArg, FunctionArgExpr,
            FunctionArgOperator, FunctionArgumentClause, FunctionArgumentList, FunctionArguments,
            Ident, ObjectName, OrderByExpr, OrderByOptions, PipeOperator, SelectItem, Spanned,
            Value, Visit, VisitMut, Visitor, VisitorMut, WindowType,
        },
        tokenizer::Span,
    },
};

use super::{
    call::{self, ChannelValue, MarkCall},
    classify::{self, call_kind, first_argument, level, CallKind, Level},
    impute::{self, GroupingKeys},
    roles::{self, Roles},
    stage4,
};
use crate::{
    error::{at, at_with_help},
    registry::Registry,
    syntax::{self, function_name},
};

/// What the lifting needs of the planner: the functions of the session, and the types of
/// expressions over a schema.
pub(super) trait Env: Registry {
    /// The type of `expr` over `schema`.
    fn data_type(&mut self, expr: Expr, schema: &DFSchema) -> Result<DataType>;
}

/// A column that the lifting adds: the expression that it replaces, its name, and the expression
/// of its stage.
pub(super) struct Added {
    pub(super) from: Expr,
    pub(super) name: Ident,
    pub(super) stage: Expr,
}

/// A discretizer of stage 1.
struct Discretized {
    added: Added,
    depth: usize,
}

/// A key of a grouping call: the channel value, its entry in the `GROUP BY` list of stage 2, and
/// the column that stage 5 reads.
struct Key {
    from: Expr,
    group: ExprWithAlias,
    column: Expr,
}

/// The lifting of the mark call `mark` over an input of `schema`, partitioned by `partition`,
/// whose chart column is `chart_name`.
pub(super) fn generate(
    env: &mut dyn Env,
    mark: &Expr,
    partition: &[Expr],
    chart_name: &Ident,
    schema: &DFSchema,
) -> Result<Vec<PipeOperator>> {
    let call = call::read(mark)?;
    for (name, value) in &call.properties {
        classify::constant(
            value,
            &format!("option `{}` of {}()", name.value, call.mark.name),
        )?;
    }
    for value in checked(&call) {
        classify::check_calls(value, env)?;
    }
    let imputation = impute::read(&call)?;
    for column in partition {
        env.data_type(column.clone(), schema)?;
    }
    let channel_levels: Vec<Level> = (call.channels.iter())
        .map(|channel| level(&channel.value, env))
        .collect();
    let order_levels: Vec<Level> = (call.order.iter())
        .map(|item| level(&item.expr, env))
        .collect();

    let mut names = Names::new(schema, partition);
    let discretizers = discretizers(&call, partition, &mut names, env)?;
    let stage1_schema = check_types(&discretizers, schema, env)?;
    let discretized: Vec<(Expr, Expr)> = (discretizers.iter())
        .map(|d| (d.added.from.clone(), column(&d.added.name)))
        .collect();

    let mut operators = Vec::new();
    let max_depth = discretizers.iter().map(|d| d.depth).max().unwrap_or(0);
    for depth in 1..=max_depth {
        let exprs = (discretizers.iter())
            .filter(|d| d.depth == depth)
            .map(|d| SelectItem::ExprWithAlias {
                expr: d.added.stage.clone(),
                alias: d.added.name.clone(),
            })
            .collect();
        operators.push(PipeOperator::Extend { exprs });
    }

    let mut stage5: Vec<(Expr, Expr)> = Vec::new();
    let aggregates = aggregates(&call, &discretized, &mut names, env);
    // A call groups when an aggregate is in a channel or in its draw order.
    let grouping = !aggregates.is_empty();
    // The roles of an imputation, which V10 checks before the planning of the stages.
    let roles = match imputation {
        Some(_) => roles::decide(&call, &channel_levels, &stage1_schema, &discretized, env)?,
        None => None,
    };
    let target = match &imputation {
        Some(imputation) => impute::target(&call, imputation, roles.as_ref(), grouping, env)?,
        None => None,
    };
    let mut grouping_keys = None;
    let mut dependent = Vec::new();
    if grouping {
        let keys = keys(&call, &channel_levels, partition, &discretized, &mut names);
        check_keys(&call, &channel_levels, &order_levels, &keys, partition, env)?;
        // The keys that are functions of the imputed key compute after stage 3 from its column,
        // so stage 2 does not group by them.
        let imputed = (target.as_ref())
            .map(|target| &call.channels[target.key].value)
            .and_then(|value| keys.iter().find(|key| &key.from == value));
        let mut groups = Vec::new();
        let mut grouped = Vec::new();
        for key in &keys {
            match imputed {
                Some(imputed) if !std::ptr::eq(key, imputed) => {
                    if impute::dependent(&key.group.expr, &imputed.group.expr) {
                        let name = key.group.alias.clone().expect("a key expression");
                        let stage = replaced(
                            &key.group.expr,
                            &[(imputed.group.expr.clone(), imputed.column.clone())],
                        );
                        dependent.push(SelectItem::ExprWithAlias {
                            expr: stage,
                            alias: name,
                        });
                        continue;
                    }
                    groups.push(key.column.clone());
                }
                _ => {}
            }
            grouped.push(key.group.clone());
        }
        if let Some(target) = &target {
            let key = match imputed {
                Some(imputed) => imputed.column.clone(),
                None => call.channels[target.key].value.clone(),
            };
            grouping_keys = Some(GroupingKeys { key, groups });
        }
        let group_by = (partition.iter())
            .map(|column| ExprWithAlias {
                expr: column.clone(),
                alias: None,
            })
            .chain(grouped)
            .map(plain)
            .collect();
        let full_table_exprs = (aggregates.iter())
            .map(|aggregate| {
                plain(ExprWithAlias {
                    expr: aggregate.stage.clone(),
                    alias: Some(aggregate.name.clone()),
                })
            })
            .collect();
        operators.push(PipeOperator::Aggregate {
            full_table_exprs,
            group_by_expr: group_by,
        });
        stage5.extend(
            keys.iter()
                .map(|key| (key.from.clone(), key.column.clone())),
        );
        stage5.extend(
            (aggregates.iter()).map(|aggregate| (aggregate.from.clone(), column(&aggregate.name))),
        );
    }
    stage5.extend(discretized.iter().cloned());

    let mut flag = None;
    if let (Some(imputation), Some(target)) = (&imputation, &target) {
        let stage3 = impute::stage3(
            &call,
            imputation,
            target,
            &channel_levels,
            partition,
            grouping_keys.as_ref(),
            &stage5,
            &mut names,
            env,
        )?;
        if let Some(stage3) = stage3 {
            operators.push(stage3.operator);
            flag = Some(stage3.flag);
        }
    }
    // The keys of the imputed key, which stage 2 does not group by, computed after stage 3, or
    // after stage 2 when a default adds no stage 3.
    if !dependent.is_empty() {
        operators.push(PipeOperator::Extend { exprs: dependent });
    }

    let (windows, roles) = stage4::windows(
        &call,
        &channel_levels,
        partition,
        &stage5,
        &stage1_schema,
        &discretized,
        roles,
        flag.as_ref(),
        &mut names,
        env,
    )?;
    if !windows.is_empty() {
        let exprs = (windows.iter())
            .map(|window| SelectItem::ExprWithAlias {
                expr: window.stage.clone(),
                alias: window.name.clone(),
            })
            .collect();
        operators.push(PipeOperator::Extend { exprs });
        stage5.extend((windows.iter()).map(|window| (window.from.clone(), column(&window.name))));
    }

    let chart = mark_call(&call, &channel_levels, &stage5, roles.as_ref(), env);
    operators.push(PipeOperator::Aggregate {
        full_table_exprs: vec![plain(ExprWithAlias {
            expr: Expr::Function(chart),
            alias: Some(chart_name.clone()),
        })],
        group_by_expr: (partition.iter())
            .map(|column| {
                plain(ExprWithAlias {
                    expr: column.clone(),
                    alias: None,
                })
            })
            .collect(),
    });
    Ok(operators)
}

/// The checked expressions of a call: its channel values and its draw-order items.
pub(super) fn checked(call: &MarkCall) -> impl Iterator<Item = &Expr> {
    (call.channels.iter().map(|channel| &channel.value))
        .chain(call.order.iter().map(|item| &item.expr))
}

/// A column reference to `name`.
pub(super) fn column(name: &Ident) -> Expr {
    Expr::Identifier(name.clone())
}

/// An entry of a pipe `AGGREGATE` without an order.
fn plain(expr: ExprWithAlias) -> ExprWithAliasAndOrderBy {
    ExprWithAliasAndOrderBy {
        expr,
        order_by: OrderByOptions {
            asc: None,
            nulls_first: None,
        },
    }
}

/// The names of the columns of a lifting: the short names of `r[lift.rule.names]`, with the
/// smallest free suffix `_2`, `_3`, and so on for a name that an input column or another added
/// column has.
pub(super) struct Names {
    taken: HashSet<String>,
}

impl Names {
    fn new(schema: &DFSchema, partition: &[Expr]) -> Self {
        let taken = (schema
            .fields()
            .iter()
            .map(|field| field.name().to_lowercase()))
        .chain(
            partition
                .iter()
                .map(|column| column.to_string().to_lowercase()),
        )
        .collect();
        Self { taken }
    }

    /// A free name from `base`, at `span`.
    pub(super) fn fresh(&mut self, base: &str, span: Span) -> Ident {
        let base = base.to_lowercase();
        let name = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base}_{n}")))
            .find(|name| !self.taken.contains(name))
            .expect("a free name");
        self.taken.insert(name.clone());
        Ident::with_span(span, name)
    }
}

/// The name of the column of `expr`, when it is a column reference.
pub(super) fn column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => Some(ident.value.clone()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|ident| ident.value.clone()),
        _ => None,
    }
}

/// The calls of the kinds that `keep` accepts in `expr`, inner calls first, each once, outside
/// the arguments of the calls of the kinds that `skip` accepts.
pub(super) fn calls(
    expr: &Expr,
    registry: &dyn Registry,
    keep: &dyn Fn(CallKind) -> bool,
    skip: &dyn Fn(CallKind) -> bool,
) -> Vec<Expr> {
    struct Collect<'a> {
        registry: &'a dyn Registry,
        keep: &'a dyn Fn(CallKind) -> bool,
        skip: &'a dyn Fn(CallKind) -> bool,
        /// Whether each expression that the visit is in skips its arguments.
        skipping: Vec<bool>,
        found: Vec<Expr>,
    }
    impl Collect<'_> {
        fn kind(&self, expr: &Expr) -> Option<CallKind> {
            match expr {
                Expr::Function(function) => call_kind(function, self.registry),
                _ => None,
            }
        }
    }
    impl Visitor for Collect<'_> {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            let skips = self.kind(expr).is_some_and(self.skip);
            self.skipping.push(skips);
            ControlFlow::Continue(())
        }
        fn post_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            self.skipping.pop();
            let inside = self.skipping.iter().any(|skip| *skip);
            if !inside && self.kind(expr).is_some_and(self.keep) && !self.found.contains(expr) {
                self.found.push(expr.clone());
            }
            ControlFlow::Continue(())
        }
    }
    let mut collect = Collect {
        registry,
        keep,
        skip,
        skipping: Vec::new(),
        found: Vec::new(),
    };
    let _ = Visit::visit(expr, &mut collect);
    collect.found
}

/// `node` with each subexpression that `map` holds replaced by its replacement, outer
/// subexpressions first.
pub(super) fn replace<T: VisitMut>(node: &mut T, map: &[(Expr, Expr)]) {
    struct Replace<'a> {
        map: &'a [(Expr, Expr)],
    }
    impl VisitorMut for Replace<'_> {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
            if let Some((_, to)) = self.map.iter().find(|(from, _)| from == expr) {
                *expr = to.clone();
            }
            ControlFlow::Continue(())
        }
    }
    let _ = VisitMut::visit(node, &mut Replace { map });
}

/// `expr` with the replacements of `map`.
pub(super) fn replaced(expr: &Expr, map: &[(Expr, Expr)]) -> Expr {
    let mut expr = expr.clone();
    replace(&mut expr, map);
    expr
}

/// The discretizers of the call, inner discretizers first, each with its column, its stage 1
/// expression, and its depth (V2). The stage 1 expression reads the columns of the discretizers
/// in its argument, and its window is the written window with `panel` resolved, or `OVER ()`.
fn discretizers(
    call: &MarkCall,
    partition: &[Expr],
    names: &mut Names,
    registry: &dyn Registry,
) -> Result<Vec<Discretized>> {
    let is_discretizer = |kind| matches!(kind, CallKind::Discretizer(_));
    let uses = (call.channels.iter().map(|c| (c.channel.name, &c.value)))
        .chain(call.order.iter().map(|item| ("order", &item.expr)));
    let mut found: Vec<Discretized> = Vec::new();
    for (channel, value) in uses {
        for from in calls(value, registry, &is_discretizer, &|_| false) {
            if found.iter().any(|d| d.added.from == from) {
                continue;
            }
            let Expr::Function(function) = &from else {
                unreachable!("a discretizer is a call")
            };
            let Some(CallKind::Discretizer(kind)) = call_kind(function, registry) else {
                unreachable!("the call is a discretizer")
            };
            let map: Vec<(Expr, Expr)> = (found.iter())
                .map(|d| (d.added.from.clone(), column(&d.added.name)))
                .collect();
            let mut stage = function.clone();
            replace(&mut stage.args, &map);
            let mut window = stage4::discretizer_window(call, partition, function)?;
            replace(&mut window, &map);
            stage.over = Some(WindowType::WindowSpec(window));
            let inner = (found.iter())
                .filter(|d| {
                    let in_args = |expr: &Expr| {
                        calls(expr, registry, &is_discretizer, &|_| false).contains(&d.added.from)
                    };
                    classify::arguments(function).any(in_args)
                })
                .map(|d| d.depth)
                .max()
                .unwrap_or(0);
            let argument = first_argument(&stage).and_then(column_name);
            let base = format!(
                "{}_{}",
                argument.unwrap_or_else(|| channel.to_string()),
                kind.suffix()
            );
            let name = names.fresh(&base, from.span());
            found.push(Discretized {
                added: Added {
                    from: from.clone(),
                    name,
                    stage: Expr::Function(stage),
                },
                depth: inner + 1,
            });
        }
    }
    Ok(found)
}

/// The types of the discretizers over the input and the columns of the discretizers of a smaller
/// depth (V9). A transform that rejects its argument reports its error at the argument. It returns
/// the schema of the input with the columns of the discretizers.
fn check_types(
    discretizers: &[Discretized],
    schema: &DFSchema,
    env: &mut dyn Env,
) -> Result<DFSchema> {
    let mut schema = schema.clone();
    let max_depth = discretizers.iter().map(|d| d.depth).max().unwrap_or(0);
    for depth in 1..=max_depth {
        let mut fields = Vec::new();
        for d in discretizers.iter().filter(|d| d.depth == depth) {
            let mut stage = d.added.stage.clone();
            syntax::lower_expr(&*env, &mut stage)?;
            let data_type = env.data_type(stage, &schema).map_err(|error| {
                if error.diagnostic().is_some() {
                    return error;
                }
                let span = match &d.added.from {
                    Expr::Function(function) => first_argument(function)
                        .map(Spanned::span)
                        .unwrap_or(function.name.span()),
                    other => other.span(),
                };
                at(span, error.message())
            })?;
            fields.push(Field::new(&d.added.name.value, data_type, true));
        }
        let added = DFSchema::from_unqualified_fields(fields.into(), Default::default())?;
        schema = schema.join(&added)?;
    }
    Ok(schema)
}

/// The aggregates of the channels and of the draw order, in the order of first use, each once,
/// with the discretizers in their arguments replaced by their columns.
fn aggregates(
    call: &MarkCall,
    discretized: &[(Expr, Expr)],
    names: &mut Names,
    registry: &dyn Registry,
) -> Vec<Added> {
    let uses = (call.channels.iter().map(|c| (c.channel.name, &c.value)))
        .chain(call.order.iter().map(|item| ("order", &item.expr)));
    let mut found: Vec<Added> = Vec::new();
    for (channel, value) in uses {
        for from in calls(
            value,
            registry,
            &|kind| kind == CallKind::Aggregate,
            &|kind| matches!(kind, CallKind::Discretizer(_)),
        ) {
            if found.iter().any(|a| a.from == from) {
                continue;
            }
            let stage = replaced(&from, discretized);
            let Expr::Function(function) = &stage else {
                unreachable!("an aggregate is a call")
            };
            let name = function_name(function).unwrap_or_default();
            let star = match &function.args {
                FunctionArguments::List(list) => {
                    list.args.is_empty()
                        || matches!(
                            list.args.as_slice(),
                            [FunctionArg::Unnamed(FunctionArgExpr::Wildcard)]
                        )
                }
                _ => true,
            };
            let base = match (name.as_str(), star, first_argument(function)) {
                ("count", true, _) => "n".to_string(),
                (_, _, Some(argument)) => format!(
                    "{name}_{}",
                    column_name(argument).unwrap_or_else(|| channel.to_string())
                ),
                (_, true, None) => name.clone(),
                (_, false, None) => format!("{name}_{channel}"),
            };
            let ident = names.fresh(&base, from.span());
            found.push(Added {
                from,
                name: ident,
                stage,
            });
        }
    }
    found
}

/// The keys of a grouping call: each channel value of level 0 or 1 that is not a constant or a
/// partition column, in channel order, each once. A column stays as it is, a discretizer is its
/// column, and another value is a key expression with the name `<channel>_expr`.
fn keys(
    call: &MarkCall,
    levels: &[Level],
    partition: &[Expr],
    discretized: &[(Expr, Expr)],
    names: &mut Names,
) -> Vec<Key> {
    let mut keys: Vec<Key> = Vec::new();
    for (channel, level) in call.channels.iter().zip(levels) {
        if !matches!(level, Level::Row | Level::Discretized) {
            continue;
        }
        let value = &channel.value;
        if keys.iter().any(|key| &key.from == value) || partition.contains(value) {
            continue;
        }
        let key = match value {
            Expr::Identifier(_) | Expr::CompoundIdentifier(_) => Key {
                from: value.clone(),
                group: ExprWithAlias {
                    expr: value.clone(),
                    alias: None,
                },
                column: value.clone(),
            },
            _ => match discretized.iter().find(|(from, _)| from == value) {
                Some((_, column)) => Key {
                    from: value.clone(),
                    group: ExprWithAlias {
                        expr: column.clone(),
                        alias: None,
                    },
                    column: column.clone(),
                },
                None => {
                    let name = names.fresh(&format!("{}_expr", channel.channel.name), value.span());
                    Key {
                        from: value.clone(),
                        group: ExprWithAlias {
                            expr: replaced(value, discretized),
                            alias: Some(name.clone()),
                        },
                        column: column(&name),
                    }
                }
            },
        };
        keys.push(key);
    }
    keys
}

/// The keys of a grouping call cover its post-aggregate expressions (V3) and its draw order (V4).
fn check_keys(
    call: &MarkCall,
    channel_levels: &[Level],
    order_levels: &[Level],
    keys: &[Key],
    partition: &[Expr],
    registry: &dyn Registry,
) -> Result<()> {
    let is_key = |expr: &Expr| {
        let key = |e: &Expr| keys.iter().any(|key| &key.from == e) || partition.contains(e);
        key(expr)
            || match expr {
                Expr::CompoundFieldAccess { root, .. } => key(root),
                Expr::CompoundIdentifier(parts) => {
                    (1..parts.len()).any(|n| key(&compound(&parts[..n])))
                }
                _ => false,
            }
    };
    let values = (call.channels.iter().map(|c| &c.value).zip(channel_levels))
        .chain(call.order.iter().map(|item| &item.expr).zip(order_levels));
    for (value, level) in values {
        if *level >= Level::Aggregated {
            post_aggregate(value, &is_key, registry)?;
        }
    }
    for (item, level) in call.order.iter().zip(order_levels) {
        if matches!(level, Level::Row | Level::Discretized) && !is_key(&item.expr) {
            return Err(at(
                item.expr.span(),
                format!(
                    "the draw order `{}` is not a key of this chart, so it would split its marks",
                    item.expr
                ),
            ));
        }
    }
    Ok(())
}

/// A column reference of the parts `parts`.
fn compound(parts: &[Ident]) -> Expr {
    match parts {
        [ident] => Expr::Identifier(ident.clone()),
        _ => Expr::CompoundIdentifier(parts.to_vec()),
    }
}

/// Each maximal subexpression of level 0 or 1 of the post-aggregate expression `expr`, outside
/// the arguments of its aggregates, is a key, a field of a key, or a constant (V3).
fn post_aggregate(
    expr: &Expr,
    is_key: &dyn Fn(&Expr) -> bool,
    registry: &dyn Registry,
) -> Result<()> {
    struct Check<'a> {
        is_key: &'a dyn Fn(&Expr) -> bool,
        registry: &'a dyn Registry,
        skip: Vec<bool>,
    }
    impl Visitor for Check<'_> {
        type Break = datafusion::common::DataFusionError;
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
            if self.skip.iter().any(|skip| *skip) {
                self.skip.push(true);
                return ControlFlow::Continue(());
            }
            let aggregate = matches!(expr, Expr::Function(f) if call_kind(f, self.registry) == Some(CallKind::Aggregate));
            let row = matches!(level(expr, self.registry), Level::Row | Level::Discretized);
            if row && !(self.is_key)(expr) {
                return ControlFlow::Break(at_with_help(
                    expr.span(),
                    format!("`{expr}` is neither a key of this chart nor inside an aggregate"),
                    format!("group by it with a channel, or aggregate it, for example max({expr})"),
                ));
            }
            self.skip
                .push(aggregate || row || level(expr, self.registry) == Level::Constant);
            ControlFlow::Continue(())
        }
        fn post_visit_expr(&mut self, _expr: &Expr) -> ControlFlow<Self::Break> {
            self.skip.pop();
            ControlFlow::Continue(())
        }
    }
    let mut check = Check {
        is_key,
        registry,
        skip: Vec::new(),
    };
    match Visit::visit(expr, &mut check) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(()),
    }
}

/// The title of a channel value whose column the lifting creates (D2): a discretizer takes the
/// title of its argument, and another value its text.
fn title(expr: &Expr, registry: &dyn Registry) -> String {
    if let Expr::Function(function) = expr {
        if matches!(
            call_kind(function, registry),
            Some(CallKind::Discretizer(_))
        ) || function_name(function).as_deref() == Some("stack")
        {
            if let Some(argument) = first_argument(function) {
                return title(argument, registry);
            }
        }
    }
    expr.to_string()
}

/// A call of `name` with `args` at `span`.
pub(super) fn function(name: &str, span: Span, args: Vec<FunctionArg>) -> Function {
    Function {
        name: ObjectName::from(vec![Ident::with_span(span, name)]),
        uses_odbc_syntax: false,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            args,
            clauses: Vec::new(),
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: Vec::new(),
    }
}

/// The named argument `name => value`.
pub(super) fn named(name: Ident, value: Expr) -> FunctionArg {
    FunctionArg::Named {
        name,
        arg: FunctionArgExpr::Expr(value),
        operator: FunctionArgOperator::RightArrow,
    }
}

/// The stage 5 call of the mark: each channel with its value in stage 5 form, the properties and
/// `coord` as written, and the draw order over the columns of the earlier stages. A scaled channel
/// whose column the lifting creates takes the title of D2, unless its `scaled` gives one.
fn mark_call(
    call: &MarkCall,
    levels: &[Level],
    stage5: &[(Expr, Expr)],
    roles: Option<&Roles>,
    registry: &dyn Registry,
) -> Function {
    let mut args = Vec::new();
    for (channel, level) in call.channels.iter().zip(levels) {
        args.push(named(
            channel.name.clone(),
            channel_value(channel, *level, stage5, registry),
        ));
    }
    for (name, value) in &call.properties {
        args.push(named(name.clone(), value.clone()));
    }
    // The orientation that the roles decide, which the chart would otherwise infer by its own rule.
    if let Some(roles) =
        roles.filter(|_| roles::has_orient(call.mark.name) && !roles::writes_orient(call))
    {
        let orient = match roles.vertical {
            true => "vertical",
            false => "horizontal",
        };
        args.push(named(
            Ident::with_span(call.span(), "orient"),
            Expr::Value(Value::SingleQuotedString(orient.to_string()).with_span(call.span())),
        ));
    }
    let mut clauses = Vec::new();
    if !call.order.is_empty() {
        let order: Vec<OrderByExpr> = (call.order.iter())
            .map(|item| OrderByExpr {
                expr: replaced(&item.expr, stage5),
                ..item.clone()
            })
            .collect();
        clauses.push(FunctionArgumentClause::OrderBy(order));
    }
    Function {
        args: FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            args,
            clauses,
        }),
        ..call.function.clone()
    }
}

/// The stage 5 form of the value of `channel`.
fn channel_value(
    channel: &ChannelValue,
    level: Level,
    stage5: &[(Expr, Expr)],
    registry: &dyn Registry,
) -> Expr {
    let value = replaced(&channel.value, stage5);
    let kind = match (&channel.wrapper, level) {
        (Some((kind, _)), _) => *kind,
        (None, Level::Constant) => channel.channel.value_kind(),
        (None, _) => channel.channel.field_kind(),
    };
    let titled = channel.options().iter().any(|option| {
        matches!(option, FunctionArg::Named { name, .. } if name.value.eq_ignore_ascii_case("title"))
    });
    let span = channel.value.span();
    let title = (value != channel.value && kind == Kind::Scaled && !titled).then(|| {
        named(
            Ident::with_span(span, "title"),
            Expr::Value(Value::SingleQuotedString(title(&channel.value, registry)).with_span(span)),
        )
    });
    let wrap = |kind: Kind, options: &[FunctionArg]| {
        let args = std::iter::once(FunctionArg::Unnamed(FunctionArgExpr::Expr(value.clone())))
            .chain(options.iter().cloned())
            .chain(title.clone())
            .collect();
        Expr::Function(function(kind.name(), span, args))
    };
    match &channel.wrapper {
        Some((kind, written)) => {
            let Expr::Function(mut rebuilt) = wrap(*kind, channel.options()) else {
                unreachable!("a call")
            };
            rebuilt.name = written.name.clone();
            Expr::Function(rebuilt)
        }
        None if title.is_some() => wrap(Kind::Scaled, &[]),
        None => value,
    }
}
