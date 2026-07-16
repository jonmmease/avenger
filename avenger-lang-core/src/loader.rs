use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{ImportCapabilities, SourceOrigin};

/// Loader-defined immutable version for source content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentVersion(String);

impl ContentVersion {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct LoadedSource {
    pub origin: SourceOrigin,
    pub text: Arc<str>,
    pub version: ContentVersion,
}

impl LoadedSource {
    pub fn new(origin: SourceOrigin, text: impl Into<Arc<str>>, version: ContentVersion) -> Self {
        Self {
            origin,
            text: text.into(),
            version,
        }
    }
}

#[async_trait]
pub trait SourceLoader: Send + Sync {
    async fn load(
        &self,
        origin: &SourceOrigin,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError>;
}

/// Deterministic loader used by frontend/compiler tests and embedded hosts.
#[derive(Clone, Debug, Default)]
pub struct InMemorySourceLoader {
    sources: Arc<RwLock<BTreeMap<SourceOrigin, LoadedSource>>>,
}

impl InMemorySourceLoader {
    pub fn insert(&self, source: LoadedSource) -> Option<LoadedSource> {
        self.sources
            .write()
            .expect("in-memory source loader lock poisoned")
            .insert(source.origin.clone(), source)
    }

    pub fn with_source(self, source: LoadedSource) -> Self {
        self.insert(source);
        self
    }
}

#[async_trait]
impl SourceLoader for InMemorySourceLoader {
    async fn load(
        &self,
        origin: &SourceOrigin,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        let allowed = match origin {
            SourceOrigin::Memory(_) => capabilities.allow_memory,
            SourceOrigin::File(path) => {
                capabilities.allow_filesystem
                    && crate::project::normalize_path(path)
                        .starts_with(crate::project::normalize_path(&capabilities.project_root))
            }
            SourceOrigin::Std(_) => capabilities.allow_std,
            SourceOrigin::Http(_) => capabilities.allow_http,
        };
        if !allowed {
            return Err(SourceLoaderError::CapabilityDenied(origin.clone()));
        }

        self.sources
            .read()
            .expect("in-memory source loader lock poisoned")
            .get(origin)
            .cloned()
            .ok_or_else(|| SourceLoaderError::NotFound(origin.clone()))
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum SourceLoaderError {
    #[error("source capability denied for {0}")]
    CapabilityDenied(SourceOrigin),
    #[error("source not found: {0}")]
    NotFound(SourceOrigin),
    #[error("failed to load {origin}: {message}")]
    Other {
        origin: SourceOrigin,
        message: String,
    },
}
