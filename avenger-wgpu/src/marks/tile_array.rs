//! Persistent `texture_2d_array` tile cache.
//!
//! Uniform-sized resource images (map tiles) bypass the per-frame image
//! atlas: each tile-size group owns one texture array whose layers are
//! assigned per resource key by a CPU-side slot allocator at scene-build
//! time, and whose pixels are synchronized once per prepared frame —
//! uploading only layers whose desired content changed (a tile arrived,
//! a placeholder became pixels, a slot was reassigned). Steady-state
//! pan/zoom frames upload nothing.
//!
//! Layer 0 of every group is reserved for the placeholder pattern.
//! Content policy mirrors the atlas path (`image.rs::unavailable_image`):
//! `Skip` → transparent layer, `DrawPlaceholder` → placeholder pixels,
//! `Error` → the frame errors; pending/missing/failed keys are reported
//! through [`WgpuImageResourceStatus`] exactly like atlas-resolved images.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use avenger_image::{ImageResourceState, RgbaImage as AvengerRgbaImage};
use avenger_resource::ResourceKey;
use avenger_scenegraph::marks::image::SceneImageUnavailablePolicy;
use wgpu::{BindGroup, BindGroupLayout, Device, Extent3d, Queue};

use crate::error::AvengerWgpuError;
use crate::image_resources::{
    WgpuImagePlaceholder, WgpuImageResourceConfig, WgpuImageResourceStatus, WgpuMissingImagePolicy,
};
use crate::marks::image::{push_unique, push_unique_failed};

/// WebGL2 downlevel `max_texture_array_layers`.
const MAX_TILE_ARRAY_LAYERS: u32 = 256;
/// GPU budget per tile-size group (48 MiB → 192 layers of 256px RGBA,
/// 48 layers of 512px).
const GROUP_BUDGET_BYTES: u64 = 48 * 1024 * 1024;
/// Layer 0 of every group holds the placeholder pattern.
const PLACEHOLDER_LAYER: u32 = 0;

