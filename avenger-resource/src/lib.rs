use serde::{Deserialize, Serialize};

pub mod render_invalidation;

pub use render_invalidation::{
    RenderInvalidation, RenderInvalidationCallback, RenderInvalidationHub,
    RenderInvalidationReason, RenderInvalidationRequest, RenderInvalidationSchedule,
    RenderInvalidationSink, RenderInvalidationSubscription,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceKey(pub String);

impl ResourceKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
}

impl From<&str> for ResourceKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ResourceKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceKind(pub String);

impl ResourceKind {
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }
}

impl From<&str> for ResourceKind {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ResourceKind {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceSource {
    Url { url: String },
    DataUri { data_uri: String },
    Opaque { provider: String, id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ResourceCachePolicy {
    pub allow_stale: bool,
    pub max_age_seconds: Option<u64>,
}

impl Default for ResourceCachePolicy {
    fn default() -> Self {
        Self {
            allow_stale: true,
            max_age_seconds: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ResourceRequest {
    pub key: ResourceKey,
    pub kind: ResourceKind,
    pub source: ResourceSource,
    #[serde(default)]
    pub priority: f32,
    #[serde(default)]
    pub cache_policy: ResourceCachePolicy,
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("resource not found: {0:?}")]
    NotFound(ResourceKey),
    #[error("resource failed: {0}")]
    Failed(String),
}
