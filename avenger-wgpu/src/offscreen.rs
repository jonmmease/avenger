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
    pub generation: u64,
    pub dimensions: CanvasDimensions,
}

impl OffscreenTarget {
    pub fn new(device: &Device, descriptor: &OffscreenTargetDescriptor, generation: u64) -> Self {
        let texture = create_offscreen_texture(device, descriptor);
        let view = texture.create_view(&TextureViewDescriptor::default());
        Self {
            texture,
            view,
            extent: descriptor.extent(),
            format: descriptor.format,
            sample_count: descriptor.sample_count,
            usage: descriptor.usage,
            generation,
            dimensions: descriptor.dimensions,
        }
    }

    pub fn resize_or_recreate(
        &mut self,
        device: &Device,
        descriptor: &OffscreenTargetDescriptor,
        generation: u64,
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

        *self = Self::new(device, descriptor, generation);
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

pub struct OffscreenTargetPool {
    targets: Vec<OffscreenTarget>,
    next_index: usize,
    next_generation: u64,
    descriptor: OffscreenTargetDescriptor,
}

impl OffscreenTargetPool {
    pub fn double_buffered(device: &Device, descriptor: OffscreenTargetDescriptor) -> Self {
        Self::new(device, descriptor, 2)
    }

    pub fn triple_buffered(device: &Device, descriptor: OffscreenTargetDescriptor) -> Self {
        Self::new(device, descriptor, 3)
    }

    pub fn new(
        device: &Device,
        descriptor: OffscreenTargetDescriptor,
        target_count: usize,
    ) -> Self {
        let target_count = target_count.max(1);
        let mut next_generation = 1;
        let mut targets = Vec::with_capacity(target_count);
        for _ in 0..target_count {
            targets.push(OffscreenTarget::new(device, &descriptor, next_generation));
            next_generation += 1;
        }

        Self {
            targets,
            next_index: 0,
            next_generation,
            descriptor,
        }
    }

    pub fn descriptor(&self) -> &OffscreenTargetDescriptor {
        &self.descriptor
    }

    pub fn resize_or_recreate(
        &mut self,
        device: &Device,
        descriptor: OffscreenTargetDescriptor,
    ) -> bool {
        let mut recreated_any = false;
        for target in &mut self.targets {
            let generation = self.next_generation;
            if target.resize_or_recreate(device, &descriptor, generation) {
                self.next_generation += 1;
                recreated_any = true;
            }
        }
        self.descriptor = descriptor;
        recreated_any
    }

    pub fn acquire_next(&mut self) -> &mut OffscreenTarget {
        let index = self.next_index;
        self.next_index = (self.next_index + 1) % self.targets.len();
        &mut self.targets[index]
    }

    pub fn acquire_next_excluding_generation(
        &mut self,
        excluded_generation: Option<u64>,
    ) -> Option<&mut OffscreenTarget> {
        let selected_index = (0..self.targets.len()).find_map(|_| {
            let index = self.next_index;
            self.next_index = (self.next_index + 1) % self.targets.len();
            if Some(self.targets[index].generation) == excluded_generation {
                None
            } else {
                Some(index)
            }
        });

        selected_index.map(|index| &mut self.targets[index])
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderedOffscreenFrame {
    pub generation: u64,
    pub extent: Extent3d,
    pub format: TextureFormat,
    pub sample_count: u32,
}

impl From<&OffscreenTarget> for RenderedOffscreenFrame {
    fn from(target: &OffscreenTarget) -> Self {
        Self {
            generation: target.generation,
            extent: target.extent,
            format: target.format,
            sample_count: target.sample_count,
        }
    }
}
