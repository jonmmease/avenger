use cosmic_text::{fontdb::Database, Attrs, Buffer, Family, FontSystem, Metrics, SwashCache};
use std::{collections::HashSet, sync::Mutex};

use super::{TextBounds, TextMeasurementConfig, TextMeasurer};
use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};

use lazy_static::lazy_static;

lazy_static! {
    pub static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(build_font_system());
    pub static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
}

fn build_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();

    // Override default families based on what system fonts are available
    let fontdb = font_system.db_mut();
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

    // Set default sans serif
    for family in ["Helvetica", "Arial", "Liberation Sans"] {
        if families.contains(family) {
            fontdb.set_sans_serif_family(family);
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
            break;
        }
    }
}

pub struct CosmicTextMeasurer {}

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
    let mut attrs = Attrs::new();
    let family = match config.font.to_lowercase().as_str() {
        "serif" => Family::Serif,
        "sans serif" | "sans-serif" => Family::SansSerif,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        "monospace" => Family::Monospace,
        _ => Family::Name(&config.font),
    };

    attrs.family = family;

    // Set font weight
    attrs.weight = match config.font_weight {
        FontWeight::Name(FontWeightNameSpec::Bold) => cosmic_text::Weight::BOLD,
        FontWeight::Name(FontWeightNameSpec::Normal) => cosmic_text::Weight::NORMAL,
        FontWeight::Number(w) => cosmic_text::Weight(*w as u16),
    };

    // Set font style
    attrs.style = match config.font_style {
        FontStyle::Normal => cosmic_text::Style::Normal,
        FontStyle::Italic => cosmic_text::Style::Italic,
    };

    // Create metrics (using size from config)
    let metrics = Metrics::new(config.font_size, config.font_size);

    // Create a buffer for measurement
    let mut buffer = Buffer::new(font_system, metrics);

    // Set the text with attributes
    buffer.set_text(
        font_system,
        config.text,
        attrs,
        cosmic_text::Shaping::Advanced,
    );
    buffer.set_size(font_system, Some(1024.0), Some(512.0));
    buffer.shape_until_scroll(font_system, false);

    buffer
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
