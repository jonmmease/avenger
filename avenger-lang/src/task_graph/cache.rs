use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use lru::LruCache;
use crate::task_graph::{value::TaskValue, variable::{Variable, VariableKind}};
use crate::error::AvengerLangError;
use datafusion_common::ScalarValue;

/// A cached result including the task value and runtime information
#[derive(Clone)]
pub struct CachedResult {
    pub value: TaskValue,
    pub runtime: Duration,
}

/// Runtime statistics for a variable's execution
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeStats {
    pub duration: Duration,
    pub was_spawned: bool,
}

/// A segmented LRU cache that divides capacity between probationary and protected segments
struct SegmentedLru<K, V> 
where 
    K: Clone + Eq + std::hash::Hash + std::fmt::Debug, 
    V: Clone
{
    /// Probationary segment (new items start here)
    probationary: LruCache<K, V>,
    
    /// Protected segment (frequently accessed items go here)
    protected: LruCache<K, V>,
    
    /// Total capacity across both segments
    total_capacity: usize,
    
    /// Capacity of the probationary segment (protected = total - probationary)
    probationary_capacity: usize,
}

impl<K, V> SegmentedLru<K, V> 
where 
    K: Clone + Eq + std::hash::Hash + std::fmt::Debug, 
    V: Clone
{
    /// Create a new segmented LRU cache with the given total capacity
    /// The probationary segment takes up 20% of capacity by default
    pub fn new(total_capacity: usize) -> Self {
        Self::new_with_probationary_ratio(total_capacity.max(1), 0.2)
    }
    
    /// Create a new segmented LRU cache with a custom probationary ratio
    /// Ratio should be between 0.0 and 1.0, representing the fraction
    /// of total capacity dedicated to the probationary segment
    pub fn new_with_probationary_ratio(total_capacity: usize, probationary_ratio: f64) -> Self {
        let total_capacity = total_capacity.max(2);
        let probationary_capacity = (total_capacity as f64 * probationary_ratio.clamp(0.1, 0.9)) as usize;
        let probationary_capacity = probationary_capacity.max(1);
        let protected_capacity = (total_capacity - probationary_capacity).max(1);
        
        Self {
            probationary: LruCache::new(probationary_capacity.nonzero_or(1)),
            protected: LruCache::new(protected_capacity.nonzero_or(1)),
            total_capacity,
            probationary_capacity,
        }
    }
    
    /// Check if the key exists in either cache
    pub fn contains(&self, key: &K) -> bool {
        self.protected.contains(key) || self.probationary.contains(key)
    }
    
    /// Get an item from the cache
    /// Items accessed from the probationary segment are promoted to the protected segment
    pub fn get(&mut self, key: &K) -> Option<&V> {
        // Check the protected segment first
        if self.protected.contains(key) {
            return self.protected.get(key);
        }
        
        // Not in protected, check probationary
        if !self.probationary.contains(key) {
            return None;
        }
        
        // Item is in probationary, we need to promote it to protected
        // First get a clone of the value
        let value = self.probationary.get(key).unwrap().clone();
        
        // Remove from probationary
        self.probationary.pop(key);
        
        // Add to protected (may evict LRU item from protected to probationary if full)
        if self.protected.len() >= self.protected.cap().get() {
            if let Some((old_key, old_val)) = self.protected.pop_lru() {
                self.probationary.put(old_key, old_val);
            }
        }
        
        // Now put the promoted item in protected
        self.protected.put(key.clone(), value);
        
        // Return reference to the newly inserted item
        self.protected.get(key)
    }
    
    /// Get an item without updating its position in the cache
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.protected.peek(key).or_else(|| self.probationary.peek(key))
    }
    
    /// Insert an item into the cache
    /// New items always go into the probationary segment first
    pub fn put(&mut self, key: K, value: V) -> Option<V> {
        // If already in protected, update it there
        if self.protected.contains(&key) {
            return self.protected.put(key, value);
        }
        
        // If already in probationary, update it there
        if self.probationary.contains(&key) {
            return self.probationary.put(key, value);
        }
        
        // Check if probationary is full before inserting
        if self.probationary.len() >= self.probationary.cap().get() {
            // Move oldest item from probationary to protected if there's room
            if let Some((old_key, old_val)) = self.probationary.pop_lru() {
                // If protected is full, we'll need to make room
                if self.protected.len() >= self.protected.cap().get() {
                    // Evict oldest item from protected back to probationary
                    if let Some((older_key, older_val)) = self.protected.pop_lru() {
                        self.probationary.put(older_key, older_val);
                    }
                }
                // Now we can move the item from probationary to protected
                self.protected.put(old_key, old_val);
            }
        }
        
        // Insert new item into probationary
        self.probationary.put(key, value)
    }
    
    /// Remove an item from the cache
    pub fn pop(&mut self, key: &K) -> Option<V> {
        self.protected.pop(key).or_else(|| self.probationary.pop(key))
    }
    
    /// Get the total number of items in the cache
    pub fn len(&self) -> usize {
        self.probationary.len() + self.protected.len()
    }
    
    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.probationary.is_empty() && self.protected.is_empty()
    }
    
    /// Clear the cache
    pub fn clear(&mut self) {
        self.probationary.clear();
        self.protected.clear();
    }
    
    #[cfg(test)]
    pub fn debug_dump(&self) {
        println!("Protected segment (cap={}): {:?}", self.protected.cap().get(),
            self.protected.iter().map(|(k, _v)| k).collect::<Vec<_>>());
        println!("Probationary segment (cap={}): {:?}", self.probationary.cap().get(),
            self.probationary.iter().map(|(k, _v)| k).collect::<Vec<_>>());
    }
}

