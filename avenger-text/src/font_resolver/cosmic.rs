use std::collections::HashSet;

use crate::measurement::cosmic::{FONT_SYSTEM, GENERIC_FAMILIES};

use super::FontResolver;

pub struct CosmicFontResolver;

impl CosmicFontResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CosmicFontResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FontResolver for CosmicFontResolver {
    fn get_available_font_families(&self) -> HashSet<String> {
        let font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");

        font_system
            .db()
            .faces()
            .flat_map(|face| {
                face.families
                    .iter()
                    .map(|(fam, _lang)| fam.clone())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn select_available_font(&self, fonts: Vec<String>) -> String {
        let available = self.get_available_font_families();
        let generic_families = GENERIC_FAMILIES.lock().unwrap();

        for font in fonts {
            // Handle generic font names by resolving to actual system fonts
            match font.to_lowercase().as_str() {
                "serif" | "sans-serif" | "sans serif" | "monospace" | "cursive" | "fantasy" => {
                    let lowercase = font.to_lowercase();
                    let normalized = if lowercase == "sans serif" {
                        "sans-serif"
                    } else {
                        &lowercase
                    };

                    if let Some(resolved) = generic_families.get(normalized) {
                        return resolved.clone();
                    }
                }
                _ => {
                    // Check if the font is available
                    if available.contains(&font) {
                        return font;
                    }
                }
            }
        }

        // No fonts found, return the system's sans-serif font as fallback
        if let Some(sans_serif) = generic_families.get("sans-serif") {
            sans_serif.clone()
        } else {
            // Last resort fallback
            "Arial".to_string()
        }
    }

    fn resolve_generic_family(&self, generic: &str) -> Option<String> {
        let generic_families = GENERIC_FAMILIES.lock().unwrap();

        let normalized = match generic.to_lowercase().as_str() {
            "sans serif" => "sans-serif".to_string(),
            other => other.to_string(),
        };

        generic_families.get(&normalized).cloned()
    }
}
