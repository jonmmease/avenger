//! Mesh image textures that stay on the GPU between scenes: for each image size, texture arrays
//! with a layer for each image.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use avenger_image::RgbaImage;
use wgpu::{BindGroup, BindGroupLayout, Device, Extent3d, Queue};

use crate::error::AvengerWgpuError;

/// The bytes that one texture array of a size holds, which sets its layers. Small arrays free
/// GPU memory soon after their images go.
const ARRAY_BUDGET: u64 = 16 << 20;
const MIN_LAYERS: u32 = 2;
/// WebGL2's least `max_texture_array_layers`.
const MAX_LAYERS: u32 = 256;

/// An image's place in the texture arrays: its size's group, the array, and the layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ArraySlot {
    pub(crate) size: [u32; 2],
    pub(crate) array: usize,
    pub(crate) layer: u32,
}

/// Assigns images to layers. An image keeps its layer while it's alive and no scene needs the
/// layer. A scene's new image takes a layer whose image is gone, in the first array that has
/// one, or else the least recently used layer that the scene doesn't draw, or else a layer of
/// another array. An array whose images are all gone, as when the image cache evicts them, is
/// freed.
#[derive(Default)]
struct Allocator {
    groups: HashMap<[u32; 2], Group>,
    /// The scene that installs, counted from the first.
    epoch: u64,
}

struct Group {
    layers: u32,
    /// Each array's layers, or `None` for an array that was freed, whose place a new array
    /// takes.
    arrays: Vec<Option<Vec<Slot>>>,
    /// Each held image's array and layer, by the address of its pixels. A slot's `Weak` keeps
    /// that allocation, so no other image takes the address while the slot holds it, and tells
    /// whether the image is alive.
    by_image: HashMap<usize, (usize, usize)>,
}

#[derive(Clone)]
struct Slot {
    /// The layer's image, or an empty `Weak` for a layer that never held one.
    image: Weak<RgbaImage>,
    last_used: u64,
}

impl Slot {
    fn empty() -> Self {
        Self {
            image: Weak::new(),
            last_used: 0,
        }
    }

    /// Whether the layer holds no live image.
    fn is_free(&self) -> bool {
        self.image.strong_count() == 0
    }
}

impl Group {
    fn slot(&self, array: usize, layer: usize) -> Option<&Slot> {
        self.arrays.get(array)?.as_ref()?.get(layer)
    }
}

impl Allocator {
    fn begin_scene(&mut self) {
        self.epoch += 1;
    }

    /// Whether `slot` holds `image`.
    fn holds(&self, slot: &ArraySlot, image: &Arc<RgbaImage>) -> bool {
        self.groups
            .get(&slot.size)
            .and_then(|group| group.slot(slot.array, slot.layer as usize))
            .and_then(|held| held.image.upgrade())
            .is_some_and(|held| Arc::ptr_eq(&held, image))
    }

    /// The image's slot, and whether its layer must upload the image.
    fn assign(&mut self, image: &Arc<RgbaImage>, layers: u32) -> (ArraySlot, bool) {
        let size = [image.width, image.height];
        let epoch = self.epoch;
        let group = self.groups.entry(size).or_insert_with(|| Group {
            layers,
            arrays: Vec::new(),
            by_image: HashMap::new(),
        });
        let address = Arc::as_ptr(image) as usize;
        let held = group
            .by_image
            .get(&address)
            .copied()
            .filter(|&(array, layer)| {
                group
                    .slot(array, layer)
                    .and_then(|slot| slot.image.upgrade())
                    .is_some_and(|held| Arc::ptr_eq(&held, image))
            });
        let ((array, layer), upload) = match held {
            Some(place) => (place, false),
            None => {
                let present = || {
                    group
                        .arrays
                        .iter()
                        .enumerate()
                        .filter_map(|(array, slots)| slots.as_ref().map(|slots| (array, slots)))
                };
                let free = present().find_map(|(array, slots)| {
                    slots
                        .iter()
                        .position(Slot::is_free)
                        .map(|layer| (array, layer))
                });
                let unused = || {
                    present()
                        .flat_map(|(array, slots)| {
                            slots
                                .iter()
                                .enumerate()
                                .map(move |(layer, slot)| ((array, layer), slot))
                        })
                        .filter(|(_, slot)| slot.last_used < epoch)
                        .min_by_key(|(_, slot)| slot.last_used)
                        .map(|(place, _)| place)
                };
                let (array, layer) = free.or_else(unused).unwrap_or_else(|| {
                    let array = group
                        .arrays
                        .iter()
                        .position(Option::is_none)
                        .unwrap_or(group.arrays.len());
                    let slots = Some(vec![Slot::empty(); group.layers as usize]);
                    if array == group.arrays.len() {
                        group.arrays.push(slots);
                    } else {
                        group.arrays[array] = slots;
                    }
                    (array, 0)
                });
                let slots = group.arrays[array]
                    .as_mut()
                    .expect("the chosen array is present");
                let evicted = slots[layer].image.as_ptr() as usize;
                if group.by_image.get(&evicted) == Some(&(array, layer)) {
                    group.by_image.remove(&evicted);
                }
                slots[layer].image = Arc::downgrade(image);
                group.by_image.insert(address, (array, layer));
                ((array, layer), true)
            }
        };
        if let Some(slots) = group.arrays[array].as_mut() {
            slots[layer].last_used = epoch;
        }
        let slot = ArraySlot {
            size,
            array,
            layer: layer as u32,
        };
        (slot, upload)
    }

