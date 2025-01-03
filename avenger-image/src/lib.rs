pub mod error;
pub mod fetcher;

#[cfg(feature = "image-request")]
pub mod reqwest_fetcher;

#[cfg(feature = "svg")]
pub mod svg;

use std::sync::Arc;

use base64::{prelude::BASE64_STANDARD, Engine};
use error::AvengerImageError;
pub use fetcher::make_image_fetcher;

use fetcher::ImageFetcher;
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

    /// Convert a url string (remote or inline) to an RgbaImage
    pub fn from_str(
        s: &str,
        fetcher: Option<Arc<dyn ImageFetcher>>,
    ) -> Result<Self, AvengerImageError> {
        if let Some(data) = s.strip_prefix("data:image/png;base64,") {
            let decoded = BASE64_STANDARD.decode(data)?;
            let img = image::load_from_memory(&decoded)?;
            Ok(Self::from_image(&img.into_rgba8()))
        } else if let Some(data) = s.strip_prefix("data:image/svg+xml;base64,") {
            cfg_if::cfg_if! {
                if #[cfg(feature = "svg")] {
                    let decoded = BASE64_STANDARD.decode(data)?;
                    let svg_str = String::from_utf8(decoded)?;
                    let png_data = svg::svg_to_png(&svg_str, 1.0)?;
                    let img = image::load_from_memory(&png_data)?;
                    Ok(Self::from_image(&img.into_rgba8()))
                } else {
                    Err(AvengerImageError::SvgSupportDisabled("SVG support not enabled".to_string()))
                }
            }
        } else if let Some(data) = s.strip_prefix("data:image/svg+xml,") {
            cfg_if::cfg_if! {
                if #[cfg(feature = "svg")] {
                    let svg_str = urlencoding::decode(data)?;
                    let png_data = svg::svg_to_png(svg_str.as_ref(), 1.0)?;
                    let img = image::load_from_memory(&png_data)?;
                    Ok(Self::from_image(&img.into_rgba8()))
                } else {
                    Err(AvengerImageError::SvgSupportDisabled("SVG support not enabled".to_string()))
                }
            }
        } else if s.starts_with("http://") || s.starts_with("https://") {
            let fetcher = fetcher.map(Ok).unwrap_or_else(make_image_fetcher)?;
            let img = fetcher.fetch_image(s)?;
            Ok(Self::from_image(&img.into_rgba8()))
        } else {
            todo!()
        }
    }
}
