use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use crate::value::{TaskValue, Variable};
use crate::error::AvengerLangError;

/// A cached result including the task value and runtime information
#[derive(Clone)]
pub struct CachedResult {
    pub value: TaskValue,
    pub runtime: Duration,
}

/// Runtime statistics for a variable's execution
#[derive(Clone, Copy)]
pub struct RuntimeStats {
    pub duration: Duration,
    pub was_spawned: bool,
}

/// A thread-safe cache for task evaluation results
pub struct TaskCache {
    values: RwLock<HashMap<u64, CachedResult>>,
    var_runtimes: RwLock<HashMap<Variable, RuntimeStats>>,
}

impl TaskCache {
    pub fn new() -> Self {
        Self {
            values: RwLock::new(HashMap::new()),
            var_runtimes: RwLock::new(HashMap::new()),
        }
    }

    /// Try to get a cached value
    pub async fn get(&self, fingerprint: u64) -> Option<TaskValue> {
        let values = self.values.read().await;
        values.get(&fingerprint).map(|result| result.value.clone())
    }

    /// Cache a value with the given fingerprint and runtime
    pub async fn insert(&self, fingerprint: u64, value: TaskValue, runtime: Duration) {
        let mut values = self.values.write().await;
        values.insert(fingerprint, CachedResult { value, runtime });
    }
    
    /// Store runtime statistics for a specific variable
    pub async fn store_variable_stats(&self, variable: Variable, stats: RuntimeStats) {
        let mut var_runtimes = self.var_runtimes.write().await;
        var_runtimes.insert(variable, stats);
    }
    
    /// Get the runtime statistics for a specific variable
    pub async fn get_variable_stats(&self, variable: &Variable) -> Option<RuntimeStats> {
        let var_runtimes = self.var_runtimes.read().await;
        var_runtimes.get(variable).copied()
    }
}