    /// Free the arrays whose images are all gone, and the sizes left without arrays. Returns
    /// the freed arrays.
    fn free(&mut self) -> Vec<([u32; 2], usize)> {
        let mut freed = Vec::new();
        self.groups.retain(|&size, group| {
            for (array, slots) in group.arrays.iter_mut().enumerate() {
                if slots
                    .as_ref()
                    .is_some_and(|slots| slots.iter().all(Slot::is_free))
                {
                    *slots = None;
                    freed.push((size, array));
                }
            }
            while group.arrays.last().is_some_and(Option::is_none) {
                group.arrays.pop();
            }
            let arrays = &group.arrays;
            group
                .by_image
                .retain(|_, &mut (array, _)| arrays.get(array).is_some_and(Option::is_some));
            !group.arrays.is_empty()
        });
        freed
    }
}

/// One texture array on the GPU, with a bind group for each filter.
struct GpuArray {
    texture: wgpu::Texture,
    linear: BindGroup,
    nearest: BindGroup,
}

/// The texture arrays of mesh images, kept across scenes by the renderer that draws them. A
/// scene's install assigns its images to layers, and the next prepare frees the arrays whose
/// images are gone, creates the arrays that new layers need, and uploads the images that their
/// layers don't hold. Several installs can come before a prepare, so an upload waits for the
/// next prepare however many installs come first.
#[derive(Default)]
pub(crate) struct TextureArrays {
    allocator: Allocator,
    gpu: HashMap<[u32; 2], Vec<Option<GpuArray>>>,
    /// The layers to upload, with their images, which they keep alive until they upload. A
    /// layer that another image took since is skipped.
    pending: Vec<(ArraySlot, Arc<RgbaImage>)>,
    /// The bytes uploaded since the last report.
    uploaded: u64,
}

impl TextureArrays {
    /// Start a scene's install. Layers that the scene doesn't use can then hold its images.
    pub(crate) fn begin_scene(&mut self) {
        self.allocator.begin_scene();
    }

    /// The layer that draws `image`. An image wider or taller than WebGL2's largest texture is
    /// an error.
    pub(crate) fn assign(&mut self, image: &Arc<RgbaImage>) -> Result<ArraySlot, AvengerWgpuError> {
        let max = wgpu::Limits::downlevel_webgl2_defaults().max_texture_dimension_2d;
        if image.width > max || image.height > max {
            return Err(AvengerWgpuError::ImageAllocationError(format!(
                "Image dimensions ({}, {}) exceed the maximum size of ({max}, {max})",
                image.width, image.height
            )));
        }
        let (slot, upload) = self
            .allocator
            .assign(image, array_layers(image.width, image.height));
        if upload {
            self.pending.push((slot, image.clone()));
        }
        Ok(slot)
    }

