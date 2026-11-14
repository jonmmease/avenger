//! Scripts: variables, temporary tables, temporary functions, and modules.
//!
//! A `DECLARE` or a `SET` is a plan of its own, of one row and one column, and a variable is a
//! placeholder of its type in the plans after it, which a column of the same name hides, so a
//! runner evaluates each variable once and binds its value, as [`super::Analysis::run`] does. A
//! temporary table is a view: a read of it plans its query in place, under a qualifier of its
//! own. A temporary function is inlined: a call plans its body with each parameter bound to its
//! argument, `ANY TYPE` taking the argument's type; an aggregate function's body computes its
//! aggregates in the caller's `AGGREGATE`, and a `NOT AGGREGATE` parameter takes a constant; a
//! table function's body is a query whose table parameters are the tables of its arguments.
//! A function is visible to the statements after its `CREATE`.
//!
//! `IMPORT MODULE` reads a module's text from the catalog, and its public functions and constants
//! are visible under the import's alias, as `m.f(x)` and `m.c`. Within the module, its private
//! functions and the modules it imports are visible too. An error in a module is reported at the
//! import, or at the call of the module's function.

use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt, Scope as CreateScope},
    TextRange,
};
use datafusion::{
    arrow::datatypes::{DataType, Field},
    common::{Column, ScalarValue},
    logical_expr::{expr::Placeholder, Cast, Expr, LogicalPlanBuilder},
};
use std::sync::Arc;

use super::{
    expr::Scope,
    model::{CallForm, Definition},
    rows::{Col, Rows},
    Pass, StatementPlan, WithName,
};
use crate::{
    library::{form::coercion, Ty},
    types,
};

/// A script variable.
#[derive(Clone, Debug)]
pub(crate) struct Variable {
    pub name: String,
    /// The placeholder of its value.
    pub id: String,
    pub ty: DataType,
    pub definition: TextRange,
}

/// The kind of a temporary function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Scalar,
    Aggregate,
    Table,
}

/// What a parameter of a temporary function takes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ParamTy {
    Value(Ty),
    /// `ANY TYPE`, which takes the type of its argument.
    Any,
    /// `ANY TABLE` or `TABLE<...>`, a table of a table function.
    Table,
}

#[derive(Clone, Debug)]
pub(crate) struct Param {
    pub name: String,
    pub ty: ParamTy,
    pub not_aggregate: bool,
}

/// A temporary function, or a function of a module.
#[derive(Clone, Debug)]
pub(crate) struct TempFunction {
    /// The path that a call names, in lower case, as `f` or `m.f` for a module's function.
    pub path: String,
    /// The name as its `CREATE` writes it.
    pub name: String,
    pub kind: Kind,
    pub params: Vec<Param>,
    pub returns: Option<Ty>,
    pub body: ast::FunctionBody,
    /// Whether a call from outside its module sees it.
    pub public: bool,
    /// The module whose names its body reads first.
    pub module: Option<String>,
}

/// A constant of a module.
#[derive(Clone, Debug)]
pub(crate) struct Constant {
    pub path: String,
    pub expr: Expr,
    pub public: bool,
}

/// The module that a `CREATE` or an `IMPORT` of a module belongs to.
#[derive(Clone, Debug)]
pub(crate) struct InModule {
    /// The path of the module's import, in lower case.
    pub path: String,
    /// The range of the import, where the module's errors go.
    pub import: TextRange,
}

fn cast(expr: Expr, ty: DataType) -> Expr {
    Expr::Cast(Cast::new(Box::new(expr), ty))
}

