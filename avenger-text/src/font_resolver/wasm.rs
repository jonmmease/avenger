use super::FontResolver;
use std::collections::HashSet;

pub struct WasmFontResolver;

impl WasmFontResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WasmFontResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FontResolver for WasmFontResolver {
    fn get_available_font_families(&self) -> HashSet<String> {
        // Font enumeration is not available in WASM/browser context
        panic!("Font enumeration is not available in WASM environment");
    }

    fn select_available_font(&self, fonts: Vec<String>) -> String {
        // In WASM, we can't check font availability, so just return the first font
        // The browser will handle fallback automatically
        fonts
            .into_iter()
            .next()
            .unwrap_or_else(|| "sans-serif".to_string())
    }

    fn resolve_generic_family(&self, generic: &str) -> Option<String> {
        // In WASM, generic families are handled by the browser
        // Return the generic family unchanged
        return Some(generic.to_string());
    }
}
