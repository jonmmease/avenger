#[cfg(feature = "image-request")]
pub mod reqwest_fetcher;

#[cfg(feature = "image-request")]
use crate::image::reqwest_fetcher::ReqwestImageFetcher;

use crate::error::AvengerVegaError;
use image::DynamicImage;

pub trait ImageFetcher {
    fn fetch_image(&self, url: &str) -> Result<DynamicImage, AvengerVegaError>;
}

pub fn make_image_fetcher() -> Result<Box<dyn ImageFetcher>, AvengerVegaError> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "image-request")] {
            Ok(Box::new(ReqwestImageFetcher::new()))
        } else {
            Err(AvengerVegaError::NoImageFetcherConfigured(
                "Image fetching requeres the image-reqwest feature flag".to_string()
            ))
        }
    }
}
