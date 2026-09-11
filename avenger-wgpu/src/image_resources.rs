use std::sync::Arc;

pub use avenger_image::ImageResourceResolver;
use avenger_image::RgbaImage;
use avenger_resource::ResourceKey;

#[derive(Clone)]
pub struct WgpuImageResourceConfig {
    pub resolver: Option<Arc<dyn ImageResourceResolver>>,
    pub missing_policy: WgpuMissingImagePolicy,
    pub placeholder: WgpuImagePlaceholder,
}

impl Default for WgpuImageResourceConfig {
    fn default() -> Self {
        Self {
            resolver: None,
            missing_policy: WgpuMissingImagePolicy::Error,
            placeholder: WgpuImagePlaceholder::Checkerboard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WgpuMissingImagePolicy {
    DrawPlaceholder,
    Skip,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WgpuImagePlaceholder {
    Transparent,
    Solid([u8; 4]),
    Checkerboard,
    Inline(RgbaImage),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WgpuImageResourceStatus {
    pub pending: Vec<ResourceKey>,
    pub missing: Vec<ResourceKey>,
    pub failed: Vec<(ResourceKey, String)>,
    pub generation: u64,
}

impl WgpuImageResourceStatus {
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.missing.is_empty() && self.failed.is_empty()
    }

    pub(crate) fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}
