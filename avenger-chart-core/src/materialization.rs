use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use async_trait::async_trait;
use datafusion::{arrow::record_batch::RecordBatch, common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::AvengerChartError;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MaterializationKey(pub String);

impl MaterializationKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
}

impl AsRef<str> for MaterializationKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MaterializationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for MaterializationKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MaterializationKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MaterializationIdentity(pub String);

impl MaterializationIdentity {
    pub fn new(identity: impl Into<String>) -> Self {
        Self(identity.into())
    }
}

impl AsRef<str> for MaterializationIdentity {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MaterializationIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for MaterializationIdentity {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MaterializationIdentity {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MaterializationKind(pub String);

impl MaterializationKind {
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }
}

impl AsRef<str> for MaterializationKind {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MaterializationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for MaterializationKind {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MaterializationKind {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MaterializationPolicy {
    pub allow_stale: bool,
}

impl Default for MaterializationPolicy {
    fn default() -> Self {
        Self { allow_stale: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaterializationOutputKind {
    RecordBatch,
    RgbaImage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RgbaImageMaterialization {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum MaterializationResult {
    RecordBatch(RecordBatch),
    RgbaImage(RgbaImageMaterialization),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MaterializationRequest {
    pub key: MaterializationKey,
    pub kind: MaterializationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<MaterializationIdentity>,
    #[serde(default)]
    pub priority: f32,
    #[serde(default)]
    pub policy: MaterializationPolicy,
    pub output_kind: MaterializationOutputKind,
    #[serde(default)]
    pub spec: serde_json::Value,
}

impl MaterializationRequest {
    pub fn new(
        key: impl Into<MaterializationKey>,
        kind: impl Into<MaterializationKind>,
        output_kind: MaterializationOutputKind,
    ) -> Self {
        Self {
            key: key.into(),
            kind: kind.into(),
            identity: None,
            priority: 0.0,
            policy: MaterializationPolicy::default(),
            output_kind,
            spec: serde_json::Value::Null,
        }
    }

    pub fn identity(mut self, identity: impl Into<MaterializationIdentity>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    pub fn priority(mut self, priority: f32) -> Self {
        self.priority = priority;
        self
    }

    pub fn policy(mut self, policy: MaterializationPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn spec(mut self, spec: serde_json::Value) -> Self {
        self.spec = spec;
        self
    }
}

pub struct MaterializationExecutionContext<'a> {
    pub session_context: &'a SessionContext,
    pub params: &'a IndexMap<String, ScalarValue>,
}

#[async_trait]
pub trait MaterializationExecutor: Send + Sync {
    fn kind(&self) -> &'static str;

    async fn run(
        &self,
        request: MaterializationRequest,
        ctx: MaterializationExecutionContext<'_>,
    ) -> Result<MaterializationResult, AvengerChartError>;
}

#[derive(Clone, Default)]
pub struct MaterializationExecutorRegistry {
    executors: Arc<Mutex<HashMap<MaterializationKind, Arc<dyn MaterializationExecutor>>>>,
}

impl MaterializationExecutorRegistry {
    pub fn register<E>(&self, executor: E)
    where
        E: MaterializationExecutor + 'static,
    {
        let kind = MaterializationKind::new(executor.kind());
        self.executors
            .lock()
            .expect("materialization executor registry lock poisoned")
            .insert(kind, Arc::new(executor));
    }

    pub fn get(&self, kind: &MaterializationKind) -> Option<Arc<dyn MaterializationExecutor>> {
        self.executors
            .lock()
            .expect("materialization executor registry lock poisoned")
            .get(kind)
            .cloned()
    }
}

#[derive(Clone, Debug)]
pub struct EvaluationInvalidation {
    pub epoch: u64,
    pub reason: EvaluationInvalidationReason,
    pub schedule: EvaluationInvalidationSchedule,
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvaluationInvalidationReason {
    MaterializationCompleted { kind: MaterializationKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationInvalidationSchedule {
    Now,
    After(Duration),
}

pub trait EvaluationInvalidationSink: Send + Sync {
    fn request_evaluation(&self, request: EvaluationInvalidationRequest);
}

#[derive(Clone, Debug)]
pub struct EvaluationInvalidationRequest {
    pub reason: EvaluationInvalidationReason,
    pub schedule: EvaluationInvalidationSchedule,
}

impl EvaluationInvalidationRequest {
    pub fn now(reason: EvaluationInvalidationReason) -> Self {
        Self {
            reason,
            schedule: EvaluationInvalidationSchedule::Now,
        }
    }
}

pub type EvaluationInvalidationCallback =
    Arc<dyn Fn(EvaluationInvalidation) + Send + Sync + 'static>;

pub struct EvaluationInvalidationSubscription {
    id: usize,
    inner: Weak<Mutex<EvaluationInvalidationHubInner>>,
}

impl Drop for EvaluationInvalidationSubscription {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let mut inner = inner
            .lock()
            .expect("evaluation invalidation hub lock poisoned");
        inner.callbacks.retain(|(id, _)| *id != self.id);
    }
}

#[derive(Clone, Default)]
pub struct EvaluationInvalidationHub {
    inner: Arc<Mutex<EvaluationInvalidationHubInner>>,
}

#[derive(Default)]
struct EvaluationInvalidationHubInner {
    epoch: u64,
    next_callback_id: usize,
    callbacks: Vec<(usize, EvaluationInvalidationCallback)>,
}

impl EvaluationInvalidationHub {
    pub fn epoch(&self) -> u64 {
        self.inner
            .lock()
            .expect("evaluation invalidation hub lock poisoned")
            .epoch
    }

    pub fn subscribe(
        &self,
        callback: EvaluationInvalidationCallback,
    ) -> EvaluationInvalidationSubscription {
        let mut inner = self
            .inner
            .lock()
            .expect("evaluation invalidation hub lock poisoned");
        let id = inner.next_callback_id;
        inner.next_callback_id = inner.next_callback_id.wrapping_add(1);
        inner.callbacks.push((id, callback));
        EvaluationInvalidationSubscription {
            id,
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl EvaluationInvalidationSink for EvaluationInvalidationHub {
    fn request_evaluation(&self, request: EvaluationInvalidationRequest) {
        let (invalidation, callbacks) = {
            let mut inner = self
                .inner
                .lock()
                .expect("evaluation invalidation hub lock poisoned");
            inner.epoch = inner.epoch.wrapping_add(1);
            let invalidation = EvaluationInvalidation {
                epoch: inner.epoch,
                reason: request.reason,
                schedule: request.schedule,
            };
            let callbacks = inner
                .callbacks
                .iter()
                .map(|(_, callback)| callback.clone())
                .collect::<Vec<_>>();
            (invalidation, callbacks)
        };

        for callback in callbacks {
            callback(invalidation.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn request(key: &str, priority: f32) -> MaterializationRequest {
        MaterializationRequest::new(key, "test", MaterializationOutputKind::RecordBatch)
            .priority(priority)
    }

    #[test]
    fn materialization_keys_are_hashable_and_comparable() {
        let left = MaterializationKey::new("view/a");
        let right = MaterializationKey::new("view/a");
        let other = MaterializationKey::new("view/b");

        assert_eq!(left, right);
        assert_ne!(left, other);
        assert_eq!(left.to_string(), "view/a");
    }

    #[test]
    fn invalidation_hub_coalesces_subscription_lifetime() {
        let hub = EvaluationInvalidationHub::default();
        let count = Arc::new(AtomicUsize::new(0));
        let count_callback = count.clone();
        let subscription = hub.subscribe(Arc::new(move |_| {
            count_callback.fetch_add(1, Ordering::SeqCst);
        }));

        hub.request_evaluation(EvaluationInvalidationRequest::now(
            EvaluationInvalidationReason::MaterializationCompleted {
                kind: MaterializationKind::new("test"),
            },
        ));
        drop(subscription);
        hub.request_evaluation(EvaluationInvalidationRequest::now(
            EvaluationInvalidationReason::MaterializationCompleted {
                kind: MaterializationKind::new("test"),
            },
        ));

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(hub.epoch(), 2);
    }

    #[test]
    fn higher_priority_request_is_the_one_to_keep() {
        let mut low = request("same", 0.25);
        let high = request("same", 2.0).spec(serde_json::json!({"winner": true}));

        if high.priority > low.priority {
            low = high;
        }

        assert_eq!(low.priority, 2.0);
        assert_eq!(low.spec, serde_json::json!({"winner": true}));
    }
}
