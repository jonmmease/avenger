use crate::marks::value::{EncodingValue, ImageAlign, ImageBaseline};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImageMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub image: EncodingValue<RgbaImage>,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub width: EncodingValue<f32>,
    pub height: EncodingValue<f32>,
    pub align: EncodingValue<ImageAlign>,
    pub baseline: EncodingValue<ImageBaseline>,
    pub aspect: bool,
    pub smooth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl RgbaImage {
    pub fn to_image(&self) -> Option<image::RgbaImage> {
        image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
    }

    pub fn from_image(img: &image::RgbaImage) -> Self {
        Self {
            width: img.width(),
            height: img.height(),
            data: img.to_vec(),
        }
    }
}
