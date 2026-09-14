use avenger_common::canvas::CanvasDimensions;
use wgpu::{
    Device, Extent3d, LoadOp, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureView, TextureViewDescriptor,
};

use crate::target::AvengerRenderTarget;

#[derive(Clone, Debug)]
pub struct OffscreenTargetDescriptor {
    pub dimensions: CanvasDimensions,
    pub format: TextureFormat,
    pub sample_count: u32,
    pub usage: TextureUsages,
    pub label: Option<String>,
}

impl OffscreenTargetDescriptor {
    pub fn new(dimensions: CanvasDimensions, format: TextureFormat) -> Self {
        Self {
            dimensions,
            format,
            sample_count: 1,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            label: None,
        }
    }

    pub fn extent(&self) -> Extent3d {
        Extent3d {
            width: self.dimensions.to_physical_width().max(1),
            height: self.dimensions.to_physical_height().max(1),
            depth_or_array_layers: 1,
        }
    }

    pub fn with_sample_count(mut self, sample_count: u32) -> Self {
        self.sample_count = sample_count.max(1);
        self
    }

    pub fn with_usage(mut self, usage: TextureUsages) -> Self {
        self.usage = usage;
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

pub struct OffscreenTarget {
    pub texture: Texture,
    pub view: TextureView,
    pub extent: Extent3d,
    pub format: TextureFormat,
    pub sample_count: u32,
    pub usage: TextureUsages,
    pub dimensions: CanvasDimensions,
}

impl OffscreenTarget {
    pub fn new(device: &Device, descriptor: &OffscreenTargetDescriptor) -> Self {
        let texture = create_offscreen_texture(device, descriptor);
        let view = texture.create_view(&TextureViewDescriptor::default());
        Self {
            texture,
            view,
            extent: descriptor.extent(),
            format: descriptor.format,
            sample_count: descriptor.sample_count,
            usage: descriptor.usage,
            dimensions: descriptor.dimensions,
        }
    }

    pub fn resize_or_recreate(
        &mut self,
        device: &Device,
        descriptor: &OffscreenTargetDescriptor,
    ) -> bool {
        if self.extent == descriptor.extent()
            && self.format == descriptor.format
            && self.sample_count == descriptor.sample_count
            && self.usage == descriptor.usage
            && self.dimensions.scale == descriptor.dimensions.scale
        {
            self.dimensions = descriptor.dimensions;
            return false;
        }

        *self = Self::new(device, descriptor);
        true
    }

    pub fn render_target(&self, load: LoadOp<wgpu::Color>) -> AvengerRenderTarget<'_> {
        AvengerRenderTarget::new(
            &self.view,
            self.extent,
            self.format,
            self.sample_count,
            load,
        )
    }
}

fn create_offscreen_texture(device: &Device, descriptor: &OffscreenTargetDescriptor) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: descriptor.label.as_deref(),
        size: descriptor.extent(),
        mip_level_count: 1,
        sample_count: descriptor.sample_count,
        dimension: TextureDimension::D2,
        format: descriptor.format,
        usage: descriptor.usage,
        view_formats: &[descriptor.format],
    })
}