    /// Free the arrays whose images are gone, create the arrays that new layers need, and upload
    /// the pending layers.
    pub(crate) fn upload(&mut self, device: &Device, queue: &Queue, layout: &BindGroupLayout) {
        for (size, array) in self.allocator.free() {
            if let Some(gpu) = self
                .gpu
                .get_mut(&size)
                .and_then(|arrays| arrays.get_mut(array))
            {
                *gpu = None;
            }
        }
        let groups = &self.allocator.groups;
        self.gpu.retain(|size, _| groups.contains_key(size));
        for (&size, group) in groups {
            let arrays = self.gpu.entry(size).or_default();
            arrays.truncate(group.arrays.len());
            arrays.resize_with(group.arrays.len(), || None);
            for (gpu, slots) in arrays.iter_mut().zip(&group.arrays) {
                if slots.is_some() && gpu.is_none() {
                    *gpu = Some(make_array(device, layout, size, group.layers));
                }
            }
        }
        for (slot, image) in std::mem::take(&mut self.pending) {
            if !self.allocator.holds(&slot, &image) {
                continue;
            }
            let Some(Some(array)) = self
                .gpu
                .get(&slot.size)
                .and_then(|arrays| arrays.get(slot.array))
            else {
                continue;
            };
            let pixels = premultiplied(&image.data);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &array.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: slot.layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * image.width),
                    rows_per_image: Some(image.height),
                },
                Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
            );
            self.uploaded += pixels.len() as u64;
        }
    }

    /// The bind group of a slot's array, with a linear or a nearest sampler.
    pub(crate) fn bind_group(&self, slot: &ArraySlot, smooth: bool) -> Option<&BindGroup> {
        let array = self.gpu.get(&slot.size)?.get(slot.array)?.as_ref()?;
        Some(if smooth {
            &array.linear
        } else {
            &array.nearest
        })
    }

    /// The bytes uploaded since the last call.
    pub(crate) fn take_uploaded(&mut self) -> u64 {
        std::mem::take(&mut self.uploaded)
    }

    /// The bytes of the arrays on the GPU.
    pub(crate) fn resident_bytes(&self) -> u64 {
        self.gpu
            .iter()
            .map(|(size, arrays)| {
                let layers = self
                    .allocator
                    .groups
                    .get(size)
                    .map_or(0, |group| group.layers);
                let present = arrays.iter().filter(|array| array.is_some()).count() as u64;
                present * u64::from(layers) * 4 * u64::from(size[0]) * u64::from(size[1])
            })
            .sum()
    }
}

/// The pixels with their colors multiplied by their alpha.
fn premultiplied(data: &[u8]) -> Vec<u8> {
    data.chunks_exact(4)
        .flat_map(|pixel| {
            crate::marks::image::premultiplied(image::Rgba([
                pixel[0], pixel[1], pixel[2], pixel[3],
            ]))
            .0
        })
        .collect()
}

/// The layers of a size's arrays, which fill [`ARRAY_BUDGET`]. A square array never has a
/// multiple of six layers, since wgpu's GL backend makes such a texture a cube map, which its
/// 2D array view can't sample.
fn array_layers(width: u32, height: u32) -> u32 {
    let layer_bytes = 4 * u64::from(width) * u64::from(height);
    let layers = (ARRAY_BUDGET / layer_bytes.max(1)).clamp(MIN_LAYERS.into(), MAX_LAYERS.into());
    let layers = layers as u32;
    if width == height && layers.is_multiple_of(6) {
        layers - 1
    } else {
        layers
    }
}

