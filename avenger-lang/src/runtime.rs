use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use futures::future::{join_all, BoxFuture};
use async_recursion::async_recursion;
use crate::cache::TaskCache;
use crate::error::AvengerLangError;
use crate::task_graph::{TaskGraph};
use crate::value::{TaskValue, Variable};
use crate::tasks::Task;

pub struct TaskGraphRuntime {
    cache: Arc<TaskCache>,
}

impl TaskGraphRuntime {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(TaskCache::new()),
        }
    }
    
    pub fn with_cache(cache: Arc<TaskCache>) -> Self {
        Self { cache }
    }

    /// Evaluate the given variables in the task graph
    pub async fn evaluate_variables(
        &self,
        graph: Arc<TaskGraph>,
        variables: &[Variable],
    ) -> Result<HashMap<Variable, TaskValue>, AvengerLangError> {
        let mut results = HashMap::new();
        
        // Process all requested variables
        let arc_self = Arc::new(self.clone());
        for var in variables {
            let result = arc_self.clone().evaluate_variable(graph.clone(), var.clone()).await?;
            results.insert(var.clone(), result);
        }
        
        Ok(results)
    }

    /// Evaluate a single variable and its dependencies
    #[async_recursion]
    async fn evaluate_variable(
        self: Arc<Self>,
        graph: Arc<TaskGraph>,
        variable: Variable,
    ) -> Result<TaskValue, AvengerLangError> {
        // Lookup the node for this variable
        let node = graph.tasks().get(&variable).ok_or_else(|| {
            AvengerLangError::VariableNotFound(format!("Variable not found: {:?}", variable))
        })?;
        
        // Check if the value is already cached
        if let Some(cached_value) = self.cache.get(node.fingerprint).await {
            return Ok(cached_value);
        }
        
        // We need to evaluate this node - first evaluate its dependencies
        let mut dependency_futures = vec![];
        
        for edge in &node.inputs {
            let dep_var = edge.source.clone();
            let graph_clone = graph.clone();
            let runtime_clone = self.clone();
            dependency_futures.push(tokio::spawn( async move {
                runtime_clone.evaluate_variable(graph_clone, dep_var).await
            }));
        }
        
        // Wait for all dependencies to complete
        let dependency_results = join_all(dependency_futures).await;
        let mut input_values = vec![];
        
        for result in dependency_results {
            match result {
                Ok(res) => input_values.push(res?),
                Err(e) => return Err(AvengerLangError::TokioJoinError(e)),
            }
        }
        
        // Now evaluate this task
        let value = node.task.evaluate(&input_values).await?;
        
        // Cache the result
        self.cache.insert(node.fingerprint, value.clone()).await;
        
        Ok(value)
    }
}

impl Clone for TaskGraphRuntime {
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::VariableKind;
    use async_trait::async_trait;
    use datafusion_common::ScalarValue;
    use std::sync::atomic::{AtomicU32, Ordering};
    
    // Re-implement MockTask for testing since it's only in the test module of tasks.rs
    #[derive(Debug, Clone)]
    struct MockTask {
        name: String,
        input_vars: Vec<Variable>,
        return_value: TaskValue,
    }

    impl MockTask {
        fn new(name: &str, input_vars: Vec<Variable>, return_value: TaskValue) -> Self {
            Self {
                name: name.to_string(),
                input_vars,
                return_value,
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
            Ok(self.return_value.clone())
        }
    }
    
    // A task that counts how many times it's been evaluated
    #[derive(Debug, Clone)]
    struct CountingTask {
        var: Variable,
        dependencies: Vec<Variable>,
        counter: Arc<AtomicU32>,
        return_value: TaskValue,
    }
    
    impl CountingTask {
        fn new(
            name: &str, 
            dependencies: Vec<Variable>, 
            counter: Arc<AtomicU32>, 
            return_value: TaskValue
        ) -> Self {
            Self {
                var: Variable::new(name.to_string(), VariableKind::ValOrExpr),
                dependencies,
                counter,
                return_value,
            }
        }
    }
    
    #[async_trait]
    impl Task for CountingTask {
        fn input_variables(&self) -> Result<Vec<Variable>, AvengerLangError> {
            Ok(self.dependencies.clone())
        }
        
        async fn evaluate(
            &self,
            _input_values: &[TaskValue],
        ) -> Result<TaskValue, AvengerLangError> {
            // Increment the counter each time evaluate is called
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(self.return_value.clone())
        }
    }

