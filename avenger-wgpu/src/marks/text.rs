use crate::canvas::CanvasDimensions;
use crate::error::AvengerWgpuError;
use crate::marks::multi::MultiVertex;
use avenger::marks::text::{FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};
use image::DynamicImage;
use wgpu::Extent3d;

pub trait TextAtlasBuilder {
    fn register_text(
        &mut self,
        text: TextInstance,
        dimensions: CanvasDimensions,
    ) -> Result<Vec<TextAtlasRegistration>, AvengerWgpuError>;

    fn build(&self) -> (Extent3d, Vec<DynamicImage>);
}

pub struct NullTextAtlasBuilder;
impl TextAtlasBuilder for NullTextAtlasBuilder {
    fn register_text(
        &mut self,
        _text: TextInstance,
        _dimensions: CanvasDimensions,
    ) -> Result<Vec<TextAtlasRegistration>, AvengerWgpuError> {
        Err(crate::error::AvengerWgpuError::TextNotEnabled(
            "Text support is not enabled".to_string(),
        ))
    }

    fn build(&self) -> (Extent3d, Vec<DynamicImage>) {
        (
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            vec![DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))],
        )
    }
}

#[derive(Clone)]
pub struct TextAtlasRegistration {
    pub atlas_index: usize,
    pub verts: Vec<MultiVertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct TextInstance<'a> {
    pub position: [f32; 2],
    pub text: &'a String,
    pub color: &'a [f32; 4],
    pub align: &'a TextAlignSpec,
    pub angle: f32,
    pub baseline: &'a TextBaselineSpec,
    pub font: &'a String,
    pub font_size: f32,
    pub font_weight: &'a FontWeightSpec,
    pub font_style: &'a FontStyleSpec,
    pub limit: f32,
}
