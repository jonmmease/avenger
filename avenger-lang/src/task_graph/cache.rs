use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use lru::LruCache;
use crate::error::AvengerLangError;
use datafusion_common::ScalarValue;

use super::value::TaskValue;
use super::variable::Variable;

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

/// A segmented LRU cache that divides items between probationary and protected segments
/// Eviction is based solely on memory usage
struct SegmentedLru<K, V> 
where 
    K: Clone + Eq + std::hash::Hash, 
    V: Clone
{
    /// Probationary segment (new items start here)
    probationary: LruCache<K, V>,
    
    /// Protected segment (frequently accessed items go here)
    protected: LruCache<K, V>,
    
    /// Current memory usage in bytes
    memory_usage: usize,
    
    /// Memory limit in bytes
    memory_limit: Option<usize>,
    
    /// Maximum number of entries allowed in the cache
    pub max_entries: Option<usize>,
    
    /// Function to calculate the size of a value
    size_of_value: Box<dyn Fn(&V) -> usize + Send + Sync>,
}

impl<K, V> SegmentedLru<K, V> 
where 
    K: Clone + Eq + std::hash::Hash, 
    V: Clone
{
    /// Create a new memory-limited segmented LRU cache
    /// 
    /// # Arguments
    /// * `memory_limit` - Optional memory limit in bytes
    /// * `size_of_value` - Function to calculate the size of a value
    pub fn new(memory_limit: Option<usize>, size_of_value: Box<dyn Fn(&V) -> usize + Send + Sync>) -> Self {
        // Use a fixed default capacity that will be overridden by max_entries if set
        let max_items = 100;
        let probationary_items = (max_items as f64 * 0.2) as usize;
        let protected_items = max_items - probationary_items;
        
        Self {
            probationary: LruCache::new(std::num::NonZeroUsize::new(probationary_items).unwrap()),
            protected: LruCache::new(std::num::NonZeroUsize::new(protected_items).unwrap()),
            memory_usage: 0,
            memory_limit,
            max_entries: None,
            size_of_value,
        }
    }
    
    /// Check if the key exists in either cache
    pub fn contains(&self, key: &K) -> bool {
        self.protected.contains(key) || self.probationary.contains(key)
    }
    
    /// Remove an item from the cache
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let from_protected = self.protected.pop(key);
        if from_protected.is_some() {
            self.track_memory(from_protected.as_ref(), None);
            return from_protected;
        }
        
        let from_probationary = self.probationary.pop(key);
        if from_probationary.is_some() {
            self.track_memory(from_probationary.as_ref(), None);
        }
        from_probationary
    }
    
    /// Track memory changes when a value is added/removed
    fn track_memory(&mut self, old_value: Option<&V>, new_value: Option<&V>) {
        // Remove old value memory
        if let Some(old) = old_value {
            let old_size = (self.size_of_value)(old);
            self.memory_usage = self.memory_usage.saturating_sub(old_size);
        }
        
        // Add new value memory
        if let Some(new) = new_value {
            let new_size = (self.size_of_value)(new);
            self.memory_usage += new_size;
        }
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
        
        // Get value from probationary before removing it
        let value = self.probationary.peek(key).unwrap().clone();
        
        // Remove from probationary and track memory change
        let old_value = self.probationary.pop(key);
        self.track_memory(old_value.as_ref(), None);
        
        // Make room in protected if needed by moving item to probationary
        if self.protected.len() >= self.protected.cap().get() {
            if let Some((old_key, old_val)) = self.protected.pop_lru() {
                // Track memory for temporary removal
                self.track_memory(Some(&old_val), None);
                
                // Add to probationary and track memory
                self.probationary.put(old_key, old_val.clone());
                self.track_memory(None, Some(&old_val));
            }
        }
        
        // Add to protected and track memory
        self.protected.put(key.clone(), value.clone());
        self.track_memory(None, Some(&value));
        
        // Evict if needed after all changes
        self.evict_if_needed();
        
        // Return reference to the newly inserted item
        self.protected.get(key)
    }
    
    /// Get an item without updating its position in the cache
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.protected.peek(key).or_else(|| self.probationary.peek(key))
    }
    
    /// Insert an item into the cache
    pub fn put(&mut self, key: K, value: V) -> Option<V> {
        // If already exists in either cache, remove it and track memory
        let old_value = if self.protected.contains(&key) {
            self.protected.pop(&key)
        } else if self.probationary.contains(&key) {
            self.probationary.pop(&key)
        } else {
            None
        };
        
        self.track_memory(old_value.as_ref(), Some(&value));
        
        // New items always go to probationary segment
        let result = self.probationary.put(key, value);
        
        // Evict if memory or entry limit exceeded
        self.evict_if_needed();
        
        result
    }
    
    /// Evict items from the cache if memory limit is exceeded
    fn evict_if_needed(&mut self) {
        // Check if we have too many entries
        if let Some(max_entries) = self.max_entries {
            let mut total_entries = self.probationary.len() + self.protected.len();
            
            // Keep evicting until we're under the max entries limit
            while total_entries > max_entries && !self.is_empty() {
                // Always evict from probationary first if available
                if !self.probationary.is_empty() {
                    if let Some((_, val)) = self.probationary.pop_lru() {
                        self.track_memory(Some(&val), None);
                    }
                } else if !self.protected.is_empty() {
                    // If probationary is empty, evict from protected
                    if let Some((_, val)) = self.protected.pop_lru() {
                        self.track_memory(Some(&val), None);
                    }
                } else {
                    // If we get here, there's nothing left to evict
                    break;
                }
                
                // Recalculate total entries
                let new_total = self.probationary.len() + self.protected.len();
                if new_total == total_entries {
                    // We didn't actually remove anything, prevent infinite loop
                    break;
                }
                total_entries = new_total;
            }
        }
        
        // Check if we have a memory limit and if we're exceeding it
        if let Some(limit) = self.memory_limit {
            while self.memory_usage > limit && !self.is_empty() {
                // Evict from probationary until only one element is left
                if self.probationary.len() > 1 {
                    if let Some((_, val)) = self.probationary.pop_lru() {
                        self.track_memory(Some(&val), None);
                        continue;
                    }
                }
                
                // If probationary has at most one item left, try evicting from protected
                if !self.protected.is_empty() {
                    if let Some((_, val)) = self.protected.pop_lru() {
                        self.track_memory(Some(&val), None);
                        continue;
                    }
                }
                
                // Last resort: evict the last item in probationary if it exists
                if self.probationary.len() == 1 {
                    if let Some((_, val)) = self.probationary.pop_lru() {
                        self.track_memory(Some(&val), None);
                        continue;
                    }
                }
                
                // If we get here, there's nothing left to evict
                break;
            }
        }
    }
    
    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.probationary.is_empty() && self.protected.is_empty()
    }
    
    /// Clear the cache
    pub fn clear(&mut self) {
        self.probationary.clear();
        self.protected.clear();
        self.memory_usage = 0;
    }
    
    /// Get current memory usage
    pub fn memory_usage(&self) -> usize {
        self.memory_usage
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
    
    /// Optional memory limit in bytes
    memory_limit: Option<usize>,
}

