pub mod error;
pub mod fetcher;
pub mod resource_cache;
#[cfg(not(target_arch = "wasm32"))]
mod scheduler;

#[cfg(all(feature = "reqwest", not(target_arch = "wasm32")))]
pub mod reqwest_fetcher;

#[cfg(feature = "svg")]
pub mod svg;

use std::sync::Arc;

use avenger_resource::ResourceKey;
use base64::{prelude::BASE64_STANDARD, Engine};
use serde::{Deserialize, Serialize};

use error::AvengerImageError;
pub use fetcher::make_image_fetcher;
use fetcher::ImageFetcher;
pub use resource_cache::{
    load_image_resource_requests_blocking, ImageResourceCache, ImageResourceLoadError,
    ImageResourceLoadOptions, DEFAULT_IMAGE_RESOURCE_CACHE_CAPACITY, IMAGE_RESOURCE_KIND,
};

#[derive(Debug, Clone, Default, PartialEq, Hash, Serialize, Deserialize)]
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
            let fetcher = fetcher.map(Ok).unwrap_or_else(make_image_fetcher)?;
            let img = fetcher.fetch_image(s)?;
            Ok(Self::from_image(&img.into_rgba8()))
        } else {
            Err(AvengerImageError::InternalError(format!(
                "Unsupported image URL: {s}"
            )))
        }
    }
}

#[derive(Debug, Clone)]
pub enum ImageResourceState {
    Ready(Arc<RgbaImage>),
    Pending,
    Missing,
    Failed(Arc<str>),
}

/// Retains a resolver's working set until the guard is dropped.
#[must_use = "keep the lease alive while the images are needed"]
#[derive(Default)]
pub struct ImageResourceLease {
    release: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl ImageResourceLease {
    pub fn new(release: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }
}

impl std::fmt::Debug for ImageResourceLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImageResourceLease")
            .finish_non_exhaustive()
    }
}

impl Drop for ImageResourceLease {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

pub trait ImageResourceResolver: Send + Sync {
    fn image_state(&self, key: &ResourceKey) -> ImageResourceState;

    fn request_image(&self, request: &avenger_resource::ResourceRequest) {
        let _ = request;
    }

    /// Pin keys before requesting them. The default is suitable for resolvers
    /// whose resources do not expire through eviction.
    fn retain_images(&self, _keys: &[ResourceKey]) -> ImageResourceLease {
        ImageResourceLease::default()
    }

    fn generation(&self) -> u64 {
        0
    }
}

/// Required requests precede prefetch, then priority descends, then FIFO.
pub(crate) fn image_request_precedes(
    candidate: &avenger_resource::ResourceRequest,
    candidate_seq: u64,
    current: &avenger_resource::ResourceRequest,
    current_seq: u64,
) -> bool {
    use avenger_resource::ResourceRequestPurpose;
    let candidate_required = candidate.purpose == ResourceRequestPurpose::Required;
    let current_required = current.purpose == ResourceRequestPurpose::Required;
    if candidate_required != current_required {
        return candidate_required;
    }
    if candidate.priority != current.priority {
        return candidate.priority > current.priority;
    }
    candidate_seq < current_seq
}

#[cfg(test)]
mod svg_feature_tests {
    use super::*;

    #[test]
    fn svg_data_urls_follow_the_svg_feature() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="3"><rect width="2" height="3" fill="red"/></svg>"#;
        let urls = [
            format!("data:image/svg+xml,{}", urlencoding::encode(svg)),
            format!("data:image/svg+xml;base64,{}", BASE64_STANDARD.encode(svg)),
        ];
        for url in urls {
            let result = RgbaImage::from_str(&url, None);
            #[cfg(feature = "svg")]
            {
                let image = result.expect("SVG feature should decode data URLs");
                assert_eq!((image.width, image.height), (4, 6));
                assert_eq!(&image.data[..4], &[255, 0, 0, 255]);
            }
            #[cfg(not(feature = "svg"))]
            assert!(matches!(
                result,
                Err(AvengerImageError::SvgSupportDisabled(_))
            ));
        }
    }
}
