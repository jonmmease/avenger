use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
use std::time::{Duration, Instant};

use async_recursion::async_recursion;
use futures::future::{join_all, FutureExt};
use tokio::time::sleep;

use crate::context::EvaluationContext;
use crate::error::AvengerLangError;
use crate::task_graph::{
    cache::{RuntimeStats, TaskCache},
    dependency::{Dependency, DependencyKind},
    tasks::Task,
    variable::Variable,
    task_graph::TaskGraph,
    value::TaskValue,
};

use super::value::TaskDataset;

// Threshold for spawning tasks (in milliseconds)
const SPAWN_THRESHOLD_MS: u64 = 50;

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
        
        // Process all requested variables in parallel
        let arc_self = Arc::new(self.clone());
        let mut eval_var_futures = vec![];
        for var in variables {
            let var = var.clone();
            let arc_self = arc_self.clone();
            let graph = graph.clone();
            let eval_var_future = tokio::spawn(async move {
                let result = arc_self.evaluate_variable(graph, &var).await?;
                if let TaskValue::Dataset { 
                    context, 
                    dataset: TaskDataset::LogicalPlan(plan) 
                } = result {
                    // Evaluate the logical plan
                    let ctx = EvaluationContext::new();
                    ctx.register_task_value_context(&context).await?;
                    let table = ctx.eval_plan(plan.clone()).await?;
                    let task_value = TaskValue::Dataset {
                        context: Default::default(),
                        dataset: TaskDataset::ArrowTable(table),
                    };
                    Ok::<_, AvengerLangError>((var.clone(), task_value))
                } else {
                    Ok::<_, AvengerLangError>((var.clone(), result))
                }
            });
            eval_var_futures.push(eval_var_future);
        }

        let eval_var_results = join_all(eval_var_futures).await;
        for res in eval_var_results {
            let (var, result) = res??;
            results.insert(var.clone(), result);
        }

        Ok(results)
    }

    /// Evaluate a single variable and its dependencies
    #[async_recursion]
    async fn evaluate_variable(
        self: Arc<Self>,
        graph: Arc<TaskGraph>,
        variable: &Variable,
    ) -> Result<TaskValue, AvengerLangError> {
        // Start timing this variable's evaluation
        let start_time = Instant::now();
        
        // Lookup the node for this variable
        let node = graph.tasks().get(variable).ok_or_else(|| {
            AvengerLangError::VariableNotFound(format!("Variable not found: {:?}", variable))
        })?;
        
        // Check if the value is already cached
        if let Some(cached_value) = self.cache.get(node.fingerprint).await {
            // Even for cached values, store that this variable was accessed
            if let Some(stats) = self.cache.get_variable_stats(&variable).await {
                self.cache.store_variable_stats(variable.clone(), stats).await;
            }
            return Ok(cached_value);
        }
        
        // We need to evaluate this node - first evaluate its dependencies
        let mut dependency_futures = vec![];
        let mut dependency_spawn_status = HashMap::new();
        
        for edge in &node.inputs {
            let dep_var = edge.source.clone();
            let _dep_node = graph.tasks().get(&dep_var).ok_or_else(|| {
                AvengerLangError::VariableNotFound(format!("Dependency variable not found: {:?}", dep_var))
            })?;
            let graph_clone = graph.clone();
            let runtime_clone = self.clone();
            
            // Check if we've seen this variable before and how long it took
            let previous_stats = runtime_clone.cache.get_variable_stats(&dep_var).await;
            
            // Determine if we should spawn based on runtime history
            let should_spawn = match previous_stats {
                // For variables we've seen before, spawn if they took > SPAWN_THRESHOLD_MS
                Some(stats) => stats.duration.as_millis() > SPAWN_THRESHOLD_MS as u128,
                // For variables we haven't seen before, spawn by default
                None => true,
            };

            // Store the spawn decision for later use
            dependency_spawn_status.insert(dep_var.clone(), should_spawn);
            
            if should_spawn {
                // Spawn a new Tokio task for this dependency
                let spawned = tokio::spawn(async move {
                    runtime_clone.evaluate_variable(graph_clone, &dep_var).await
                });
                
                // Convert JoinHandle<Result<T, E>> to Future<Output=Result<T, E>>
                let mapped = async move {
                    match spawned.await {
                        Ok(result) => result,
                        Err(err) => Err(AvengerLangError::TokioJoinError(err)),
                    }
                }.boxed();
                
                dependency_futures.push(mapped);
            } else {
                // Run directly without spawning
                let future = async move {
                    runtime_clone.evaluate_variable(graph_clone, &dep_var).await
                }.boxed();
                
                dependency_futures.push(future);
            }
        }
        
        // Wait for all dependencies to complete
        let dependency_results = join_all(dependency_futures).await;
        let mut input_values = vec![];
        
        for result in dependency_results {
            input_values.push(result?);
        }
        
        // Now evaluate this task
        let value = node.task.evaluate(&input_values).await?;
        
        // Calculate runtime for this task
        let runtime = start_time.elapsed();
        
        // Cache the result with runtime information
        self.cache.insert(node.fingerprint, value.clone(), runtime).await;
        
        // Store runtime specifically for this variable, along with whether it was spawned
        // Note: The top-level variable is never "spawned" in our definition since it's directly evaluated
        self.cache.store_variable_stats(
            variable.clone(), 
            RuntimeStats { 
                duration: runtime,
                was_spawned: false // Top-level variable is not spawned
            }
        ).await;
        
        // Also store spawn stats for all dependencies that we processed
        for (dep_var, was_spawned) in dependency_spawn_status.into_iter() {
            if let Some(stats) = self.cache.get_variable_stats(&dep_var).await {
                self.cache.store_variable_stats(
                    dep_var, 
                    RuntimeStats { 
                        duration: stats.duration,
                        was_spawned
                    }
                ).await;
            }
        }
        
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
    use crate::task_graph::tasks::Task;
    use crate::task_graph::dependency::DependencyKind;
    use async_trait::async_trait;
    use datafusion_common::ScalarValue;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::time::sleep;
    
    /// Helper function to convert a Variable to a Dependency
    fn variable_to_dependency(var: &Variable, kind: DependencyKind) -> Dependency {
        Dependency { variable: var.clone(), kind }
    }
    
    // Re-implement MockTask for testing since it's only in the test module of tasks.rs
    #[derive(Debug, Clone)]
    struct MockTask {
        name: String,
        input_vars: Vec<Dependency>,
        return_value: TaskValue,
    }

    impl MockTask {
        fn new(name: &str, input_vars: Vec<Dependency>, return_value: TaskValue) -> Self {
            Self {
                name: name.to_string(),
                input_vars,
                return_value,
            }
        }
    }

    #[async_trait]
    impl Task for MockTask {
        fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
            Ok(self.input_vars.clone())
        }

        async fn evaluate(
            &self,
            _input_values: &[TaskValue],
        ) -> Result<TaskValue, AvengerLangError> {
            Ok(self.return_value.clone())
        }

        fn fingerprint(&self) -> Result<u64, AvengerLangError> {
            let mut hasher = DefaultHasher::new();
            self.name.hash(&mut hasher);
            Ok(hasher.finish())
        }
    }
    
    // A task with configurable artificial delay
    #[derive(Debug, Clone)]
    struct DelayedTask {
        name: String,
        dependencies: Vec<Dependency>,
        delay: Duration,
        return_value: TaskValue,
    }
    
    impl DelayedTask {
        fn new(
            name: &str, 
            dependencies: Vec<Dependency>, 
            delay_ms: u64,
            return_value: TaskValue,
        ) -> Self {
            Self {
                name: name.to_string(),
                dependencies,
                delay: Duration::from_millis(delay_ms),
                return_value,
            }
        }
    }
    
    #[async_trait]
    impl Task for DelayedTask {
        fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
            Ok(self.dependencies.clone())
        }
        
        async fn evaluate(
            &self,
            _input_values: &[TaskValue],
        ) -> Result<TaskValue, AvengerLangError> {
            // Simulate work by sleeping
            sleep(self.delay).await;
            Ok(self.return_value.clone())
        }

        fn fingerprint(&self) -> Result<u64, AvengerLangError> {
            let mut hasher = DefaultHasher::new();
            self.name.hash(&mut hasher);
            Ok(hasher.finish())
        }
    }
    
    // A task that counts how many times it's been evaluated
    #[derive(Debug, Clone)]
    struct CountingTask {
        dep: Dependency,
        dependencies: Vec<Dependency>,
        counter: Arc<AtomicU32>,
        return_value: TaskValue,
    }
    
    impl CountingTask {
        fn new(
            name: &str, 
            dependencies: Vec<Dependency>, 
            counter: Arc<AtomicU32>, 
            return_value: TaskValue
        ) -> Self {
            Self {
                dep: Dependency::new(name.to_string(), DependencyKind::ValOrExpr),
                dependencies,
                counter,
                return_value,
            }
        }
    }
    
    #[async_trait]
    impl Task for CountingTask {
        fn input_dependencies(&self) -> Result<Vec<Dependency>, AvengerLangError> {
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

        fn fingerprint(&self) -> Result<u64, AvengerLangError> {
            let mut hasher = DefaultHasher::new();
            self.dep.hash(&mut hasher);
            Ok(hasher.finish())
        }
    }

    #[tokio::test]
    async fn test_evaluate_single_variable() -> Result<(), AvengerLangError> {
        // Create a simple task graph with a single task
        let mut tasks = HashMap::new();
        let var = Variable::new("test");
        
        let task = MockTask::new(
            "test_task", 
            vec![], 
            TaskValue::Val { value: ScalarValue::Int32(Some(42)) }
        );
        
        tasks.insert(var.clone(), Arc::new(task) as Arc<dyn Task>);
        
        let graph = Arc::new(TaskGraph::try_new(tasks)?);
        let runtime = TaskGraphRuntime::new();
        
        // Evaluate the variable
        let results = runtime.evaluate_variables(graph, &[var.clone()]).await?;
        
        // Check the result
        assert_eq!(results.len(), 1);
        
        if let TaskValue::Val { value: ScalarValue::Int32(Some(value)) } = &results[&var] {
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
        
        let var_a = Variable::new("A");
        let dep_a = variable_to_dependency(&var_a, DependencyKind::ValOrExpr);
        let var_b = Variable::new("B");
        let dep_b = variable_to_dependency(&var_b, DependencyKind::ValOrExpr);
        let var_c = Variable::new("C");
        let dep_c = variable_to_dependency(&var_c, DependencyKind::ValOrExpr);
        
        // Task A has no dependencies
        let task_a = MockTask::new(
            "task_a", 
            vec![], 
            TaskValue::Val { value: ScalarValue::Int32(Some(1)) }
        );
        
        // Task B depends on A
        let task_b = MockTask::new(
            "task_b", 
            vec![dep_a], 
            TaskValue::Val { value: ScalarValue::Int32(Some(2)) }
        );
        
        // Task C depends on B
        let task_c = MockTask::new(
            "task_c", 
            vec![dep_b], 
            TaskValue::Val { value: ScalarValue::Int32(Some(3)) }
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
        
        if let TaskValue::Val { value: ScalarValue::Int32(Some(value)) } = &results[&var_c] {
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
        let var = Variable::new("test");
        let counter = Arc::new(AtomicU32::new(0));
        
        let task = CountingTask::new(
            "test",
            vec![],
            counter.clone(),
            TaskValue::Val { value: ScalarValue::Int32(Some(99)) }
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
        
        let var_a = Variable::new("A");
        let dep_a = variable_to_dependency(&var_a, DependencyKind::ValOrExpr);
        let var_b = Variable::new("B");
        let dep_b = variable_to_dependency(&var_b, DependencyKind::ValOrExpr);
        let var_c = Variable::new("C");
        let dep_c = variable_to_dependency(&var_c, DependencyKind::ValOrExpr);
        
        let counter_a = Arc::new(AtomicU32::new(0));
        let counter_b = Arc::new(AtomicU32::new(0));
        let counter_c = Arc::new(AtomicU32::new(0));
        
        // Task A has no dependencies
        let task_a = CountingTask::new(
            "A",
            vec![],
            counter_a.clone(),
            TaskValue::Val { value: ScalarValue::Int32(Some(1)) }
        );
        
        // Task B depends on A
        let task_b = CountingTask::new(
            "B",
            vec![dep_a],
            counter_b.clone(),
            TaskValue::Val { value: ScalarValue::Int32(Some(2)) }
        );
        
        // Task C depends on B
        let task_c = CountingTask::new(
            "C",
            vec![dep_b],
            counter_c.clone(),
            TaskValue::Val { value: ScalarValue::Int32(Some(3)) }
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
        let var = Variable::new("test");
        let counter = Arc::new(AtomicU32::new(0));
        
        let task = CountingTask::new(
            "test",
            vec![],
            counter.clone(),
            TaskValue::Val { value: ScalarValue::Int32(Some(99)) }
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
        
        let var1 = Variable::new("test1");
        let var2 = Variable::new("test2");
        
        let counter1 = Arc::new(AtomicU32::new(0));
        let counter2 = Arc::new(AtomicU32::new(0));
        
        let task1 = CountingTask::new(
            "test1",
            vec![],
            counter1.clone(),
            TaskValue::Val { value: ScalarValue::Int32(Some(99)) }
        );
        
        let task2 = CountingTask::new(
            "test2",
            vec![],
            counter2.clone(),
            TaskValue::Val { value: ScalarValue::Int32(Some(99)) }
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

    #[tokio::test]
    async fn test_task_runtimes() -> Result<(), AvengerLangError> {
        // Create a task graph with tasks of different runtime characteristics
        let mut tasks = HashMap::new();
        
        // Define variables
        let var_a = Variable::new("A");
        let dep_a = variable_to_dependency(&var_a, DependencyKind::ValOrExpr);
        let var_b = Variable::new("B");
        let dep_b = variable_to_dependency(&var_b, DependencyKind::ValOrExpr);
        let var_c = Variable::new("C");
        let dep_c = variable_to_dependency(&var_c, DependencyKind::ValOrExpr);
        let var_d = Variable::new("D");
        let dep_d = variable_to_dependency(&var_d, DependencyKind::ValOrExpr);
        
        // Task A: Fast and independent (25ms, below threshold)
        let task_a = DelayedTask::new(
            "task_a",
            vec![],
            25,
            TaskValue::Val { value: ScalarValue::Int32(Some(1)) },
        );
        
        // Task B: Medium speed, depends on A (60ms, above threshold)
        let task_b = DelayedTask::new(
            "task_b",
            vec![dep_a.clone()],
            60,
            TaskValue::Val { value: ScalarValue::Int32(Some(2)) },
        );
        
        // Task C: Slow, depends on A (200ms, above threshold)
        let task_c = DelayedTask::new(
            "task_c",
            vec![dep_a.clone()],
            200,
            TaskValue::Val { value: ScalarValue::Int32(Some(3)) },
        );
        
        // Task D: Very slow, depends on B and C (will be run in parallel)
        let task_d = DelayedTask::new(
            "task_d",
            vec![dep_b, dep_c],
            150,
            TaskValue::Val { value: ScalarValue::Int32(Some(4)) },
        );
        
        tasks.insert(var_a.clone(), Arc::new(task_a) as Arc<dyn Task>);
        tasks.insert(var_b.clone(), Arc::new(task_b) as Arc<dyn Task>);
        tasks.insert(var_c.clone(), Arc::new(task_c) as Arc<dyn Task>);
        tasks.insert(var_d.clone(), Arc::new(task_d) as Arc<dyn Task>);
        
        let graph = Arc::new(TaskGraph::try_new(tasks)?);
        let runtime = TaskGraphRuntime::new();
        
        // First run - everything should spawn because we have no runtime history
        let start = Instant::now();
        let _results = runtime.evaluate_variables(graph.clone(), &[var_d.clone()]).await?;
        let total_time = start.elapsed();
        
        // Get the cached runtimes and spawn status
        let a_stats = runtime.cache.get_variable_stats(&var_a).await.unwrap();
        let b_stats = runtime.cache.get_variable_stats(&var_b).await.unwrap();
        let c_stats = runtime.cache.get_variable_stats(&var_c).await.unwrap();
        let d_stats = runtime.cache.get_variable_stats(&var_d).await.unwrap();
        
        println!("Task A runtime: {:?}, spawned: {}", a_stats.duration, a_stats.was_spawned);
        println!("Task B runtime: {:?}, spawned: {}", b_stats.duration, b_stats.was_spawned);
        println!("Task C runtime: {:?}, spawned: {}", c_stats.duration, c_stats.was_spawned);
        println!("Task D runtime: {:?}, spawned: {}", d_stats.duration, d_stats.was_spawned);
        println!("Total measured time: {:?}", total_time);
        
        // In the first run, all tasks should be spawned except D (the top-level task)
        assert!(a_stats.was_spawned, "Task A should be spawned in first run");
        assert!(b_stats.was_spawned, "Task B should be spawned in first run");
        assert!(c_stats.was_spawned, "Task C should be spawned in first run");
        assert!(!d_stats.was_spawned, "Task D should not be spawned (it's the top-level task)");
        
        // Verify runtime constraints
        assert!(a_stats.duration.as_millis() >= 20, "Task A should take at least 20ms");
        assert!(b_stats.duration.as_millis() >= 55, "Task B should take at least 55ms");
        assert!(c_stats.duration.as_millis() >= 195, "Task C should take at least 195ms");
        assert!(d_stats.duration.as_millis() >= 145, "Task D should take at least 145ms");
        
        // Second run - now we have runtime history:
        // A should NOT be spawned (under threshold)
        // B and C should be spawned (over threshold)
        let mut tasks2 = HashMap::new();
        
        // Define variables for the second test
        let var_a2 = Variable::new("A2");
        let dep_a2 = variable_to_dependency(&var_a2, DependencyKind::ValOrExpr);
        let var_b2 = Variable::new("B2");
        let dep_b2 = variable_to_dependency(&var_b2, DependencyKind::ValOrExpr);
        let var_c2 = Variable::new("C2");
        let dep_c2 = variable_to_dependency(&var_c2, DependencyKind::ValOrExpr);
        let var_d2 = Variable::new("D2");
        let dep_d2 = variable_to_dependency(&var_d2, DependencyKind::ValOrExpr);
        
        // Same task types as before, different variables
        let task_a2 = DelayedTask::new(
            "task_a2",
            vec![],
            25,
            TaskValue::Val { value: ScalarValue::Int32(Some(1)) },
        );
        
        let task_b2 = DelayedTask::new(
            "task_b2",
            vec![dep_a2.clone()],
            60,
            TaskValue::Val { value: ScalarValue::Int32(Some(2)) },
        );
        
        let task_c2 = DelayedTask::new(
            "task_c2",
            vec![dep_a2.clone()],
            200, 
            TaskValue::Val { value: ScalarValue::Int32(Some(3)) },
        );
        
        let task_d2 = DelayedTask::new(
            "task_d2",
            vec![dep_b2, dep_c2],
            150,
            TaskValue::Val { value: ScalarValue::Int32(Some(4)) },
        );
        
        tasks2.insert(var_a2.clone(), Arc::new(task_a2) as Arc<dyn Task>);
        tasks2.insert(var_b2.clone(), Arc::new(task_b2) as Arc<dyn Task>);
        tasks2.insert(var_c2.clone(), Arc::new(task_c2) as Arc<dyn Task>);
        tasks2.insert(var_d2.clone(), Arc::new(task_d2) as Arc<dyn Task>);
        
        let graph2 = Arc::new(TaskGraph::try_new(tasks2)?);
        
        // Pre-populate the runtime cache with the values from the first run
        // This simulates already having runtime history for similar variables
        runtime.cache.store_variable_stats(
            var_a2.clone(), 
            RuntimeStats { 
                duration: a_stats.duration,
                was_spawned: false // Will be updated during execution
            }
        ).await;
        
        runtime.cache.store_variable_stats(
            var_b2.clone(), 
            RuntimeStats { 
                duration: b_stats.duration,
                was_spawned: false // Will be updated during execution
            }
        ).await;
        
        runtime.cache.store_variable_stats(
            var_c2.clone(), 
            RuntimeStats { 
                duration: c_stats.duration,
                was_spawned: false // Will be updated during execution
            }
        ).await;
        
        runtime.cache.store_variable_stats(
            var_d2.clone(), 
            RuntimeStats { 
                duration: d_stats.duration,
                was_spawned: false // Will be updated during execution
            }
        ).await;
        
        // Run the second evaluation
        let start2 = Instant::now();
        let _results2 = runtime.evaluate_variables(graph2.clone(), &[var_d2.clone()]).await?;
        let total_time2 = start2.elapsed();
        
        // Get the updated runtimes and spawn status
        let a2_stats = runtime.cache.get_variable_stats(&var_a2).await.unwrap();
        let b2_stats = runtime.cache.get_variable_stats(&var_b2).await.unwrap();
        let c2_stats = runtime.cache.get_variable_stats(&var_c2).await.unwrap();
        let d2_stats = runtime.cache.get_variable_stats(&var_d2).await.unwrap();
        
        println!("Second run:");
        println!("Task A2 runtime: {:?}, spawned: {}", a2_stats.duration, a2_stats.was_spawned);
        println!("Task B2 runtime: {:?}, spawned: {}", b2_stats.duration, b2_stats.was_spawned);
        println!("Task C2 runtime: {:?}, spawned: {}", c2_stats.duration, c2_stats.was_spawned);
        println!("Task D2 runtime: {:?}, spawned: {}", d2_stats.duration, d2_stats.was_spawned);
        println!("Total measured time: {:?}", total_time2);
        
        // Verify spawn decisions based on runtime history
        assert!(!a2_stats.was_spawned, "Task A2 should NOT be spawned (under 50ms threshold)");
        assert!(b2_stats.was_spawned, "Task B2 should be spawned (over 50ms threshold)");
        assert!(c2_stats.was_spawned, "Task C2 should be spawned (over 50ms threshold)");
        assert!(!d2_stats.was_spawned, "Task D2 should not be spawned (it's the top-level task)");
        
        Ok(())
    }
}
