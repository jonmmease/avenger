use crate::error::AvengerVegaError;
use crate::image::ImageFetcher;
use image::DynamicImage;
use reqwest::blocking::{Client, ClientBuilder};

pub struct ReqwestImageFetcher {
    client: Client,
}

static USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

impl Default for ReqwestImageFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestImageFetcher {
    pub fn new() -> Self {
        Self {
            client: ClientBuilder::new()
                .user_agent(USER_AGENT)
                .build()
                .expect("Failed to construct reqwest client"),
        }
    }
}

impl ImageFetcher for ReqwestImageFetcher {
    fn fetch_image(&self, url: &str) -> Result<DynamicImage, AvengerVegaError> {
        let img_data = self.client.get(url).send()?.bytes()?.to_vec();
        let diffuse_image = image::load_from_memory(img_data.as_slice())?;
        Ok(diffuse_image)
    }
}
