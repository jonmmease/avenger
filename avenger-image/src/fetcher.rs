use std::sync::Arc;

#[cfg(feature = "image-request")]
use crate::reqwest_fetcher::ReqwestImageFetcher;

use crate::error::AvengerImageError;
use image::DynamicImage;

pub trait ImageFetcher {
    fn fetch_image(&self, url: &str) -> Result<DynamicImage, AvengerImageError>;
}

pub fn make_image_fetcher() -> Result<Arc<dyn ImageFetcher>, AvengerImageError> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "image-request")] {
            Ok(Arc::new(ReqwestImageFetcher::new()))
        } else {
            Err(AvengerImageError::NoImageFetcherConfigured(
                "Image fetching requires the image-reqwest feature flag".to_string()
            ))
        }
    }
}
