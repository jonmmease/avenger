use crate::{
    error::{AvengerRuntimeError, DuplicateResult},
    value::TaskValue,
};
use lru::LruCache;
use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, MutexGuard, RwLock};

#[derive(Debug, Clone)]
struct CachedValue {
    value: TaskValue,
    _duration: Duration,
}

impl CachedValue {
    pub fn size_of(&self) -> usize {
        self.value.size_of()
    }
}

type Initializer = Arc<RwLock<Option<Result<TaskValue, AvengerRuntimeError>>>>;

pub struct RuntimeCacheConfig {
    capacity: Option<usize>,
    size_limit: Option<usize>,
}

impl Default for RuntimeCacheConfig {
    fn default() -> Self {
        Self {
            capacity: Some(256),
            size_limit: None,
        }
    }
}

/// The Cache uses a Segmented LRU (SLRU) cache policy
/// (https://en.wikipedia.org/wiki/Cache_replacement_policies#Segmented_LRU_(SLRU)) where both the
/// protected and probationary LRU caches are limited by capacity (number of entries) and memory
/// limit.
#[derive(Debug, Clone)]
pub struct RuntimeCache {
    protected_cache: Arc<Mutex<LruCache<u64, CachedValue>>>,
    probationary_cache: Arc<Mutex<LruCache<u64, CachedValue>>>,
    protected_fraction: f64,
    initializers: Arc<RwLock<HashMap<u64, Initializer>>>,
    size: Arc<AtomicUsize>,
    protected_memory: Arc<AtomicUsize>,
    probationary_memory: Arc<AtomicUsize>,
    capacity: Option<usize>,
    memory_limit: Option<usize>,
    // Mapping from identity fingerprints to duration of the prior evaluation
    durations: Arc<Mutex<LruCache<u64, Duration>>>,
}

