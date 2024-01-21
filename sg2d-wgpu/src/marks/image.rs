use crate::error::Sg2dWgpuError;
use crate::marks::texture_mark::{TextureMarkBatch, TextureMarkShader};
use etagere::Size;
use itertools::izip;
use sg2d::marks::image::ImageMark;
use wgpu::{Extent3d, VertexBufferLayout};

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
    shader: String,
    vertex_entry_point: String,
    fragment_entry_point: String,
    batches: Vec<TextureMarkBatch>,
    texture_size: Extent3d,
}

impl ImageShader {
    pub fn from_image_mark(mark: &ImageMark) -> Result<Self, Sg2dWgpuError> {
        let mut verts: Vec<ImageVertex> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        let mut batches: Vec<TextureMarkBatch> = Vec::new();
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
            if let Some(allocation) =
                atlas_allocator.allocate(Size::new(img.width as i32, img.height as i32))
            {
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

                let tex_x0 = x0 as f32 / texture_size.width as f32;
                let tex_x1 = x1 as f32 / texture_size.width as f32;
                let tex_y0 = y0 as f32 / texture_size.height as f32;
                let tex_y1 = y1 as f32 / texture_size.height as f32;

                // Vertex index offset
                let offset = verts.len() as u16;

                // Upper left
                verts.push(ImageVertex {
                    position: [*x, *y],
                    tex_coord: [tex_x0, tex_y0],
                });
                // Lower left
                verts.push(ImageVertex {
                    position: [*x, *y + *height],
                    tex_coord: [tex_x0, tex_y1],
                });
                // Lower right
                verts.push(ImageVertex {
                    position: [*x + *width, *y + *height],
                    tex_coord: [tex_x1, tex_y1],
                });
                // Upper right
                verts.push(ImageVertex {
                    position: [*x + *width, *y],
                    tex_coord: [tex_x1, tex_y0],
                });

                // Indices
                indices.push(offset);
                indices.push(offset + 1);
                indices.push(offset + 2);

                indices.push(offset);
                indices.push(offset + 2);
                indices.push(offset + 3);
            } else {
                // TODO: reallocate, create new batch
                todo!()
            }
        }
        let stop_index = indices.len() as u32;

        batches.push(TextureMarkBatch {
            indices: start_index..stop_index,
            image: image::DynamicImage::ImageRgba8(texture_image),
        });

        Ok(Self {
            verts,
            indices,
            batches,
            texture_size,
            shader: include_str!("image.wgsl").to_string(),
            vertex_entry_point: "vs_main".to_string(),
            fragment_entry_point: "fs_main".to_string(),
        })
    }
}

impl TextureMarkShader for ImageShader {
    type Vertex = ImageVertex;

    fn verts(&self) -> &[Self::Vertex] {
        self.verts.as_slice()
    }

    fn indices(&self) -> &[u16] {
        self.indices.as_slice()
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

    fn batches(&self) -> &[TextureMarkBatch] {
        self.batches.as_slice()
    }

    fn texture_size(&self) -> Extent3d {
        self.texture_size
    }
}
