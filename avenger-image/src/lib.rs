pub mod cache;
pub mod error;
pub mod fetcher;
pub mod resolver;

#[cfg(all(feature = "reqwest", not(target_arch = "wasm32")))]
pub mod reqwest_fetcher;

#[cfg(feature = "svg")]
pub mod svg;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod disk;

use std::sync::Arc;

use base64::{prelude::BASE64_STANDARD, Engine};
pub use cache::{ImageCache, ImageCacheOptions};
#[cfg(not(target_arch = "wasm32"))]
pub use disk::DiskCacheOptions;
use error::AvengerImageError;
pub use fetcher::{default_image_fetcher, FetchResponse, ImageFetcher, Validators};
pub use resolver::{ImageLease, ImageResolver, ImageState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Hash, Serialize, Deserialize)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// Whether the image has pixels, and its data holds four bytes for each of them.
    pub fn has_pixels(&self) -> bool {
        self.width > 0
            && self.height > 0
            && (self.width as usize)
                .checked_mul(self.height as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                == Some(self.data.len())
    }

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
        } else if let Some(_data) = s.strip_prefix("data:image/svg+xml;base64,") {
            cfg_if::cfg_if! {
                if #[cfg(feature = "svg")] {
                    let decoded = BASE64_STANDARD.decode(_data)?;
                    let svg_str = String::from_utf8(decoded)?;
                    let png_data = svg::svg_to_png(&svg_str, 2.0)?;
                    let img = image::load_from_memory(&png_data)?;
                    Ok(Self::from_image(&img.into_rgba8()))
                } else {
                    Err(AvengerImageError::SvgSupportDisabled("SVG support not enabled".to_string()))
                }
            }
        } else if let Some(_data) = s.strip_prefix("data:image/svg+xml,") {
            cfg_if::cfg_if! {
                if #[cfg(feature = "svg")] {
                    let svg_str = urlencoding::decode(_data)?;
                    let png_data = svg::svg_to_png(svg_str.as_ref(), 2.0)?;
                    let img = image::load_from_memory(&png_data)?;
                    Ok(Self::from_image(&img.into_rgba8()))
                } else {
                    Err(AvengerImageError::SvgSupportDisabled("SVG support not enabled".to_string()))
                }
            }
        } else if s.starts_with("http://") || s.starts_with("https://") {
            let fetcher = fetcher.unwrap_or_else(default_image_fetcher);
            let response = fetcher.fetch(s, &Validators::default())?;
            if !response.is_success() {
                return Err(AvengerImageError::HttpStatus {
                    url: s.to_string(),
                    status: response.status,
                });
            }
            Self::decode(s, &response.body)
        } else {
            Err(AvengerImageError::InternalError(format!(
                "Unsupported image URL: {s}"
            )))
        }
    }
}

impl RgbaImage {
    /// Decode an image fetched from `url`: an SVG image when the URL's path ends in `.svg`, which
    /// rasterizes at twice its size, or else a format that the `image` crate reads.
    pub fn decode(url: &str, bytes: &[u8]) -> Result<Self, AvengerImageError> {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        if path.ends_with(".svg") {
            cfg_if::cfg_if! {
                if #[cfg(feature = "svg")] {
                    let svg_str = std::str::from_utf8(bytes).map_err(|error| {
                        AvengerImageError::InternalError(format!("SVG image {url} isn't UTF-8: {error}"))
                    })?;
                    let png_data = svg::svg_to_png(svg_str, 2.0)?;
                    let img = image::load_from_memory(&png_data)?;
                    Ok(Self::from_image(&img.into_rgba8()))
                } else {
                    Err(AvengerImageError::SvgSupportDisabled(
                        "Fetching SVG images requires the svg feature flag".to_string()
                    ))
                }
            }
        } else {
            let img = image::load_from_memory(bytes)?;
            Ok(Self::from_image(&img.into_rgba8()))
        }
    }
}
