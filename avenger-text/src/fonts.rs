// Embedded font data for Atkinson Hyperlegible Next
// This font is designed for improved readability and legibility

// Include the font files at compile time
const ATKINSON_HYPERLEGIBLE_REGULAR: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Regular.ttf"
);
const ATKINSON_HYPERLEGIBLE_BOLD: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Bold.ttf"
);
const ATKINSON_HYPERLEGIBLE_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Italic.ttf"
);
const ATKINSON_HYPERLEGIBLE_BOLD_ITALIC: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-BoldItalic.ttf"
);
const ATKINSON_HYPERLEGIBLE_MEDIUM: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Medium.ttf"
);
const ATKINSON_HYPERLEGIBLE_SEMIBOLD: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-SemiBold.ttf"
);
const ATKINSON_HYPERLEGIBLE_LIGHT: &[u8] = include_bytes!(
    "../../avenger-chart/fonts/Atkinson_Hyperlegible_Next/AtkinsonHyperlegibleNext-Light.ttf"
);

#[cfg(feature = "cosmic-text")]
pub fn load_embedded_fonts(fontdb: &mut cosmic_text::fontdb::Database) {
    // Load all variants of Atkinson Hyperlegible
    fontdb.load_font_data(Vec::from(ATKINSON_HYPERLEGIBLE_REGULAR));
    fontdb.load_font_data(Vec::from(ATKINSON_HYPERLEGIBLE_BOLD));
    fontdb.load_font_data(Vec::from(ATKINSON_HYPERLEGIBLE_ITALIC));
    fontdb.load_font_data(Vec::from(ATKINSON_HYPERLEGIBLE_BOLD_ITALIC));
    fontdb.load_font_data(Vec::from(ATKINSON_HYPERLEGIBLE_MEDIUM));
    fontdb.load_font_data(Vec::from(ATKINSON_HYPERLEGIBLE_SEMIBOLD));
    fontdb.load_font_data(Vec::from(ATKINSON_HYPERLEGIBLE_LIGHT));
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    #[cfg(feature = "cosmic-text")]
    fn test_font_families() {
        use cosmic_text::fontdb::Database;
        use std::collections::HashSet;
        
        let mut fontdb = Database::new();
        load_embedded_fonts(&mut fontdb);
        
        let families: HashSet<String> = fontdb
            .faces()
            .flat_map(|face| {
                face.families
                    .iter()
                    .map(|(fam, _lang)| fam.clone())
                    .collect::<Vec<_>>()
            })
            .collect();
        
        println!("Embedded font families:");
        for family in &families {
            println!("  {}", family);
        }
        
        // Check that Atkinson Hyperlegible is loaded
        let has_atkinson = families.iter().any(|f| 
            f.contains("Atkinson") || f.contains("Hyperlegible")
        );
        assert!(has_atkinson, "Atkinson Hyperlegible font not found!");
    }
}