/// A thread-safe cache for task evaluation results, using segmented LRU for eviction
pub struct TaskCache {
    values: RwLock<SegmentedLru<u64, CachedResult>>,
    var_runtimes: RwLock<SegmentedLru<Variable, RuntimeStats>>,
    
    /// Default capacity for fingerprint-based cache
    values_capacity: usize,
    
    /// Default capacity for variable runtime stats cache
    stats_capacity: usize,
}

impl TaskCache {
    /// Create a new cache with default capacity (1000 items for each cache)
    pub fn new() -> Self {
        Self::with_capacity(1000, 1000)
    }
    
    /// Create a new cache with custom capacities
    pub fn with_capacity(values_capacity: usize, stats_capacity: usize) -> Self {
        Self {
            values: RwLock::new(SegmentedLru::<u64, CachedResult>::new(values_capacity)),
            var_runtimes: RwLock::new(SegmentedLru::<Variable, RuntimeStats>::new(stats_capacity)),
            values_capacity,
            stats_capacity,
        }
    }

    /// Try to get a cached value, promoting it if found
    pub async fn get(&self, fingerprint: u64) -> Option<TaskValue> {
        let mut values = self.values.write().await;
        values.get(&fingerprint).map(|result| result.value.clone())
    }
    
    /// Try to peek at a cached value without promoting it
    pub async fn peek(&self, fingerprint: u64) -> Option<TaskValue> {
        let values = self.values.read().await;
        values.peek(&fingerprint).map(|result| result.value.clone())
    }
    
    /// Check if a value exists in the cache
    pub async fn contains(&self, fingerprint: u64) -> bool {
        let values = self.values.read().await;
        values.contains(&fingerprint)
    }

    /// Cache a value with the given fingerprint and runtime
    pub async fn insert(&self, fingerprint: u64, value: TaskValue, runtime: Duration) {
        let mut values = self.values.write().await;
        values.put(fingerprint, CachedResult { value, runtime });
    }
    
    /// Store runtime statistics for a specific variable
    pub async fn store_variable_stats(&self, variable: Variable, stats: RuntimeStats) {
        let mut var_runtimes = self.var_runtimes.write().await;
        var_runtimes.put(variable, stats);
    }
    
    /// Get the runtime statistics for a specific variable, promoting it if found
    pub async fn get_variable_stats(&self, variable: &Variable) -> Option<RuntimeStats> {
        let mut var_runtimes = self.var_runtimes.write().await;
        var_runtimes.get(variable).copied()
    }
    
    /// Get the runtime statistics without promoting it
    pub async fn peek_variable_stats(&self, variable: &Variable) -> Option<RuntimeStats> {
        let var_runtimes = self.var_runtimes.read().await;
        var_runtimes.peek(variable).copied()
    }
    
    /// Store just the runtime for a specific variable (backward compatibility)
    pub async fn store_variable_runtime(&self, variable: Variable, duration: Duration) {
        self.store_variable_stats(variable, RuntimeStats { 
            duration, 
            was_spawned: false // Default value
        }).await;
    }
    
    /// Get just the runtime for a specific variable (backward compatibility)
    pub async fn get_variable_runtime(&self, variable: &Variable) -> Option<Duration> {
        self.get_variable_stats(variable).await.map(|stats| stats.duration)
    }
    