impl RuntimeCache {
    pub fn new(config: RuntimeCacheConfig) -> Self {
        Self {
            protected_cache: Arc::new(Mutex::new(LruCache::unbounded())),
            probationary_cache: Arc::new(Mutex::new(LruCache::unbounded())),
            protected_fraction: 0.5,
            initializers: Default::default(),
            capacity: config.capacity,
            memory_limit: config.size_limit,
            size: Arc::new(AtomicUsize::new(0)),
            protected_memory: Arc::new(AtomicUsize::new(0)),
            probationary_memory: Arc::new(AtomicUsize::new(0)),
            durations: Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(512).unwrap()))),
        }
    }

    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    pub fn memory_limit(&self) -> Option<usize> {
        self.memory_limit
    }

    pub fn size(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    pub fn total_memory(&self) -> usize {
        self.protected_memory() + self.probationary_memory()
    }

    pub fn protected_memory(&self) -> usize {
        self.protected_memory.load(Ordering::Relaxed)
    }

    pub fn probationary_memory(&self) -> usize {
        self.probationary_memory.load(Ordering::Relaxed)
    }

    pub async fn clear(&self) {
        // Clear the values cache. There may still be initializers representing in progress
        // futures which will not be cleared.
        self.protected_cache.lock().await.clear();
        self.probationary_cache.lock().await.clear();
        self.protected_memory.store(0, Ordering::Relaxed);
        self.probationary_memory.store(0, Ordering::Relaxed);
        self.size.store(0, Ordering::Relaxed);
    }

    async fn get(&self, state_fingerprint: u64) -> Option<CachedValue> {
        let mut protected = self.protected_cache.lock().await;
        let mut probationary = self.probationary_cache.lock().await;

        if protected.contains(&state_fingerprint) {
            protected.get(&state_fingerprint).cloned()
        } else if probationary.contains(&state_fingerprint) {
            // Promote entry from probationary to protected
            let value = probationary.pop(&state_fingerprint).unwrap();
            let value_memory = value.size_of();
            protected.put(state_fingerprint, value.clone());

            self.protected_memory
                .fetch_add(value_memory, Ordering::Relaxed);
            self.probationary_memory
                .fetch_sub(value_memory, Ordering::Relaxed);

            // Balance caches
            self.balance(&mut protected, &mut probationary);

            Some(value)
        } else {
            None
        }
    }

    fn pop_protected_lru(
        &self,
        protected: &mut MutexGuard<LruCache<u64, CachedValue>>,
        probationary: &mut MutexGuard<LruCache<u64, CachedValue>>,
    ) {
        // Remove one protected LRU entry
        let (key, popped_value) = protected.pop_lru().unwrap();
        let popped_memory = popped_value.size_of();

        // Decrement protected memory
        self.protected_memory
            .fetch_sub(popped_memory, Ordering::Relaxed);

        // Add entry to probationary cache
        probationary.put(key, popped_value);

        // Increment probationary memory
        self.probationary_memory
            .fetch_add(popped_memory, Ordering::Relaxed);
    }

    fn pop_probationary_lru(&self, probationary: &mut MutexGuard<LruCache<u64, CachedValue>>) {
        let (_, popped_value) = probationary.pop_lru().unwrap();
        let popped_memory = popped_value.size_of();

        // Decrement protected memory
        self.probationary_memory
            .fetch_sub(popped_memory, Ordering::Relaxed);
    }

    fn balance(
        &self,
        protected: &mut MutexGuard<LruCache<u64, CachedValue>>,
        probationary: &mut MutexGuard<LruCache<u64, CachedValue>>,
    ) {
        // Compute capacity and memory limits for both protected and probationary caches
        let (protected_capacity, probationary_capacity) = if let Some(capacity) = self.capacity {
            let protected_capacity = (capacity as f64 * self.protected_fraction).ceil() as usize;
            (
                Some(protected_capacity),
                Some(capacity - protected_capacity),
            )
        } else {
            (None, None)
        };

        let (protected_mem_limit, probationary_mem_limit) =
            if let Some(memory_limit) = self.memory_limit {
                let protected_mem_limit =
                    (memory_limit as f64 * self.protected_fraction).ceil() as usize;
                (
                    Some(protected_mem_limit),
                    Some(memory_limit - protected_mem_limit),
                )
            } else {
                (None, None)
            };

        // Step 1: Shrink protected cache until it satisfies limits, moving evicted items to
        //         probationary cache
        // Pop to capacity limit
        if let Some(capacity) = protected_capacity {
            while protected.len() > 1 && protected.len() > capacity {
                self.pop_protected_lru(protected, probationary);
            }
        }

        // Pop LRU to memory limit
        if let Some(memory_limit) = protected_mem_limit {
            while protected.len() > 1
                && self.protected_memory.load(Ordering::Relaxed) > memory_limit
            {
                self.pop_protected_lru(protected, probationary);
            }
        }

        // Step 2: Shrink probationary cache until it satisfies limits,
        //         decrementing memory estimate
        if let Some(capacity) = probationary_capacity {
            while probationary.len() > 1 && probationary.len() > capacity {
                self.pop_probationary_lru(probationary);
            }
        }

        // Pop LRU to memory limit
        if let Some(memory_limit) = probationary_mem_limit {
            while probationary.len() > 1
                && self.probationary_memory.load(Ordering::Relaxed) > memory_limit
            {
                self.pop_probationary_lru(probationary);
            }
        }

        // Step 3: Update size atomics
        self.size
            .store(protected.len() + probationary.len(), Ordering::Relaxed);
    }

    async fn set_value(
        &self,
        state_fingerprint: u64,
        value: TaskValue,
        calculation_millis: Duration,
    ) {
        let cache_value = CachedValue {
            value,
            _duration: calculation_millis,
        };
        let value_memory = cache_value.size_of();

        let mut protected = self.protected_cache.lock().await;
        let mut probationary = self.probationary_cache.lock().await;
        if protected.contains(&state_fingerprint) {
            // Set on protected to update usage
            protected.put(state_fingerprint, cache_value);
        } else if probationary.contains(&state_fingerprint) {
            // Promote from probationary to protected
            protected.put(
                state_fingerprint,
                probationary.pop(&state_fingerprint).unwrap(),
            );
            self.protected_memory
                .fetch_add(value_memory, Ordering::Relaxed);
            self.probationary_memory
                .fetch_sub(value_memory, Ordering::Relaxed);
            self.balance(&mut protected, &mut probationary);
        } else {
            // Add to probationary and update memory usage
            probationary.put(state_fingerprint, cache_value);
            self.probationary_memory
                .fetch_add(value_memory, Ordering::Relaxed);
            self.balance(&mut protected, &mut probationary);
        }
    }

    async fn remove_initializer(&self, state_fingerprint: u64) -> Option<Initializer> {
        self.initializers.write().await.remove(&state_fingerprint)
    }

    pub async fn get_or_try_insert_with<F>(
        &self,
        state_fingerprint: u64,
        identity_fingerprint: u64,
        init: F,
    ) -> Result<TaskValue, AvengerRuntimeError>
    where
        F: Future<Output = Result<TaskValue, AvengerRuntimeError>> + Send + 'static,
    {
        // Check if present in the values cache
        if let Some(value) = self.get(state_fingerprint).await {
            return Ok(value.value);
        }

        // Check if present in initializers
        // let mut initializers_lock = self.initializers.write().await;
        let initializer = {
            self.initializers
                .write()
                .await
                .get(&state_fingerprint)
                .cloned()
        };

        if let Some(initializer) = initializer {
            // Calculation is in progress, await on Arc clone of it's initializer
            // Drop lock on initializers collection
            let result = initializer.read().await;
            let result = match result.as_ref() {
                None => {
                    self.spawn_initializer(state_fingerprint, identity_fingerprint, init)
                        .await
                }
                Some(result) => result.duplicate(),
            };
            result
        } else {
            self.spawn_initializer(state_fingerprint, identity_fingerprint, init)
                .await
        }
    }

    async fn spawn_initializer<F>(
        &self,
        state_fingerprint: u64,
        identity_fingerprint: u64,
        init: F,
    ) -> Result<TaskValue, AvengerRuntimeError>
    where
        F: Future<Output = Result<TaskValue, AvengerRuntimeError>> + Send + 'static,
    {
        // Create new initializer
        let initializer: Initializer = Arc::new(RwLock::new(None));

        // Get and hold write lock for initializer
        let mut initializer_lock = initializer.write().await;

        // Store Arc clone of initializer in initializers map
        self.initializers
            .write()
            .await
            .insert(state_fingerprint, initializer.clone());

        // Check if we have a duration for this identity fingerprint
        let duration = self
            .durations
            .lock()
            .await
            .get(&identity_fingerprint)
            .cloned();

        // spawn tasks that were previously more than 100ms, and tasks
        // we don't have a duration for
        let should_spawn = match duration {
            Some(duration) => duration > Duration::from_millis(100),
            None => true,
        };

        let start = Instant::now();
        let res = if should_spawn {
            tokio::spawn(init).await?
        } else {
            init.await
        };

        match res {
            Ok(value) => {
                *initializer_lock = Some(Ok(value.clone()));

                // Check if we should add value to long-term cache
                let duration = start.elapsed();

                // Set result value and duration
                self.set_value(state_fingerprint, value.clone(), duration)
                    .await;

                // Store duration for identity fingerprint
                self.durations
                    .lock()
                    .await
                    .put(identity_fingerprint, duration);

                // Stored initializer no longer required. Initializers are Arc
                // pointers, so it's fine to drop initializer from here even if
                // other tasks are still awaiting on it.
                self.remove_initializer(state_fingerprint).await;
                Ok(value)
            }
            Err(e) => {
                // Remove initializer so that another future can try again
                *initializer_lock = Some(Err(e.duplicate()));
                self.remove_initializer(state_fingerprint).await;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod test_cache {
    use std::time::Duration;

    // use crate::task_graph::cache::{NodeValue, VegaFusionCache};
    // use tokio::time::Duration;
    // use vegafusion_common::data::scalar::ScalarValue;
    // use vegafusion_common::error::Result;
    // use vegafusion_core::task_graph::task_value::TaskValue;
    use super::*;

    use datafusion_common::ScalarValue;

    async fn make_value(value: ScalarValue) -> Result<TaskValue, AvengerRuntimeError> {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        Ok(TaskValue::Val { value })
    }

    #[tokio::test]
    async fn try_cache() {
        let cache = RuntimeCache::new(RuntimeCacheConfig {
            capacity: Some(4),
            size_limit: None,
        });

        let value_future1 = cache.get_or_try_insert_with(1, 1, make_value(ScalarValue::from(23.5)));
        let value_future2 = cache.get_or_try_insert_with(2, 2, make_value(ScalarValue::from(33.5)));
        let value_future3 = cache.get_or_try_insert_with(3, 3, make_value(ScalarValue::from(43.5)));

        tokio::time::sleep(Duration::from_millis(100)).await;
        println!("{:?}", cache.initializers);

        // assert_eq!(cache.num_values().await, 0);
        // assert_eq!(cache.num_initializers().await, 1);

        let futures = vec![value_future1, value_future2];
        let values = futures::future::join_all(futures).await;

        let next_value = value_future3.await;

        // tokio::time::sleep(Duration::from_millis(300));
        println!("{:?}", cache.initializers);
        // assert_eq!(cache.num_values().await, 1);
        // assert_eq!(cache.num_initializers().await, 0);

        println!("values: {values:?}");
        println!("next_value: {next_value:?}");
        println!("durations: {:?}", cache.durations.lock().await);
    }
}
