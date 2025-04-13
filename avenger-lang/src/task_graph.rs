use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

use indexmap::IndexMap;
use petgraph::algo::toposort;
use petgraph::graph::DiGraph;
use petgraph::stable_graph::NodeIndex;
use petgraph::Direction;
use async_trait::async_trait;

use crate::{tasks::Task, value::{TaskValue, Variable, VariableKind}, error::AvengerLangError};


pub struct IncomingEdge {
    pub source: Variable,
    pub output_variable: Option<Variable>,
}

pub struct OutgoingEdge {
    pub target: Variable,
}

pub struct TaskNode {
    pub variable: Variable,
    pub task: Box<dyn Task>,
    pub inputs: Vec<IncomingEdge>,
    pub outputs: Vec<OutgoingEdge>,
    pub fingerprint: u64,
}


pub struct TaskGraph {
    tasks: IndexMap<Variable, TaskNode>,
}

impl TaskGraph {
    pub fn try_new(mut tasks: HashMap<Variable, Box<dyn Task>>) -> Result<Self, AvengerLangError> {
        // Build a directed graph for topological sorting
        let mut graph = DiGraph::<Variable, ()>::new();
        let mut node_indices = HashMap::new();
        
        // First, add all nodes to the graph
        for variable in tasks.keys() {
            let idx = graph.add_node(variable.clone());
            node_indices.insert(variable.clone(), idx);
        }
        
        // Track which task produces each output variable
        let mut output_var_producers: HashMap<Variable, Variable> = HashMap::new();
        
        // Collect output variables from each task
        for (variable, task) in &tasks {
            let output_variables = task.output_variables()?;

            // Record that this task produces these output variables
            for output_var in output_variables {
                output_var_producers.insert(output_var.clone(), variable.clone());
            }
        }
        
        // Then, add edges based on task dependencies
        for (variable, task) in &tasks {
            let target_idx = node_indices[variable];
            
            // Get input dependencies for this task
            let input_variables = task.input_variables()?;
            
            // Add edges from each input dependency to this task
            for input_var in input_variables {
                // If the input variable is a direct output of another task
                if let Some(source_idx) = node_indices.get(&input_var) {
                    // Add edge from input to the current task
                    graph.add_edge(*source_idx, target_idx, ());
                } 
                // If the input variable is an output variable of another task
                else if let Some(producer_var) = output_var_producers.get(&input_var) {
                    if let Some(source_idx) = node_indices.get(producer_var) {
                        // Add edge from the producing task to the current task
                        graph.add_edge(*source_idx, target_idx, ());
                    }
                }
            }
        }
        
        // Perform topological sort
        let sorted_indices = match toposort(&graph, None) {
            Ok(indices) => indices,
            Err(cycle) => {
                // Handle cycles here if needed
                return Err(AvengerLangError::DependencyCycle(
                    format!("Dependency cycle detected in task graph: {:?}", cycle)
                ));
            }
        };
        
        // Build task nodes and collect them in topological order
        let mut sorted_tasks = IndexMap::new();
        
        // Create a cache to store the output variables for each task
        let mut task_outputs: HashMap<Variable, Vec<Variable>> = HashMap::new();
        for (variable, task) in &tasks {
            let outputs = task.output_variables()?;
            task_outputs.insert(variable.clone(), outputs);
        }
        
        // Create a map to store the fingerprints of tasks as they are computed
        let mut fingerprints: HashMap<Variable, u64> = HashMap::new();
        
        for idx in sorted_indices {
            let variable = graph[idx].clone();
            
            // Take ownership of the task from the HashMap
            let task = tasks.remove(&variable).expect("Task should exist");
            
            // Get input variables for this task
            let input_vars = task.input_variables()?;
            
            // Build inputs (incoming edges)
            let mut inputs: Vec<IncomingEdge> = Vec::new();
            
            // Keep track of which input variables we've already processed to avoid duplicates
            let mut processed_input_vars = std::collections::HashSet::new();
            
            // Collect parent variables for fingerprinting
            let mut parent_variables = Vec::new();
            
            // Get all incoming neighbors in the graph
            for neighbor_idx in graph.neighbors_directed(idx, Direction::Incoming) {
                let source = graph[neighbor_idx].clone();
                parent_variables.push(source.clone());
                let source_outputs = task_outputs.get(&source).cloned().unwrap_or_default();
                
                // Check if this source directly provides any input variables
                let mut found_relevant_output = false;
                
                // Create a separate edge for each relevant output
                for input_var in &input_vars {
                    // Only create an edge if we haven't processed this input variable yet
                    if source_outputs.contains(input_var) && processed_input_vars.insert(input_var.clone()) {
                        // This is an output variable from the source task
                        inputs.push(IncomingEdge {
                            source: source.clone(),
                            output_variable: Some(input_var.clone()),
                        });
                        found_relevant_output = true;
                    }
                }
                
                // If this is a direct dependency (not via an output variable)
                // add an edge without an output variable
                if !found_relevant_output && input_vars.contains(&source) {
                    inputs.push(IncomingEdge {
                        source: source.clone(),
                        output_variable: None,
                    });
                }
            }
            
            // Build outputs (outgoing edges)
            let outputs = graph
                .neighbors_directed(idx, Direction::Outgoing)
                .map(|neighbor_idx| {
                    let target = graph[neighbor_idx].clone();
                    OutgoingEdge { target }
                })
                .collect();
            
            // Calculate content hash for this task
            let mut hasher = DefaultHasher::new();
            variable.hash(&mut hasher);  // Hash the variable name
            
            // Since Task trait doesn't implement Debug, we'll use other task properties
            // Hash the task's input variables
            for input_var in &input_vars {
                input_var.hash(&mut hasher);
            }
            
            // Hash the task's output variables
            if let Ok(output_vars) = task.output_variables() {
                for output_var in &output_vars {
                    output_var.hash(&mut hasher);
                }
            }
            
            // Get the content hash
            let content_hash = hasher.finish();
            
            // Calculate final fingerprint by combining with parent fingerprints
            let mut final_hasher = DefaultHasher::new();
            content_hash.hash(&mut final_hasher);
            
            // Add parent fingerprints to the hash if there are any
            if !parent_variables.is_empty() {
                for parent in &parent_variables {
                    if let Some(parent_fingerprint) = fingerprints.get(parent) {
                        parent_fingerprint.hash(&mut final_hasher);
                    }
                }
            }
            
            let fingerprint = final_hasher.finish();
            
            // Store the fingerprint for potential child nodes to use
            fingerprints.insert(variable.clone(), fingerprint);
            
            // Create the task node
            let task_node = TaskNode {
                variable: variable.clone(),
                task,
                inputs,
                outputs,
                fingerprint,
            };
            
            sorted_tasks.insert(variable, task_node);
        }
        
        Ok(TaskGraph { tasks: sorted_tasks })
    }    

