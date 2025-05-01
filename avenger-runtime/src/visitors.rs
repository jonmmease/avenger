use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

use avenger_lang2::{ast::{ComponentProp, DatasetProp, ExprProp, PropBinding, ValProp}, parser::AvengerParser, visitor::{AvengerVisitor, VisitorContext}};
use sqlparser::ast::{Expr as SqlExpr, Ident, ObjectName, Query as SqlQuery, Visit, Visitor, VisitorMut};

use crate::{component_registry::{ComponentRegistry, ExprPropRegistration, PropRegistration, ValPropRegistration}, context::TaskEvaluationContext, dependency::{Dependency, DependencyKind}, error::AvengerRuntimeError, scope::Scope, tasks::{DatasetPropTask, ExprPropTask, GroupMarkTask, MarkTask, Task, ValPropTask}, variable::Variable};


pub struct CollectDependenciesVisitor {
    /// The variables that are dependencies of the task, without leading @
    deps: Vec<Dependency>,
}

impl CollectDependenciesVisitor {
    pub fn new() -> Self {
        Self { deps: vec![] }
    }
}

impl Visitor for CollectDependenciesVisitor {
    type Break = Result<(), AvengerRuntimeError>;

    /// Replace tables of the form @table_name with the true mangled table name
    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();

        // Handle dataset references
        if table_name.starts_with("@") {
            // Drop leading @ and split on .
            let parts = table_name[1..].split(".").map(|s| s.to_string()).collect::<Vec<_>>();

            self.deps.push(Dependency::new(
                parts, DependencyKind::Dataset)
            );
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &SqlExpr) -> ControlFlow<Self::Break> {
        match &expr {
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    self.deps.push(Dependency::new(
                        vec![ident.value[1..].to_string()], DependencyKind::ValOrExpr)
                    );
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mut parts: Vec<String> = idents.iter().map(|ident| ident.value.clone()).collect();
                    // Drop the leading @
                    parts[0] = parts[0][1..].to_string();
                    self.deps.push(Dependency::new(
                        parts, DependencyKind::ValOrExpr)
                    );
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

pub fn collect_expr_dependencies(expr: &SqlExpr) -> Result<Vec<Dependency>, AvengerRuntimeError> {
    let mut visitor = CollectDependenciesVisitor::new();
    if let ControlFlow::Break(Result::Err(err)) = expr.visit(&mut visitor) {
        return Err(err);
    }
    Ok(visitor.deps)
}

pub fn collect_query_dependencies(query: &SqlQuery) -> Result<Vec<Dependency>, AvengerRuntimeError> {
    let mut visitor = CollectDependenciesVisitor::new();
    if let ControlFlow::Break(Result::Err(err)) = query.visit(&mut visitor) {
        return Err(err);
    }
    Ok(visitor.deps)
}



pub struct CompilationVisitor<'a> {
    ctx: &'a TaskEvaluationContext,
}

impl<'a> CompilationVisitor<'a> {
    pub fn new(ctx: &'a TaskEvaluationContext) -> Self {
        Self { ctx }
    }
}

impl<'a> VisitorMut for CompilationVisitor<'a> {
    type Break = Result<(), AvengerRuntimeError>;

    fn pre_visit_relation(&mut self, relation: &mut datafusion_sql::sqlparser::ast::ObjectName) -> ControlFlow<Self::Break> {
        let table_name = relation.to_string();
    
        if table_name.starts_with("@") {
            let mut parts = relation.0.iter().map(|ident| ident.value.clone()).collect::<Vec<_>>();

            // Join on __ into a single string
            parts = vec![parts.join("__")];

            // Update the relation to use the mangled name
            let idents = parts.iter().map(|s| Ident::new(s.to_string())).collect::<Vec<_>>();

            *relation = ObjectName(idents);
        }

        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut SqlExpr) -> ControlFlow<Self::Break> {
        match expr.clone() {
            SqlExpr::Identifier(ident) => {
                if ident.value.starts_with("@") {
                    let variable = Variable::new(vec![ident.value[1..].to_string()]);

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.ctx.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value
                    if !self.ctx.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerRuntimeError::ExpressionNotFound(
                            format!("Val or Expr {} not found", variable.name())))
                        );
                    }
                }
            }
            SqlExpr::CompoundIdentifier(idents) => {
                if !idents.is_empty() && idents[0].value.starts_with("@") {
                    let mut parts = idents.iter().map(|ident| ident.value.clone()).collect::<Vec<_>>();
                    parts[0] = parts[0][1..].to_string();
                    let variable = Variable::new(parts);

                    // Check if this is a reference to an expression
                    if let Ok(registered_expr) = self.ctx.get_expr(&variable) {
                        *expr = SqlExpr::Nested(Box::new(registered_expr.clone()));
                        return ControlFlow::Continue(());
                    }

                    // Otherwise it must be a reference to a value
                    if !self.ctx.has_val(&variable) {
                        return ControlFlow::Break(Err(AvengerRuntimeError::ExpressionNotFound(
                            format!("Val or Expr {} not found", variable.name())))
                        );
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}


pub struct TaskBuilderVisitor<'a> {
    registry: &'a ComponentRegistry,
    scope: &'a Scope,
    tasks: HashMap<Variable, Arc<dyn Task>>
}

impl<'a> TaskBuilderVisitor<'a> {
    pub fn new(registry: &'a ComponentRegistry, scope: &'a Scope) -> Self {
        Self { registry, scope, tasks: HashMap::new() }
    }

    pub fn make_variable(&self, name: &str, context: &VisitorContext) -> Variable {
        let mut path = context.path.clone();
        path.push(name.to_string());
        Variable::new(path)
    }

    pub fn build(self) -> HashMap<Variable, Arc<dyn Task>> {
        self.tasks
    }
}

impl<'a> Visitor for TaskBuilderVisitor<'a> {
    type Break = Result<(), AvengerRuntimeError>;
}

impl<'a> AvengerVisitor for TaskBuilderVisitor<'a> {
    fn pre_visit_val_prop(&mut self, statement: &ValProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        let variable = self.make_variable(statement.name(), context);
        
        let mut expr = statement.expr.clone();
        if let Err(err) = self.scope.resolve_sql_expr(&mut expr, &context.path) {
            return ControlFlow::Break(Err(err));
        }
        let task = ValPropTask::new(expr);
        self.tasks.insert(variable, Arc::new(task));
        ControlFlow::Continue(())
    }

    fn pre_visit_expr_prop(&mut self, statement: &ExprProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        let variable = self.make_variable(statement.name(), context);
        let mut expr = statement.expr.clone();
        if let Err(err) = self.scope.resolve_sql_expr(&mut expr, &context.path) {
            return ControlFlow::Break(Err(err));
        }
        let task = ExprPropTask::new(expr);
        self.tasks.insert(variable, Arc::new(task));
        ControlFlow::Continue(())
    }

    fn pre_visit_dataset_prop(&mut self, statement: &DatasetProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        let variable = self.make_variable(statement.name(), context);
        let mut query = statement.query.clone();
        if let Err(err) = self.scope.resolve_sql_query(&mut query, &context.path) {
            return ControlFlow::Break(Err(err));
        }
        let task = DatasetPropTask::new(query, false);
        self.tasks.insert(variable, Arc::new(task));
        ControlFlow::Continue(())
    }

    fn pre_visit_prop_binding(&mut self, prop_binding: &PropBinding, context: &VisitorContext) -> ControlFlow<Self::Break> {
        let variable = self.make_variable(&prop_binding.name.value, context);

        let Some(component_spec) = self.registry.lookup_component(&context.component_type) else {
            return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                "Unknown component type: {}", context.component_type))));
        };

