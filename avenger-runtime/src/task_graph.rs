use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::sync::Arc;

use indexmap::IndexMap;
use petgraph::algo::toposort;
use petgraph::graph::DiGraph;
use petgraph::Direction;

use crate::error::AvengerRuntimeError;
use crate::tasks::Task;
use crate::variable::Variable;

// use super::compiler::ExtractComponentDefinitionsVisitor;
// use super::component::ComponentSpec;
// use super::component_registry::{ComponentRegistry, PropRegistration};
// use super::scope::{Scope, ScopePath};
// use super::tasks::{DatasetDeclTask, ExprDeclTask, GroupMarkTask, MarkTask, RectMarkTask, ValDeclTask};
// use super::variable::Variable;


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

    // Here:
    // pub fn from_file(file: &AvengerFile, component_registry: Arc<ComponentRegistry>) -> Result<Self, AvengerRuntimeError> {
    //     let mut builder = TaskGraphBuilder::new(
    //         Scope::from_file(&file)?, component_registry
    //     );
    //     file.accept(&mut builder)?;
    //     builder.build()
    // }
}