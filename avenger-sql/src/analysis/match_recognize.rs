//! `MATCH_RECOGNIZE`, after a source or as a pipe operator, in GoogleSQL's subset: one row for
//! each match, with the partition columns and the measures.
//!
//! The pass plans the clause in four steps over its input:
//!
//! 1. A projection with a window node computes the row's position in its partition,
//!    `ROW_NUMBER()` over the clause's partition and order, and one `BOOL` column for each pattern
//!    variable, `COALESCE(condition, FALSE)`, whose `PREV` and `NEXT` are `LAG` and `LEAD` over the
//!    same partition and order.
//! 2. The `MatchRecognize` node of avenger-datafusion-transforms runs the matcher, and outputs each
//!    row of each match with its match number and its variable, and one row without a variable
//!    for each empty match.
//! 3. A projection copies the input's columns for each pattern variable, so a name such as
//!    `b.price` reads columns of its own, and the pass tells which rows each aggregate reads.
//! 4. An aggregate grouped by the partition columns and the match number computes the measures:
//!    an aggregate of a variable's columns reads that variable's rows, by a `FILTER` on the row's
//!    variable, and another aggregate reads the rows of the match.
//!
//! The expressions of the clause plan in a [`Frame`], which tells the planning of names and calls
//! what the clause allows, with GoogleSQL's messages.

use avenger_datafusion_transforms::match_recognize::{
    self as node, Anchor, MatchRecognizeCall, Pattern, Program, Skip,
};
use avenger_sql_syntax::{
    ast::{self, AstNode, LiteralKind, PrefixOp},
    SyntaxElement, SyntaxKind, TextRange,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, DFSchema, ScalarValue,
    },
    functions_window::{
        lead_lag::{lag_udwf, lead_udwf},
        row_number::row_number_udwf,
    },
    logical_expr::{
        expr::AggregateFunction,
        utils::{find_aggregate_exprs, find_window_exprs},
        Case, Cast, Expr, LogicalPlan, LogicalPlanBuilder, SortExpr, WindowFunctionDefinition,
    },
};

use super::{
    expr::{Scope, ARGUMENT},
    model::Definition,
    rows::{self, plan_name, Col, Item, RangeVar, Rows},
    Pass,
};
use crate::library::match_recognize::marker_args;

pub(crate) const PARTITION_CLAUSE: &str = "MATCH_RECOGNIZE PARTITION BY clause";
pub(crate) const ORDER_CLAUSE: &str = "MATCH_RECOGNIZE ORDER BY clause";
pub(crate) const DEFINE_CLAUSE: &str = "MATCH_RECOGNIZE DEFINE predicate";
pub(crate) const MEASURES_CLAUSE: &str = "MATCH_RECOGNIZE MEASURES clause";

/// The names of the columns of the node's match number and the row's variable.
const MATCH_NUMBER: &str = "$match_number";
const CLASSIFIER: &str = "$classifier";

/// A clause of a `MATCH_RECOGNIZE` whose expressions the pass plans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clause {
    /// `PARTITION BY` or `ORDER BY`.
    Keys,
    Define,
    Measures,
}

/// The clause of a `MATCH_RECOGNIZE` that the pass plans, at the depth of the queries that enclose
/// it, so a subquery of the clause is not in it.
pub(crate) struct Frame {
    /// The number of enclosing scopes, `Pass::outers`, of the clause's expressions.
    pub depth: usize,
    pub clause: Clause,
    /// The clause as messages name it, such as `MATCH_RECOGNIZE DEFINE predicate`.
    pub name: &'static str,
    /// The pattern variables, as `DEFINE` spells them, and the ranges of their definitions.
    pub variables: Vec<(String, TextRange)>,
    /// In `MEASURES`, the copies of the input's columns for each variable, in the order of
    /// `variables`.
    pub copies: Vec<RangeVar>,
    /// In `MEASURES`, the partition columns, which a measure reads outside aggregates.
    pub groups: Vec<Column>,
    /// In `MEASURES`, the column of the row's position.
    pub position: Option<Column>,
    /// The fields of the input's value tables, which a variable doesn't read.
    pub value_fields: Vec<String>,
}

/// The rows that an aggregate of `MEASURES` reads: the match's, or a variable's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Range {
    All,
    Variable(usize),
}

fn string(value: &str) -> Expr {
    Expr::Literal(ScalarValue::Utf8(Some(value.to_string())), None)
}