        let Some(prop_type) = component_spec.props.get(&prop_binding.name.value) else {
            return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                "Unknown property {} for component {}", prop_binding.name, context.component_type))));
        };

        match prop_type {
            PropRegistration::Val(_) => {
                let Ok(mut sql_expr) = prop_binding.expr.clone().into_expr() else {
                    return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                        "Expression for property {} must be a value", prop_binding.name))));
                };
                if let Err(err) = self.scope.resolve_sql_expr(&mut sql_expr, &context.path) {
                    return ControlFlow::Break(Err(err));
                }
                self.tasks.insert(variable, Arc::new(ValPropTask::new(sql_expr)));
            }
            PropRegistration::Expr(_) => {
                let Ok(mut sql_expr) = prop_binding.expr.clone().into_expr() else {
                    return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                        "Expression for property {} must be a value or expression", prop_binding.name))));
                };
                if let Err(err) = self.scope.resolve_sql_expr(&mut sql_expr, &context.path) {
                    return ControlFlow::Break(Err(err));
                }
                self.tasks.insert(variable, Arc::new(ExprPropTask::new(sql_expr)));
            },
            PropRegistration::Dataset(_) => {
                let Ok(mut query) = prop_binding.expr.clone().into_query() else {
                    return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                        "Expression for property {} must be a query", prop_binding.name))));
                };
                if let Err(err) = self.scope.resolve_sql_query(&mut query, &context.path) {
                    return ControlFlow::Break(Err(err));
                }
                self.tasks.insert(variable, Arc::new(DatasetPropTask::new(query, false)));
            },
        }
        ControlFlow::Continue(())
    }

    fn post_visit_component_prop(&mut self, statement: &ComponentProp, context: &VisitorContext) -> ControlFlow<Self::Break> {
        // Get the component type
        let component_type = statement.component_type.value.clone();

        // Lookup the component type in the registry
        let Some(component_spec) = self.registry.lookup_component(&component_type) else {
            return ControlFlow::Break(Err(AvengerRuntimeError::ComponentNotFound(component_type)));
        };

        // let statement_val_props = statement.val_props();
        let statement_bindings = statement.prop_bindings();
        let mut val_csv_parts = Vec::new();
        for (name, prop) in component_spec.props.iter() {
            if let PropRegistration::Val(_) = prop {
                if statement_bindings.contains_key(name) {
                    // We have a binding for this property, use its value
                    val_csv_parts.push(format!("@{name} as {name}"));
                } else if let PropRegistration::Val(ValPropRegistration { default, .. }) = prop {
                    if let Some(default) = default {
                        val_csv_parts.push(format!("{} as {name}", default.to_string()));
                    } else {
                        // TODO: What should we do here?
                        // val_csv_parts.push(format!("NULL as {name}"));
                    }
                }
            }
        }

        let val_props_csv = if val_csv_parts.is_empty() {
            "1 as _unit".to_string()
        } else {
            val_csv_parts.join(", ")
        };

        let Ok(mut query) = AvengerParser::parse_single_query(
            &format!("SELECT {val_props_csv}")
        ) else {
            return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                "Failed to parse config query for component {}", component_type))));
        };

        let mut child_path = context.path.clone();
        child_path.push(statement.name());
        if let Err(err) = self.scope.resolve_sql_query(&mut query, &child_path) {
            return ControlFlow::Break(Err(err));
        }

        let task = DatasetPropTask { query, eval: true };
        let mut parts = child_path.clone();
        parts.push("config".to_string());
        let config_variable = Variable::new(parts);
        self.tasks.insert(config_variable.clone(), Arc::new(task));

        // Handle mark-specific tasks
        if let Some(mark_type) = self.registry.lookup_mark_type(
            &statement.component_type.value
        ) {
            let mut expr_csv_parts = Vec::new();
            for (name, prop) in component_spec.props.iter() {
                if let PropRegistration::Expr(_) = prop {
                    if statement_bindings.contains_key(name) {
                        // We have a binding for this property, use its value
                        expr_csv_parts.push(format!("@{name} as {name}"));
                    } else if let PropRegistration::Expr(ExprPropRegistration { default, .. }) = prop {
                        if let Some(default) = default {
                            expr_csv_parts.push(format!("{} as {name}", default.to_string()));
                        } else {
                            // TODO: What should we do here?
                            // expr_csv_parts.push(format!("NULL as {name}"));
                        }
                    }
                }
            }

            let expr_props_csv = if expr_csv_parts.is_empty() {
                "1 as _unit".to_string()
            } else {
                expr_csv_parts.join(", ")
            };

            let mut query_res = if statement_bindings.contains_key("data") {
                AvengerParser::parse_single_query(
                    &format!("SELECT {expr_props_csv} FROM @data")
                )
            } else {
                AvengerParser::parse_single_query(
                    &format!("SELECT {expr_props_csv}")
                )
            };
            let mut query = if let Ok(query) = query_res {
                query
            } else {
                return ControlFlow::Break(Err(AvengerRuntimeError::InternalError(format!(
                    "Failed to parse config query for component {}", component_type))));
            };

            if let Err(err) = self.scope.resolve_sql_query(&mut query, &child_path) {
                return ControlFlow::Break(Err(err));
            }

            // Create a task for the encoded data
            // Mark task expects eval: true
            let task = DatasetPropTask { query, eval: true };
            let mut parts = child_path.clone();
            parts.push("encoded_data".to_string());
            let encoded_data_variable = Variable::new(parts);
            self.tasks.insert(encoded_data_variable.clone(), Arc::new(task));

            // Create a task to build the mark
            let mut parts = child_path.clone();
            parts.push("_mark".to_string());
            let mark_variable = Variable::new(parts);
            let task = MarkTask::new(encoded_data_variable, config_variable, mark_type);
            self.tasks.insert(mark_variable, Arc::new(task));
        } else {
            // Treat all non-marks as groups for now.
            // Later we'll have components that aren't groups or marks.
            let mark_vars = statement.component_props().into_iter().filter_map(|(name, prop)| {
                let mut parts = child_path.clone();
                parts.push(name.clone());
                parts.push("_mark".to_string());
                Some(Variable::new(parts))
            }).collect::<Vec<_>>();

            // Build group mark variable
            let mut parts = child_path.clone();
            parts.push("_mark".to_string());
            let group_mark_variable = Variable::new(parts);

            // Build task
            let task = GroupMarkTask::new(config_variable, mark_vars);
            self.tasks.insert(group_mark_variable, Arc::new(task));
        }
        ControlFlow::Continue(())
    }
}