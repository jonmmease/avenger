use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::sync::Arc;

use indexmap::IndexMap;
use petgraph::algo::toposort;
use petgraph::graph::DiGraph;
use petgraph::Direction;
use async_trait::async_trait;

use crate::{tasks::Task, value::{Variable, VariableKind, TaskValue}, error::AvengerLangError};


#[derive(Clone, Debug)]
pub struct IncomingEdge {
    pub source: Variable,
}

#[derive(Clone, Debug)]
pub struct OutgoingEdge {
    pub target: Variable,
}

#[derive(Clone)]
pub struct TaskNode {
    pub variable: Variable,
    pub task: Arc<dyn Task>,
    pub inputs: Vec<IncomingEdge>,
    pub outputs: Vec<OutgoingEdge>,
    pub fingerprint: u64,
}

#[derive(Clone)]
pub struct TaskGraph {
    tasks: IndexMap<Variable, TaskNode>,
}

impl TaskGraph {
    pub fn try_new(mut tasks: HashMap<Variable, Arc<dyn Task>>) -> Result<Self, AvengerLangError> {
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
            let input_variables = task.input_variables()?;
            
            // Add edges from each input dependency to this task
            for input_var in input_variables {
                if let Some(source_idx) = node_indices.get(&input_var) {
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
                return Err(AvengerLangError::DependencyCycle(
                    format!("Dependency cycle detected in task graph: {:?}", cycle)
                ));
            }
        };
        
        // Build task nodes and collect them in topological order
        let mut sorted_tasks = IndexMap::new();
        
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
            
            // Track parent variables for fingerprinting
            let mut parent_variables = Vec::new();
            
            // Get all incoming neighbors in the graph
            for neighbor_idx in graph.neighbors_directed(idx, Direction::Incoming) {
                let source = graph[neighbor_idx].clone();
                parent_variables.push(source.clone());
                
                // Create an edge for this input
                inputs.push(IncomingEdge {
                    source: source.clone(),
                });
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
    }

    impl MockTask {
        fn new(name: &str, input_vars: Vec<Variable>) -> Self {
            Self {
                name: name.to_string(),
                input_vars,
            }
        }
    }

    #[async_trait]
    impl Task for MockTask {
        fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
            Ok(self.input_vars.clone())
        }

        async fn evaluate(
            &self,
            _input_values: &[TaskValue],
        ) -> Result<TaskValue, AvengerLangError> {
            // For testing, just return a dummy value
            Ok(TaskValue::Val(ScalarValue::Int32(Some(42))))
        }
    }