fn make_array(device: &Device, layout: &BindGroupLayout, size: [u32; 2], layers: u32) -> GpuArray {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        size: Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        label: Some("mesh_image_array"),
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let bind_group = |filter: wgpu::FilterMode| {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter,
            min_filter: filter,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
            label: Some("mesh_image_array_bind_group"),
        })
    };
    GpuArray {
        linear: bind_group(wgpu::FilterMode::Linear),
        nearest: bind_group(wgpu::FilterMode::Nearest),
        texture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> Arc<RgbaImage> {
        Arc::new(RgbaImage {
            width,
            height,
            data: vec![0; (4 * width * height) as usize],
        })
    }

    /// No square size gets an array that wgpu's GL backend would take for a cube map.
    #[test]
    fn square_arrays_are_never_cube_maps() {
        for size in 1..=2048 {
            let layers = array_layers(size, size);
            assert!(!layers.is_multiple_of(6), "{size}px: {layers} layers");
            assert!((MIN_LAYERS..=MAX_LAYERS).contains(&layers), "{size}px");
        }
        assert_eq!(array_layers(256, 256), 64);
        assert_eq!(array_layers(240, 240), 71);
    }

    /// An image keeps its layer from scene to scene, and only new images upload.
    #[test]
    fn images_keep_their_layers_across_scenes() {
        let mut allocator = Allocator::default();
        let images: Vec<_> = (0..3).map(|_| image(2, 2)).collect();
        allocator.begin_scene();
        let first: Vec<_> = images
            .iter()
            .map(|image| allocator.assign(image, 8))
            .collect();
        assert!(first.iter().all(|(_, upload)| *upload));
        allocator.begin_scene();
        for (image, (slot, _)) in images.iter().zip(&first) {
            assert_eq!(allocator.assign(image, 8), (*slot, false));
        }
    }

    /// A scene's new images take the least recently used layers that the scene doesn't use, and
    /// a layer holds its image until another image takes it.
    #[test]
    fn layers_outside_the_scene_hold_its_new_images() {
        let mut allocator = Allocator::default();
        let old: Vec<_> = (0..8).map(|_| image(2, 2)).collect();
        allocator.begin_scene();
        let slots: Vec<_> = old
            .iter()
            .map(|image| allocator.assign(image, 8).0)
            .collect();
        // The next scene keeps the last six and adds two.
        allocator.begin_scene();
        for image in &old[2..] {
            allocator.assign(image, 8);
        }
        assert!(allocator.holds(&slots[0], &old[0]));
        let new: Vec<_> = (0..2).map(|_| image(2, 2)).collect();
        let taken: Vec<_> = new.iter().map(|image| allocator.assign(image, 8)).collect();
        assert_eq!(taken, [(slots[0], true), (slots[1], true)]);
        assert!(!allocator.holds(&slots[0], &old[0]) && allocator.holds(&slots[0], &new[0]));
        assert_eq!(allocator.groups[&[2, 2]].arrays.len(), 1);
        // An evicted image uploads again.
        allocator.begin_scene();
        assert!(allocator.assign(&old[0], 8).1);
    }

    /// A scene with more images of a size than its arrays hold gets another array.
    #[test]
    fn a_full_size_grows_another_array() {
        let mut allocator = Allocator::default();
        let images: Vec<_> = (0..9).map(|_| image(2, 2)).collect();
        allocator.begin_scene();
        let slots: Vec<_> = images
            .iter()
            .map(|image| allocator.assign(image, 8).0)
            .collect();
        assert_eq!(slots[7].array, 0);
        assert_eq!((slots[8].array, slots[8].layer), (1, 0));
        assert_eq!(allocator.groups[&[2, 2]].arrays.len(), 2);
    }

    /// Images of other sizes take layers of their own sizes' arrays.
    #[test]
    fn sizes_have_their_own_arrays() {
        let mut allocator = Allocator::default();
        allocator.begin_scene();
        let (small, _) = allocator.assign(&image(2, 2), 8);
        let (wide, _) = allocator.assign(&image(4, 2), 8);
        assert_eq!((small.size, wide.size), ([2, 2], [4, 2]));
        assert_eq!((small.layer, wide.layer), (0, 0));
    }

    /// A new image takes the layer of an image that's gone before it evicts a live one.
    #[test]
    fn a_gone_images_layer_is_taken_first() {
        let mut allocator = Allocator::default();
        allocator.begin_scene();
        let mut images: Vec<_> = (0..8).map(|_| image(2, 2)).collect();
        let slots: Vec<_> = images
            .iter()
            .map(|image| allocator.assign(image, 8).0)
            .collect();
        images.remove(5);
        allocator.begin_scene();
        let (taken, upload) = allocator.assign(&image(2, 2), 8);
        assert_eq!(taken, slots[5]);
        assert!(upload);
    }

    /// An array frees once its images are gone, and a size frees with its last array. A freed
    /// array's place goes to the next array that the size needs.
    #[test]
    fn arrays_free_when_their_images_are_gone() {
        let mut allocator = Allocator::default();
        allocator.begin_scene();
        let mut images: Vec<_> = (0..9).map(|_| image(2, 2)).collect();
        for image in &images {
            allocator.assign(image, 8);
        }
        assert!(allocator.free().is_empty());
        // The ninth image is the second array's only one, and it outlives the first array's.
        let ninth = images.pop().unwrap();
        drop(images);
        assert_eq!(allocator.free(), [([2, 2], 0)]);
        // A scene that fills the second array places its next array where the first was.
        allocator.begin_scene();
        allocator.assign(&ninth, 8);
        let new: Vec<_> = (0..8).map(|_| image(2, 2)).collect();
        let slots: Vec<_> = new
            .iter()
            .map(|image| allocator.assign(image, 8).0)
            .collect();
        assert!(slots[..7].iter().all(|slot| slot.array == 1));
        assert_eq!(slots[7].array, 0);
        drop((ninth, new));
        assert_eq!(allocator.free(), [([2, 2], 0), ([2, 2], 1)]);
        assert!(allocator.groups.is_empty());
    }
}
