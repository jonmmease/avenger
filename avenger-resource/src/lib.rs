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
    /// Permit the previous result to remain visible while an expired resource
    /// is refreshed. A refresh failure is still reported as a failure.
    pub allow_stale: bool,
    /// Maximum result age at request time. `None` allows indefinite reuse and
    /// zero requests a refresh. Reading a cached resource does not start a load.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceRequestPurpose {
    #[default]
    Required,
    Prefetch,
}

/// Identifies a retargetable prefetch working set (e.g. one tile layer in
/// one geo viewport: `"geo/{viewport_id}/{layer_id}"`). Prefetch requests
/// carrying the same scope form one atomically-replaceable set.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrefetchScope(pub String);

impl PrefetchScope {
    pub fn new(scope: impl Into<String>) -> Self {
        Self(scope.into())
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
    #[serde(default)]
    pub purpose: ResourceRequestPurpose,
    /// Projected pixel center of the resource at plan time, in CANVAS
    /// coordinates (the same frame as pointer events), when known. Lets
    /// schedulers order fetches by distance to a focus point.
    #[serde(default)]
    pub screen_center: Option<[f32; 2]>,
    /// The retargetable prefetch set this request belongs to, if any.
    /// Only meaningful for `Prefetch`-purpose requests.
    #[serde(default)]
    pub prefetch_scope: Option<PrefetchScope>,
}

impl ResourceRequest {
    /// A `Required`-purpose request with default priority, cache policy,
    /// and no scheduling metadata.
    pub fn new(key: ResourceKey, kind: ResourceKind, source: ResourceSource) -> Self {
        Self {
            key,
            kind,
            source,
            priority: 0.0,
            cache_policy: ResourceCachePolicy::default(),
            purpose: ResourceRequestPurpose::Required,
            screen_center: None,
            prefetch_scope: None,
        }
    }
}

/// Recomputes a prefetch working set for a cursor position, published per
/// evaluation by coordinate-system guides and consumed by fetch schedulers
/// when the hover cursor comes to rest.
pub trait PrefetchRetargetPlanner: Send + Sync {
    /// The scope whose queued prefetch entries this planner replaces.
    fn scope(&self) -> &PrefetchScope;

    /// Recompute the prefetch working set for a cursor position in canvas
    /// pixels. Returns `None` when the cursor is outside this planner's
    /// plot rect, meaning: leave the current set alone.
    fn plan(&self, cursor_canvas_px: [f32; 2]) -> Option<Vec<ResourceRequest>>;
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("resource not found: {0:?}")]
    NotFound(ResourceKey),
    #[error("resource failed: {0}")]
    Failed(String),
}