    pub fn tasks(&self) -> &IndexMap<Variable, TaskNode> {
        &self.tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use datafusion_common::ScalarValue;

    // Helper function to create a Variable for testing
    fn create_var(name: &str) -> Variable {
        Variable::new(name.to_string(), VariableKind::ValOrExpr)
    }

    // Mock implementation of Task for testing
    #[derive(Debug)]
    struct MockTask {
        name: String,
        input_vars: Vec<Variable>,
        output_vars: Vec<Variable>,
    }

    impl MockTask {
        fn new(name: &str, input_vars: Vec<Variable>) -> Self {
            Self {
                name: name.to_string(),
                input_vars,
                output_vars: vec![],
            }
        }

        fn with_outputs(name: &str, input_vars: Vec<Variable>, output_vars: Vec<Variable>) -> Self {
            Self {
                name: name.to_string(),
                input_vars,
                output_vars,
            }
        }
    }

    #[async_trait]
    impl Task for MockTask {
        fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
            Ok(self.input_vars.clone())
        }

        fn output_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
            Ok(self.output_vars.clone())
        }

        async fn evaluate(
            &self,
            _input_values: &[TaskValue],
        ) -> Result<(TaskValue, Vec<TaskValue>), AvengerLangError> {
            // For testing, just return a dummy value
            Ok((TaskValue::Val(ScalarValue::Int32(Some(42))), vec![]))
        }
    }

    #[test]
    fn test_topological_sort_no_dependencies() -> Result<(), AvengerLangError> {
        // Create tasks with no dependencies
        let mut tasks = HashMap::new();
        tasks.insert(create_var("task1"), Box::new(MockTask::new("Task 1", vec![])) as Box<dyn Task>);
        tasks.insert(create_var("task2"), Box::new(MockTask::new("Task 2", vec![])) as Box<dyn Task>);
        tasks.insert(create_var("task3"), Box::new(MockTask::new("Task 3", vec![])) as Box<dyn Task>);

        let graph = TaskGraph::try_new(tasks)?;
        
        // Since there are no dependencies, all tasks should be in the graph
        assert_eq!(graph.tasks().len(), 3);
        Ok(())
    }

    #[test]
    fn test_topological_sort_with_dependencies() -> Result<(), AvengerLangError> {
        // Create tasks with dependencies
        // task3 depends on task2, which depends on task1
        let mut tasks = HashMap::new();
        let task1_var = create_var("task1");
        let task2_var = create_var("task2");
        let task3_var = create_var("task3");
        
        tasks.insert(task1_var.clone(), Box::new(MockTask::new("Task 1", vec![])) as Box<dyn Task>);
        tasks.insert(task2_var.clone(), Box::new(MockTask::new("Task 2", vec![task1_var.clone()])) as Box<dyn Task>);
        tasks.insert(task3_var.clone(), Box::new(MockTask::new("Task 3", vec![task2_var.clone()])) as Box<dyn Task>);

        let graph = TaskGraph::try_new(tasks)?;
        
        // Check that the order is correct
        let keys: Vec<_> = graph.tasks().keys().collect();
        
        // task1 should come before task2, and task2 should come before task3
        let task1_idx = keys.iter().position(|&k| k == &task1_var).unwrap();
        let task2_idx = keys.iter().position(|&k| k == &task2_var).unwrap();
        let task3_idx = keys.iter().position(|&k| k == &task3_var).unwrap();
        
        assert!(task1_idx < task2_idx);
        assert!(task2_idx < task3_idx);
        Ok(())
    }

    #[test]
    fn test_cycle_detection() -> Result<(), AvengerLangError>{
        // Create tasks with a dependency cycle
        // task1 -> task2 -> task3 -> task1
        let mut tasks = HashMap::new();
        let task1_var = create_var("task1");
        let task2_var = create_var("task2");
        let task3_var = create_var("task3");
        
        tasks.insert(task1_var.clone(), Box::new(MockTask::new("Task 1", vec![task3_var.clone()])) as Box<dyn Task>);
        tasks.insert(task2_var.clone(), Box::new(MockTask::new("Task 2", vec![task1_var.clone()])) as Box<dyn Task>);
        tasks.insert(task3_var.clone(), Box::new(MockTask::new("Task 3", vec![task2_var.clone()])) as Box<dyn Task>);

        // This should panic due to the cycle
        if let Err(e) = TaskGraph::try_new(tasks) {
            assert!(e.to_string().contains("Dependency cycle detected in task graph"));
        } else {
            panic!("Expected a dependency cycle error");
        }
        Ok(())
    }

    #[test]
    fn test_complex_dependency_graph() -> Result<(), AvengerLangError> {
        // Create a more complex dependency graph:
        //     A
        //    / \
        //   B   C
        //  / \ /
        // D   E
        let mut tasks = HashMap::new();
        let a_var = create_var("A");
        let b_var = create_var("B");
        let c_var = create_var("C");
        let d_var = create_var("D");
        let e_var = create_var("E");
        
        tasks.insert(a_var.clone(), Box::new(MockTask::new("A", vec![])) as Box<dyn Task>);
        tasks.insert(b_var.clone(), Box::new(MockTask::new("B", vec![a_var.clone()])) as Box<dyn Task>);
        tasks.insert(c_var.clone(), Box::new(MockTask::new("C", vec![a_var.clone()])) as Box<dyn Task>);
        tasks.insert(d_var.clone(), Box::new(MockTask::new("D", vec![b_var.clone()])) as Box<dyn Task>);
        tasks.insert(e_var.clone(), Box::new(MockTask::new("E", vec![b_var.clone(), c_var.clone()])) as Box<dyn Task>);

        let graph = TaskGraph::try_new(tasks)?;
        
        let keys: Vec<_> = graph.tasks().keys().collect();
        
        // A should come before B and C
        // B should come before D and E
        // C should come before E
        let a_idx = keys.iter().position(|&k| k == &a_var).unwrap();
        let b_idx = keys.iter().position(|&k| k == &b_var).unwrap();
        let c_idx = keys.iter().position(|&k| k == &c_var).unwrap();
        let d_idx = keys.iter().position(|&k| k == &d_var).unwrap();
        let e_idx = keys.iter().position(|&k| k == &e_var).unwrap();
        
        assert!(a_idx < b_idx);
        assert!(a_idx < c_idx);
        assert!(b_idx < d_idx);
        assert!(b_idx < e_idx);
        assert!(c_idx < e_idx);
        Ok(())
    }

    #[test]
    fn test_task_with_output_variables() -> Result<(), AvengerLangError> {
        // Create a graph where one task produces output variables:
        //
        //   A (produces B, C)
        //    \
        //     D (depends on B, C)
        //
        // Task A produces variables B and C
        // Task D depends on variables B and C
        
        let mut tasks = HashMap::new();
        let a_var = create_var("A");
        let b_var = create_var("B");
        let c_var = create_var("C");
        let d_var = create_var("D");
        
        // Task A produces outputs B and C
        tasks.insert(
            a_var.clone(), 
            Box::new(MockTask::with_outputs("A", vec![], vec![b_var.clone(), c_var.clone()])) as Box<dyn Task>
        );
        
        // Task D depends on outputs B and C from Task A
        tasks.insert(
            d_var.clone(), 
            Box::new(MockTask::new("D", vec![b_var.clone(), c_var.clone()])) as Box<dyn Task>
        );

        let graph = TaskGraph::try_new(tasks)?;
        
        // Check that D has A as its dependency
        let d_node = graph.tasks().get(&d_var).unwrap();
        
        // Print more detailed information to debug the test
        println!("D inputs: {}", d_node.inputs.len());
        for (i, input) in d_node.inputs.iter().enumerate() {
            println!("Input {}: source = {:?}, output_variable = {:?}", i, input.source, input.output_variable);
        }
        
        // We should now have two separate input edges from A, one for B and one for C
        assert_eq!(d_node.inputs.len(), 2, "Expected D to have 2 input edges from A, one for each output variable");
        
        // Check that both inputs are from A
        assert_eq!(d_node.inputs[0].source, a_var);
        assert_eq!(d_node.inputs[1].source, a_var);
        
        // Check that both B and C are represented in the output variables
        let has_b = d_node.inputs.iter().any(|edge| 
            edge.output_variable.as_ref().map_or(false, |v| v == &b_var)
        );
        let has_c = d_node.inputs.iter().any(|edge| 
            edge.output_variable.as_ref().map_or(false, |v| v == &c_var)
        );
        
        assert!(has_b, "Expected an edge with B as output variable");
        assert!(has_c, "Expected an edge with C as output variable");
        
        Ok(())
    }

    #[test]
    fn test_fingerprint() -> Result<(), AvengerLangError> {
        // Test 1: Identical tasks should have same fingerprint
        let mut tasks1 = HashMap::new();
        let mut tasks2 = HashMap::new();
        
        let a1_var = create_var("A1");
        let a2_var = create_var("A2");
        
        // Create two identical tasks with different names
        tasks1.insert(a1_var.clone(), Box::new(MockTask::new("Task A1", vec![])) as Box<dyn Task>);
        tasks2.insert(a2_var.clone(), Box::new(MockTask::new("Task A2", vec![])) as Box<dyn Task>);
        
        let graph1 = TaskGraph::try_new(tasks1)?;
        let graph2 = TaskGraph::try_new(tasks2)?;
        
        // Fingerprints should be different as variables are different
        let a1_fingerprint = graph1.tasks().get(&a1_var).unwrap().fingerprint;
        let a2_fingerprint = graph2.tasks().get(&a2_var).unwrap().fingerprint;
        assert_ne!(a1_fingerprint, a2_fingerprint, "Tasks with different variables should have different fingerprints");
        
        // Test 2: Changing a task's inputs should change its fingerprint
        let mut tasks3 = HashMap::new();
        let mut tasks4 = HashMap::new();
        
        let b_var = create_var("B");
        let c1_var = create_var("C1");
        let c2_var = create_var("C2");
        
        // Create base task B
        tasks3.insert(b_var.clone(), Box::new(MockTask::new("Task B", vec![])) as Box<dyn Task>);
        // Create task C1 that depends on B
        tasks3.insert(c1_var.clone(), Box::new(MockTask::new("Task C1", vec![b_var.clone()])) as Box<dyn Task>);
        
        // Create the same tasks for the second graph
        tasks4.insert(b_var.clone(), Box::new(MockTask::new("Task B", vec![])) as Box<dyn Task>);
        // But C2 has no dependencies
        tasks4.insert(c2_var.clone(), Box::new(MockTask::new("Task C2", vec![])) as Box<dyn Task>);
        
        let graph3 = TaskGraph::try_new(tasks3)?;
        let graph4 = TaskGraph::try_new(tasks4)?;
        
        // Fingerprints should be different as C1 has a dependency but C2 doesn't
        let c1_fingerprint = graph3.tasks().get(&c1_var).unwrap().fingerprint;
        let c2_fingerprint = graph4.tasks().get(&c2_var).unwrap().fingerprint;
        assert_ne!(c1_fingerprint, c2_fingerprint, "Tasks with different dependencies should have different fingerprints");
        
        // Test 3: Dependency chain - changes should cascade down
        let mut tasks5 = HashMap::new();
        
        let d_var = create_var("D");
        let e_var = create_var("E");
        let f1_var = create_var("F1");
        
        // Create tasks with a dependency chain: D -> E -> F1
        tasks5.insert(d_var.clone(), Box::new(MockTask::new("Task D", vec![])) as Box<dyn Task>);
        tasks5.insert(e_var.clone(), Box::new(MockTask::new("Task E", vec![d_var.clone()])) as Box<dyn Task>);
        tasks5.insert(f1_var.clone(), Box::new(MockTask::new("Task F1", vec![e_var.clone()])) as Box<dyn Task>);
        
        let graph5 = TaskGraph::try_new(tasks5)?;
        
        let mut tasks6 = HashMap::new();
        let f2_var = create_var("F2");
        
        // Create similar chain but with different output for the first task
        tasks6.insert(
            d_var.clone(), 
            Box::new(MockTask::with_outputs("Task D", vec![], vec![create_var("output")])) as Box<dyn Task>
        );
        tasks6.insert(e_var.clone(), Box::new(MockTask::new("Task E", vec![d_var.clone()])) as Box<dyn Task>);
        tasks6.insert(f2_var.clone(), Box::new(MockTask::new("Task F2", vec![e_var.clone()])) as Box<dyn Task>);
        
        let graph6 = TaskGraph::try_new(tasks6)?;
        
        // First task should have different fingerprints due to different outputs
        let d1_fingerprint = graph5.tasks().get(&d_var).unwrap().fingerprint;
        let d2_fingerprint = graph6.tasks().get(&d_var).unwrap().fingerprint;
        assert_ne!(d1_fingerprint, d2_fingerprint, "Tasks with different outputs should have different fingerprints");
        
        // Second task should also be different as its parent changed
        let e1_fingerprint = graph5.tasks().get(&e_var).unwrap().fingerprint;
        let e2_fingerprint = graph6.tasks().get(&e_var).unwrap().fingerprint;
        assert_ne!(e1_fingerprint, e2_fingerprint, "Tasks with different parent fingerprints should have different fingerprints");
        
        // Third task should also be different as its ancestor changed
        let f1_fingerprint = graph5.tasks().get(&f1_var).unwrap().fingerprint;
        let f2_fingerprint = graph6.tasks().get(&f2_var).unwrap().fingerprint;
        assert_ne!(f1_fingerprint, f2_fingerprint, "Tasks with different ancestor fingerprints should have different fingerprints");
        
        Ok(())
    }
}
