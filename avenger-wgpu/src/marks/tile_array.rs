//! Persistent texture arrays retain tile pixels across scene changes.
//! Resource availability uses the same resolution helper as image atlases.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

use avenger_image::RgbaImage as AvengerRgbaImage;
use avenger_resource::ResourceKey;
use avenger_scenegraph::marks::image::{SceneImageResource, SceneImageUnavailablePolicy};
use wgpu::{BindGroup, BindGroupLayout, Device, Extent3d, Queue};

use crate::error::AvengerWgpuError;
use crate::image_resources::{
    resolve_image_resource, ImageSizeRequirement, ResolvedImageContent, WgpuImageResourceConfig,
    WgpuImageResourceStatus,
};
use crate::marks::image::make_placeholder;

/// WebGL2 downlevel `max_texture_array_layers`.
const MAX_TILE_ARRAY_LAYERS: u32 = 256;
/// GPU budget per tile-size group (48 MiB → 192 layers of 256px RGBA,
/// 48 layers of 512px).
const GROUP_BUDGET_BYTES: u64 = 48 * 1024 * 1024;

fn tile_group_capacity(size: u32) -> u32 {
    let per_layer = 4 * u64::from(size) * u64::from(size);
    u32::try_from(GROUP_BUDGET_BYTES / per_layer.max(1))
        .unwrap_or(MAX_TILE_ARRAY_LAYERS)
        .clamp(8, MAX_TILE_ARRAY_LAYERS)
}

/// What a layer currently holds on the GPU. Textures are zero-initialized,
/// so `Empty` (fully transparent) is the initial state of every layer.
enum SlotContent {
    Empty,
    Placeholder,
    /// Uploaded pixels, identified by the source `Arc` so a revalidated
    /// tile (new `Arc` for the same key) re-uploads.
    Image(Weak<AvengerRgbaImage>),
}