    #[tokio::test]
    async fn test_evaluate_single_variable() -> Result<(), AvengerLangError> {
        // Create a simple task graph with a single task
        let mut tasks = HashMap::new();
        let var = Variable::new("test".to_string(), VariableKind::ValOrExpr);
        
        let task = MockTask::new(
            "test_task", 
            vec![], 
            TaskValue::Val(ScalarValue::Int32(Some(42)))
        );
        
        tasks.insert(var.clone(), Arc::new(task) as Arc<dyn Task>);
        
        let graph = Arc::new(TaskGraph::try_new(tasks)?);
        let runtime = TaskGraphRuntime::new();
        
        // Evaluate the variable
        let results = runtime.evaluate_variables(graph, &[var.clone()]).await?;
        
        // Check the result
        assert_eq!(results.len(), 1);
        
        if let TaskValue::Val(ScalarValue::Int32(Some(value))) = &results[&var] {
            assert_eq!(*value, 42);
        } else {
            panic!("Expected Int32(42)");
        }
        
        Ok(())
    }

    #[tokio::test]
    async fn test_evaluate_with_dependencies() -> Result<(), AvengerLangError> {
        // Create a task graph with dependencies
        let mut tasks = HashMap::new();
        
        let var_a = Variable::new("A".to_string(), VariableKind::ValOrExpr);
        let var_b = Variable::new("B".to_string(), VariableKind::ValOrExpr);
        let var_c = Variable::new("C".to_string(), VariableKind::ValOrExpr);
        
        // Task A has no dependencies
        let task_a = MockTask::new(
            "task_a", 
            vec![], 
            TaskValue::Val(ScalarValue::Int32(Some(1)))
        );
        
        // Task B depends on A
        let task_b = MockTask::new(
            "task_b", 
            vec![var_a.clone()], 
            TaskValue::Val(ScalarValue::Int32(Some(2)))
        );
        
        // Task C depends on B
        let task_c = MockTask::new(
            "task_c", 
            vec![var_b.clone()], 
            TaskValue::Val(ScalarValue::Int32(Some(3)))
        );
        
        tasks.insert(var_a.clone(), Arc::new(task_a) as Arc<dyn Task>);
        tasks.insert(var_b.clone(), Arc::new(task_b) as Arc<dyn Task>);
        tasks.insert(var_c.clone(), Arc::new(task_c) as Arc<dyn Task>);
        
        let graph = Arc::new(TaskGraph::try_new(tasks)?);
        let runtime = TaskGraphRuntime::new();
        
        // Evaluate variable C (which requires evaluating A and B first)
        let results = runtime.evaluate_variables(graph, &[var_c.clone()]).await?;
        
        // Check the result
        assert_eq!(results.len(), 1);
        
        if let TaskValue::Val(ScalarValue::Int32(Some(value))) = &results[&var_c] {
            assert_eq!(*value, 3);
        } else {
            panic!("Expected Int32(3)");
        }
        
        Ok(())
    }