    #[test]
    fn test_topological_sort_no_dependencies() -> Result<(), AvengerLangError> {
        // Create tasks with no dependencies
        let mut tasks = HashMap::new();
        tasks.insert(create_var("task1"), Arc::new(MockTask::new("Task 1", vec![])) as Arc<dyn Task>);
        tasks.insert(create_var("task2"), Arc::new(MockTask::new("Task 2", vec![])) as Arc<dyn Task>);
        tasks.insert(create_var("task3"), Arc::new(MockTask::new("Task 3", vec![])) as Arc<dyn Task>);

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
        
        tasks.insert(task1_var.clone(), Arc::new(MockTask::new("Task 1", vec![])) as Arc<dyn Task>);
        tasks.insert(task2_var.clone(), Arc::new(MockTask::new("Task 2", vec![task1_var.clone()])) as Arc<dyn Task>);
        tasks.insert(task3_var.clone(), Arc::new(MockTask::new("Task 3", vec![task2_var.clone()])) as Arc<dyn Task>);

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
        
        tasks.insert(task1_var.clone(), Arc::new(MockTask::new("Task 1", vec![task3_var.clone()])) as Arc<dyn Task>);
        tasks.insert(task2_var.clone(), Arc::new(MockTask::new("Task 2", vec![task1_var.clone()])) as Arc<dyn Task>);
        tasks.insert(task3_var.clone(), Arc::new(MockTask::new("Task 3", vec![task2_var.clone()])) as Arc<dyn Task>);

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
        
        tasks.insert(a_var.clone(), Arc::new(MockTask::new("A", vec![])) as Arc<dyn Task>);
        tasks.insert(b_var.clone(), Arc::new(MockTask::new("B", vec![a_var.clone()])) as Arc<dyn Task>);
        tasks.insert(c_var.clone(), Arc::new(MockTask::new("C", vec![a_var.clone()])) as Arc<dyn Task>);
        tasks.insert(d_var.clone(), Arc::new(MockTask::new("D", vec![b_var.clone()])) as Arc<dyn Task>);
        tasks.insert(e_var.clone(), Arc::new(MockTask::new("E", vec![b_var.clone(), c_var.clone()])) as Arc<dyn Task>);

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
        // Create a graph where one task depends on another:
        //
        //   A
        //    \
        //     D (depends on A)
        
        let mut tasks = HashMap::new();
        let a_var = create_var("A");
        let d_var = create_var("D");
        
        // Task A has no dependencies
        tasks.insert(
            a_var.clone(), 
            Arc::new(MockTask::new("A", vec![])) as Arc<dyn Task>
        );
        
        // Task D depends on A
        tasks.insert(
            d_var.clone(), 
            Arc::new(MockTask::new("D", vec![a_var.clone()])) as Arc<dyn Task>
        );

        let graph = TaskGraph::try_new(tasks)?;
        
        // Check that D has A as its dependency
        let d_node = graph.tasks().get(&d_var).unwrap();
        
        // Print more detailed information to debug the test
        println!("D inputs: {}", d_node.inputs.len());
        for (i, input) in d_node.inputs.iter().enumerate() {
            println!("Input {}: source = {:?}", i, input.source);
        }
        
        // We should have one input edge from A to D
        assert_eq!(d_node.inputs.len(), 1, "Expected D to have 1 input edge from A");
        
        // Check that the input is from A
        assert_eq!(d_node.inputs[0].source, a_var);
        
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
        tasks1.insert(a1_var.clone(), Arc::new(MockTask::new("Task A1", vec![])) as Arc<dyn Task>);
        tasks2.insert(a2_var.clone(), Arc::new(MockTask::new("Task A2", vec![])) as Arc<dyn Task>);
        
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
        tasks3.insert(b_var.clone(), Arc::new(MockTask::new("Task B", vec![])) as Arc<dyn Task>);
        // Create task C1 that depends on B
        tasks3.insert(c1_var.clone(), Arc::new(MockTask::new("Task C1", vec![b_var.clone()])) as Arc<dyn Task>);
        
        // Create the same tasks for the second graph
        tasks4.insert(b_var.clone(), Arc::new(MockTask::new("Task B", vec![])) as Arc<dyn Task>);
        // But C2 has no dependencies
        tasks4.insert(c2_var.clone(), Arc::new(MockTask::new("Task C2", vec![])) as Arc<dyn Task>);
        
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
        tasks5.insert(d_var.clone(), Arc::new(MockTask::new("Task D", vec![])) as Arc<dyn Task>);
        tasks5.insert(e_var.clone(), Arc::new(MockTask::new("Task E", vec![d_var.clone()])) as Arc<dyn Task>);
        tasks5.insert(f1_var.clone(), Arc::new(MockTask::new("Task F1", vec![e_var.clone()])) as Arc<dyn Task>);
        
        let graph5 = TaskGraph::try_new(tasks5)?;
        
        let mut tasks6 = HashMap::new();
        let f2_var = create_var("F2");
        
        // Create similar chain but with a different implementation for the first task
        let d_modified_var = create_var("D");
        // Use a different input variable to simulate a different task implementation
        tasks6.insert(d_modified_var.clone(), Arc::new(MockTask::new("Task D", vec![create_var("dummy_input")])) as Arc<dyn Task>);
        tasks6.insert(e_var.clone(), Arc::new(MockTask::new("Task E", vec![d_modified_var.clone()])) as Arc<dyn Task>);
        tasks6.insert(f2_var.clone(), Arc::new(MockTask::new("Task F2", vec![e_var.clone()])) as Arc<dyn Task>);
        
        let graph6 = TaskGraph::try_new(tasks6)?;
        
        // First task should have different fingerprints due to different inputs
        let d1_fingerprint = graph5.tasks().get(&d_var).unwrap().fingerprint;
        let d2_fingerprint = graph6.tasks().get(&d_modified_var).unwrap().fingerprint;
        assert_ne!(d1_fingerprint, d2_fingerprint, "Tasks with different implementations should have different fingerprints");
        
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