pub(crate) fn tile_group_capacity(size: u32) -> u32 {
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

struct Slot {
    key: Option<ResourceKey>,
    fallback_key: Option<ResourceKey>,
    policy: SceneImageUnavailablePolicy,
    last_used_epoch: u64,
    content: SlotContent,
}

struct SlotGroup {
    capacity: u32,
    slots: Vec<Slot>,
    by_key: HashMap<ResourceKey, u32>,
}

impl SlotGroup {
    fn new(capacity: u32) -> Self {
        Self {
            capacity,
            // Layer 0: reserved placeholder layer, always "in use".
            slots: vec![Slot {
                key: None,
                fallback_key: None,
                policy: SceneImageUnavailablePolicy::DrawPlaceholder,
                last_used_epoch: u64::MAX,
                content: SlotContent::Empty,
            }],
            by_key: HashMap::new(),
        }
    }
}

/// CPU-side layer assignment, shared between the scene builder (which
/// assigns layers while adding marks) and the renderer core (which syncs
/// pixels at prepare time). Slot assignments are stable for the lifetime
/// of a scene: eviction only reuses layers not referenced by the current
/// epoch, so vertex data referencing a layer never dangles.
#[derive(Default)]
pub struct TileSlotAllocator {
    groups: HashMap<u32, SlotGroup>,
    epoch: u64,
}

impl TileSlotAllocator {
    /// Start a new scene: layers assigned in earlier scenes become
    /// evictable (but stay resident, so returning viewports re-use them
    /// for free).
    pub fn begin_scene(&mut self) {
        self.epoch += 1;
    }

    /// Assign (or re-use) a layer for `key` in the `size` group. Returns
    /// `None` when the group is saturated with current-scene tiles — the
    /// caller falls back to the per-frame atlas path.
    pub fn assign(
        &mut self,
        size: u32,
        key: &ResourceKey,
        fallback_key: Option<&ResourceKey>,
        policy: SceneImageUnavailablePolicy,
    ) -> Option<u32> {
        let epoch = self.epoch;
        let group = self
            .groups
            .entry(size)
            .or_insert_with(|| SlotGroup::new(tile_group_capacity(size)));

        if let Some(&layer) = group.by_key.get(key) {
            let slot = &mut group.slots[layer as usize];
            slot.last_used_epoch = epoch;
            slot.policy = policy;
            slot.fallback_key = fallback_key.cloned();
            return Some(layer);
        }

        let layer = if (group.slots.len() as u32) < group.capacity {
            group.slots.push(Slot {
                key: None,
                fallback_key: None,
                policy,
                last_used_epoch: 0,
                content: SlotContent::Empty,
            });
            (group.slots.len() - 1) as u32
        } else {
            // Evict the least-recently-used layer not referenced by the
            // current scene (never layer 0).
            let victim = group
                .slots
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, slot)| slot.last_used_epoch < epoch)
                .min_by_key(|(_, slot)| slot.last_used_epoch)
                .map(|(index, _)| index as u32)?;
            if let Some(old_key) = group.slots[victim as usize].key.take() {
                group.by_key.remove(&old_key);
            }
            victim
        };

        let slot = &mut group.slots[layer as usize];
        slot.key = Some(key.clone());
        slot.fallback_key = fallback_key.cloned();
        slot.policy = policy;
        slot.last_used_epoch = epoch;
        // Content is intentionally left as-is: the sync pass compares it
        // against the new key's desired content and re-uploads.
        group.by_key.insert(key.clone(), layer);
        Some(layer)
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
pub struct TileTextureArrays {
    allocator: Arc<Mutex<TileSlotAllocator>>,
    gpu: HashMap<u32, GpuGroup>,
    stats: TileUploadStats,
    total: TileUploadStats,
}

impl TileTextureArrays {
    pub fn new() -> Self {
        Self {
            allocator: Arc::new(Mutex::new(TileSlotAllocator::default())),
            gpu: HashMap::new(),
            stats: TileUploadStats::default(),
            total: TileUploadStats::default(),
        }
    }

    /// Shared handle for the scene builder (`MultiMarkRenderer`).
    pub fn allocator(&self) -> Arc<Mutex<TileSlotAllocator>> {
        self.allocator.clone()
    }

    pub fn begin_scene(&mut self) {
        self.allocator
            .lock()
            .expect("tile slot allocator poisoned")
            .begin_scene();
    }

    /// Uploads performed by the most recent [`Self::sync`].
    pub fn frame_stats(&self) -> TileUploadStats {
        self.stats
    }

    /// Cumulative uploads since creation.
    pub fn total_stats(&self) -> TileUploadStats {
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
            let gpu = match self.gpu.entry(size) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(make_gpu_group(device, tile_layout, size, group.capacity))
                }
            };

            // Placeholder layer 0: upload once (and again only if the
            // placeholder config produces different pixels — not tracked;
            // the config is fixed per canvas).
            if !matches!(
                group.slots[PLACEHOLDER_LAYER as usize].content,
                SlotContent::Placeholder
            ) {
                let placeholder = placeholder_pixels(size, &config.placeholder)?;
                upload_layer(
                    queue,
                    &gpu.texture,
                    size,
                    PLACEHOLDER_LAYER,
                    placeholder.as_raw(),
                    &mut self.stats,
                    &mut self.total,
                );
                group.slots[PLACEHOLDER_LAYER as usize].content = SlotContent::Placeholder;
            }

            for (layer_index, slot) in group
                .slots
                .iter_mut()
                .enumerate()
                .skip(1)
                .filter(|(_, slot)| slot.last_used_epoch == epoch)
            {
                let layer = layer_index as u32;
                let Some(key) = slot.key.clone() else {
                    continue;
                };
                match desired_slot_content(&key, slot, size, config, &mut status)? {
                    DesiredContent::Image(image) => {
                        if !slot.content.matches_image(&image) {
                            let pixels = image.to_image().ok_or_else(|| {
                                AvengerWgpuError::ConversionError(format!(
                                    "Failed to convert ready resource image {key:?} to rgba image"
                                ))
                            })?;
                            upload_layer(
                                queue,
                                &gpu.texture,
                                size,
                                layer,
                                pixels.as_raw(),
                                &mut self.stats,
                                &mut self.total,
                            );
                            slot.content = SlotContent::Image(Arc::downgrade(&image));
                        }
                    }
                    DesiredContent::Placeholder => {
                        if !matches!(slot.content, SlotContent::Placeholder) {
                            let placeholder = placeholder_pixels(size, &config.placeholder)?;
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
                    DesiredContent::Empty => {
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

impl Default for TileTextureArrays {
    fn default() -> Self {
        Self::new()
    }
}

enum DesiredContent {
    Image(Arc<AvengerRgbaImage>),
    Placeholder,
    Empty,
}

/// Mirror of `image.rs::resolve_resource_image` + `unavailable_image` for
/// array layers: same status recording, same policy mapping, same
/// dimension checks, same fallback-key handling, same `Error` behavior.
fn desired_slot_content(
    key: &ResourceKey,
    slot: &Slot,
    size: u32,
    config: &WgpuImageResourceConfig,
    status: &mut WgpuImageResourceStatus,
) -> Result<DesiredContent, AvengerWgpuError> {
    let Some(resolver) = config.resolver.as_ref() else {
        push_unique(&mut status.missing, key.clone());
        return unavailable_content(slot, config, "No WGPU image resource resolver configured");
    };

    match resolver.image_state(key) {
        ImageResourceState::Ready(image) => {
            if image.width == size && image.height == size {
                Ok(DesiredContent::Image(image))
            } else {
                let message = format!(
                    "Ready resource image {key:?} has dimensions ({}, {}), expected ({size}, {size})",
                    image.width, image.height
                );
                push_unique_failed(status, key.clone(), message.clone());
                unavailable_content(slot, config, &message)
            }
        }
        ImageResourceState::Pending => {
            push_unique(&mut status.pending, key.clone());
            fallback_or_unavailable(slot, size, config, status, "pending")
        }
        ImageResourceState::Missing => {
            push_unique(&mut status.missing, key.clone());
            fallback_or_unavailable(slot, size, config, status, "missing")
        }
        ImageResourceState::Failed(error) => {
            push_unique_failed(status, key.clone(), error.to_string());
            fallback_or_unavailable(slot, size, config, status, error.as_ref())
        }
    }
}

fn fallback_or_unavailable(
    slot: &Slot,
    size: u32,
    config: &WgpuImageResourceConfig,
    status: &mut WgpuImageResourceStatus,
    reason: &str,
) -> Result<DesiredContent, AvengerWgpuError> {
    if let (Some(resolver), Some(fallback_key)) =
        (config.resolver.as_ref(), slot.fallback_key.as_ref())
    {
        if let ImageResourceState::Ready(image) = resolver.image_state(fallback_key) {
            if image.width == size && image.height == size {
                return Ok(DesiredContent::Image(image));
            }
            push_unique_failed(
                status,
                fallback_key.clone(),
                format!(
                    "Fallback resource image {fallback_key:?} has dimensions ({}, {}), expected ({size}, {size})",
                    image.width, image.height
                ),
            );
        }
    }
    unavailable_content(slot, config, reason)
}

fn unavailable_content(
    slot: &Slot,
    config: &WgpuImageResourceConfig,
    reason: &str,
) -> Result<DesiredContent, AvengerWgpuError> {
    let missing_policy = match slot.policy {
        SceneImageUnavailablePolicy::RendererDefault => config.missing_policy,
        SceneImageUnavailablePolicy::Skip => WgpuMissingImagePolicy::Skip,
        SceneImageUnavailablePolicy::DrawPlaceholder => WgpuMissingImagePolicy::DrawPlaceholder,
        SceneImageUnavailablePolicy::Error => WgpuMissingImagePolicy::Error,
    };
    match missing_policy {
        WgpuMissingImagePolicy::DrawPlaceholder => Ok(DesiredContent::Placeholder),
        WgpuMissingImagePolicy::Skip => Ok(DesiredContent::Empty),
        WgpuMissingImagePolicy::Error => {
            Err(AvengerWgpuError::ImageResourceError(reason.to_string()))
        }
    }
}

fn placeholder_pixels(
    size: u32,
    placeholder: &WgpuImagePlaceholder,
) -> Result<image::RgbaImage, AvengerWgpuError> {
    crate::marks::image::make_placeholder(size, size, placeholder)
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
        pixels,
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

    fn key(name: &str) -> ResourceKey {
        ResourceKey::new(name)
    }

    #[test]
    fn capacity_scales_with_tile_size() {
        assert_eq!(tile_group_capacity(256), 192);
        assert_eq!(tile_group_capacity(512), 48);
        assert_eq!(tile_group_capacity(64), 256); // clamped to WebGL2 max
    }

    #[test]
    fn assign_reuses_layers_across_scenes_and_never_evicts_current_epoch() {
        let mut allocator = TileSlotAllocator::default();
        allocator.begin_scene();
        let policy = SceneImageUnavailablePolicy::Skip;

        let a = allocator.assign(256, &key("a"), None, policy).expect("a");
        let b = allocator.assign(256, &key("b"), None, policy).expect("b");
        assert_ne!(a, b);
        assert!(a >= 1 && b >= 1, "layer 0 is reserved");
        // Same scene, same key → same layer.
        assert_eq!(allocator.assign(256, &key("a"), None, policy), Some(a));

        // Next scene: residents are reused for free.
        allocator.begin_scene();
        assert_eq!(allocator.assign(256, &key("a"), None, policy), Some(a));

        // Saturate a tiny group to exercise eviction.
        let mut allocator = TileSlotAllocator::default();
        allocator.begin_scene();
        allocator.groups.insert(256, SlotGroup::new(3)); // layer 0 + 2 slots
        let a = allocator.assign(256, &key("a"), None, policy).expect("a");
        let b = allocator.assign(256, &key("b"), None, policy).expect("b");
        // Group full of current-epoch slots → overflow (atlas fallback).
        assert_eq!(allocator.assign(256, &key("c"), None, policy), None);

        // New scene referencing only "b": "a" is evictable.
        allocator.begin_scene();
        assert_eq!(allocator.assign(256, &key("b"), None, policy), Some(b));
        let c = allocator.assign(256, &key("c"), None, policy).expect("c");
        assert_eq!(c, a, "LRU slot reused");
        // "a" was evicted; re-assigning it now must fail (group full again).
        assert_eq!(allocator.assign(256, &key("a"), None, policy), None);
    }
}
