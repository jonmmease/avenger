use serde::{Deserialize, Serialize};
use crate::marks::value::{EncodingValue, ImageAlign};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImageMark {
    image: EncodingValue<RgbaImage>,
    x: EncodingValue<f32>,
    y: EncodingValue<f32>,
    width: EncodingValue<f32>,
    height: EncodingValue<f32>,
    aspect: bool,
    smooth: bool,
    align: ImageAlign,
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

    pub fn from_image(&self, img: &image::RgbaImage) -> Self {
        Self {
            width: img.width(),
            height: img.height(),
            data: img.to_vec(),
        }
    }
}

