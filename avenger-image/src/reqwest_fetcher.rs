use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{
    HeaderMap, AGE, CACHE_CONTROL, ETAG, EXPIRES, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
};

use crate::error::AvengerImageError;
use crate::fetcher::{FetchResponse, ImageFetcher, Validators};

/// The User-Agent of the default fetcher. Tile servers such as OpenStreetMap's ask each app to
/// identify itself, so an app should set its own.
pub const DEFAULT_USER_AGENT: &str = concat!(
    "avenger/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/jonmmease/avenger)"
);

/// How long the default fetcher waits for a response.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Fetches images with one blocking reqwest client, over rustls. The client keeps its
/// connections, and negotiates HTTP/2 where the server offers it, so one connection carries
/// concurrent fetches.
pub struct ReqwestImageFetcher {
    client: Client,
}

impl ReqwestImageFetcher {
    /// A fetcher that sends `user_agent` and gives up on a response after `timeout`.
    pub fn new(user_agent: &str, timeout: Duration) -> Result<Self, AvengerImageError> {
        Ok(Self {
            client: Client::builder()
                .user_agent(user_agent)
                .timeout(timeout)
                .use_rustls_tls()
                .build()?,
        })
    }
}

impl ImageFetcher for ReqwestImageFetcher {
    fn fetch(
        &self,
        url: &str,
        validators: &Validators,
    ) -> Result<FetchResponse, AvengerImageError> {
        let mut request = self.client.get(url);
        if let Some(etag) = &validators.etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = &validators.last_modified {
            request = request.header(IF_MODIFIED_SINCE, last_modified);
        }
        let response = request.send()?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let header = |name| header_string(&headers, name);
        Ok(FetchResponse {
            status,
            cache_control: header(CACHE_CONTROL),
            expires: header(EXPIRES),
            age: header(AGE),
            validators: Validators {
                etag: header(ETAG),
                last_modified: header(LAST_MODIFIED),
            },
            body: response.bytes()?.to_vec(),
        })
    }
}

fn header_string(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}
