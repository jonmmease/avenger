use std::collections::HashMap;

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
    pub fingerprint: String,
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
        
        // Then, add edges based on task dependencies
        for (variable, task) in &tasks {
            let target_idx = node_indices[variable];
            
            // Get input dependencies for this task
            let input_variables = match task.input_variables() {
                Ok(vars) => vars,
                Err(_) => vec![], // Handle error appropriately
            };
            
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
        for idx in sorted_indices {
            let variable = graph[idx].clone();
            
            // Take ownership of the task from the HashMap
            let task = tasks.remove(&variable).expect("Task should exist");
            
            // Build inputs (incoming edges)
            let inputs = graph
                .neighbors_directed(idx, Direction::Incoming)
                .map(|neighbor_idx| {
                    let source = graph[neighbor_idx].clone();
                    IncomingEdge {
                        source,
                        output_variable: None, // This would need more context to determine
                    }
                })
                .collect();
            
            // Build outputs (outgoing edges)
            let outputs = graph
                .neighbors_directed(idx, Direction::Outgoing)
                .map(|neighbor_idx| {
                    let target = graph[neighbor_idx].clone();
                    OutgoingEdge { target }
                })
                .collect();
            
            // Create fingerprint (this would need a proper implementation)
            let fingerprint = format!("{:?}", variable); // Placeholder
            
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
}
