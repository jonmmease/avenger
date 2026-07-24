use std::{
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use avenger_lang_core::{
    ContentVersion, ImportCapabilities, LoadedSource, SourceLoader, SourceLoaderError,
    SourceOrigin, module_graph::normalize_path,
};
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub struct SourceLoaderLimits {
    pub max_source_bytes: usize,
    pub max_redirects: usize,
}

impl Default for SourceLoaderLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 2 * 1024 * 1024,
            max_redirects: 5,
        }
    }
}

/// Production source loader for project files, bundled standard definitions,
/// and capability-gated HTTP(S) imports.
#[derive(Clone)]
pub struct DefaultSourceLoader {
    project_root: PathBuf,
    limits: SourceLoaderLimits,
    http: reqwest::Client,
}

impl DefaultSourceLoader {
    pub fn new(project_root: impl Into<PathBuf>) -> Result<Self, SourceLoaderError> {
        Self::with_limits(project_root, SourceLoaderLimits::default())
    }

    pub fn with_limits(
        project_root: impl Into<PathBuf>,
        limits: SourceLoaderLimits,
    ) -> Result<Self, SourceLoaderError> {
        let project_root = normalize_path(&project_root.into());
        let http = reqwest::Client::builder()
            .redirect(Policy::limited(limits.max_redirects))
            .build()
            .map_err(|error| SourceLoaderError::Other {
                origin: SourceOrigin::Memory("<http-client>".into()),
                message: error.to_string(),
            })?;
        Ok(Self {
            project_root,
            limits,
            http,
        })
    }

    fn load_file(
        &self,
        requested: &Path,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        let origin = SourceOrigin::File(requested.to_path_buf());
        if !capabilities.allow_filesystem {
            return Err(SourceLoaderError::CapabilityDenied(origin));
        }
        let configured_root = normalize_path(&capabilities.project_root);
        let loader_root = fs::canonicalize(&self.project_root).unwrap_or(self.project_root.clone());
        let capability_root = fs::canonicalize(&configured_root).unwrap_or(configured_root);
        let root = if loader_root.starts_with(&capability_root) {
            loader_root
        } else {
            capability_root
        };
        let candidate = normalize_path(requested);
        if !candidate.starts_with(&root) && !candidate.starts_with(&self.project_root) {
            return Err(SourceLoaderError::CapabilityDenied(origin));
        }
        let canonical = fs::canonicalize(&candidate)
            .map_err(|_| SourceLoaderError::NotFound(SourceOrigin::File(candidate.clone())))?;
        if !canonical.starts_with(&root) {
            return Err(SourceLoaderError::CapabilityDenied(SourceOrigin::File(
                canonical,
            )));
        }
        let bytes = fs::read(&canonical).map_err(|error| SourceLoaderError::Other {
            origin: SourceOrigin::File(canonical.clone()),
            message: error.to_string(),
        })?;
        if bytes.len() > self.limits.max_source_bytes {
            return Err(SourceLoaderError::Other {
                origin: SourceOrigin::File(canonical),
                message: format!(
                    "source is {} bytes; limit is {} bytes",
                    bytes.len(),
                    self.limits.max_source_bytes
                ),
            });
        }
        let text = String::from_utf8(bytes).map_err(|error| SourceLoaderError::Other {
            origin: SourceOrigin::File(canonical.clone()),
            message: format!("source is not UTF-8: {error}"),
        })?;
        let version = content_version(text.as_bytes());
        Ok(LoadedSource::new(
            SourceOrigin::File(canonical),
            text,
            version,
        ))
    }

    fn load_std(
        &self,
        path: &str,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        let origin = SourceOrigin::Std(path.to_owned());
        if !capabilities.allow_std {
            return Err(SourceLoaderError::CapabilityDenied(origin));
        }
        let (canonical_path, text) = match path {
            "marks/error_bar" => (
                "marks/error_bar.avenger",
                include_str!("../stdlib/marks/error_bar.avenger"),
            ),
            _ => return Err(SourceLoaderError::NotFound(origin)),
        };
        Ok(LoadedSource::new(
            SourceOrigin::Std(canonical_path.to_owned()),
            text,
            ContentVersion::new(format!("stdlib-1:sha256:{}", sha256_hex(text.as_bytes()))),
        ))
    }

    async fn load_http(
        &self,
        url: &str,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        let requested = SourceOrigin::Http(url.to_owned());
        if !capabilities.allow_http {
            return Err(SourceLoaderError::CapabilityDenied(requested));
        }
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| SourceLoaderError::Other {
                origin: requested.clone(),
                message: error.to_string(),
            })?;
        let final_origin = SourceOrigin::Http(response.url().to_string());
        if response
            .content_length()
            .is_some_and(|length| length > self.limits.max_source_bytes as u64)
        {
            return Err(SourceLoaderError::Other {
                origin: final_origin,
                message: format!("source exceeds {} byte limit", self.limits.max_source_bytes),
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|error| SourceLoaderError::Other {
                    origin: final_origin.clone(),
                    message: error.to_string(),
                })?
        {
            if bytes.len() + chunk.len() > self.limits.max_source_bytes {
                return Err(SourceLoaderError::Other {
                    origin: final_origin,
                    message: format!("source exceeds {} byte limit", self.limits.max_source_bytes),
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        let text = String::from_utf8(bytes).map_err(|error| SourceLoaderError::Other {
            origin: final_origin.clone(),
            message: format!("source is not UTF-8: {error}"),
        })?;
        let version = content_version(text.as_bytes());
        Ok(LoadedSource::new(final_origin, text, version))
    }
}

#[async_trait]
impl SourceLoader for DefaultSourceLoader {
    async fn load(
        &self,
        origin: &SourceOrigin,
        capabilities: &ImportCapabilities,
    ) -> Result<LoadedSource, SourceLoaderError> {
        match origin {
            SourceOrigin::File(path) => self.load_file(path, capabilities),
            SourceOrigin::Std(path) => self.load_std(path, capabilities),
            SourceOrigin::Http(url) => self.load_http(url, capabilities).await,
            SourceOrigin::Memory(_) => Err(SourceLoaderError::CapabilityDenied(origin.clone())),
        }
    }
}

fn content_version(bytes: &[u8]) -> ContentVersion {
    ContentVersion::new(format!("sha256:{}", sha256_hex(bytes)))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl std::fmt::Debug for DefaultSourceLoader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DefaultSourceLoader")
            .field("project_root", &self.project_root)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
