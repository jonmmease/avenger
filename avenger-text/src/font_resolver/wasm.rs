use std::collections::HashSet;

use crate::FontResolutionOptions;

use super::FontResolver;

const BUNDLED_SANS_SERIF_FAMILY: &str = "Atkinson Hyperlegible Next";

pub struct WasmFontResolver {
    use_browser_fonts: bool,
}

impl WasmFontResolver {
    pub fn new() -> Self {
        Self::with_font_resolution(&FontResolutionOptions::default())
    }

    pub fn with_font_resolution(options: &FontResolutionOptions) -> Self {
        Self {
            use_browser_fonts: options.load_system_fonts,
        }
    }

    fn is_generic_family(font: &str) -> bool {
        matches!(
            font.to_lowercase().as_str(),
            "serif" | "sans-serif" | "sans serif" | "monospace" | "cursive" | "fantasy"
        )
    }
}

impl Default for WasmFontResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FontResolver for WasmFontResolver {
    fn get_available_font_families(&self) -> HashSet<String> {
        let mut families = HashSet::new();
        if !self.use_browser_fonts {
            families.insert(BUNDLED_SANS_SERIF_FAMILY.to_string());
        }
        families
    }

    fn select_available_font(&self, fonts: Vec<String>) -> String {
        if self.use_browser_fonts {
            // Browser-backed mode cannot enumerate fonts; preserve the requested
            // family chain and let the browser choose the concrete face.
            return fonts
                .into_iter()
                .next()
                .unwrap_or_else(|| "sans-serif".to_string());
        }

        for font in fonts {
            if font == BUNDLED_SANS_SERIF_FAMILY || Self::is_generic_family(&font) {
                return BUNDLED_SANS_SERIF_FAMILY.to_string();
            }
        }

        BUNDLED_SANS_SERIF_FAMILY.to_string()
    }

    fn resolve_generic_family(&self, generic: &str) -> Option<String> {
        if self.use_browser_fonts {
            // In browser-backed mode, generic families are handled by the browser.
            return Some(generic.to_string());
        }

        if Self::is_generic_family(generic) {
            Some(BUNDLED_SANS_SERIF_FAMILY.to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_resolver_uses_bundled_sans_serif() {
        let resolver = WasmFontResolver::new();

        assert_eq!(
            resolver.resolve_generic_family("sans-serif").as_deref(),
            Some(BUNDLED_SANS_SERIF_FAMILY)
        );
        assert_eq!(
            resolver.select_available_font(vec!["Missing Font".to_string(), "serif".to_string()]),
            BUNDLED_SANS_SERIF_FAMILY
        );
        assert!(resolver
            .get_available_font_families()
            .contains(BUNDLED_SANS_SERIF_FAMILY));
    }

    #[test]
    fn system_font_mode_delegates_to_browser() {
        let resolver = WasmFontResolver::with_font_resolution(&FontResolutionOptions {
            load_system_fonts: true,
            ..Default::default()
        });

        assert_eq!(
            resolver.resolve_generic_family("sans-serif").as_deref(),
            Some("sans-serif")
        );
        assert_eq!(
            resolver.select_available_font(vec!["Inter".to_string(), "sans-serif".to_string()]),
            "Inter"
        );
        assert!(resolver.get_available_font_families().is_empty());
    }
}
