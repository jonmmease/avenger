use std::{collections::HashSet, path::PathBuf};

#[cfg(feature = "cosmic-text")]
pub mod cosmic;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Trait for font availability and resolution
pub trait FontResolver: Send + Sync {
    /// Get all available font families on the system
    fn get_available_font_families(&self) -> HashSet<String>;

    /// Select the first available font from a list of font family names
    /// Returns the actual system font name, resolving generic families like "sans-serif"
    fn select_available_font(&self, fonts: Vec<String>) -> String;

    /// Resolve a generic font family (serif, sans-serif, etc.) to a specific system font
    fn resolve_generic_family(&self, generic: &str) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontResolutionOptions {
    pub load_system_fonts: bool,
    pub extra_font_dirs: Vec<PathBuf>,
    pub missing_font: MissingFontPolicy,
}

impl Default for FontResolutionOptions {
    fn default() -> Self {
        Self {
            load_system_fonts: false,
            extra_font_dirs: Vec::new(),
            missing_font: MissingFontPolicy::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingFontPolicy {
    Error,
    Warn,
    Fallback,
}

#[cfg(all(feature = "cosmic-text", not(target_arch = "wasm32")))]
pub fn default_font_resolver() -> impl FontResolver {
    crate::font_resolver::cosmic::CosmicFontResolver::new()
}

#[cfg(target_arch = "wasm32")]
pub fn default_font_resolver() -> impl FontResolver {
    crate::font_resolver::wasm::WasmFontResolver::new()
}
