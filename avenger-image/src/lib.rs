pub mod error;
pub mod fetcher;

#[cfg(feature = "image-request")]
pub mod reqwest_fetcher;

#[cfg(feature = "svg")]
pub mod svg;

pub use fetcher::make_image_fetcher;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
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
