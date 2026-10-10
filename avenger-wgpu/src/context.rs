//! GPU resources that renderers on one device share.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use wgpu::{Device, TextureFormat};

use crate::marks::{
    instanced_mark::InstancedMarkPipeline, multi::MultiMarkRenderResources, symbol::SymbolShader,
};

/// The render pipelines of one device, which every renderer that draws into the same texture
/// format with the same sample count shares. A host that draws many charts creates one context
/// and builds each chart's renderer from it, so the pipelines compile once.
#[derive(Clone)]
pub struct AvengerGpuContext {
    device: Device,
    pipelines: Arc<Mutex<HashMap<(TextureFormat, u32), RendererPipelines>>>,
}

/// The pipelines that a renderer draws with.
#[derive(Clone)]
pub(crate) struct RendererPipelines {
    pub multi: MultiMarkRenderResources,
    pub symbol: Arc<InstancedMarkPipeline>,
}

impl AvengerGpuContext {
    pub fn new(device: &Device) -> Self {
        Self {
            device: device.clone(),
            pipelines: Default::default(),
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The pipelines for a texture format and sample count, built the first time they're needed.
    pub(crate) fn pipelines(&self, format: TextureFormat, sample_count: u32) -> RendererPipelines {
        let mut pipelines = self
            .pipelines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        pipelines
            .entry((format, sample_count))
            .or_insert_with(|| RendererPipelines {
                multi: MultiMarkRenderResources::new(&self.device, format, sample_count),
                symbol: Arc::new(SymbolShader::pipeline(&self.device, format, sample_count)),
            })
            .clone()
    }
}
