use crate::canvas::CanvasDimensions;
use crate::error::AvengerWgpuError;
use crate::marks::basic_mark::{BasicMarkBatch, BasicMarkShader};
use avenger::marks::group::GroupBounds;
use avenger::marks::image::ImageMark;
use avenger::marks::value::{ImageAlign, ImageBaseline};
use etagere::Size;
use itertools::izip;
use wgpu::{Extent3d, FilterMode, VertexBufferLayout};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImageUniform {
    pub size: [f32; 2],
    pub origin: [f32; 2],
    pub group_size: [f32; 2],
    pub scale: f32,
    pub clip: f32,
}

impl ImageUniform {
    pub fn new(dimensions: CanvasDimensions, group_bounds: GroupBounds, clip: bool) -> Self {
        Self {
            size: dimensions.size,
            scale: dimensions.scale,
            origin: [group_bounds.x, group_bounds.y],
            group_size: [
                group_bounds.width.unwrap_or(0.0),
                group_bounds.height.unwrap_or(0.0),
            ],
            clip: if clip && group_bounds.width.is_some() && group_bounds.height.is_some() {
                1.0
            } else {
                0.0
            },
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImageVertex {
    pub position: [f32; 2],
    pub tex_coord: [f32; 2],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
    0 => Float32x2,     // position
    1 => Float32x2,     // tex_coord
];

impl ImageVertex {
    pub fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<ImageVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }
    }
}

pub struct ImageShader {
    verts: Vec<ImageVertex>,
    indices: Vec<u16>,
    uniform: ImageUniform,
    smooth: bool,
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
    batches: Vec<BasicMarkBatch>,
    texture_size: Extent3d,
}

impl ImageShader {
    pub fn from_image_mark(
        mark: &ImageMark,
        dimensions: CanvasDimensions,
        group_bounds: GroupBounds,
    ) -> Result<Self, AvengerWgpuError> {
        let mut verts: Vec<ImageVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        let mut batches: Vec<BasicMarkBatch> = Vec::new();
        let aspect = mark.aspect;

        // Compute texture size
        let limits = wgpu::Limits::downlevel_webgl2_defaults();
        let texture_size = Extent3d {
            width: limits.max_texture_dimension_1d,
            height: limits.max_texture_dimension_2d,
            depth_or_array_layers: 1,
        };

        // Allocate image for texture
        let mut texture_image = image::RgbaImage::new(texture_size.width, texture_size.height);
        let mut atlas_allocator = etagere::AtlasAllocator::new(Size::new(
            texture_size.width as i32,
            texture_size.height as i32,
        ));

        let mut start_index = indices.len() as u32;
        for (img, x, y, width, height, baseline, align) in izip!(
            mark.image_iter(),
            mark.x_iter(),
            mark.y_iter(),
            mark.width_iter(),
            mark.height_iter(),
            mark.baseline_iter(),
            mark.align_iter(),
        ) {
            let Some(rgba_image) = img.to_image() else {
                continue;
            };

            let allocation = match atlas_allocator
                .allocate(Size::new(img.width as i32, img.height as i32))
            {
                Some(allocation) => allocation,
                None => {
                    // Current allocator is full
                    // Add previous batch
                    batches.push(BasicMarkBatch {
                        indices_range: start_index..indices.len() as u32,
                        image: Some(image::DynamicImage::ImageRgba8(texture_image)),
                    });

                    // create new allocator, new texture image, new batch
                    atlas_allocator = etagere::AtlasAllocator::new(Size::new(
                        texture_size.width as i32,
                        texture_size.height as i32,
                    ));

                    let Some(allocation) =
                        atlas_allocator.allocate(Size::new(img.width as i32, img.height as i32))
                    else {
                        if img.width > texture_size.width || img.height > texture_size.height {
                            return Err(AvengerWgpuError::ImageAllocationError(format!(
                                "Image dimensions ({}, {}) exceed the maximum size of ({}, {})",
                                img.width, img.height, texture_size.width, texture_size.height
                            )));
                        } else {
                            return Err(AvengerWgpuError::ImageAllocationError(
                                "Unknown error".to_string(),
                            ));
                        }
                    };

                    // Create a new texture image
                    texture_image = image::RgbaImage::new(texture_size.width, texture_size.height);

                    // update start_index
                    start_index = indices.len() as u32;

                    allocation
                }
            };

            // Write image to allocated portion of final texture image
            let p0 = allocation.rectangle.min;
            let p1 = allocation.rectangle.max;
            let x0 = p0.x;
            let x1 = p1.x.min(x0 + img.width as i32);
            let y0 = p0.y;
            let y1 = p1.y.min(y0 + img.height as i32);
            for (src_x, dest_x) in (x0..x1).enumerate() {
                for (src_y, dest_y) in (y0..y1).enumerate() {
                    texture_image.put_pixel(
                        dest_x as u32,
                        dest_y as u32,
                        *rgba_image.get_pixel(src_x as u32, src_y as u32),
                    );
                }
            }

            // Compute texture coordinates
            let tex_x0 = x0 as f32 / texture_size.width as f32;
            let tex_x1 = x1 as f32 / texture_size.width as f32;
            let tex_y0 = y0 as f32 / texture_size.height as f32;
            let tex_y1 = y1 as f32 / texture_size.height as f32;

            // Vertex index offset
            let offset = verts.len() as u16;

            // Compute image left
            let left = match *align {
                ImageAlign::Left => *x,
                ImageAlign::Center => *x - *width / 2.0,
                ImageAlign::Right => *x - *width,
            };
            // Compute image top
            let top = match *baseline {
                ImageBaseline::Top => *y,
                ImageBaseline::Middle => *y - *height / 2.0,
                ImageBaseline::Bottom => *y - *height,
            };

            // Adjust position and dimensions if aspect ratio should be preserved
            let (left, top, width, height) = if aspect {
                let img_aspect = img.width as f32 / img.height as f32;
                let outline_aspect = *width / *height;
                if img_aspect > outline_aspect {
                    // image is wider than the box, so we scale
                    // image to box width and center vertically
                    let aspect_height = *width / img_aspect;
                    let aspect_top = top + (*height - aspect_height) / 2.0;
                    (left, aspect_top, *width, aspect_height)
                } else if img_aspect < outline_aspect {
                    // image is taller than the box, so we scale
                    // image to box height an center horizontally
                    let aspect_width = *height * img_aspect;
                    let aspect_left = left + (*width - aspect_width) / 2.0;
                    (aspect_left, top, aspect_width, *height)
                } else {
                    (left, top, *width, *height)
                }
            } else {
                (left, top, *width, *height)
            };

            verts.push(ImageVertex {
                position: [left, top],
                tex_coord: [tex_x0, tex_y0],
            });
            // Lower left
            verts.push(ImageVertex {
                position: [left, top + height],
                tex_coord: [tex_x0, tex_y1],
            });
            // Lower right
            verts.push(ImageVertex {
                position: [left + width, top + height],
                tex_coord: [tex_x1, tex_y1],
            });
            // Upper right
            verts.push(ImageVertex {
                position: [left + width, top],
                tex_coord: [tex_x1, tex_y0],
            });

            // Indices
            indices.push(offset);
            indices.push(offset + 1);
            indices.push(offset + 2);

            indices.push(offset);
            indices.push(offset + 2);
            indices.push(offset + 3);
        }
        batches.push(BasicMarkBatch {
            indices_range: start_index..indices.len() as u32,
            image: Some(image::DynamicImage::ImageRgba8(texture_image)),
        });

        Ok(Self {
            verts,
            indices,
            uniform: ImageUniform::new(dimensions, group_bounds, mark.clip),
            smooth: mark.smooth,
            batches,
            texture_size,
            shader: include_str!("image.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }
}

impl BasicMarkShader for ImageShader {
    type Vertex = ImageVertex;
    type Uniform = ImageUniform;

    fn verts(&self) -> &[Self::Vertex] {
        self.verts.as_slice()
    }

    fn indices(&self) -> &[u16] {
        self.indices.as_slice()
    }

    fn uniform(&self) -> Self::Uniform {
        self.uniform
    }

    fn shader(&self) -> &str {
        self.shader.as_str()
    }

    fn vertex_entry_point(&self) -> &str {
        self.vertex_entry_point.as_str()
    }

    fn fragment_entry_point(&self) -> &str {
        self.fragment_entry_point.as_str()
    }

    fn vertex_desc(&self) -> VertexBufferLayout<'static> {
        ImageVertex::desc()
    }

    fn batches(&self) -> &[BasicMarkBatch] {
        self.batches.as_slice()
    }

    fn texture_size(&self) -> Extent3d {
        self.texture_size
    }

    fn mag_filter(&self) -> FilterMode {
        if self.smooth {
            FilterMode::Linear
        } else {
            FilterMode::Nearest
        }
    }
}
