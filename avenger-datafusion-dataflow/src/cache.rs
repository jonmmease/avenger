use std::{collections::HashMap, sync::Arc};

use crate::{inputs::MaterializedValue, ScopeInstance, SnapshotId};

/// Retention policy for completed reusable values. Query-local sharing is always enabled.
#[derive(Clone, Debug)]
pub enum CachePolicy {
    Disabled,
    Lru(CacheConfig),
}
impl Default for CachePolicy {
    fn default() -> Self {
        Self::Lru(CacheConfig::default())
    }
}

/// Shared runtime limits. Both limits must be positive when retention is enabled.
#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub max_bytes: usize,
    pub max_entries: usize,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_bytes: 128 * 1024 * 1024,
            max_entries: 1024,
        }
    }
}

/// Current retained charges across every preparation belonging to a runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub evictions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BindingKey {
    Table(SnapshotId),
    // IPC preserves types, nulls, and floating-point bits, including nested scalars.
    Scalar(Arc<[u8]>),
    Expr(Arc<crate::expr_input::ExprKey>),
}
impl BindingKey {
    pub(crate) fn size(&self) -> usize {
        match self {
            Self::Table(_) => std::mem::size_of::<Self>(),
            Self::Scalar(v) => v.len() + std::mem::size_of::<Self>(),
            Self::Expr(v) => v.size() + std::mem::size_of::<Self>(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ValueKey {
    pub namespace: u64,
    pub node: usize,
    pub instance: Option<ScopeInstance>,
    pub inputs: Vec<(usize, BindingKey)>,
    pub base_inputs: Vec<(usize, BindingKey)>,
    /// Originating preparation and its clear epoch, independent of ordinary eviction.
    pub upstream: Option<(u64, u64)>,
}
impl ValueKey {
    pub(crate) fn size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.instance.as_ref().map_or(0, ScopeInstance::size)
            + self
                .inputs
                .iter()
                .chain(&self.base_inputs)
                .map(|(_, v)| std::mem::size_of::<usize>() + v.size())
                .sum::<usize>()
    }
}

struct Entry {
    value: MaterializedValue,
    bytes: usize,
    used: u64,
}

pub(crate) struct Cache {
    policy: CachePolicy,
    entries: HashMap<ValueKey, Entry>,
    epochs: HashMap<u64, u64>,
    tick: u64,
    stats: CacheStats,
}
impl Cache {
    fn current(&self, key: &ValueKey, epoch: u64) -> bool {
        self.epochs.get(&key.namespace) == Some(&epoch)
            && key
                .upstream
                .is_none_or(|(namespace, epoch)| self.epochs.get(&namespace) == Some(&epoch))
    }

    pub fn new(policy: CachePolicy) -> Self {
        Self {
            policy,
            entries: HashMap::new(),
            epochs: HashMap::new(),
            tick: 0,
            stats: CacheStats::default(),
        }
    }
    pub fn enabled(&self) -> bool {
        matches!(self.policy, CachePolicy::Lru(_))
    }
    pub fn stats(&self) -> CacheStats {
        self.stats
    }
    pub fn register(&mut self, namespace: u64) {
        self.epochs.insert(namespace, 0);
    }
    pub fn epoch(&self, namespace: u64) -> u64 {
        self.epochs[&namespace]
    }
    pub fn clear(&mut self, namespace: u64) {
        *self
            .epochs
            .get_mut(&namespace)
            .expect("registered namespace") += 1;
        self.entries.retain(|key, entry| {
            if key.namespace == namespace
                || key.upstream.is_some_and(|(origin, _)| origin == namespace)
            {
                self.stats.bytes -= entry.bytes;
                false
            } else {
                true
            }
        });
        self.stats.entries = self.entries.len();
    }
    pub fn remove(&mut self, namespace: u64) {
        self.clear(namespace);
        self.epochs.remove(&namespace);
    }
    pub fn get(&mut self, key: &ValueKey, epoch: u64) -> Option<MaterializedValue> {
        if !self.current(key, epoch) {
            return None;
        }
        let entry = self.entries.get_mut(key)?;
        self.tick += 1;
        entry.used = self.tick;
        Some(entry.value.clone())
    }
    pub fn insert(&mut self, key: ValueKey, epoch: u64, value: MaterializedValue) -> bool {
        let CachePolicy::Lru(config) = &self.policy else {
            return false;
        };
        let (max_bytes, max_entries) = (config.max_bytes, config.max_entries);
        if !self.current(&key, epoch) {
            return false;
        }
        let bytes = value.size().saturating_add(key.size()).saturating_add(128);
        if bytes > max_bytes {
            return false;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.stats.bytes -= old.bytes;
        }
        while self.entries.len() >= max_entries || self.stats.bytes > max_bytes - bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
                .expect("nonempty full cache");
            self.stats.bytes -= self.entries.remove(&oldest).expect("known entry").bytes;
            self.stats.evictions += 1;
        }
        self.tick += 1;
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                used: self.tick,
            },
        );
        self.stats.bytes += bytes;
        self.stats.entries = self.entries.len();
        true
    }
}
