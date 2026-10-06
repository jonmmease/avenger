pub mod engine;
pub mod error;
pub mod font_resolver;
pub mod fonts;
pub mod math;
pub mod measurement;
pub mod path;
pub mod pdf;
pub mod rasterization;
pub mod text_edit;
mod text_line;
pub mod types;

pub use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};
pub use avenger_typst_label::{referenced_params, LabelParamValue, LabelParams, RegisteredFont};
pub use engine::{default_text_engine, TextEngine};
pub use font_resolver::{FontResolutionOptions, FontResolver, MissingFontPolicy};
pub use fonts::default_font_resolution;
pub use math::{empty_label_params, label_params_fingerprint};

/// A formatter provider in a cache key, compared and hashed by identity. Holding the `Arc` keeps
/// a dropped provider's address from being reused while the key exists.
pub struct ProviderIdentity<P: ?Sized>(std::sync::Arc<P>);

impl<P: ?Sized> ProviderIdentity<P> {
    pub fn new(provider: &std::sync::Arc<P>) -> Self {
        Self(provider.clone())
    }
}

impl<P: ?Sized> Clone for ProviderIdentity<P> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<P: ?Sized> PartialEq for ProviderIdentity<P> {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<P: ?Sized> Eq for ProviderIdentity<P> {}

impl<P: ?Sized> std::hash::Hash for ProviderIdentity<P> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::sync::Arc::as_ptr(&self.0).cast::<()>().hash(state);
    }
}

impl<P: ?Sized> std::fmt::Debug for ProviderIdentity<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProviderIdentity")
            .field(&std::sync::Arc::as_ptr(&self.0).cast::<()>())
            .finish()
    }
}
