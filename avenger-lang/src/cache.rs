use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::value::TaskValue;
use crate::error::AvengerLangError;

/// A thread-safe cache for task evaluation results
pub struct TaskCache {
    values: RwLock<HashMap<u64, TaskValue>>,
}

impl TaskCache {
    pub fn new() -> Self {
        Self {
            values: RwLock::new(HashMap::new()),
        }
    }

    /// Try to get a cached value
    pub async fn get(&self, fingerprint: u64) -> Option<TaskValue> {
        let values = self.values.read().await;
        values.get(&fingerprint).cloned()
    }

    /// Cache a value with the given fingerprint
    pub async fn insert(&self, fingerprint: u64, value: TaskValue) {
        let mut values = self.values.write().await;
        values.insert(fingerprint, value);
    }
}
