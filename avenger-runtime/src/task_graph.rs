use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::ops::ControlFlow;
use std::sync::Arc;

use avenger_lang2::ast::{AvengerFile, ComponentProp, DatasetProp, ExprProp, PropBinding, ValProp};
use avenger_lang2::parser::AvengerParser;
use avenger_lang2::visitor::{AvengerVisitor, VisitorContext};
use indexmap::IndexMap;
use petgraph::algo::toposort;
use petgraph::graph::DiGraph;
use petgraph::Direction;
use sqlparser::ast::Visitor;

use crate::component_registry::{ComponentRegistry, ExprPropRegistration, PropRegistration, ValPropRegistration};
use crate::error::AvengerRuntimeError;
use crate::scope::PropertyScope;
use crate::tasks::{DatasetPropTask, ExprPropTask, GroupMarkTask, MarkTask, Task, ValPropTask};
use crate::variable::Variable;


#[derive(Clone, Debug)]
pub struct IncomingEdge {
    pub source: Variable,
}

#[derive(Clone, Debug)]
pub struct OutgoingEdge {
    pub target: Variable,
}

#[derive(Clone, Debug)]
pub struct TaskNode {
    pub variable: Variable,
    pub task: Arc<dyn Task>,
    pub inputs: Vec<IncomingEdge>,
    pub outputs: Vec<OutgoingEdge>,

    /// The fingerprint of the task. This is the fingerprint of the task and all
    /// its parents. If fingerprints match, the task value is guaranteed to be the same.
    pub fingerprint: u64,

    /// The identity fingerprint of the task. This is the fingerprint of the task
    /// without the hash of the root variables. This is so the identity fingerprint
    /// is stable across different values of root variables.
    pub identity_fingerprint: u64,
}

#[derive(Clone, Debug)]
pub struct TaskGraph {
    tasks: IndexMap<Variable, TaskNode>,
}

impl TaskGraph {
    pub fn try_new(mut tasks: HashMap<Variable, Arc<dyn Task>>) -> Result<Self, AvengerRuntimeError> {
        // Build a directed graph for topological sorting
        let mut graph = DiGraph::<Variable, ()>::new();
        let mut node_indices = HashMap::new();
        
        // First, add all nodes to the graph
        for variable in tasks.keys() {
            let idx = graph.add_node(variable.clone());
            node_indices.insert(variable.clone(), idx);
        }
        
        // Add edges based on task dependencies
        for (variable, task) in &tasks {
            let target_idx = node_indices[variable];
            
            // Get input dependencies for this task
            let input_variables = task.input_dependencies()?;
            
            // Add edges from each input dependency to this task
            for input_var in input_variables {
                if let Some(source_idx) = node_indices.get(&input_var.variable) {
                    // Add edge from input to the current task
                    graph.add_edge(*source_idx, target_idx, ());
                }
            }
        }
        
        // Perform topological sort
        let sorted_indices = match toposort(&graph, None) {
            Ok(indices) => indices,
            Err(cycle) => {
                // Handle cycles here if needed
                return Err(AvengerRuntimeError::DependencyCycle(
                    format!("Dependency cycle detected in task graph: {:?}", cycle)
                ));
            }
        };
        
        // Build task nodes and collect them in topological order
        let mut sorted_tasks: IndexMap<Variable, TaskNode> = IndexMap::new();
        
        // Create a map to store the fingerprints of tasks as they are computed
        let mut fingerprints: HashMap<Variable, u64> = HashMap::new();
        let mut identity_fingerprints: HashMap<Variable, u64> = HashMap::new();
        
        for idx in sorted_indices {
            let node_var = graph[idx].clone();
            
            // Take ownership of the task from the HashMap
            let task = tasks.remove(&node_var)
                .ok_or_else(|| AvengerRuntimeError::InternalError(format!("Task should exist for variable {:?}", node_var)))?;
            
            // Get input variables for this task
            let input_deps = task.input_dependencies()?;
            let parent_variables: Vec<Variable> = input_deps.iter().map(
                |dep| dep.variable.clone()
            ).collect();
            let inputs: Vec<IncomingEdge> = input_deps.iter().map(
                |dep| IncomingEdge { source: dep.variable.clone() }
            ).collect();
                        
            // Build outputs (outgoing edges)
            let outputs = graph
                .neighbors_directed(idx, Direction::Outgoing)
                .map(|neighbor_idx| {
                    let target = graph[neighbor_idx].clone();
                    OutgoingEdge { target }
                })
                .collect();
            
            // Calculate fingerprint by combining this tasks's fingerprint with 
            // the fingerprints of its parents
            let mut fingerprint_hasher = DefaultHasher::new();
            
            
            // Update state fingerprints
            for parent in &parent_variables {
                if let Some(parent_fingerprint) = fingerprints.get(parent) {
                    parent_fingerprint.hash(&mut fingerprint_hasher);
                }
            }
            task.fingerprint()?.hash(&mut fingerprint_hasher);

            // Update identity fingerprints
            // Identity fingerprints differ in that they don't include the hash of root variables
            // rather than the hash of the task itself. This is so the identity fingerprint
            // is stable across different values of root variables.
            let mut identity_fingerprint_hasher = DefaultHasher::new();

            if parent_variables.is_empty() {
                // There are no parents, so use the task's variable as identity fingerprint
                node_var.hash(&mut identity_fingerprint_hasher);
            } else {
                // There are parents, so use their identity fingerprints
                for parent in &parent_variables {
                    if let Some(parent_identity_fingerprint) = identity_fingerprints.get(parent) {
                        parent_identity_fingerprint.hash(&mut identity_fingerprint_hasher);
                    }
                }
                task.fingerprint()?.hash(&mut identity_fingerprint_hasher);
            }
            
            let fingerprint = fingerprint_hasher.finish();
            let identity_fingerprint = identity_fingerprint_hasher.finish();
            
            // Store the fingerprint for potential child nodes to use
            fingerprints.insert(node_var.clone(), fingerprint);
            identity_fingerprints.insert(node_var.clone(), identity_fingerprint);
            
            // Create the task node
            let task_node = TaskNode {
                variable: node_var.clone(),
                task,
                inputs,
                outputs,
                fingerprint,
                identity_fingerprint,
            };
            
            sorted_tasks.insert(node_var, task_node);
        }
        
        Ok(TaskGraph { tasks: sorted_tasks })
    }    

    pub fn tasks(&self) -> &IndexMap<Variable, TaskNode> {
        &self.tasks
    }

    pub fn from_file(file_ast: &AvengerFile) -> Result<Self, AvengerRuntimeError> {
        // Get scope
        let scope = PropertyScope::from_file(file_ast)?;
        let component_registry = ComponentRegistry::new_with_marks();

        let mut builder = TaskBuilderVisitor::new(
            &component_registry, &scope, 
        );
        if let ControlFlow::Break(err) = file_ast.visit(&mut builder) {
            return Err(AvengerRuntimeError::InternalError(format!(
                "Error building task graph: {:?}", err
            )));
        };
        TaskGraph::try_new(builder.build())
    }
}



pub struct TaskBuilderVisitor<'a> {
    registry: &'a ComponentRegistry,
    scope: &'a PropertyScope,
    tasks: HashMap<Variable, Arc<dyn Task>>
}

impl<'a> TaskBuilderVisitor<'a> {
    pub fn new(registry: &'a ComponentRegistry, scope: &'a PropertyScope) -> Self {
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

            let query_res = if statement_bindings.contains_key("data") {
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
            let mark_vars = statement.component_props().into_iter().filter_map(|(name, _prop)| {
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