impl Pass<'_> {
    /// The plan of the value `expr` in one row, as its column `$value`.
    fn value_plan(
        &mut self,
        expr: Expr,
        range: TextRange,
    ) -> Option<datafusion::logical_expr::LogicalPlan> {
        let one = self.one_row();
        let plan = LogicalPlanBuilder::from(one.plan.expect("one row"))
            .project(vec![expr.alias("$value")])
            .and_then(|b| b.build());
        match plan {
            Ok(plan) => Some(plan),
            Err(error) => {
                self.datafusion_error(range, error);
                None
            }
        }
    }

    /// `expr` as a value of the type `ty`, or `None` with GoogleSQL's error at `range`.
    fn value_of_type(
        &mut self,
        expr: Expr,
        ty: &DataType,
        range: TextRange,
        scope: Scope,
    ) -> Option<Expr> {
        let want = Ty::of_arrow(ty)?;
        let found = self
            .type_of(&expr, scope)
            .and_then(|ty| Ty::of_arrow(&ty))?;
        let literal = matches!(expr, Expr::Literal(..));
        if coercion(&found, literal, &want).is_none() {
            self.error(range, format!("Expected type {want}; found {found}"));
            return None;
        }
        Some(match self.type_of(&expr, scope) {
            Some(current) if current == *ty => expr,
            _ => cast(expr, ty.clone()),
        })
    }

    /// The plans of a `DECLARE`: a variable for each name, of the type, of the value of its
    /// `DEFAULT` or `NULL`.
    pub(super) fn declare(&mut self, declare: &ast::DeclareStmt) -> Vec<StatementPlan> {
        let names: Vec<(String, TextRange)> = declare
            .names()
            .filter_map(|name| Some((name.text()?, name.syntax().text_range())))
            .collect();
        let declared = match declare.ty() {
            Some(ty) => match types::arrow_type(&ty, self.type_options) {
                Ok(ty) => Some(ty),
                Err(error) => {
                    self.error(error.range, error.message);
                    return Vec::new();
                }
            },
            None => None,
        };
        let one = self.one_row();
        let scope = Scope::of(&one, "DECLARE");
        let value = match declare.expr() {
            Some(expr) => {
                let at = expr.syntax().text_range();
                let Some(planned) = self.expr(&expr, scope) else {
                    return Vec::new();
                };
                match &declared {
                    Some(ty) => match self.value_of_type(planned, ty, at, scope) {
                        Some(value) => value,
                        None => return Vec::new(),
                    },
                    None => planned,
                }
            }
            None => cast(
                Expr::Literal(ScalarValue::Null, None),
                declared.clone().unwrap_or(DataType::Int64),
            ),
        };
        let ty = match (&declared, self.type_of(&value, scope)) {
            (Some(ty), _) => ty.clone(),
            (None, Some(DataType::Null)) | (None, None) => DataType::Int64,
            (None, Some(ty)) => ty,
        };
        let value = match self.type_of(&value, scope) {
            Some(current) if current == ty => value,
            _ => cast(value, ty.clone()),
        };
        let mut plans = Vec::new();
        for (name, definition) in names {
            if self
                .variables
                .iter()
                .any(|v| v.name.eq_ignore_ascii_case(&name))
            {
                self.error(definition, format!("Variable '{name}' redeclaration"));
                continue;
            }
            let id = format!("$__var_{}", self.variables.len() + 1);
            let Some(plan) = self.value_plan(value.clone(), definition) else {
                continue;
            };
            self.variables.push(Variable {
                name: name.clone(),
                id: id.clone(),
                ty: ty.clone(),
                definition,
            });
            plans.push(StatementPlan::Variable { name, id, plan });
        }
        plans
    }

    /// The plans of a `SET` of one variable, or of several from the fields of a struct.
    pub(super) fn set_variable(&mut self, set: &ast::SetStmt) -> Vec<StatementPlan> {
        let targets: Vec<ast::NameRef> = match (set.name_ref(), set.name_list()) {
            (Some(name), _) => vec![name],
            (None, Some(list)) => list.name_refs().collect(),
            (None, None) => return Vec::new(),
        };
        let mut variables = Vec::new();
        for target in &targets {
            let Some(name) = target.text() else {
                return Vec::new();
            };
            let at = target.syntax().text_range();
            match self
                .variables
                .iter()
                .find(|v| v.name.eq_ignore_ascii_case(&name))
                .cloned()
            {
                Some(variable) => {
                    self.use_name(
                        at,
                        Definition::Variable {
                            name: variable.name.clone(),
                            definition: Some(variable.definition),
                            ty: Some(variable.ty.clone()),
                        },
                    );
                    variables.push(variable);
                }
                None => {
                    self.error(at, format!("Undeclared variable: {name}"));
                    return Vec::new();
                }
            }
        }
        let Some(expr) = set.expr() else {
            return Vec::new();
        };
        let at = expr.syntax().text_range();
        let one = self.one_row();
        let scope = Scope::of(&one, "SET");
        let Some(value) = self.expr(&expr, scope) else {
            return Vec::new();
        };
        let values: Vec<Expr> = match variables.len() {
            1 => vec![value],
            n => match self.type_of(&value, scope) {
                Some(DataType::Struct(fields)) if fields.len() == n => (fields.iter())
                    .map(|field| {
                        datafusion::functions::core::expr_fn::get_field(
                            value.clone(),
                            field.name().as_str(),
                        )
                    })
                    .collect(),
                _ => {
                    self.error(
                        at,
                        format!("SET of {n} variables takes a struct of {n} fields"),
                    );
                    return Vec::new();
                }
            },
        };
        let mut plans = Vec::new();
        for (variable, value) in variables.into_iter().zip(values) {
            let Some(value) = self.value_of_type(value, &variable.ty, at, scope) else {
                return Vec::new();
            };
            if let Some(plan) = self.value_plan(value, at) {
                plans.push(StatementPlan::Variable {
                    name: variable.name,
                    id: variable.id,
                    plan,
                });
            }
        }
        plans
    }

    /// The placeholder of the script variable `name`, when the script declares one.
    pub(super) fn variable(&mut self, name: &str, range: TextRange) -> Option<Expr> {
        let variable = (self.variables.iter())
            .find(|v| v.name.eq_ignore_ascii_case(name))
            .cloned()?;
        self.use_name(
            range,
            Definition::Variable {
                name: variable.name.clone(),
                definition: Some(variable.definition),
                ty: Some(variable.ty.clone()),
            },
        );
        let field = Arc::new(Field::new(variable.id.clone(), variable.ty.clone(), true));
        Some(Expr::Placeholder(Placeholder::new_with_field(
            variable.id,
            Some(field),
        )))
    }

    /// A `CREATE TEMP TABLE`: the table of its query, or an empty table of its columns.
    pub(super) fn create_table(&mut self, create: &ast::CreateTableStmt) {
        let range = create.syntax().text_range();
        let Some(path) = create.name_path() else {
            return;
        };
        let Some(names) = path.names() else {
            return;
        };
        let path_range = path.syntax().text_range();
        if create.scope_kind() != Some(CreateScope::Temp) {
            self.error(
                range,
                "avenger-sql creates temporary tables only, with CREATE TEMP TABLE",
            );
            return;
        }
        let name = names.join(".");
        let existing =
            (self.temp_tables.iter()).position(|table| table.name.eq_ignore_ascii_case(&name));
        if existing.is_some() && create.if_not_exists() {
            return;
        }
        if existing.is_some() && !create.or_replace() {
            self.error(path_range, format!("Table {name} already exists"));
            return;
        }
        let columns: Option<Vec<(String, DataType, TextRange)>> =
            create.column_list().map(|list| {
                list.column_defs()
                    .filter_map(|column| {
                        let name = column.name()?;
                        let ty = types::arrow_type(&column.ty()?, self.type_options).ok()?;
                        Some((name.text()?, ty, name.syntax().text_range()))
                    })
                    .collect()
            });
        let rows = match (create.query(), columns) {
            (Some(query), columns) => {
                let rows = self.query(&query);
                match columns {
                    None => rows,
                    Some(columns) => self.typed_columns(rows, &columns, range),
                }
            }
            (None, Some(columns)) => {
                let exprs: Vec<Expr> = (columns.iter())
                    .map(|(name, ty, _)| {
                        cast(Expr::Literal(ScalarValue::Null, None), ty.clone()).alias(name)
                    })
                    .collect();
                let plan = LogicalPlanBuilder::empty(false)
                    .project(exprs)
                    .and_then(|b| b.build());
                let cols = (columns.iter())
                    .map(|(name, _, at)| Col {
                        name: name.clone(),
                        column: Column::from_name(name),
                        definition: Some(*at),
                    })
                    .collect();
                match plan {
                    Ok(plan) => Rows::new(plan, cols, Vec::new()),
                    Err(error) => {
                        self.datafusion_error(range, error);
                        return;
                    }
                }
            }
            (None, None) => {
                self.error(range, "CREATE TEMP TABLE takes columns or AS and a query");
                return;
            }
        };
        if rows.plan.is_none() {
            return;
        }
        if let Some(existing) = existing {
            self.temp_tables.remove(existing);
        }
        self.temp_tables.push(WithName {
            name,
            definition: path_range,
            rows,
        });
    }

    /// `rows` with the names and the types of `columns`, by place.
    fn typed_columns(
        &mut self,
        rows: Rows,
        columns: &[(String, DataType, TextRange)],
        range: TextRange,
    ) -> Rows {
        if rows.columns.len() != columns.len() {
            self.error(
                range,
                format!(
                    "The table has {} columns but its query has {}",
                    columns.len(),
                    rows.columns.len()
                ),
            );
            return rows.fail();
        }
        let items = (rows.columns.iter().zip(columns))
            .map(|(col, (name, ty, at))| super::rows::Item::Compute {
                name: name.clone(),
                expr: cast(Expr::Column(col.column.clone()), ty.clone()),
                definition: Some(*at),
            })
            .collect();
        self.projected(rows, items, false, false, range)
    }

    /// The rows of the temporary table `names`, or of a table argument of a table function.
    pub(super) fn temp_table(&mut self, names: &[String], range: TextRange) -> Option<WithName> {
        let name = names.join(".");
        let found = (self.temp_tables.iter().rev())
            .find(|table| table.name.eq_ignore_ascii_case(&name))
            .cloned()?;
        self.use_name(
            range,
            Definition::Query {
                name: found.name.clone(),
                definition: Some(found.definition),
            },
        );
        Some(found)
    }

    /// A `CREATE TEMP FUNCTION`, or a function of the module `module`.
    pub(super) fn create_function(
        &mut self,
        create: &ast::CreateFunctionStmt,
        module: Option<&InModule>,
    ) {
        let range = create.syntax().text_range();
        let Some(names) = create.name_path().and_then(|path| path.names()) else {
            return;
        };
        let name = names.join(".");
        let public = match (module, create.scope_kind()) {
            (Some(_), Some(CreateScope::Public)) => true,
            (Some(_), _) => false,
            (None, Some(CreateScope::Temp)) => true,
            (None, _) => {
                self.error(
                    range,
                    "avenger-sql creates temporary functions only, with CREATE TEMP FUNCTION",
                );
                return;
            }
        };
        let kind = match create.function_kind() {
            ast::FunctionKind::Scalar => Kind::Scalar,
            ast::FunctionKind::Aggregate => Kind::Aggregate,
            ast::FunctionKind::Table => Kind::Table,
        };
        let mut params = Vec::new();
        for parameter in create
            .param_list()
            .iter()
            .flat_map(|list| list.parameters())
        {
            let Some(param_name) = parameter.name().and_then(|n| n.text()) else {
                return;
            };
            let ty = match parameter.param_type() {
                Some(ast::ParamType::Type(ty)) => match types::arrow_type(&ty, self.type_options) {
                    Ok(ty) => match Ty::of_arrow(&ty) {
                        Some(ty) => ParamTy::Value(ty),
                        None => return,
                    },
                    Err(error) => {
                        self.error(error.range, error.message);
                        return;
                    }
                },
                Some(ast::ParamType::AnyType(_)) => ParamTy::Any,
                Some(ast::ParamType::AnyTable(_) | ast::ParamType::TableType(_)) => ParamTy::Table,
                None => return,
            };
            params.push(Param {
                name: param_name,
                ty,
                not_aggregate: parameter.aggregate_kw().is_some(),
            });
        }
        let returns = match create.returns_clause().and_then(|returns| returns.ty()) {
            Some(ty) => match types::arrow_type(&ty, self.type_options) {
                Ok(ty) => Ty::of_arrow(&ty),
                Err(error) => {
                    self.error(error.range, error.message);
                    return;
                }
            },
            None => None,
        };
        let Some(body) = create.function_body() else {
            return;
        };
        let path = match module {
            Some(module) => format!("{}.{}", module.path, name.to_ascii_lowercase()),
            None => name.to_ascii_lowercase(),
        };
        let existing = self.functions.iter().position(|f| f.path == path);
        if existing.is_some() && create.if_not_exists() {
            return;
        }
        if existing.is_some() && !create.or_replace() {
            self.error(range, format!("Function {name} already exists"));
            return;
        }
        let function = TempFunction {
            path,
            name,
            kind,
            params,
            returns,
            body,
            public,
            module: module.map(|m| m.path.clone()),
        };
        // The body, with each parameter of a type, planned once for its errors and the model.
        if kind != Kind::Table
            && function
                .params
                .iter()
                .all(|p| matches!(p.ty, ParamTy::Value(_)))
        {
            let args: Vec<Expr> = (function.params.iter())
                .map(|param| match &param.ty {
                    ParamTy::Value(ty) => cast(
                        Expr::Literal(ScalarValue::Null, None),
                        ty.arrow(self.type_options),
                    ),
                    _ => unreachable!(),
                })
                .collect();
            let one = self.one_row();
            let scope = Scope::of(&one, "the function's body");
            let scope = match kind {
                Kind::Aggregate => scope.aggregates(),
                _ => scope,
            };
            if self.body(&function, args, scope).is_none() {
                return;
            }
        }
        if let Some(existing) = existing {
            self.functions.remove(existing);
        }
        self.functions.push(function);
    }

    /// The temporary function or the module function that a call names, in the module whose body
    /// the pass plans first.
    pub(super) fn temp_function(&self, names: &[String]) -> Option<TempFunction> {
        let path = names.join(".").to_ascii_lowercase();
        for module in self.module_context.iter().rev() {
            let key = format!("{module}.{path}");
            if let Some(function) = self.functions.iter().find(|f| f.path == key) {
                return Some(function.clone());
            }
        }
        (self.functions.iter())
            .find(|f| f.path == path && (f.public || f.module.is_none()))
            .cloned()
    }

    /// The constant of a module at `names`, in the module whose body the pass plans first.
    pub(super) fn module_constant(&self, names: &[String]) -> Option<Expr> {
        let path = names.join(".").to_ascii_lowercase();
        for module in self.module_context.iter().rev() {
            let key = format!("{module}.{path}");
            if let Some(constant) = self.constants.iter().find(|c| c.path == key) {
                return Some(constant.expr.clone());
            }
        }
        (self.constants.iter())
            .find(|c| c.path == path && c.public)
            .map(|c| c.expr.clone())
    }

    /// The plan of the body of `function`, of a scalar or an aggregate function, with its
    /// parameters bound to `args`, in the caller's `scope`, which the body reads only for the types
    /// of the arguments and its permission of aggregates.
    fn body(&mut self, function: &TempFunction, args: Vec<Expr>, scope: Scope) -> Option<Expr> {
        let expr = function.body.expr()?;
        let bindings: Vec<(String, Expr)> = (function.params.iter().zip(args))
            .map(|(param, arg)| (param.name.clone(), arg))
            .collect();
        let saved_params = std::mem::replace(&mut self.params, bindings);
        let saved_outers = std::mem::take(&mut self.outers);
        let saved_variables = std::mem::take(&mut self.variables);
        let pushed = function
            .module
            .clone()
            .map(|m| self.module_context.push(m))
            .is_some();
        let body_scope = Scope {
            columns: &[],
            vars: &[],
            ..scope
        };
        let planned = self.expr(&expr, body_scope);
        if pushed {
            self.module_context.pop();
        }
        self.variables = saved_variables;
        self.outers = saved_outers;
        self.params = saved_params;
        let planned = planned?;
        let Some(returns) = &function.returns else {
            return Some(planned);
        };
        let found = self
            .type_of(&planned, scope)
            .and_then(|ty| Ty::of_arrow(&ty))?;
        if coercion(&found, matches!(planned, Expr::Literal(..)), returns).is_none() {
            self.error(
                expr.syntax().text_range(),
                format!(
                    "Function declared to return {returns} but the function body produces incompatible type {found}"
                ),
            );
            return None;
        }
        self.coerce_to(planned, returns, scope)
    }

    /// The argument of the parameter `name` of the body of the function that the pass inlines.
    pub(super) fn param_arg(&self, name: &str) -> Option<Expr> {
        (self.params.iter())
            .find(|(param, _)| param.eq_ignore_ascii_case(name))
            .map(|(_, arg)| arg.clone())
    }

    /// The arguments of a call of `function`, by the place or the name of each parameter, or
    /// `None` with an error.
    fn bind<T: Clone>(
        &mut self,
        function: &TempFunction,
        args: Vec<(Option<String>, T, TextRange)>,
        range: TextRange,
    ) -> Option<Vec<(T, TextRange)>> {
        let mut bound: Vec<Option<(T, TextRange)>> = vec![None; function.params.len()];
        let mut next = 0;
        for (name, arg, at) in args {
            let index = match name {
                None => {
                    let index = next;
                    next += 1;
                    index
                }
                Some(name) => match function
                    .params
                    .iter()
                    .position(|p| p.name.eq_ignore_ascii_case(&name))
                {
                    Some(index) => index,
                    None => {
                        self.error(
                            at,
                            format!("Function {} has no parameter named {name}", function.name),
                        );
                        return None;
                    }
                },
            };
            if index >= function.params.len() {
                self.error(
                    range,
                    format!(
                        "No matching signature for function {}: it takes {} arguments",
                        function.name,
                        function.params.len()
                    ),
                );
                return None;
            }
            bound[index] = Some((arg, at));
        }
        if let Some(missing) = bound.iter().position(Option::is_none) {
            self.error(
                range,
                format!(
                    "No matching signature for function {}: the argument {} is missing",
                    function.name, function.params[missing].name
                ),
            );
            return None;
        }
        Some(
            bound
                .into_iter()
                .map(|arg| arg.expect("an argument"))
                .collect(),
        )
    }

    /// A call of the temporary scalar or aggregate function `function`.
    pub(super) fn call_temp(
        &mut self,
        function: TempFunction,
        call: &ast::CallExpr,
        scope: Scope,
        name_range: TextRange,
    ) -> Option<Expr> {
        let range = call.syntax().text_range();
        self.use_name(
            name_range,
            Definition::Function {
                name: function.name.clone(),
            },
        );
        if function.kind == Kind::Table {
            self.error(
                range,
                format!("Table-valued function {} goes in FROM", function.name),
            );
            return None;
        }
        if function.kind == Kind::Aggregate && !scope.aggregates {
            self.error(
                range,
                format!(
                    "Aggregate function {} not allowed in {}",
                    function.name, scope.clause
                ),
            );
            return None;
        }
        let arg_scope = match function.kind {
            Kind::Aggregate => Scope {
                aggregates: false,
                windows: false,
                ..scope
            },
            _ => scope,
        };
        let mut args = Vec::new();
        for argument in call.arg_list().iter().flat_map(|list| list.arguments()) {
            let at = argument.syntax().text_range();
            let (name, expr) = match &argument {
                ast::Argument::Arg(arg) => (None, arg.expr()),
                ast::Argument::NamedArg(arg) => (arg.name_ref().and_then(|n| n.text()), arg.expr()),
                _ => {
                    self.error(at, format!("{} takes no TABLE argument", function.name));
                    return None;
                }
            };
            let planned = self.expr(&expr?, arg_scope);
            args.push((name, planned, at));
        }
        let args: Vec<(Option<String>, Expr, TextRange)> = (args.into_iter())
            .map(|(name, planned, at)| Some((name, planned?, at)))
            .collect::<Option<_>>()?;
        let bound = self.bind(&function, args, range)?;
        // Each argument in the type of its parameter.
        let mut values = Vec::new();
        let mut types = Vec::new();
        for ((value, at), param) in bound.into_iter().zip(&function.params) {
            let found = self
                .type_of(&value, scope)
                .and_then(|ty| Ty::of_arrow(&ty))?;
            types.push(found.to_string());
            let value = match &param.ty {
                ParamTy::Value(want) => {
                    if coercion(&found, matches!(value, Expr::Literal(..)), want).is_none() {
                        self.error(
                            at,
                            format!(
                                "No matching signature for function {}: unable to coerce type {found} to expected type {want}",
                                function.name
                            ),
                        );
                        return None;
                    }
                    self.coerce_to(value, want, scope)?
                }
                ParamTy::Any => value,
                ParamTy::Table => {
                    self.error(at, format!("{} takes no table", function.name));
                    return None;
                }
            };
            if param.not_aggregate
                && !matches!(
                    value,
                    Expr::Literal(..) | Expr::Placeholder(_) | Expr::Cast(_)
                )
            {
                self.error(
                    at,
                    format!(
                        "The argument {} of {} is a constant",
                        param.name, function.name
                    ),
                );
                return None;
            }
            values.push(value);
        }
        if self.call_depth > 32 {
            self.error(
                range,
                format!("The calls of {} recur too deeply", function.name),
            );
            return None;
        }
        // The body's names and errors belong to its definition: the model keeps them from its
        // `CREATE`, and an error of an inlined body goes to the call.
        let (names, errors) = (self.model.names.len(), self.diagnostics.len());
        self.call_depth += 1;
        let body = self.body(&function, values, scope);
        self.call_depth -= 1;
        self.model.names.truncate(names);
        let calls = self.model.calls.len();
        for diagnostic in &mut self.diagnostics[errors..] {
            diagnostic.message = format!("In {}: {}", function.name, diagnostic.message);
            diagnostic.range = range;
        }
        let body = body?;
        let result = self
            .type_of(&body, scope)
            .map(|ty| types::googlesql_name(&ty))
            .unwrap_or_default();
        self.model.calls.truncate(calls);
        self.model.calls.push(CallForm {
            range,
            form: format!("{}({}) -> {result}", function.name, types.join(", ")),
        });
        Some(body)
    }

    /// The rows of a call of a table function in `FROM`.
    pub(super) fn table_function(&mut self, source: &ast::TvfSource) -> Rows {
        let range = source.syntax().text_range();
        let failed = Rows::failed(Vec::new(), Vec::new());
        let Some(path) = source.path() else {
            return failed;
        };
        let Some(names) = path.names() else {
            return failed;
        };
        let function = self.temp_function(&names).filter(|f| f.kind == Kind::Table);
        if function.is_none()
            && matches!(names.as_slice(), [name] if name.eq_ignore_ascii_case("impute"))
        {
            return self.impute_source(source, path.syntax().text_range());
        }
        let Some(function) = function else {
            self.error(
                path.syntax().text_range(),
                format!("Table-valued function not found: {}", names.join(".")),
            );
            return failed;
        };
        let arguments: Vec<ast::Argument> = source
            .arg_list()
            .iter()
            .flat_map(|list| list.arguments())
            .collect();
        self.call_table_function(function, &path, None, arguments, source.alias(), range)
    }

    /// The rows of a call of the table function `function`, named by `path`, with `arguments`,
    /// and in a pipe `CALL`, with `input`, the pipe input, as its first argument, which binds its
    /// first parameter, a table.
    pub(super) fn call_table_function(
        &mut self,
        function: TempFunction,
        path: &ast::Path,
        input: Option<Rows>,
        arguments: Vec<ast::Argument>,
        alias: Option<ast::Alias>,
        range: TextRange,
    ) -> Rows {
        let failed = Rows::failed(Vec::new(), Vec::new());
        self.use_name(
            path.syntax().text_range(),
            Definition::Function {
                name: function.name.clone(),
            },
        );
        // The arguments: values, which read only the script's variables, and tables.
        enum Value {
            Expr(Expr),
            Table(Rows),
        }
        let mut args: Vec<(Option<String>, Value, TextRange)> = Vec::new();
        // The pipe input binds the first table parameter, in its place among the positional
        // arguments.
        let input = match input {
            Some(input) => {
                let Some(index) =
                    (function.params.iter()).position(|p| matches!(p.ty, ParamTy::Table))
                else {
                    self.error(
                        path.syntax().text_range(),
                        "Table-valued function cannot be called with pipe CALL syntax because its signature does not include a table-typed argument",
                    );
                    return failed;
                };
                Some((index, input))
            }
            None => None,
        };
        for argument in arguments {
            let at = argument.syntax().text_range();
            let (name, value) = match &argument {
                ast::Argument::Arg(arg) => (None, arg.expr().map(Ok)),
                ast::Argument::NamedArg(arg) => {
                    (arg.name_ref().and_then(|n| n.text()), arg.expr().map(Ok))
                }
                ast::Argument::TableArg(arg) => (None, arg.path().map(Err)),
                ast::Argument::NamedTableArg(arg) => {
                    (arg.name_ref().and_then(|n| n.text()), arg.path().map(Err))
                }
            };
            let value = match value {
                Some(Ok(expr)) => {
                    let one = self.one_row();
                    match self.expr(&expr, Scope::of(&one, "a table function's argument")) {
                        Some(expr) => Value::Expr(expr),
                        None => return failed,
                    }
                }
                Some(Err(table)) => {
                    let Some(table_names) = table.names() else {
                        return failed;
                    };
                    let rows = self.table_rows(&table_names, table.syntax().text_range());
                    if rows.plan.is_none() {
                        return failed;
                    }
                    Value::Table(rows)
                }
                None => return failed,
            };
            args.push((name, value, at));
        }
        if let Some((index, input)) = input {
            let positional = args.iter().take_while(|(name, ..)| name.is_none()).count();
            args.insert(index.min(positional), (None, Value::Table(input), range));
        }
        // Bind by index, since `Value` doesn't clone cheaply.
        let indexed: Vec<(Option<String>, usize, TextRange)> = (args.iter().enumerate())
            .map(|(i, (name, _, at))| (name.clone(), i, *at))
            .collect();
        let Some(bound) = self.bind(&function, indexed, range) else {
            return failed;
        };
        let mut values: Vec<Option<Value>> =
            args.into_iter().map(|(_, value, _)| Some(value)).collect();
        let mut bindings = Vec::new();
        let mut tables = Vec::new();
        for ((index, at), param) in bound.into_iter().zip(&function.params) {
            match (values[index].take().expect("a bound argument"), &param.ty) {
                (Value::Table(rows), ParamTy::Table) => tables.push(WithName {
                    name: param.name.clone(),
                    definition: at,
                    rows,
                }),
                (Value::Expr(expr), ParamTy::Value(want)) => {
                    let one = self.one_row();
                    let scope = Scope::of(&one, "a table function's argument");
                    match self.value_of_type(expr, &want.arrow(self.type_options), at, scope) {
                        Some(value) => bindings.push((param.name.clone(), value)),
                        None => return failed,
                    }
                }
                (Value::Expr(expr), ParamTy::Any) => bindings.push((param.name.clone(), expr)),
                _ => {
                    self.error(
                        at,
                        format!(
                            "The argument {} of {} is of the wrong kind",
                            param.name, function.name
                        ),
                    );
                    return failed;
                }
            }
        }
        let query = match (function.body.query(), function.body.expr()) {
            (Some(query), _) => Some(query),
            (None, Some(ast::Expr::SubqueryExpr(subquery))) => {
                subquery.paren_query().and_then(|p| p.query())
            }
            _ => None,
        };
        let Some(query) = query else {
            self.error(range, format!("The body of {} is no query", function.name));
            return failed;
        };
        let saved_params = std::mem::replace(&mut self.params, bindings);
        let saved_outers = std::mem::take(&mut self.outers);
        let saved_tables = self.temp_tables.len();
        self.temp_tables.extend(tables);
        let pushed = function
            .module
            .clone()
            .map(|m| self.module_context.push(m))
            .is_some();
        let (names_len, operators, errors) = (
            self.model.names.len(),
            self.model.operators.len(),
            self.diagnostics.len(),
        );
        let rows = self.query(&query);
        self.model.names.truncate(names_len);
        self.model.operators.truncate(operators);
        for diagnostic in &mut self.diagnostics[errors..] {
            diagnostic.message = format!("In {}: {}", function.name, diagnostic.message);
            diagnostic.range = range;
        }
        if pushed {
            self.module_context.pop();
        }
        self.temp_tables.truncate(saved_tables);
        self.outers = saved_outers;
        self.params = saved_params;
        let alias = alias
            .and_then(|alias| alias.name())
            .and_then(|name| Some((name.text()?, name.syntax().text_range())));
        let alias = alias.or_else(|| Some((function.name.clone(), path.syntax().text_range())));
        self.close(rows, alias, false, range)
    }

    /// The rows of the table at `names` as a source: a pipe `WITH` name, a temporary table, or a
    /// table of the catalog.
    pub(super) fn table_rows(&mut self, names: &[String], range: TextRange) -> Rows {
        if let Some(table) = self.temp_table(names, range) {
            let alias = Some((names.last().cloned().unwrap_or_default(), range));
            return self.close(table.rows, alias, false, range);
        }
        match self.catalog_table(names, range) {
            Some(rows) => rows,
            None => {
                self.error(range, format!("Table not found: {}", names.join(".")));
                Rows::failed(Vec::new(), Vec::new())
            }
        }
    }

    /// `IMPORT MODULE`: the module's functions and constants, under the import's alias, within the
    /// module `parent` for an import of a module.
    pub(super) fn import(&mut self, import: &ast::ImportStmt, parent: Option<&InModule>) {
        let Some(path) = import.path() else {
            return;
        };
        let Some(names) = path.names() else {
            return;
        };
        let path_range = path.syntax().text_range();
        let Some(text) = self.catalog.module(&names) else {
            self.error(path_range, format!("Module not found: {}", names.join(".")));
            return;
        };
        let alias = (import.alias().and_then(|a| a.name()).and_then(|n| n.text()))
            .unwrap_or_else(|| names.last().cloned().unwrap_or_default())
            .to_ascii_lowercase();
        let key = match parent {
            Some(parent) => format!("{}.{alias}", parent.path),
            None => alias,
        };
        let parse = avenger_sql_syntax::parse_module(&text);
        let at = parent.map_or(path_range, |parent| parent.import);
        if let Some(diagnostic) = parse.diagnostics().first() {
            self.error(
                at,
                format!("In module {}: {}", names.join("."), diagnostic.message),
            );
            return;
        }
        let Some(module) = parse.module() else {
            return;
        };
        let context = InModule {
            path: key.clone(),
            import: at,
        };
        let (names_len, operators, calls, errors) = (
            self.model.names.len(),
            self.model.operators.len(),
            self.model.calls.len(),
            self.diagnostics.len(),
        );
        self.module_context.push(key.clone());
        for stmt in module.stmts() {
            match stmt {
                ast::Stmt::ImportStmt(inner) => self.import(&inner, Some(&context)),
                ast::Stmt::CreateFunctionStmt(create) => {
                    self.create_function(&create, Some(&context))
                }
                ast::Stmt::CreateConstantStmt(constant) => {
                    let (Some(name), Some(expr)) = (
                        constant.name_path().and_then(|path| path.names()),
                        constant.expr(),
                    ) else {
                        continue;
                    };
                    let one = self.one_row();
                    let Some(value) = self.expr(&expr, Scope::of(&one, "a constant")) else {
                        continue;
                    };
                    self.constants.push(Constant {
                        path: format!("{key}.{}", name.join(".").to_ascii_lowercase()),
                        expr: value,
                        public: constant.scope_kind() == Some(CreateScope::Public),
                    });
                }
                other => self.error(
                    other.syntax().text_range(),
                    "A module holds functions, constants and imports",
                ),
            }
        }
        self.module_context.pop();
        self.model.names.truncate(names_len);
        self.model.operators.truncate(operators);
        self.model.calls.truncate(calls);
        let module_name = names.join(".");
        for diagnostic in &mut self.diagnostics[errors..] {
            if !diagnostic.message.starts_with("In module ") {
                diagnostic.message = format!("In module {module_name}: {}", diagnostic.message);
            }
            diagnostic.range = at;
        }
        self.use_name(path_range, Definition::Module { names });
    }
}
