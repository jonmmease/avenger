use crate::error::VegaSceneGraphError;
use crate::image::ImageFetcher;
use image::DynamicImage;
use reqwest::blocking::Client;

pub struct ReqwestImageFetcher {
    client: Client,
}

impl ReqwestImageFetcher {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl ImageFetcher for ReqwestImageFetcher {
    fn fetch_image(&self, url: &str) -> Result<DynamicImage, VegaSceneGraphError> {
        let img_data = self.client.get(url).send()?.bytes()?.to_vec();
        let diffuse_image = image::load_from_memory(img_data.as_slice())?;
        Ok(diffuse_image)
    }
}