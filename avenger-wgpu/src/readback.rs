use image::imageops::crop_imm;
use wgpu::{
    Buffer, BufferAddress, BufferDescriptor, BufferUsages, CommandEncoder, Device, Extent3d,
    MapMode, Origin3d, TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture,
    TextureAspect,
};

use crate::error::AvengerWgpuError;

pub(crate) struct TextureReadback {
    buffer: Buffer,
    texture_extent: Extent3d,
    padded_width: u32,
    padded_height: u32,
}

impl TextureReadback {
    pub(crate) fn new(device: &Device, texture_extent: Extent3d) -> Self {
        let padded_width = align_to_256(texture_extent.width);
        let padded_height = align_to_256(texture_extent.height);
        let u32_size = std::mem::size_of::<u32>() as u32;
        let output_buffer_size = (u32_size * padded_width * padded_height) as BufferAddress;

        let buffer = device.create_buffer(&BufferDescriptor {
            size: output_buffer_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            label: Some("Avenger Texture Readback Buffer"),
            mapped_at_creation: false,
        });

        Self {
            buffer,
            texture_extent,
            padded_width,
            padded_height,
        }
    }

    pub(crate) fn texture_extent(&self) -> Extent3d {
        self.texture_extent
    }

    pub(crate) fn encode_copy_from_texture(&self, encoder: &mut CommandEncoder, texture: &Texture) {
        let u32_size = std::mem::size_of::<u32>() as u32;

        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                aspect: TextureAspect::All,
                texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
            },
            TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(u32_size * self.padded_width),
                    rows_per_image: Some(self.padded_height),
                },
            },
            self.texture_extent,
        );
    }

    pub(crate) async fn read_rgba8(
        &self,
        device: &Device,
        crop_width: u32,
        crop_height: u32,
    ) -> Result<image::RgbaImage, AvengerWgpuError> {
        let buffer_slice = self.buffer.slice(..);

        // The poll must happen after map_async is registered and before awaiting
        // the callback, otherwise native PNG rendering can hang.
        let (tx, rx) = futures_intrusive::channel::shared::oneshot_channel();
        buffer_slice.map_async(MapMode::Read, move |result| {
            tx.send(result).ok();
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

        let map_result = rx.receive().await.ok_or_else(|| {
            AvengerWgpuError::ConversionError("Texture readback callback was dropped".to_string())
        })?;
        map_result.map_err(|err| {
            AvengerWgpuError::ConversionError(format!("Texture readback map failed: {err}"))
        })?;

        let data = buffer_slice.get_mapped_range();
        let maybe_img =
            image::RgbaImage::from_vec(self.padded_width, self.padded_height, data.to_vec());
        drop(data);
        self.buffer.unmap();

        let img_buf = maybe_img.ok_or_else(|| {
            AvengerWgpuError::ImageAllocationError(
                "Failed to allocate texture readback image".to_string(),
            )
        })?;

        Ok(crop_imm(&img_buf, 0, 0, crop_width, crop_height).to_image())
    }
}

fn align_to_256(value: u32) -> u32 {
    256 * value.div_ceil(256)
}
