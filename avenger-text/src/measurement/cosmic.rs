use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use cosmic_text::{
    fontdb::{self, Database},
    ttf_parser, Attrs, Buffer, Family, FontSystem, Metrics as CosmicMetrics, SwashCache,
};
use lazy_static::lazy_static;

use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};

use super::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig, TextMeasurer};

lazy_static! {
    pub static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(build_font_system());
    pub static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
    pub static ref GENERIC_FAMILIES: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
}

fn build_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();

    // Load embedded fonts first
    let fontdb = font_system.db_mut();
    crate::fonts::load_embedded_fonts(fontdb);

    // Override default families based on what system fonts are available
    setup_default_fonts(fontdb);
    font_system
}

fn setup_default_fonts(fontdb: &mut Database) {
    let families: HashSet<String> = fontdb
        .faces()
        .flat_map(|face| {
            face.families
                .iter()
                .map(|(fam, _lang)| fam.clone())
                .collect::<Vec<_>>()
        })
        .collect();

    let mut generic_families = GENERIC_FAMILIES.lock().unwrap();

    // Set default sans serif
    for family in ["Helvetica", "Arial", "Liberation Sans"] {
        if families.contains(family) {
            fontdb.set_sans_serif_family(family);
            generic_families.insert("sans-serif".to_string(), family.to_string());
            break;
        }
    }

    // Set default monospace font family
    for family in [
        "Courier New",
        "Courier",
        "Liberation Mono",
        "DejaVu Sans Mono",
    ] {
        if families.contains(family) {
            fontdb.set_monospace_family(family);
            generic_families.insert("monospace".to_string(), family.to_string());
            break;
        }
    }

    // Set default serif font family
    for family in [
        "Times New Roman",
        "Times",
        "Liberation Serif",
        "DejaVu Serif",
    ] {
        if families.contains(family) {
            fontdb.set_serif_family(family);
            generic_families.insert("serif".to_string(), family.to_string());
            break;
        }
    }
}

pub struct CosmicTextMeasurer {}

impl Default for CosmicTextMeasurer {
    fn default() -> Self {
        Self::new()
    }
}

impl CosmicTextMeasurer {
    pub fn new() -> Self {
        Self {}
    }
}

impl TextMeasurer for CosmicTextMeasurer {
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        let mut font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");

        let buffer = make_cosmic_text_buffer(config, &mut font_system);
        measure_text_buffer(&buffer)
    }

    fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        let font_system = FONT_SYSTEM
            .lock()
            .expect("Failed to acquire lock on FONT_SYSTEM");

        measure_font_metrics_with_cosmic(config, &font_system)
            .unwrap_or_else(|| FontMetrics::fallback(config.font_size))
    }
}

fn measure_font_metrics_with_cosmic(
    config: &FontMetricsConfig,
    font_system: &FontSystem,
) -> Option<FontMetrics> {
    let attrs = make_cosmic_attrs(config.font, config.font_weight, config.font_style);
    let families = [attrs.family];
    let query = fontdb::Query {
        families: &families,
        weight: attrs.weight,
        stretch: attrs.stretch,
        style: attrs.style,
    };

    let font_id = font_system.db().query(&query)?;
    font_system
        .db()
        .with_face_data(font_id, |font_data, face_index| {
            ttf_parser::Face::parse(font_data, face_index)
                .ok()
                .map(|face| metrics_from_ttf_face(&face, config.font_size))
        })
        .flatten()
}

fn metrics_from_ttf_face(face: &ttf_parser::Face<'_>, font_size: f32) -> FontMetrics {
    let scale = font_size / face.units_per_em() as f32;
    let ascent = face.ascender().max(0) as f32 * scale;
    let descent = (-face.descender()).max(0) as f32 * scale;
    let height = ascent + descent;
    let line_gap = face.line_gap().max(0) as f32 * scale;
    let line_height = (height + line_gap).max(height).max(font_size);

    FontMetrics {
        ascent,
        descent,
        height,
        line_gap,
        line_height,
    }
}

