use std::sync::{Arc, OnceLock};

#[cfg(all(feature = "reqwest", not(target_arch = "wasm32")))]
use crate::reqwest_fetcher::ReqwestImageFetcher;

use crate::error::AvengerImageError;

/// The validators of a cached response, which make a request conditional: a server whose copy
/// still matches them answers 304 Not Modified without a body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    /// The response's `ETag` header, sent back as `If-None-Match`.
    pub etag: Option<String>,
    /// The response's `Last-Modified` header, sent back as `If-Modified-Since`.
    pub last_modified: Option<String>,
}

/// A server's response to an image request: its status, the headers that say how long the
/// response stays fresh, its validators, and its encoded bytes.
#[derive(Debug, Clone, Default)]
pub struct FetchResponse {
    pub status: u16,
    /// The `Cache-Control` header.
    pub cache_control: Option<String>,
    /// The `Expires` header.
    pub expires: Option<String>,
    /// The `Age` header: how long, in seconds, a shared cache has held the response.
    pub age: Option<String>,
    /// The `Date` header: when the server sent the response.
    pub date: Option<String>,
    /// The `Retry-After` header of a failure: how long to wait before the next request, in
    /// seconds or as a date.
    pub retry_after: Option<String>,
    pub validators: Validators,
    /// The encoded image.
    pub body: Vec<u8>,
}

impl FetchResponse {
    /// Whether the status is a success, 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Fetches encoded images over HTTP. A fetcher is shared by the threads that fetch, and returns
/// every response it receives, whatever its status. It fails only when no response arrives.
pub trait ImageFetcher: Send + Sync {
    /// Fetch `url`, conditionally on `validators` when they aren't empty.
    fn fetch(&self, url: &str, validators: &Validators)
        -> Result<FetchResponse, AvengerImageError>;
}

/// The fetcher that images use unless they name another: reqwest on native targets with the
/// `reqwest` feature, with avenger's User-Agent. It's built once, so every fetch shares its
/// HTTP client and the client's connections. Without the feature, its fetches fail.
pub fn default_image_fetcher() -> Arc<dyn ImageFetcher> {
    static FETCHER: OnceLock<Arc<dyn ImageFetcher>> = OnceLock::new();
    FETCHER
        .get_or_init(|| {
            cfg_if::cfg_if! {
                if #[cfg(all(feature = "reqwest", not(target_arch = "wasm32")))] {
                    match ReqwestImageFetcher::new(
                        crate::reqwest_fetcher::DEFAULT_USER_AGENT,
                        crate::reqwest_fetcher::DEFAULT_TIMEOUT,
                    ) {
                        Ok(fetcher) => Arc::new(fetcher),
                        Err(error) => Arc::new(NoImageFetcher(error.to_string())),
                    }
                } else {
                    Arc::new(NoImageFetcher(
                        "image fetching requires the reqwest feature on native targets".to_string(),
                    ))
                }
            }
        })
        .clone()
}

/// A fetcher whose fetches fail, for builds that can't fetch.
struct NoImageFetcher(String);

impl ImageFetcher for NoImageFetcher {
    fn fetch(&self, _url: &str, _: &Validators) -> Result<FetchResponse, AvengerImageError> {
        Err(AvengerImageError::NoImageFetcherConfigured(self.0.clone()))
    }
}