    /// Clear all caches
    pub async fn clear(&self) {
        let mut values = self.values.write().await;
        let mut var_runtimes = self.var_runtimes.write().await;
        
        values.clear();
        var_runtimes.clear();
    }
    
    /// Get the capacities of the caches
    pub fn capacities(&self) -> (usize, usize) {
        (self.values_capacity, self.stats_capacity)
    }
}

/// Extension trait to convert a usize to NonZeroUsize for LruCache capacity
trait NonZeroOrExt {
    fn nonzero_or(self, default: usize) -> std::num::NonZeroUsize;
}

impl NonZeroOrExt for usize {
    fn nonzero_or(self, default: usize) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(self).unwrap_or(std::num::NonZeroUsize::new(default).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_segmented_lru_basic() {
        let mut cache = SegmentedLru::<String, i32>::new(10);
        
        // Insert items using the public API
        cache.put("a".to_string(), 1);
        cache.put("b".to_string(), 2);
        cache.put("c".to_string(), 3);
        
        // Verify items can be retrieved
        assert_eq!(cache.get(&"a".to_string()).copied(), Some(1));
        assert_eq!(cache.get(&"b".to_string()).copied(), Some(2));
        assert_eq!(cache.get(&"c".to_string()).copied(), Some(3));
        
        // Verify non-existent item returns None
        assert_eq!(cache.get(&"d".to_string()), None);
    }
    
    #[test]
    fn test_segmented_lru_eviction() {
        // Create a cache with total capacity 5, probationary segment of 2
        let mut cache = SegmentedLru::<String, i32>::new_with_probationary_ratio(5, 0.4);
        
        // Insert more items than probationary capacity
        cache.put("a".to_string(), 1);
        cache.put("b".to_string(), 2);
        cache.put("c".to_string(), 3); // This should move 'a' to protected rather than evicting it
        
        // First access to 'b' and 'c' - they should be in probationary
        assert_eq!(cache.get(&"b".to_string()).copied(), Some(2)); // 'b' promoted to protected
        assert_eq!(cache.get(&"c".to_string()).copied(), Some(3)); // 'c' promoted to protected
        
        // 'a' should be in protected, not evicted with our improved implementation
        assert_eq!(cache.get(&"a".to_string()).copied(), Some(1)); // 'a' is in protected
        
        // Add more items to fill up probationary again
        cache.put("d".to_string(), 4);
        cache.put("e".to_string(), 5);
        cache.put("f".to_string(), 6); // With our implementation, this moves oldest item to protected
                                       // but also can evict other items when protected is full
        
        // Check final state
        assert_eq!(cache.get(&"b".to_string()).copied(), Some(2)); // Still in protected or probationary
        assert_eq!(cache.get(&"c".to_string()).copied(), Some(3)); // Still in protected or probationary
        assert_eq!(cache.get(&"d".to_string()).copied(), Some(4)); // Should be in protected
        assert_eq!(cache.get(&"e".to_string()), None);             // 'e' was evicted when 'f' was added
        assert_eq!(cache.get(&"f".to_string()).copied(), Some(6)); // 'f' is in probationary
    }
    
    #[tokio::test]
    async fn test_task_cache() {
        // Create a TaskCache with small capacity for testing
        let cache = TaskCache::with_capacity(5, 5);
        
        // Insert some values
        let fingerprint1 = 100;
        let value1 = TaskValue::Val(datafusion_common::ScalarValue::Utf8(Some("test1".to_string())));
        cache.insert(fingerprint1, value1.clone(), Duration::from_millis(10)).await;
        
        // Verify we can get the value back
        assert_eq!(cache.get(fingerprint1).await, Some(value1.clone()));
        
        // Check that the cache contains the fingerprint
        assert!(cache.contains(fingerprint1).await);
        
        // Insert a second value
        let fingerprint2 = 200;
        let value2 = TaskValue::Val(datafusion_common::ScalarValue::Utf8(Some("test2".to_string())));
        cache.insert(fingerprint2, value2.clone(), Duration::from_millis(20)).await;
        
        // Verify both values are retrievable
        assert_eq!(cache.get(fingerprint1).await, Some(value1));
        assert_eq!(cache.get(fingerprint2).await, Some(value2));
        
        // Check a non-existent key
        let non_existent = 300;
        assert_eq!(cache.get(non_existent).await, None);
        assert!(!cache.contains(non_existent).await);
    }
}