pub fn measure_text_buffer(buffer: &Buffer) -> TextBounds {
    let runs = buffer.layout_runs().collect::<Vec<_>>();

    if runs.is_empty() {
        return TextBounds::empty();
    }

    // Get metrics across all runs
    let mut max_ascent = 0.0;
    let mut max_descent = 0.0;
    let mut max_line_height = 0.0;

    for run in &runs {
        let ascent = run.line_y - run.line_top;
        let descent = run.line_height - ascent;

        max_ascent = f32::max(max_ascent, ascent);
        max_descent = f32::max(max_descent, descent);
        max_line_height = f32::max(max_line_height, run.line_height);
    }

    // Calculate total width from all runs
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;

    for run in runs {
        for glyph in run.glyphs {
            min_x = min_x.min(glyph.x);
            max_x = max_x.max(glyph.x + glyph.w);
        }
    }

    let width = if min_x == f32::MAX {
        0.0
    } else {
        max_x - min_x
    };
    let height = max_ascent + max_descent;

    TextBounds {
        width,
        height,
        ascent: max_ascent,
        descent: max_descent,
        line_height: max_line_height,
    }
}

pub fn make_cosmic_text_buffer(
    config: &TextMeasurementConfig,
    font_system: &mut FontSystem,
) -> Buffer {
    let attrs = make_cosmic_attrs(config.font, config.font_weight, config.font_style);

    // Create metrics (using size from config)
    let metrics = CosmicMetrics::new(config.font_size, config.font_size);

    // Create a buffer for measurement
    let mut buffer = Buffer::new(font_system, metrics);

    // Set the text with attributes
    buffer.set_text(
        font_system,
        config.text,
        &attrs,
        cosmic_text::Shaping::Advanced,
    );
    buffer.set_size(font_system, Some(1024.0), Some(512.0));
    buffer.shape_until_scroll(font_system, false);

    buffer
}

fn make_cosmic_attrs<'a>(
    font: &'a str,
    font_weight: &FontWeight,
    font_style: &FontStyle,
) -> Attrs<'a> {
    let mut attrs = Attrs::new();
    let family = match font.to_lowercase().as_str() {
        "serif" => Family::Serif,
        "sans serif" | "sans-serif" => Family::SansSerif,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        "monospace" => Family::Monospace,
        _ => Family::Name(font),
    };

    attrs.family = family;

    // Set font weight
    attrs.weight = match font_weight {
        FontWeight::Name(FontWeightNameSpec::Bold) => cosmic_text::Weight::BOLD,
        FontWeight::Name(FontWeightNameSpec::Normal) => cosmic_text::Weight::NORMAL,
        FontWeight::Number(w) => cosmic_text::Weight(*w as u16),
    };

    // Set font style
    attrs.style = match font_style {
        FontStyle::Normal => cosmic_text::Style::Normal,
        FontStyle::Italic => cosmic_text::Style::Italic,
    };

    attrs
}

pub fn register_font_directory(dir: &str) {
    let mut font_system = FONT_SYSTEM
        .lock()
        .expect("Failed to acquire lock on FONT_SYSTEM");
    let fontdb = font_system.db_mut();
    fontdb.load_fonts_dir(dir);
    setup_default_fonts(fontdb);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FontStyle, FontWeight};

    #[test]
    fn test_cosmic_text_measurer() {
        let measurer = CosmicTextMeasurer::new();

        let config = TextMeasurementConfig {
            text: "Hello, World!",
            font: "serif",
            font_size: 16.0,
            font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
            font_style: &FontStyle::Normal,
        };

        let bounds = measurer.measure_text_bounds(&config);

        println!("{:?}", bounds);

        assert!(bounds.width > 0.0);
        assert!(bounds.height > 0.0);
        assert!(bounds.ascent > 0.0);
        assert!(bounds.descent > 0.0);
        assert!(bounds.line_height > 0.0);
    }
}