impl TaskCache {
    /// Create a new cache with default capacity (1000 items for each cache)
    pub fn new() -> Self {
        Self::with_capacity(1000, 1000)
    }
    
    /// Create a new cache with custom capacities
    pub fn with_capacity(values_capacity: usize, stats_capacity: usize) -> Self {
        Self::with_options(values_capacity, stats_capacity, None)
    }
    
    /// Create a new cache with custom capacities and optional memory limit
    pub fn with_options(values_capacity: usize, stats_capacity: usize, memory_limit: Option<usize>) -> Self {
        // Create the size calculation function for CachedResults
        let size_of_cached_result = Box::new(|result: &CachedResult| -> usize {
            // Base size of CachedResult struct plus the approximate size of the TaskValue
            std::mem::size_of::<CachedResult>() + result.value.size_of()
        });
        
        // Create the size calculation function for RuntimeStats
        let size_of_runtime_stats = Box::new(|_: &RuntimeStats| -> usize {
            // RuntimeStats is a small fixed-size struct
            std::mem::size_of::<RuntimeStats>()
        });
        
        Self {
            values: RwLock::new(SegmentedLru::new(memory_limit, size_of_cached_result)),
            var_runtimes: RwLock::new(SegmentedLru::new(None, size_of_runtime_stats)),
            values_capacity,
            stats_capacity,
            memory_limit,
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
    
    /// Get the capacities of the caches and memory limit
    pub fn capacities(&self) -> (usize, usize, Option<usize>) {
        (self.values_capacity, self.stats_capacity, self.memory_limit)
    }
    
    /// Get the current memory usage of the value cache
    pub async fn memory_usage(&self) -> usize {
        let values = self.values.read().await;
        values.memory_usage()
    }
    
    /// Get the memory limit if set
    pub fn memory_limit(&self) -> Option<usize> {
        self.memory_limit
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
        // Create a size estimator that treats all i32 values as size 1
        let size_estimator = Box::new(|i: &i32| 1);
        
        // Create a cache with no memory limit but max 10 entries (based on size estimator)
        let mut cache = SegmentedLru::<String, i32>::new(None, size_estimator);
        
        // Insert a few items
        cache.put("a".to_string(), 1);
        cache.put("b".to_string(), 2);
        cache.put("c".to_string(), 3);
        
        // Check that they're there
        assert_eq!(cache.get(&"a".to_string()).copied(), Some(1));
        assert_eq!(cache.get(&"b".to_string()).copied(), Some(2));
        assert_eq!(cache.get(&"c".to_string()).copied(), Some(3));
        assert_eq!(cache.get(&"d".to_string()), None);
        
        // Update an item
        cache.put("b".to_string(), 22);
        assert_eq!(cache.get(&"b".to_string()).copied(), Some(22));
        
        // Remove an item
        cache.remove(&"a".to_string());
        assert_eq!(cache.get(&"a".to_string()), None);
    }
    
    #[test]
    fn test_segmented_lru_eviction() {
        // Create a size estimator that treats all i32 values as size 1
        let size_estimator = Box::new(|_i: &i32| 1);
        
        // Create a cache with capacity for 3 items and no memory limit
        let mut cache = SegmentedLru::<String, i32>::new(None, size_estimator);
        
        // Set max entries
        cache.max_entries = Some(3);
        
        // Insert up to capacity
        cache.put("a".to_string(), 1);
        cache.put("b".to_string(), 2);
        cache.put("c".to_string(), 3);
        
        // They should all be there
        assert_eq!(cache.peek(&"a".to_string()).copied(), Some(1));
        assert_eq!(cache.peek(&"b".to_string()).copied(), Some(2));
        assert_eq!(cache.peek(&"c".to_string()).copied(), Some(3));
        
        // Access "b" to make it more recently used
        cache.get(&"b".to_string());
        
        // Access "c" to make it more recently used
        cache.get(&"c".to_string());
        
        // At this point, "a" is the least recently used entry
        
        // Add a new item to trigger eviction
        cache.put("d".to_string(), 4);
        
        // Check which items are still in the cache and print state for debugging
        println!("After eviction: total items = {}", cache.probationary.len() + cache.protected.len());
        println!("  a: {}", cache.peek(&"a".to_string()).is_some());
        println!("  b: {}", cache.peek(&"b".to_string()).is_some());
        println!("  c: {}", cache.peek(&"c".to_string()).is_some());
        println!("  d: {}", cache.peek(&"d".to_string()).is_some());
        
        // The least recently used item should be evicted (a)
        assert_eq!(cache.peek(&"a".to_string()), None, "Item 'a' should be evicted as LRU");
        assert_eq!(cache.peek(&"b".to_string()).copied(), Some(2), "Item 'b' should still be in cache");
        assert_eq!(cache.peek(&"c".to_string()).copied(), Some(3), "Item 'c' should still be in cache");
        assert_eq!(cache.peek(&"d".to_string()).copied(), Some(4), "Item 'd' should be in cache");
    }
    
    #[test]
    fn test_segmented_lru_memory_limit() {
        // Create a size estimator that uses the number of digits as the size
        let size_estimator = Box::new(|i: &i32| {
            if *i < 10 { 1 }          // single digit: size 1
            else if *i < 100 { 2 }    // double digit: size 2
            else { 3 }                // triple digit: size 3
        });
        
        // Create a cache with memory limit of 5
        let mut cache = SegmentedLru::<String, i32>::new(Some(5), size_estimator);
        
        // Insert some items with varying sizes
        cache.put("a".to_string(), 1);   // Size 1
        cache.put("b".to_string(), 10);  // Size 2
        
        // Check what was stored - these should both fit in memory limit of 5
        assert_eq!(cache.peek(&"a".to_string()).copied(), Some(1));  // Size 1
        assert_eq!(cache.peek(&"b".to_string()).copied(), Some(10)); // Size 2
        
        // Access "a" to make it recently used
        cache.get(&"a".to_string());
        
        // Now "b" is the least recently used item
        
        // Insert an item that exceeds the remaining memory limit
        cache.put("c".to_string(), 100); // Size 3 (total would be 6, exceeds limit of 5)
        
        // The least recently used item "b" should be evicted to make room
        assert_eq!(cache.peek(&"a".to_string()).copied(), Some(1));  // Size 1, recently used
        assert_eq!(cache.peek(&"b".to_string()), None);              // Should be evicted
        assert_eq!(cache.peek(&"c".to_string()).copied(), Some(100)); // Size 3, newly added
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
    
    #[tokio::test]
    async fn test_task_cache_memory_limit() {
        // Create a string with a known size (about ~25 bytes for the struct + string data)
        let make_string_value = |s: &str| -> TaskValue {
            TaskValue::Val(datafusion_common::ScalarValue::Utf8(Some(s.to_string())))
        };
        
        // Create a TaskCache with a very low memory limit (100 bytes)
        let cache = Arc::new(TaskCache::with_options(10, 5, Some(100)));
        
        // Insert several values
        let fingerprints = [100, 200, 300, 400, 500];
        let values = [
            make_string_value("small_value_1"),
            make_string_value("small_value_2"),
            make_string_value("small_value_3"),
            make_string_value("small_value_4"),
            // Create a "large" string that will push us over the memory limit
            make_string_value(&"x".repeat(80)),  // ~105 bytes with struct overhead
        ];
        
        // Insert values
        for (i, (&fp, val)) in fingerprints.iter().zip(values.iter()).enumerate() {
            println!("Inserting value {} with fingerprint {}", i, fp);
            cache.insert(fp, val.clone(), Duration::from_millis(i as u64 * 10)).await;
            
            // Debug: print memory usage after each insertion
            let current_memory = cache.memory_usage().await;
            println!("  Current memory usage: {} bytes", current_memory);
        }
        
        // Memory usage should not exceed our limit
        let memory_usage = cache.memory_usage().await;
        println!("Final memory usage: {} bytes (limit: 100 bytes)", memory_usage);
        assert!(memory_usage <= 100, "Memory usage ({} bytes) exceeds limit (100 bytes)", memory_usage);
        
        // Check which items are still in the cache
        for (i, &fp) in fingerprints.iter().enumerate() {
            let present = cache.contains(fp).await;
            println!("Item {} (fingerprint {}): present = {}", i, fp, present);
        }
        
        // The cache should have evicted some items to stay under the memory limit
        let cache_ref = cache.clone();
        let present_count = futures::future::join_all(
            fingerprints.iter().map(|&fp| {
                let cache_clone = cache_ref.clone();
                async move { 
                    cache_clone.contains(fp).await 
                }
            })
        ).await.iter().filter(|&present| *present).count();
        
        println!("Total items in cache: {}", present_count);
        
        // Ensure eviction happened
        assert!(present_count < fingerprints.len(), 
            "Expected some items to be evicted, but all {} items are still present", fingerprints.len());
        
        // Check the cache's internal memory usage tracking is working correctly
        let memory_usage = cache.memory_usage().await;
        let expected_limit = 100;
        println!("Memory usage: {} bytes, limit: {} bytes", memory_usage, expected_limit);
        assert!(memory_usage <= expected_limit, 
            "Memory usage ({} bytes) should not exceed the limit ({} bytes)", 
            memory_usage, expected_limit);
    }
}

