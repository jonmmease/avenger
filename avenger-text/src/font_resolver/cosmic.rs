use std::collections::{HashMap, HashSet};

use cosmic_text::fontdb;

use crate::measurement::cosmic::{FONT_SYSTEM, GENERIC_FAMILIES};
use crate::FontResolutionOptions;

use super::FontResolver;

pub struct CosmicFontResolver {
    available_families: Option<HashSet<String>>,
    generic_families: Option<HashMap<String, String>>,
}

impl CosmicFontResolver {
    pub fn new() -> Self {
        Self {
            available_families: None,
            generic_families: None,
        }
    }

    pub fn with_font_resolution(options: &FontResolutionOptions) -> Self {
        let font_system = crate::fonts::build_cosmic_font_system(options);
        Self {
            available_families: Some(available_families(font_system.db())),
            generic_families: Some(generic_families(font_system.db())),
        }
    }

    fn available_families(&self) -> HashSet<String> {
        if let Some(available_families) = &self.available_families {
            return available_families.clone();
        }

        let font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");
        available_families(font_system.db())
    }

    fn generic_families(&self) -> HashMap<String, String> {
        if let Some(generic_families) = &self.generic_families {
            return generic_families.clone();
        }

        GENERIC_FAMILIES.lock().unwrap().clone()
    }
}

impl Default for CosmicFontResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FontResolver for CosmicFontResolver {
    fn get_available_font_families(&self) -> HashSet<String> {
        self.available_families()
    }

    fn select_available_font(&self, fonts: Vec<String>) -> String {
        let available = self.available_families();
        let generic_families = self.generic_families();

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
        let generic_families = self.generic_families();

        let normalized = match generic.to_lowercase().as_str() {
            "sans serif" => "sans-serif".to_string(),
            other => other.to_string(),
        };

        generic_families.get(&normalized).cloned()
    }
}

fn available_families(fontdb: &fontdb::Database) -> HashSet<String> {
    fontdb
        .faces()
        .flat_map(|face| {
            face.families
                .iter()
                .map(|(fam, _lang)| fam.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn generic_families(fontdb: &fontdb::Database) -> HashMap<String, String> {
    let mut families = HashMap::new();
    for (generic, family) in [
        ("sans-serif", fontdb::Family::SansSerif),
        ("serif", fontdb::Family::Serif),
        ("monospace", fontdb::Family::Monospace),
        ("cursive", fontdb::Family::Cursive),
        ("fantasy", fontdb::Family::Fantasy),
    ] {
        if let Some(resolved) = resolve_family(fontdb, family) {
            families.insert(generic.to_string(), resolved);
        }
    }
    families
}

fn resolve_family(fontdb: &fontdb::Database, family: fontdb::Family<'_>) -> Option<String> {
    let families = [family];
    let query = fontdb::Query {
        families: &families,
        weight: fontdb::Weight(400),
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    };
    let id = fontdb.query(&query)?;
    let face = fontdb.face(id)?;
    face.families.first().map(|(family, _lang)| family.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_resolver_uses_bundled_sans_serif_when_system_fonts_are_disabled() {
        let resolver = CosmicFontResolver::with_font_resolution(&FontResolutionOptions::default());

        assert_eq!(
            resolver.resolve_generic_family("sans-serif").as_deref(),
            Some("Atkinson Hyperlegible Next")
        );
        assert_eq!(
            resolver.select_available_font(vec!["sans-serif".to_string()]),
            "Atkinson Hyperlegible Next"
        );
        assert_eq!(
            resolver.select_available_font(vec!["Missing Font".to_string()]),
            "Atkinson Hyperlegible Next"
        );
    }

    #[test]
    fn option_resolver_selects_extra_font_dir_families() {
        let caveat_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-vega-test-data/fonts/Caveat/static");
        let resolver = CosmicFontResolver::with_font_resolution(&FontResolutionOptions {
            extra_font_dirs: vec![caveat_dir],
            ..Default::default()
        });

        assert_eq!(
            resolver.select_available_font(vec!["Caveat".to_string(), "sans-serif".to_string()]),
            "Caveat"
        );
    }
}