    #[tokio::test]
    async fn test_basic_caching() -> Result<(), AvengerLangError> {
        // Create a task that we can track invocations
        let mut tasks = HashMap::new();
        let var = Variable::new("test".to_string(), VariableKind::ValOrExpr);
        let counter = Arc::new(AtomicU32::new(0));
        
        let task = CountingTask::new(
            "test",
            vec![],
            counter.clone(),
            TaskValue::Val(ScalarValue::Int32(Some(99)))
        );
        
        tasks.insert(var.clone(), Arc::new(task) as Arc<dyn Task>);
        
        let graph = Arc::new(TaskGraph::try_new(tasks)?);
        let runtime = TaskGraphRuntime::new();
        
        // Evaluate the variable twice
        let _results1 = runtime.evaluate_variables(graph.clone(), &[var.clone()]).await?;
        let _results2 = runtime.evaluate_variables(graph.clone(), &[var.clone()]).await?;
        
        // The task should only be evaluated once due to caching
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_cached_dependencies() -> Result<(), AvengerLangError> {
        // Test that dependencies are properly cached
        let mut tasks = HashMap::new();
        
        let var_a = Variable::new("A".to_string(), VariableKind::ValOrExpr);
        let var_b = Variable::new("B".to_string(), VariableKind::ValOrExpr);
        let var_c = Variable::new("C".to_string(), VariableKind::ValOrExpr);
        
        let counter_a = Arc::new(AtomicU32::new(0));
        let counter_b = Arc::new(AtomicU32::new(0));
        let counter_c = Arc::new(AtomicU32::new(0));
        
        // Task A has no dependencies
        let task_a = CountingTask::new(
            "A",
            vec![],
            counter_a.clone(),
            TaskValue::Val(ScalarValue::Int32(Some(1)))
        );
        
        // Task B depends on A
        let task_b = CountingTask::new(
            "B",
            vec![var_a.clone()],
            counter_b.clone(),
            TaskValue::Val(ScalarValue::Int32(Some(2)))
        );
        
        // Task C depends on B
        let task_c = CountingTask::new(
            "C",
            vec![var_b.clone()],
            counter_c.clone(),
            TaskValue::Val(ScalarValue::Int32(Some(3)))
        );
        
        tasks.insert(var_a.clone(), Arc::new(task_a) as Arc<dyn Task>);
        tasks.insert(var_b.clone(), Arc::new(task_b) as Arc<dyn Task>);
        tasks.insert(var_c.clone(), Arc::new(task_c) as Arc<dyn Task>);
        
        let graph = Arc::new(TaskGraph::try_new(tasks)?);
        let runtime = TaskGraphRuntime::new();
        
        // First evaluate C, which should evaluate A and B as well
        let _results1 = runtime.evaluate_variables(graph.clone(), &[var_c.clone()]).await?;
        
        // Each task should be evaluated once
        assert_eq!(counter_a.load(Ordering::SeqCst), 1);
        assert_eq!(counter_b.load(Ordering::SeqCst), 1);
        assert_eq!(counter_c.load(Ordering::SeqCst), 1);
        
        // Now evaluate B, which should use cached values for A
        let _results2 = runtime.evaluate_variables(graph.clone(), &[var_b.clone()]).await?;
        
        // A and B should still only be evaluated once
        assert_eq!(counter_a.load(Ordering::SeqCst), 1);
        assert_eq!(counter_b.load(Ordering::SeqCst), 1);
        
        // Finally, evaluate all variables
        let _results3 = runtime.evaluate_variables(graph.clone(), &[var_a.clone(), var_b.clone(), var_c.clone()]).await?;
        
        // All tasks should still only be evaluated once
        assert_eq!(counter_a.load(Ordering::SeqCst), 1);
        assert_eq!(counter_b.load(Ordering::SeqCst), 1);
        assert_eq!(counter_c.load(Ordering::SeqCst), 1);
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_shared_cache() -> Result<(), AvengerLangError> {
        // Test that multiple runtimes can share a cache
        let mut tasks = HashMap::new();
        let var = Variable::new("test".to_string(), VariableKind::ValOrExpr);
        let counter = Arc::new(AtomicU32::new(0));
        
        let task = CountingTask::new(
            "test",
            vec![],
            counter.clone(),
            TaskValue::Val(ScalarValue::Int32(Some(99)))
        );
        
        tasks.insert(var.clone(), Arc::new(task) as Arc<dyn Task>);
        
        let graph = Arc::new(TaskGraph::try_new(tasks)?);
        
        // Create a shared cache
        let shared_cache = Arc::new(TaskCache::new());
        
        // Create two runtimes with the shared cache
        let runtime1 = TaskGraphRuntime::with_cache(shared_cache.clone());
        let runtime2 = TaskGraphRuntime::with_cache(shared_cache);
        
        // Evaluate with the first runtime
        let _results1 = runtime1.evaluate_variables(graph.clone(), &[var.clone()]).await?;
        
        // The task should be evaluated once
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        
        // Evaluate with the second runtime
        let _results2 = runtime2.evaluate_variables(graph.clone(), &[var.clone()]).await?;
        
        // The task should still only be evaluated once, due to shared cache
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_fingerprint_caching() -> Result<(), AvengerLangError> {
        // Test that tasks with the same fingerprint share cache entries
        // Create two identical tasks (different variables but same implementation/dependencies)
        let mut tasks1 = HashMap::new();
        let mut tasks2 = HashMap::new();
        
        let var1 = Variable::new("test1".to_string(), VariableKind::ValOrExpr);
        let var2 = Variable::new("test2".to_string(), VariableKind::ValOrExpr);
        
        let counter1 = Arc::new(AtomicU32::new(0));
        let counter2 = Arc::new(AtomicU32::new(0));
        
        let task1 = CountingTask::new(
            "test1",
            vec![],
            counter1.clone(),
            TaskValue::Val(ScalarValue::Int32(Some(99)))
        );
        
        let task2 = CountingTask::new(
            "test2",
            vec![],
            counter2.clone(),
            TaskValue::Val(ScalarValue::Int32(Some(99)))
        );
        
        tasks1.insert(var1.clone(), Arc::new(task1) as Arc<dyn Task>);
        tasks2.insert(var2.clone(), Arc::new(task2) as Arc<dyn Task>);
        
        let graph1 = Arc::new(TaskGraph::try_new(tasks1)?);
        let graph2 = Arc::new(TaskGraph::try_new(tasks2)?);
        
        // Get fingerprints
        let fingerprint1 = graph1.tasks().get(&var1).unwrap().fingerprint;
        let fingerprint2 = graph2.tasks().get(&var2).unwrap().fingerprint;
        
        // Test different fingerprints (variables are different)
        assert_ne!(fingerprint1, fingerprint2);
        
        // Create a runtime
        let runtime = TaskGraphRuntime::new();
        
        // Evaluate both graphs
        let _results1 = runtime.evaluate_variables(graph1.clone(), &[var1.clone()]).await?;
        let _results2 = runtime.evaluate_variables(graph2.clone(), &[var2.clone()]).await?;
        
        // Both tasks should be evaluated once
        assert_eq!(counter1.load(Ordering::SeqCst), 1);
        assert_eq!(counter2.load(Ordering::SeqCst), 1);
        
        Ok(())
    }
}
