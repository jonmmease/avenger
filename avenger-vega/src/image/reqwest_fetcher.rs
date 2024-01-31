use crate::error::AvengerVegaError;
use crate::image::ImageFetcher;
use image::DynamicImage;
use reqwest::blocking::{Client, ClientBuilder};

#[cfg(feature = "svg")]
use crate::image::svg::svg_to_png;

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
        let img_data = if url.ends_with(".svg") {
            cfg_if::cfg_if! {
                if #[cfg(feature = "svg")] {
                    let svg_str = self.client.get(url).send()?.text()?;
                    svg_to_png(&svg_str, 2.0)?
                } else {
                    return Err(AvengerVegaError::SvgSupportDisabled(
                        "Fetching SVG images requires the svg feature flag".to_string()
                    ))
                }
            }
        } else {
            self.client.get(url).send()?.bytes()?.to_vec()
        };
        let diffuse_image = image::load_from_memory(img_data.as_slice())?;
        Ok(diffuse_image)
    }
}