impl SlotContent {
    fn matches_image(&self, image: &Arc<AvengerRgbaImage>) -> bool {
        match self {
            SlotContent::Image(weak) => weak
                .upgrade()
                .is_some_and(|uploaded| Arc::ptr_eq(&uploaded, image)),
            _ => false,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct SlotKey {
    key: ResourceKey,
    fallback_key: Option<ResourceKey>,
    policy: SceneImageUnavailablePolicy,
}

impl SlotKey {
    fn new(resource: &SceneImageResource, policy: SceneImageUnavailablePolicy) -> Self {
        Self {
            key: resource.key.clone(),
            fallback_key: resource.fallback_key.clone(),
            policy,
        }
    }
}

struct Slot {
    key: SlotKey,
    last_used_epoch: u64,
    content: SlotContent,
}

struct SlotGroup {
    capacity: u32,
    slots: Vec<Slot>,
    by_key: HashMap<SlotKey, u32>,
}

impl SlotGroup {
    fn new(capacity: u32) -> Self {
        Self {
            capacity,
            slots: Vec::new(),
            by_key: HashMap::new(),
        }
    }
    fn assign(&mut self, key: SlotKey, epoch: u64) -> Option<u32> {
        if let Some(&layer) = self.by_key.get(&key) {
            self.slots[layer as usize].last_used_epoch = epoch;
            return Some(layer);
        }
        let layer = if self.slots.len() < self.capacity as usize {
            let layer = self.slots.len();
            self.slots.push(Slot {
                key: key.clone(),
                last_used_epoch: epoch,
                content: SlotContent::Empty,
            });
            layer
        } else {
            let (layer, slot) = self
                .slots
                .iter_mut()
                .enumerate()
                .filter(|(_, slot)| slot.last_used_epoch < epoch)
                .min_by_key(|(_, slot)| slot.last_used_epoch)?;
            self.by_key.remove(&slot.key);
            slot.key = key.clone();
            slot.last_used_epoch = epoch;
            layer
        };
        self.by_key.insert(key, layer as u32);
        Some(layer as u32)
    }
}

/// Layers used by the current scene stay assigned until the next scene begins.
#[derive(Default)]
pub(crate) struct TileSlotAllocator {
    groups: HashMap<u32, SlotGroup>,
    epoch: u64,
}

impl TileSlotAllocator {
    pub(crate) fn begin_scene(&mut self) {
        self.epoch += 1;
    }

    pub(crate) fn assign(
        &mut self,
        size: u32,
        resource: &SceneImageResource,
        policy: SceneImageUnavailablePolicy,
    ) -> Option<u32> {
        self.groups
            .entry(size)
            .or_insert_with(|| SlotGroup::new(tile_group_capacity(size)))
            .assign(SlotKey::new(resource, policy), self.epoch)
    }

    /// Reserve the complete mark before changing slots, so atlas fallback leaves no partial assignments.
    pub(crate) fn assign_many(
        &mut self,
        size: u32,
        resources: &[&SceneImageResource],
        policy: SceneImageUnavailablePolicy,
    ) -> Option<Vec<u32>> {
        let group = self
            .groups
            .entry(size)
            .or_insert_with(|| SlotGroup::new(tile_group_capacity(size)));
        let keys: Vec<_> = resources
            .iter()
            .map(|resource| SlotKey::new(resource, policy))
            .collect();
        let protected: HashSet<_> = keys
            .iter()
            .chain(
                group
                    .slots
                    .iter()
                    .filter(|slot| slot.last_used_epoch == self.epoch)
                    .map(|slot| &slot.key),
            )
            .collect();
        if protected.len() > group.capacity as usize {
            return None;
        }

        // Protect residents requested later in this mark before selecting any eviction victims.
        for key in &keys {
            if let Some(&layer) = group.by_key.get(key) {
                group.slots[layer as usize].last_used_epoch = self.epoch;
            }
        }
        let layers = keys
            .into_iter()
            .map(|key| {
                group
                    .assign(key, self.epoch)
                    .expect("reservation leaves capacity for every key")
            })
            .collect();
        Some(layers)
    }
}

/// Per-frame upload accounting (reset each sync).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileUploadStats {
    pub layers_uploaded: u32,
    pub bytes_uploaded: u64,
}

struct GpuGroup {
    texture: wgpu::Texture,
    bind_group_linear: BindGroup,
    bind_group_nearest: BindGroup,
}

/// The GPU half: one `texture_2d_array` per tile-size group, kept alive
/// across scene rebuilds by the renderer core, synchronized to the
/// allocator + resolver state once per prepared frame.
pub(crate) struct TileTextureArrays {
    allocator: Arc<Mutex<TileSlotAllocator>>,
    gpu: HashMap<u32, GpuGroup>,
    stats: TileUploadStats,
    total: TileUploadStats,
}

impl TileTextureArrays {
    pub(crate) fn new() -> Self {
        Self {
            allocator: Arc::new(Mutex::new(TileSlotAllocator::default())),
            gpu: HashMap::new(),
            stats: TileUploadStats::default(),
            total: TileUploadStats::default(),
        }
    }

    /// Shared handle for the scene builder (`MultiMarkRenderer`).
    pub(crate) fn allocator(&self) -> Arc<Mutex<TileSlotAllocator>> {
        self.allocator.clone()
    }

    pub(crate) fn begin_scene(&mut self) {
        self.allocator
            .lock()
            .expect("tile slot allocator poisoned")
            .begin_scene();
    }

    /// Uploads performed by the most recent [`Self::sync`].
    pub(crate) fn frame_stats(&self) -> TileUploadStats {
        self.stats
    }

    /// Cumulative uploads since creation.
    pub(crate) fn total_stats(&self) -> TileUploadStats {
        self.total
    }

    /// Bind group for a batch (`None` when the group has never synced —
    /// callers skip the draw; this cannot happen in the normal
    /// build-frame flow, which syncs before encoding).
    pub(crate) fn bind_group(&self, size: u32, smooth: bool) -> Option<&BindGroup> {
        self.gpu.get(&size).map(|group| {
            if smooth {
                &group.bind_group_linear
            } else {
                &group.bind_group_nearest
            }
        })
    }

    /// Bring every current-scene layer to its desired content, mirroring
    /// the atlas path's resolution semantics and status reporting. Cheap
    /// when nothing changed: one resolver query + pointer compare per
    /// visible tile, zero uploads.
    pub(crate) fn sync(
        &mut self,
        device: &Device,
        queue: &Queue,
        tile_layout: &BindGroupLayout,
        config: &WgpuImageResourceConfig,
    ) -> Result<WgpuImageResourceStatus, AvengerWgpuError> {
        self.stats = TileUploadStats::default();
        let mut status = WgpuImageResourceStatus::default();
        let allocator = self.allocator.clone();
        let mut allocator = allocator.lock().expect("tile slot allocator poisoned");
        let epoch = allocator.epoch;

        for (&size, group) in allocator.groups.iter_mut() {
            if !group.slots.iter().any(|slot| slot.last_used_epoch == epoch) {
                continue;
            }
            let gpu = match self.gpu.entry(size) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(make_gpu_group(device, tile_layout, size, group.capacity))
                }
            };

            for (layer_index, slot) in group
                .slots
                .iter_mut()
                .enumerate()
                .filter(|(_, slot)| slot.last_used_epoch == epoch)
            {
                let layer = layer_index as u32;
                match resolve_image_resource(
                    &slot.key.key,
                    slot.key.fallback_key.as_ref(),
                    ImageSizeRequirement::Tile(size),
                    slot.key.policy,
                    config,
                    &mut status,
                )? {
                    ResolvedImageContent::Image(image) => {
                        if !slot.content.matches_image(&image) {
                            upload_layer(
                                queue,
                                &gpu.texture,
                                size,
                                layer,
                                &image.data,
                                &mut self.stats,
                                &mut self.total,
                            );
                            slot.content = SlotContent::Image(Arc::downgrade(&image));
                        }
                    }
                    ResolvedImageContent::Placeholder => {
                        if !matches!(slot.content, SlotContent::Placeholder) {
                            let placeholder = make_placeholder(size, size, &config.placeholder)?;
                            upload_layer(
                                queue,
                                &gpu.texture,
                                size,
                                layer,
                                placeholder.as_raw(),
                                &mut self.stats,
                                &mut self.total,
                            );
                            slot.content = SlotContent::Placeholder;
                        }
                    }
                    ResolvedImageContent::Empty => {
                        if !matches!(slot.content, SlotContent::Empty) {
                            let zeros = vec![0u8; (4 * size * size) as usize];
                            upload_layer(
                                queue,
                                &gpu.texture,
                                size,
                                layer,
                                &zeros,
                                &mut self.stats,
                                &mut self.total,
                            );
                            slot.content = SlotContent::Empty;
                        }
                    }
                }
            }
        }

        Ok(status)
    }
}