impl Pass<'_> {
    /// The frame of the clause whose expressions the pass plans, when it plans them directly,
    /// outside a subquery.
    pub(crate) fn mr_frame(&self) -> Option<&Frame> {
        (self.match_recognize.last()).filter(|frame| frame.depth == self.outers.len())
    }

    /// The clause of the frame that the pass plans directly, `DEFINE` or `MEASURES`, which a
    /// subquery in it can't read the columns of.
    pub(crate) fn mr_correlated_clause(&self) -> Option<Clause> {
        (self.mr_frame())
            .map(|frame| frame.clause)
            .filter(|clause| *clause != Clause::Keys)
    }

    /// A path that starts with a pattern variable, in `DEFINE` or `MEASURES`: a column of the
    /// variable's rows in an aggregate of `MEASURES`, and otherwise GoogleSQL's error. `None` for
    /// a path that doesn't start with one.
    pub(crate) fn pattern_variable_path(
        &mut self,
        names: &[String],
        refs: &[ast::NameRef],
        range: TextRange,
        scope: Scope,
    ) -> Option<Option<Expr>> {
        let frame = self.mr_frame()?;
        if frame.clause == Clause::Keys {
            return None;
        }
        let index =
            (frame.variables.iter()).position(|(name, _)| name.eq_ignore_ascii_case(&names[0]))?;
        let (name, definition) = frame.variables[index].clone();
        let clause = frame.clause;
        let copies = frame.copies.get(index).cloned();
        let frame_fields = frame.value_fields.clone();
        self.use_name(
            refs[0].syntax().text_range(),
            Definition::RangeVariable {
                name: name.clone(),
                definition: Some(definition),
            },
        );
        let Some(column) = names.get(1) else {
            self.error(
                range,
                format!(
                    "Pattern variable {} cannot be used as a value table",
                    names[0]
                ),
            );
            return Some(None);
        };
        if clause == Clause::Define || scope.clause != ARGUMENT {
            self.error(range, "Cannot access columns through pattern variables.");
            return Some(None);
        }
        let copies = copies.expect("the copies of a variable of MEASURES");
        let found: Vec<&Col> = (copies.columns.iter())
            .filter(|col| col.name.eq_ignore_ascii_case(column))
            .collect();
        let col = match found.as_slice() {
            [col] => (*col).clone(),
            [] => {
                let field = (frame_fields.iter()).any(|field| field.eq_ignore_ascii_case(column));
                let message = match field {
                    true => format!(
                        "Name {}.{column} is a value table field, but a column was expected",
                        names[0]
                    ),
                    false => format!("Name {column} not found inside {name}"),
                };
                self.error(refs[1].syntax().text_range(), message);
                return Some(None);
            }
            _ => {
                self.error(
                    refs[1].syntax().text_range(),
                    format!("Column name {column} is ambiguous"),
                );
                return Some(None);
            }
        };
        let mut expr = Expr::Column(col.column.clone());
        for (field, name_ref) in names.iter().zip(refs).skip(2) {
            expr = self.field(expr, field, name_ref.syntax().text_range(), scope)?;
        }
        Some(Some(expr))
    }

    /// GoogleSQL's error for a column `expr` of the path `names` that a measure reads outside
    /// aggregates, when it isn't a partition column.
    pub(crate) fn mr_unaggregated(
        &self,
        expr: &Expr,
        names: &[String],
        scope: Scope,
    ) -> Option<String> {
        let frame = self.mr_frame()?;
        if frame.clause != Clause::Measures || scope.clause != MEASURES_CLAUSE {
            return None;
        }
        match expr {
            Expr::Column(column) if !frame.groups.contains(column) => Some(format!(
                "MATCH_RECOGNIZE MEASURES clause expression references column {} which is neither grouped nor aggregated",
                names.join(".")
            )),
            _ => None,
        }
    }

    /// Whether the call of `name` may plan where the pass is, or `false` after GoogleSQL's error:
    /// the functions of `MATCH_RECOGNIZE` in their clauses, and no window functions in the clause.
    pub(crate) fn mr_call_allowed(
        &mut self,
        name: &str,
        call: &ast::CallExpr,
        scope: Scope,
        safe: bool,
    ) -> bool {
        let range = call.syntax().text_range();
        let list = call.arg_list();
        let frame = self.mr_frame().map(|frame| (frame.clause, frame.name));
        let clause = frame.map(|(clause, _)| clause);
        if let (Some((_, clause_name)), Some(_)) = (frame, call.over_clause()) {
            self.error(
                range,
                format!("Analytic function not allowed in {clause_name}"),
            );
            return false;
        }
        match name {
            "PREV" | "NEXT" => {
                if clause != Some(Clause::Define) {
                    self.error(range, format!("function {name} must be directly inside a MATCH_RECOGNIZE's DEFINE clause, without any enclosing subquery"));
                    return false;
                }
                if safe {
                    self.error(
                        range,
                        format!("Function {name} does not support SAFE error mode"),
                    );
                    return false;
                }
                if list
                    .as_ref()
                    .is_some_and(|list| list.null_treatment().is_some())
                {
                    self.error(
                        range,
                        "IGNORE NULLS and RESPECT NULLS are not supported on scalar functions",
                    );
                    return false;
                }
                let offset = (list.iter())
                    .flat_map(|list| list.arguments())
                    .nth(1)
                    .and_then(|argument| match argument {
                        ast::Argument::Arg(arg) => arg.expr(),
                        _ => None,
                    });
                let window = if name == "PREV" { "LAG" } else { "LEAD" };
                let message = match &offset {
                    None => None,
                    Some(ast::Expr::Literal(literal)) => match literal.kind() {
                        Some(LiteralKind::Null) => {
                            Some(format!("Argument 2 to {name} must be non-NULL"))
                        }
                        _ => None,
                    },
                    Some(ast::Expr::ParamExpr(_)) => None,
                    Some(ast::Expr::PrefixExpr(prefix))
                        if prefix.operator() == Some(PrefixOp::Neg)
                            && matches!(prefix.expr(), Some(ast::Expr::Literal(ref l)) if l.kind() == Some(LiteralKind::Int)) =>
                    {
                        Some(format!(
                            "The offset to the function {window} must not be negative"
                        ))
                    }
                    Some(_) => Some(format!(
                        "Argument 2 to {name} must be a literal or query parameter"
                    )),
                };
                if let Some(message) = message {
                    let at = offset
                        .map(|offset| offset.syntax().text_range())
                        .unwrap_or(range);
                    self.error(at, message);
                    return false;
                }
                true
            }
            "FIRST" | "LAST" => {
                if clause != Some(Clause::Measures) {
                    self.error(range, format!("aggregate function {name} can only be used inside a MATCH_RECOGNIZE's MEASURES clause"));
                    return false;
                }
                if list
                    .as_ref()
                    .is_some_and(|list| list.distinct_kw().is_some())
                {
                    self.error(
                        range,
                        format!("Aggregate function {name} does not support DISTINCT in arguments"),
                    );
                    return false;
                }
                true
            }
            "MATCH_NUMBER" | "MATCH_ROW_NUMBER" | "CLASSIFIER" => {
                if clause != Some(Clause::Measures) {
                    let in_subquery = (self.match_recognize.iter()).any(|frame| {
                        frame.clause == Clause::Measures && frame.depth < self.outers.len()
                    });
                    let message = match in_subquery {
                        true => format!("{name} must be called directly from the MEASURES clause, not from a subquery"),
                        false => format!("function {name} can only be used inside a MATCH_RECOGNIZE's MEASURES clause"),
                    };
                    self.error(range, message);
                    return false;
                }
                if name != "MATCH_NUMBER" && scope.clause != ARGUMENT {
                    self.error(range, format!("Function {name} must be aggregated"));
                    return false;
                }
                true
            }
            "GROUPING" if clause == Some(Clause::Measures) => {
                self.error(
                    range,
                    "GROUPING must have an argument that exists within the group-by expression list.",
                );
                false
            }
            _ => true,
        }
    }

    /// The order and the `NULL`s of an aggregate of `MEASURES`: `FIRST` and `LAST` read the rows
    /// in their order without `NULL`s, and an aggregate that takes `ORDER BY` reads them in their
    /// order unless it orders them itself or takes `DISTINCT`.
    pub(crate) fn mr_aggregate_order(
        &self,
        name: &str,
        takes_order: bool,
        distinct: bool,
        order_by: Vec<SortExpr>,
        nulls: Option<datafusion::logical_expr::expr::NullTreatment>,
    ) -> (
        Vec<SortExpr>,
        Option<datafusion::logical_expr::expr::NullTreatment>,
    ) {
        let Some(position) = (self.mr_frame())
            .filter(|frame| frame.clause == Clause::Measures)
            .and_then(|frame| frame.position.clone())
        else {
            return (order_by, nulls);
        };
        let in_order = vec![SortExpr::new(Expr::Column(position), true, true)];
        match name {
            "FIRST" | "LAST" => (
                in_order,
                Some(datafusion::logical_expr::expr::NullTreatment::IgnoreNulls),
            ),
            _ if takes_order && order_by.is_empty() && !distinct => (in_order, nulls),
            _ => (order_by, nulls),
        }
    }

    /// The rows of a `MATCH_RECOGNIZE` over `rows`.
    pub(super) fn match_recognize(
        &mut self,
        rows: Rows,
        clause: &ast::MatchRecognizeClause,
        range: TextRange,
    ) -> Rows {
        let rows = rows.unordered();
        let mut ok = true;
        // The pattern variables of `DEFINE`, each once.
        let mut variables: Vec<(String, TextRange)> = Vec::new();
        let mut conditions: Vec<Option<ast::Expr>> = Vec::new();
        for definition in clause
            .define()
            .iter()
            .flat_map(|define| define.pattern_definitions())
        {
            let Some(name) = definition.name() else {
                ok = false;
                continue;
            };
            let Some(text) = name.text() else {
                ok = false;
                continue;
            };
            let at = name.syntax().text_range();
            if variables
                .iter()
                .any(|(other, _)| other.eq_ignore_ascii_case(&text))
            {
                self.error(
                    at,
                    format!("Pattern variable `{text}` is defined multiple times"),
                );
                ok = false;
                continue;
            }
            variables.push((text, at));
            conditions.push(definition.expr());
        }
        // The names of the output, for the model after an error.
        let partition_syntax: Vec<ast::Expr> = (clause.partition_by().into_iter())
            .flat_map(|p| p.exprs().collect::<Vec<_>>())
            .collect();
        let measures: Vec<ast::Measure> = (clause.measures().into_iter())
            .flat_map(|m| m.measures().collect::<Vec<_>>())
            .collect();
        let mut output_names: Vec<Col> = Vec::new();
        for expr in &partition_syntax {
            let name = match expr {
                ast::Expr::PathExpr(path) => (path.path().and_then(|p| p.names()))
                    .and_then(|names| names.last().cloned())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            output_names.push(self.syntax_col(&name));
        }
        for measure in &measures {
            let name = (measure.name().and_then(|n| n.text())).unwrap_or_default();
            let mut col = self.syntax_col(&name);
            col.definition = measure.name().map(|n| n.syntax().text_range());
            output_names.push(col);
        }
        let failed = |columns: &[Col]| {
            let vars = super::reshape::alias_var(clause.alias(), columns);
            Rows::failed(columns.to_vec(), vars)
        };
        // The pattern.
        let mut used = vec![false; variables.len()];
        let pattern = match clause.row_pattern().and_then(|p| p.pattern_alternation()) {
            Some(alternation) => self.pattern(&alternation, &variables, &mut used),
            None => None,
        };
        let Some(pattern) = pattern else {
            return failed(&output_names);
        };
        for ((name, at), used) in variables.iter().zip(&used) {
            if !used {
                self.error(
                    *at,
                    format!("Pattern variable `{name}` is defined but not used in the pattern"),
                );
                ok = false;
            }
        }
        if let Err(message) = Program::compile(&pattern) {
            let at = clause
                .row_pattern()
                .map(|p| p.syntax().text_range())
                .unwrap_or(range);
            self.error(at, message);
            ok = false;
        }
        let skip = match clause.after_match_skip() {
            Some(skip) if skip.to_kw().is_some() => Skip::ToNextRow,
            _ => Skip::PastLastRow,
        };
        let longest = self.mr_options(clause.options());
        // The partition and the order.
        let depth = self.outers.len();
        self.match_recognize.push(Frame {
            depth,
            clause: Clause::Keys,
            name: PARTITION_CLAUSE,
            variables: variables.clone(),
            copies: Vec::new(),
            groups: Vec::new(),
            position: None,
            value_fields: Vec::new(),
        });
        let mut partition = Vec::new();
        for expr in &partition_syntax {
            if let ast::Expr::Literal(literal) = expr {
                if literal.kind() == Some(LiteralKind::Int) {
                    self.error(
                        expr.syntax().text_range(),
                        "Ordinals are not allowed in MATCH_RECOGNIZE PARTITION BY clause",
                    );
                    ok = false;
                    continue;
                }
            }
            match self.expr(expr, Scope::of(&rows, PARTITION_CLAUSE)) {
                Some(planned) => partition.push(planned),
                None => ok = false,
            }
        }
        self.match_recognize.last_mut().expect("the frame").name = ORDER_CLAUSE;
        let mut order = Vec::new();
        for key in clause
            .order_by()
            .iter()
            .flat_map(|order| order.order_keys())
        {
            let Some(expr) = key.expr() else {
                ok = false;
                continue;
            };
            let at = expr.syntax().text_range();
            if let ast::Expr::Literal(literal) = &expr {
                if literal.kind() == Some(LiteralKind::Int) {
                    self.error(
                        at,
                        "Ordinals are not allowed in MATCH_RECOGNIZE ORDER BY clause",
                    );
                    ok = false;
                    continue;
                }
            }
            let scope = Scope::of(&rows, ORDER_CLAUSE);
            let Some(planned) = self.expr(&expr, scope) else {
                ok = false;
                continue;
            };
            if let Some(DataType::Struct(_)) = self.type_of(&planned, scope) {
                self.error(at, "Ordering by expressions of type STRUCT is not allowed");
                ok = false;
                continue;
            }
            let asc = key.desc_kw().is_none();
            let nulls_first = match (key.first_kw(), key.last_kw()) {
                (Some(_), _) => true,
                (_, Some(_)) => false,
                _ => asc,
            };
            order.push(SortExpr::new(planned, asc, nulls_first));
        }
        // `DEFINE`, over the rows of the input.
        {
            let frame = self.match_recognize.last_mut().expect("the frame");
            frame.clause = Clause::Define;
            frame.name = DEFINE_CLAUSE;
        }
        let mut planned_conditions = Vec::new();
        for condition in &conditions {
            let Some(condition) = condition else {
                ok = false;
                continue;
            };
            let at = condition.syntax().text_range();
            let scope = Scope::of(&rows, DEFINE_CLAUSE);
            let planned = self
                .expr(condition, scope)
                .and_then(|planned| self.boolean(planned, at, DEFINE_CLAUSE, scope))
                .and_then(|planned| self.navigated(planned, &partition, &order, at));
            match planned {
                Some(planned) => planned_conditions.push(planned),
                None => ok = false,
            }
        }
        let frame = self.match_recognize.pop().expect("the frame");
        if rows.plan.is_none() || !ok {
            // The measures, for the model of their names.
            self.mr_measures_names(&rows, &measures, frame);
            return failed(&output_names);
        }
        // 1. The position and the variables of each row.
        let visible = rows.columns.len();
        let mut items: Vec<Item> = rows.columns.iter().cloned().map(Item::Carry).collect();
        let mut partition_items = Vec::new();
        for (index, expr) in partition.iter().enumerate() {
            match expr {
                Expr::Column(column) => match rows.columns.iter().find(|col| col.column == *column)
                {
                    Some(col) => partition_items.push(Item::Carry(col.clone())),
                    None => partition_items.push(Item::Compute {
                        name: format!("$partition_by_col{}", index + 1),
                        expr: expr.clone(),
                        definition: None,
                    }),
                },
                _ => partition_items.push(Item::Compute {
                    name: format!("$partition_by_col{}", index + 1),
                    expr: expr.clone(),
                    definition: None,
                }),
            }
        }
        let partitions = partition_items.len();
        items.extend(partition_items);
        let row_number = super::reshape::window(
            WindowFunctionDefinition::WindowUDF(row_number_udwf()),
            Vec::new(),
            partition.clone(),
            order.clone(),
        );
        items.push(Item::Compute {
            name: "$position".to_string(),
            expr: Expr::Cast(Cast::new(Box::new(row_number), DataType::Int64)),
            definition: None,
        });
        for (index, condition) in planned_conditions.into_iter().enumerate() {
            items.push(Item::Compute {
                name: format!("$define{}", index + 1),
                expr: datafusion::functions::expr_fn::coalesce(vec![
                    condition,
                    Expr::Literal(ScalarValue::Boolean(Some(false)), None),
                ]),
                definition: None,
            });
        }
        let defined = self.projected(rows.clone(), items, true, false, range);
        let Some(plan) = defined.plan.clone() else {
            return failed(&output_names);
        };
        let column_at = |index: usize| defined.columns[index].column.clone();
        let partition_columns: Vec<Column> =
            (visible..visible + partitions).map(column_at).collect();
        let position = column_at(visible + partitions);
        let variable_columns: Vec<Column> = (0..variables.len())
            .map(|index| column_at(visible + partitions + 1 + index))
            .collect();
        // 2. The matches.
        let call = MatchRecognizeCall {
            partition_by: partition_columns.clone(),
            position: position.clone(),
            variables: variable_columns,
            names: variables.iter().map(|(name, _)| name.clone()).collect(),
            pattern,
            skip,
            longest,
            match_number: MATCH_NUMBER.to_string(),
            classifier: CLASSIFIER.to_string(),
        };
        let plan = match node::plan(plan, call) {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return failed(&output_names);
            }
        };
        let match_number = Column::from_name(MATCH_NUMBER);
        let classifier = Column::from_name(CLASSIFIER);
        // 3. The copies of the input's columns for each variable.
        let mut exprs: Vec<Expr> = plan
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        let mut copies = Vec::new();
        for (name, definition) in &variables {
            let qualifier = self.qualifiers.fresh("_");
            let mut columns = Vec::new();
            for (index, col) in rows.columns.iter().enumerate() {
                let field = plan_name(&col.name, index);
                exprs.push(
                    Expr::Column(col.column.clone())
                        .alias_qualified(Some(qualifier.clone()), &field),
                );
                columns.push(Col {
                    name: col.name.clone(),
                    column: Column::new(Some(qualifier.clone()), field),
                    definition: col.definition,
                });
            }
            copies.push(RangeVar {
                name: name.clone(),
                definition: Some(*definition),
                columns,
                value: None,
            });
        }
        let plan = match LogicalPlanBuilder::from(plan)
            .project(exprs)
            .and_then(|b| b.build())
        {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return failed(&output_names);
            }
        };
        // 4. The measures, grouped by the partition and the match.
        let shadowed = |var: &RangeVar| {
            (variables.iter()).any(|(name, _)| name.eq_ignore_ascii_case(&var.name))
        };
        let matched = Rows {
            plan: Some(plan),
            columns: rows.columns.clone(),
            vars: rows
                .vars
                .iter()
                .filter(|var| !shadowed(var))
                .cloned()
                .collect(),
            order: Vec::new(),
        };
        let mut groups: Vec<Expr> = partition_columns
            .iter()
            .cloned()
            .map(Expr::Column)
            .collect();
        groups.push(Expr::Column(match_number.clone()));
        let mut group_columns = partition_columns.clone();
        // A partition expression that is a column of the input reads as that column.
        for expr in &partition {
            if let Expr::Column(column) = expr {
                group_columns.push(column.clone());
            }
        }
        self.match_recognize.push(Frame {
            depth,
            clause: Clause::Measures,
            name: MEASURES_CLAUSE,
            variables: variables.clone(),
            copies: copies.clone(),
            groups: group_columns,
            position: Some(position.clone()),
            value_fields: (rows.vars.iter())
                .filter(|var| var.value.is_some())
                .flat_map(|var| var.columns.iter().map(|col| col.name.clone()))
                .collect(),
        });
        let mut planned_measures = Vec::new();
        for measure in &measures {
            let Some(expr) = measure.expr() else {
                ok = false;
                continue;
            };
            let scope = Scope::of(&matched, MEASURES_CLAUSE)
                .aggregates()
                .grouped(Some(&groups));
            match self.expr(&expr, scope) {
                Some(planned) => planned_measures.push((planned, expr.syntax().text_range())),
                None => ok = false,
            }
        }
        self.match_recognize.pop();
        if !ok {
            return failed(&output_names);
        }
        let mut items_exprs = Vec::new();
        for (expr, at) in planned_measures {
            let expr = of_match(expr, &match_number, &position, &classifier);
            match self.ranged(
                expr,
                &copies,
                &variables,
                &classifier,
                &position,
                &match_number,
                at,
            ) {
                Some(expr) => items_exprs.push((expr, at)),
                None => ok = false,
            }
        }
        if !ok {
            return failed(&output_names);
        }
        let Some(plan) = matched.plan.clone() else {
            return failed(&output_names);
        };
        // The windows of `HAVING MAX` and `HAVING MIN`, before the aggregate.
        let windows = find_window_exprs(items_exprs.iter().map(|(expr, _)| expr));
        let plan = match windows.is_empty() {
            true => Ok(plan),
            false => LogicalPlanBuilder::window_plan(plan, windows.clone()),
        };
        let rebased: datafusion::common::Result<Vec<Expr>> = (items_exprs.iter())
            .map(|(expr, _)| rows::rebase(expr.clone(), &windows))
            .collect();
        let (plan, measure_exprs) = match (plan, rebased) {
            (Ok(plan), Ok(exprs)) => (plan, exprs),
            (Err(error), _) | (_, Err(error)) => {
                self.datafusion_error(range, error);
                return failed(&output_names);
            }
        };
        let aggregates = find_aggregate_exprs(&measure_exprs);
        let plan = LogicalPlanBuilder::from(plan)
            .aggregate(groups.clone(), aggregates.clone())
            .and_then(|b| b.build());
        let plan: LogicalPlan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return failed(&output_names);
            }
        };
        let output = plan.schema().columns();
        let mut grouped: Vec<(Expr, Column)> =
            groups.iter().cloned().zip(output.iter().cloned()).collect();
        for (expr, column) in partition.iter().zip(&output) {
            if let Expr::Column(_) = expr {
                grouped.push((expr.clone(), column.clone()));
            }
        }
        let mut projection = Vec::new();
        for (index, col) in output_names.iter().take(partitions).enumerate() {
            projection.push(Item::Compute {
                name: col.name.clone(),
                expr: Expr::Column(output[index].clone()),
                definition: partition_syntax.get(index).map(|e| e.syntax().text_range()),
            });
        }
        for ((expr, at), col) in measure_exprs
            .into_iter()
            .zip(&items_exprs)
            .map(|(e, (_, at))| (e, *at))
            .zip(output_names.iter().skip(partitions))
        {
            let rebased = rows::rebase(expr, &aggregates)
                .and_then(|expr| super::query::regroup(expr, &grouped));
            let rebased = match rebased {
                Ok(expr) => expr,
                Err(error) => {
                    self.datafusion_error(at, error);
                    return failed(&output_names);
                }
            };
            if let Some(column) = (rebased.column_refs().into_iter()).find(|c| !output.contains(c))
            {
                self.error(
                    at,
                    format!("MATCH_RECOGNIZE MEASURES clause expression references column {} which is neither grouped nor aggregated", column.name),
                );
                return failed(&output_names);
            }
            projection.push(Item::Compute {
                name: col.name.clone(),
                expr: rebased,
                definition: col.definition,
            });
        }
        let rows = self.projected(
            Rows::new(plan, Vec::new(), Vec::new()),
            projection,
            false,
            false,
            range,
        );
        let vars = super::reshape::alias_var(clause.alias(), &rows.columns);
        Rows { vars, ..rows }
    }

    /// Plans the measures after an error in the clauses before them, so the model resolves their
    /// names, over the input's columns.
    fn mr_measures_names(&mut self, rows: &Rows, measures: &[ast::Measure], frame: Frame) {
        let copies = (frame.variables.iter())
            .map(|(name, definition)| RangeVar {
                name: name.clone(),
                definition: Some(*definition),
                columns: rows.columns.clone(),
                value: None,
            })
            .collect();
        let failed = Rows::failed(rows.columns.clone(), rows.vars.clone());
        self.match_recognize.push(Frame {
            clause: Clause::Measures,
            name: MEASURES_CLAUSE,
            copies,
            ..frame
        });
        for measure in measures {
            if let Some(expr) = measure.expr() {
                let scope = Scope::of(&failed, MEASURES_CLAUSE).aggregates();
                let errors = self.diagnostics.len();
                self.expr(&expr, scope);
                // The errors of names without a plan would repeat the clause's error.
                self.diagnostics.truncate(errors);
            }
        }
        self.match_recognize.pop();
    }

    /// `use_longest_match` of `OPTIONS`, or `false`, with GoogleSQL's errors.
    fn mr_options(&mut self, options: Option<ast::Options>) -> bool {
        let mut longest = None;
        for entry in options.iter().flat_map(|options| options.option_entries()) {
            let Some(name) = entry.name().and_then(|name| name.text()) else {
                continue;
            };
            let at = entry.syntax().text_range();
            if !name.eq_ignore_ascii_case("use_longest_match") {
                self.error(at, format!("Unrecognized MATCH_RECOGNIZE option `{name}`"));
                continue;
            }
            if longest.is_some() {
                self.error(
                    at,
                    "MATCH_RECOGNIZE option `use_longest_match` cannot be specified multiple times",
                );
                continue;
            }
            longest = Some(false);
            let Some(value) = entry.expr() else {
                continue;
            };
            let value_at = value.syntax().text_range();
            let empty = DFSchema::empty();
            let scope = constant_scope(&empty, "MATCH_RECOGNIZE OPTIONS");
            let Some(planned) = self.expr(&value, scope) else {
                continue;
            };
            let ty = self.type_of(&planned, scope);
            if !matches!(ty, Some(DataType::Boolean) | Some(DataType::Null)) {
                let name = ty
                    .map(|ty| crate::types::googlesql_name(&ty))
                    .unwrap_or_default();
                self.error(value_at, format!("MATCH_RECOGNIZE option `use_longest_match` requires a boolean value, but found {name}"));
                continue;
            }
            let literal = match &planned {
                Expr::Cast(cast) => cast.expr.as_ref(),
                other => other,
            };
            match literal {
                Expr::Literal(ScalarValue::Boolean(Some(value)), _) => longest = Some(*value),
                Expr::Literal(_, _) => self.error(value_at, "MATCH_RECOGNIZE option `use_longest_match` cannot be NULL"),
                Expr::Placeholder(_) => self.error(value_at, "avenger-sql takes a BOOL literal as the option `use_longest_match`, not a query parameter"),
                _ => self.error(value_at, "MATCH_RECOGNIZE option `use_longest_match` requires a boolean literal or parameter"),
            }
        }
        longest.unwrap_or(false)
    }

    /// The pattern of `alternation`, whose variables are those of `variables`, marking each that
    /// it uses in `used`, or `None` after an error.
    fn pattern(
        &mut self,
        alternation: &ast::PatternAlternation,
        variables: &[(String, TextRange)],
        used: &mut [bool],
    ) -> Option<Pattern> {
        let mut alternatives = Vec::new();
        let mut current = Pattern::Empty;
        let mut ok = true;
        for element in alternation.syntax().children_with_tokens() {
            match element {
                SyntaxElement::Node(node) => {
                    if let Some(sequence) = ast::PatternSequence::cast(node) {
                        match self.sequence(&sequence, variables, used) {
                            Some(pattern) => current = pattern,
                            None => ok = false,
                        }
                    }
                }
                SyntaxElement::Token(token) => match token.kind() {
                    SyntaxKind::BIT_OR => {
                        alternatives.push(std::mem::replace(&mut current, Pattern::Empty))
                    }
                    // `||` is an empty alternative between two others.
                    SyntaxKind::CONCAT => {
                        alternatives.push(std::mem::replace(&mut current, Pattern::Empty));
                        alternatives.push(Pattern::Empty);
                    }
                    _ => {}
                },
            }
        }
        alternatives.push(current);
        if !ok {
            return None;
        }
        Some(match alternatives.len() {
            1 => alternatives.pop().expect("an alternative"),
            _ => Pattern::Alternation(alternatives),
        })
    }

    fn sequence(
        &mut self,
        sequence: &ast::PatternSequence,
        variables: &[(String, TextRange)],
        used: &mut [bool],
    ) -> Option<Pattern> {
        let mut parts = Vec::new();
        let mut ok = true;
        for term in sequence.pattern_terms() {
            let (pattern, quantifier) = match term {
                ast::PatternTerm::PatternVariable(variable) => {
                    let Some(name_ref) = variable.name_ref() else {
                        ok = false;
                        continue;
                    };
                    let Some(name) = name_ref.text() else {
                        ok = false;
                        continue;
                    };
                    let at = name_ref.syntax().text_range();
                    let Some(index) = variables
                        .iter()
                        .position(|(v, _)| v.eq_ignore_ascii_case(&name))
                    else {
                        self.error(
                            at,
                            format!(
                                "Pattern variable `{name}` is not defined in the DEFINE clause"
                            ),
                        );
                        ok = false;
                        continue;
                    };
                    used[index] = true;
                    let (defined, definition) = &variables[index];
                    self.use_name(
                        at,
                        Definition::RangeVariable {
                            name: defined.clone(),
                            definition: Some(*definition),
                        },
                    );
                    (Some(Pattern::Variable(index)), variable.quantifier())
                }
                ast::PatternTerm::PatternGroup(group) => {
                    let inner = match group.pattern_alternation() {
                        Some(alternation) => self.pattern(&alternation, variables, used),
                        None => Some(Pattern::Empty),
                    };
                    (inner, group.quantifier())
                }
                ast::PatternTerm::PatternAnchor(anchor) => {
                    let anchor = match anchor.caret() {
                        Some(_) => Anchor::Start,
                        None => Anchor::End,
                    };
                    (Some(Pattern::Anchor(anchor)), None)
                }
            };
            let Some(pattern) = pattern else {
                ok = false;
                continue;
            };
            match quantifier {
                Some(quantifier) => match self.quantified(pattern, &quantifier) {
                    Some(pattern) => parts.push(pattern),
                    None => ok = false,
                },
                None => parts.push(pattern),
            }
        }
        if !ok {
            return None;
        }
        Some(match parts.len() {
            0 => Pattern::Empty,
            1 => parts.pop().expect("a part"),
            _ => Pattern::Concat(parts),
        })
    }

    /// `body` with `quantifier`.
    fn quantified(&mut self, body: Pattern, quantifier: &ast::Quantifier) -> Option<Pattern> {
        let at = quantifier.syntax().text_range();
        let tokens: Vec<SyntaxKind> = (quantifier.syntax().children_with_tokens())
            .filter_map(|element| element.into_token())
            .map(|token| token.kind())
            .filter(|kind| *kind != SyntaxKind::WHITESPACE && *kind != SyntaxKind::COMMENT)
            .collect();
        let questions = tokens
            .iter()
            .filter(|kind| **kind == SyntaxKind::POSITIONAL)
            .count();
        let (min, max, reluctant) = match tokens.first() {
            Some(SyntaxKind::POSITIONAL) => (0, Some(1), questions > 1),
            Some(SyntaxKind::STAR) => (0, None, questions > 0),
            Some(SyntaxKind::PLUS) => (1, None, questions > 0),
            Some(SyntaxKind::L_BRACE) => {
                let min = match quantifier.quantifier_min().and_then(|m| m.expr()) {
                    Some(expr) => self.bound(&expr)?,
                    None => 0,
                };
                let max = match (
                    quantifier.comma(),
                    quantifier.quantifier_max().and_then(|m| m.expr()),
                ) {
                    (None, _) => Some(min),
                    (Some(_), Some(expr)) => Some(self.bound(&expr)?),
                    (Some(_), None) => None,
                };
                if max.is_some_and(|max| max < min) {
                    self.error(
                        at,
                        "Quantifier upper bound cannot be smaller than the lower bound",
                    );
                    return None;
                }
                (min, max, questions > 0)
            }
            _ => return None,
        };
        Some(Pattern::Repeat {
            body: Box::new(body),
            min,
            max,
            greedy: !reluctant,
        })
    }

    /// A bound of a quantifier, an `INT64` literal.
    fn bound(&mut self, expr: &ast::Expr) -> Option<u32> {
        let at = expr.syntax().text_range();
        let empty = DFSchema::empty();
        let planned = self.expr(expr, constant_scope(&empty, "MATCH_RECOGNIZE PATTERN"))?;
        match planned {
            Expr::Literal(ScalarValue::Int64(Some(value)), _) if value < 0 => {
                self.error(at, "Negative quantifier bound value is not allowed");
                None
            }
            Expr::Literal(ScalarValue::Int64(Some(value)), _) => match u32::try_from(value) {
                Ok(value) if (value as usize) < node::MAX_INSTRUCTIONS => Some(value),
                _ => {
                    self.error(at, format!("The PATTERN of MATCH_RECOGNIZE compiles to more than {} instructions; write fewer or smaller bounded quantifiers", node::MAX_INSTRUCTIONS));
                    None
                }
            },
            Expr::Placeholder(_) => {
                self.error(at, "avenger-sql takes INT64 literals as the bounds of a quantifier, not query parameters");
                None
            }
            _ => {
                self.error(
                    at,
                    "A quantifier bound must be an INT64 literal or query parameter",
                );
                None
            }
        }
    }

    /// `condition` with each `PREV` and `NEXT` as `LAG` and `LEAD` over `partition` in `order`, or
    /// `None` after GoogleSQL's error for one in another.
    fn navigated(
        &mut self,
        condition: Expr,
        partition: &[Expr],
        order: &[SortExpr],
        at: TextRange,
    ) -> Option<Expr> {
        let mut nested = None;
        let _ = condition.apply(|e| {
            for outer in ["PREV", "NEXT"] {
                if let Some(args) = marker_args(e, outer) {
                    for inner in ["PREV", "NEXT"] {
                        let found = args.iter().any(|arg| {
                            arg.exists(|e| Ok(marker_args(e, inner).is_some()))
                                .unwrap_or(false)
                        });
                        if found {
                            nested = Some(inner);
                        }
                    }
                }
            }
            Ok(TreeNodeRecursion::Continue)
        });
        if let Some(inner) = nested {
            self.error(
                at,
                format!("function {inner} cannot be nested in another PREV() or NEXT() call"),
            );
            return None;
        }
        let transformed = condition.transform_up(|e| {
            for (name, udwf) in [("PREV", lag_udwf()), ("NEXT", lead_udwf())] {
                if let Some(args) = marker_args(&e, name) {
                    let window = super::reshape::window(
                        WindowFunctionDefinition::WindowUDF(udwf),
                        args.to_vec(),
                        partition.to_vec(),
                        order.to_vec(),
                    );
                    return Ok(Transformed::yes(window));
                }
            }
            Ok(Transformed::no(e))
        });
        match transformed {
            Ok(transformed) => Some(transformed.data),
            Err(error) => {
                self.datafusion_error(at, error);
                None
            }
        }
    }

    /// `expr`, a measure, with each aggregate reading the rows of its range: the rows of one
    /// variable when it reads that variable's columns, and the rows of the match otherwise, or
    /// `None` after GoogleSQL's error for an aggregate of two ranges.
    #[allow(clippy::too_many_arguments)]
    fn ranged(
        &mut self,
        expr: Expr,
        copies: &[RangeVar],
        variables: &[(String, TextRange)],
        classifier: &Column,
        position: &Column,
        match_number: &Column,
        at: TextRange,
    ) -> Option<Expr> {
        let range_of = |column: &Column| -> Option<Range> {
            if column == classifier || column == position || column == match_number {
                return None;
            }
            match copies
                .iter()
                .position(|var| var.columns.iter().any(|col| col.column == *column))
            {
                Some(index) => Some(Range::Variable(index)),
                None => Some(Range::All),
            }
        };
        let name = |range: Range| match range {
            Range::All => "all input rows".to_string(),
            Range::Variable(index) => format!("pattern variable {}", variables[index].0),
        };
        let mut error = None;
        let transformed = expr.transform_down(|e| {
            let Expr::AggregateFunction(aggregate) = &e else {
                return Ok(Transformed::no(e));
            };
            let mut range: Option<Range> = None;
            let mut parts: Vec<&Expr> = aggregate.params.args.iter().collect();
            parts.extend(aggregate.params.order_by.iter().map(|key| &key.expr));
            if let Some(filter) = &aggregate.params.filter {
                parts.push(filter);
            }
            for part in parts {
                let _ = part.apply(|e| {
                    if let Expr::Column(column) = e {
                        if let Some(found) = range_of(column) {
                            match range {
                                None => range = Some(found),
                                Some(current) if current != found && error.is_none() => {
                                    error = Some(format!(
                                        "Column access ranges over {} in an expression that already ranges over a {}",
                                        name(found),
                                        name(current)
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(TreeNodeRecursion::Continue)
                });
            }
            let condition = match range {
                Some(Range::Variable(index)) => {
                    Expr::Column(classifier.clone()).eq(string(&variables[index].0))
                }
                _ => Expr::Column(classifier.clone()).is_not_null(),
            };
            let mut aggregate: AggregateFunction = aggregate.clone();
            let filter = match aggregate.params.filter.take() {
                Some(filter) => scoped(*filter, &condition)?.and(condition),
                None => condition,
            };
            aggregate.params.filter = Some(Box::new(filter));
            Ok(Transformed::new(
                Expr::AggregateFunction(aggregate),
                true,
                TreeNodeRecursion::Jump,
            ))
        });
        if let Some(message) = error {
            self.error(at, message);
            return None;
        }
        match transformed {
            Ok(transformed) => Some(transformed.data),
            Err(error) => {
                self.datafusion_error(at, error);
                None
            }
        }
    }
}

/// `filter`, the filter of an aggregate, with the windows of its `HAVING MAX` or `HAVING MIN`
/// over the rows of `condition` only.
fn scoped(filter: Expr, condition: &Expr) -> datafusion::common::Result<Expr> {
    filter
        .transform_up(|e| match e {
            Expr::WindowFunction(mut window) => {
                window.params.args = (window.params.args.into_iter())
                    .map(|arg| {
                        Expr::Case(Case::new(
                            None,
                            vec![(Box::new(condition.clone()), Box::new(arg))],
                            None,
                        ))
                    })
                    .collect();
                Ok(Transformed::new(
                    Expr::WindowFunction(window),
                    true,
                    TreeNodeRecursion::Jump,
                ))
            }
            e => Ok(Transformed::no(e)),
        })
        .map(|t| t.data)
}

/// `expr` with `MATCH_NUMBER()`, `MATCH_ROW_NUMBER()` and `CLASSIFIER()` as their columns.
fn of_match(expr: Expr, match_number: &Column, position: &Column, classifier: &Column) -> Expr {
    expr.transform_up(|e| {
        for (name, column) in [
            ("MATCH_NUMBER", match_number),
            ("MATCH_ROW_NUMBER", position),
            ("CLASSIFIER", classifier),
        ] {
            if marker_args(&e, name).is_some() {
                return Ok(Transformed::yes(Expr::Column(column.clone())));
            }
        }
        Ok(Transformed::no(e))
    })
    .map(|t| t.data)
    .expect("the columns of a match")
}

/// The scope of a constant of the clause, such as a bound of a quantifier.
fn constant_scope<'s>(empty: &'s DFSchema, clause: &'static str) -> Scope<'s> {
    Scope {
        columns: &[],
        vars: &[],
        schema: Some(empty),
        clause,
        aggregates: false,
        windows: false,
        groups: Some(&[]),
    }
}
