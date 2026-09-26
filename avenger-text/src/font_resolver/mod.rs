use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

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
    pub registered_fonts: Vec<avenger_typst_label::RegisteredFont>,
    pub default_sans_serif_family: Option<String>,
    pub default_monospace_family: Option<String>,
    pub default_math_family: Option<String>,
    pub missing_font: MissingFontPolicy,
}

impl Default for FontResolutionOptions {
    fn default() -> Self {
        Self {
            load_system_fonts: true,
            extra_font_dirs: Vec::new(),
            registered_fonts: Vec::new(),
            default_sans_serif_family: None,
            default_monospace_family: None,
            default_math_family: None,
            missing_font: MissingFontPolicy::Error,
        }
    }
}

pub use avenger_typst_label::MissingFontPolicy;

pub struct FontdbFontResolver {
    available_families: HashSet<String>,
    generic_families: HashMap<String, String>,
}

impl FontdbFontResolver {
    pub fn new() -> Self {
        Self::with_font_resolution(&crate::fonts::default_font_resolution())
    }

    pub fn with_font_resolution(options: &FontResolutionOptions) -> Self {
        let fontdb = crate::fonts::build_fontdb(options);
        Self {
            available_families: available_families(&fontdb),
            generic_families: generic_families(&fontdb),
        }
    }
}

impl Default for FontdbFontResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FontResolver for FontdbFontResolver {
    fn get_available_font_families(&self) -> HashSet<String> {
        self.available_families.clone()
    }

    fn select_available_font(&self, fonts: Vec<String>) -> String {
        for font in fonts {
            match normalized_generic_family(&font).as_deref() {
                Some(generic) => {
                    if let Some(resolved) = self.generic_families.get(generic) {
                        return resolved.clone();
                    }
                }
                None => {
                    if self.available_families.contains(&font) {
                        return font;
                    }
                }
            }
        }

        self.generic_families
            .get("sans-serif")
            .cloned()
            .unwrap_or_else(|| "sans-serif".to_string())
    }

    fn resolve_generic_family(&self, generic: &str) -> Option<String> {
        self.generic_families
            .get(&normalized_generic_family(generic)?)
            .cloned()
    }
}

pub fn default_font_resolver() -> impl FontResolver {
    FontdbFontResolver::new()
}

fn available_families(fontdb: &fontdb::Database) -> HashSet<String> {
    fontdb
        .faces()
        .flat_map(|face| {
            face.families
                .iter()
                .map(|(family, _lang)| family.clone())
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
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    };
    let id = fontdb.query(&query)?;
    let face = fontdb.face(id)?;
    face.families.first().map(|(family, _lang)| family.clone())
}

fn normalized_generic_family(generic: &str) -> Option<String> {
    match generic.to_lowercase().as_str() {
        "sans-serif" | "sans serif" => Some("sans-serif".to_string()),
        "serif" => Some("serif".to_string()),
        "monospace" => Some("monospace".to_string()),
        "cursive" => Some("cursive".to_string()),
        "fantasy" => Some("fantasy".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_resolver_uses_system_fonts_by_default() {
        let resolver = FontdbFontResolver::with_font_resolution(&FontResolutionOptions::default());

        let mut system = fontdb::Database::new();
        system.load_system_fonts();
        // Generic aliases depend on the host's fontconfig setup. This test
        // checks discovery, including machines with no system fonts.
        assert_eq!(
            resolver.get_available_font_families(),
            available_families(&system)
        );
    }

    #[test]
    fn default_resolver_uses_bundled_lato_for_sans_serif() {
        let resolver = default_font_resolver();

        assert_eq!(
            resolver.resolve_generic_family("sans-serif").as_deref(),
            Some("Lato")
        );
        assert_eq!(
            resolver.select_available_font(vec!["sans-serif".to_string()]),
            "Lato"
        );
    }

    #[test]
    fn option_resolver_uses_generic_name_when_fonts_are_disabled() {
        let resolver = FontdbFontResolver::with_font_resolution(&FontResolutionOptions {
            load_system_fonts: false,
            ..Default::default()
        });

        assert_eq!(resolver.resolve_generic_family("sans-serif"), None);
        assert_eq!(
            resolver.select_available_font(vec!["Missing Font".to_string()]),
            "sans-serif"
        );
    }

    #[test]
    fn option_resolver_selects_extra_font_dir_families() {
        let caveat_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-vega-test-data/fonts/Caveat/static");
        let resolver = FontdbFontResolver::with_font_resolution(&FontResolutionOptions {
            extra_font_dirs: vec![caveat_dir],
            ..Default::default()
        });

        assert_eq!(
            resolver.select_available_font(vec!["Caveat".to_string(), "sans-serif".to_string()]),
            "Caveat"
        );
    }
}