fn make_gpu_group(
    device: &Device,
    tile_layout: &BindGroupLayout,
    size: u32,
    capacity: u32,
) -> GpuGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        size: Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: capacity,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        label: Some("tile_array_texture"),
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler_for = |filter: wgpu::FilterMode| {
        device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter,
            min_filter: filter,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        })
    };
    let make_bind_group = |sampler: &wgpu::Sampler| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: tile_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
            label: Some("tile_array_bind_group"),
        })
    };
    let bind_group_linear = make_bind_group(&sampler_for(wgpu::FilterMode::Linear));
    let bind_group_nearest = make_bind_group(&sampler_for(wgpu::FilterMode::Nearest));
    GpuGroup {
        texture,
        bind_group_linear,
        bind_group_nearest,
    }
}

fn upload_layer(
    queue: &Queue,
    texture: &wgpu::Texture,
    size: u32,
    layer: u32,
    pixels: &[u8],
    stats: &mut TileUploadStats,
    total: &mut TileUploadStats,
) {
    let premultiplied = crate::image_resources::premultiplied_pixels(pixels);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &premultiplied,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size),
            rows_per_image: Some(size),
        },
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    stats.layers_uploaded += 1;
    stats.bytes_uploaded += pixels.len() as u64;
    total.layers_uploaded += 1;
    total.bytes_uploaded += pixels.len() as u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(key: &str) -> SceneImageResource {
        SceneImageResource {
            key: ResourceKey::new(key),
            intrinsic_width: 2,
            intrinsic_height: 2,
            fallback_key: None,
        }
    }

    #[test]
    fn tile_array_reservations_preserve_residents_and_are_atomic() {
        let mut allocator = TileSlotAllocator::default();
        allocator.groups.insert(2, SlotGroup::new(2));
        allocator.begin_scene();
        let policy = SceneImageUnavailablePolicy::Skip;
        let (a, b, c) = (resource("a"), resource("b"), resource("c"));
        let first = allocator.assign_many(2, &[&a, &b], policy).unwrap();
        assert_eq!(allocator.assign(2, &a, policy), Some(first[0]));
        assert!(allocator.assign(2, &c, policy).is_none());
        allocator.begin_scene();
        assert!(allocator.assign_many(2, &[&a, &b, &c], policy).is_none());
        assert!(allocator.groups[&2]
            .slots
            .iter()
            .all(|slot| slot.last_used_epoch < allocator.epoch));
        // The new key cannot evict a resident requested later in the same mark.
        let next = allocator.assign_many(2, &[&c, &a], policy).unwrap();
        assert_eq!(next, vec![first[1], first[0]]);
        assert!(allocator.assign(2, &b, policy).is_none());
    }
}
