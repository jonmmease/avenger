use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    path::{typst_path_item_to_text_path_item, TextPathItem, TextPathKind},
    types::{FontStyle, FontWeight},
};

use crate::math::TextMarkupConfig;

use crate::text_line::{
    bounds_from_metrics, tight_bounds_from_metrics, typeset_line, TextLineMeasurer,
};

#[derive(Debug, Clone)]
pub struct TextPdfExtractionConfig<'a> {
    pub text: &'a str,
    pub color: [f32; 4],
    pub font: &'a str,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub limit: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPdfDrawItem {
    GlyphRun(usize),
    PathItem(usize),
}

#[derive(Debug, Clone)]
pub struct TextPdfBuffer {
    pub bounds: TextBounds,
    pub semantic_text: String,
    pub font_resources: Vec<avenger_typst::MathFontResource>,
    pub glyph_runs: Vec<avenger_typst::MathPdfGlyphRun>,
    pub items: Vec<TextPathItem>,
    pub draw_items: Vec<TextPdfDrawItem>,
}

impl TextPdfBuffer {
    pub fn new(bounds: TextBounds, semantic_text: String) -> Self {
        Self {
            bounds,
            semantic_text,
            font_resources: Vec::new(),
            glyph_runs: Vec::new(),
            items: Vec::new(),
            draw_items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TextPdfExtractorImpl {
    typst: avenger_typst::AvengerTypst,
    math: TextMarkupConfig,
}

impl TextPdfExtractorImpl {
    pub(crate) fn new(typst: avenger_typst::AvengerTypst, math: TextMarkupConfig) -> Self {
        Self { typst, math }
    }

    fn measure_text_bounds(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        TextLineMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    pub(crate) fn extract_pdf(
        &self,
        config: &TextPdfExtractionConfig,
    ) -> Result<TextPdfBuffer, AvengerTextError> {
        let text = crate::measurement::truncate_text_to_limit_with(
            config.text,
            config.limit,
            |candidate| {
                let measurement = TextMeasurementConfig {
                    text: candidate,
                    font: config.font,
                    font_size: config.font_size,
                    font_weight: config.font_weight,
                    font_style: config.font_style,
                };
                self.measure_text_bounds(&measurement)
                    .map(|bounds| bounds.width)
            },
        )?;
        let result = typeset_line(
            &self.typst,
            &self.math,
            &text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            config.color,
            avenger_typst::TextLineOutputRequest {
                paths: true,
                raster: None,
                pdf_text_layer: true,
                positioned_runs: false,
            },
        )?;
        let tight_bounds = tight_bounds_from_metrics(result.artifact.metrics);
        let bounds = bounds_from_metrics(
            result.artifact.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let y_offset = bounds.ascent - tight_bounds.ascent;
        let text_len = text.len();
        let mut output = TextPdfBuffer::new(bounds, text);

        if let Some(pdf_text) = result.artifact.pdf_text {
            output.semantic_text = pdf_text.semantic_text;
            for mut run in pdf_text.glyph_runs {
                for glyph in &mut run.glyphs {
                    glyph.transform.dy += y_offset;
                }
                let index = output.glyph_runs.len();
                output.glyph_runs.push(run);
                output.draw_items.push(TextPdfDrawItem::GlyphRun(index));
            }
        }
        output.font_resources = result.artifact.font_resources;

        if let Some(paths) = result.artifact.paths {
            for item in paths.items {
                if !matches!(item.kind, avenger_typst::MathPathKind::MathShape) {
                    continue;
                }
                let item = typst_path_item_to_text_path_item(item, 0..text_len, 0.0, y_offset);
                if item.kind != TextPathKind::MathShape {
                    continue;
                }
                let index = output.items.len();
                output.items.push(item);
                output.draw_items.push(TextPdfDrawItem::PathItem(index));
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
pub(crate) fn validate_glyph_run_text_ranges(
    glyph_runs: &[avenger_typst::MathPdfGlyphRun],
) -> bool {
    glyph_runs.iter().all(|run| {
        run.glyphs.iter().all(|glyph| {
            glyph.text_range.start <= glyph.text_range.end
                && glyph.text_range.end <= run.text.len()
                && run.text.is_char_boundary(glyph.text_range.start)
                && run.text.is_char_boundary(glyph.text_range.end)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};

    static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
    static STYLE: FontStyle = FontStyle::Normal;
    static COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn engine() -> crate::TextEngine {
        crate::TextEngine::with_default_config().unwrap()
    }

    fn pdf_config(text: &str) -> TextPdfExtractionConfig<'_> {
        TextPdfExtractionConfig {
            text,
            color: COLOR,
            font: "sans-serif",
            font_size: 14.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            limit: f32::INFINITY,
        }
    }

    fn pdf_config_with_font<'a>(text: &'a str, font: &'a str) -> TextPdfExtractionConfig<'a> {
        TextPdfExtractionConfig {
            text,
            color: COLOR,
            font,
            font_size: 14.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            limit: f32::INFINITY,
        }
    }

    #[test]
    fn extracts_plain_text_pdf_glyph_runs() {
        let buffer = engine().extract_pdf(&pdf_config("Hello")).unwrap();

        assert!(buffer.bounds.width > 0.0);
        assert_eq!(buffer.glyph_runs.len(), 1);
        assert_eq!(buffer.glyph_runs[0].text, "Hello");
        assert!(validate_glyph_run_text_ranges(&buffer.glyph_runs));
        assert!(buffer
            .draw_items
            .iter()
            .any(|item| matches!(item, TextPdfDrawItem::GlyphRun(_))));
    }

    #[test]
    fn pdf_glyph_baseline_is_offset_into_padded_line_bounds() {
        let buffer = engine().extract_pdf(&pdf_config("X Position")).unwrap();
        let first_glyph_y = buffer.glyph_runs[0].glyphs[0].transform.dy;

        assert!(
            (first_glyph_y - buffer.bounds.ascent).abs() < 0.001,
            "first glyph baseline {first_glyph_y} should match padded ascent {}",
            buffer.bounds.ascent
        );
    }

    #[test]
    fn extracts_math_pdf_glyph_runs_and_shape_paths() {
        let buffer = engine().extract_pdf(&pdf_config("$a / b$")).unwrap();

        assert!(buffer.glyph_runs.len() >= 2);
        assert!(buffer
            .items
            .iter()
            .any(|item| item.kind == TextPathKind::MathShape));
        assert!(validate_glyph_run_text_ranges(&buffer.glyph_runs));
    }

    #[test]
    fn extracts_named_emoji_as_pdf_glyph_run_text() {
        let buffer = engine()
            .extract_pdf(&pdf_config("Mood #emoji.face"))
            .unwrap();

        assert!(buffer.glyph_runs.iter().any(|run| run.text.contains('😀')));
        assert!(validate_glyph_run_text_ranges(&buffer.glyph_runs));
    }

    #[test]
    fn extracts_complex_script_pdf_ranges_on_char_boundaries() {
        for sample in ["ABC שלום", "नमस्ते data"] {
            let buffer = engine().extract_pdf(&pdf_config(sample)).unwrap();
            assert!(
                validate_glyph_run_text_ranges(&buffer.glyph_runs),
                "{sample} should have valid glyph text ranges"
            );
        }
    }

    #[test]
    fn configured_extra_font_dirs_drive_pdf_glyph_extraction() {
        let caveat_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-vega-test-data/fonts/Caveat/static");
        let engine = crate::TextEngine::with_font_resolution(&crate::FontResolutionOptions {
            extra_font_dirs: vec![caveat_dir],
            ..Default::default()
        })
        .unwrap();

        let buffer = engine
            .extract_pdf(&pdf_config_with_font("Caveat", "Caveat"))
            .unwrap();

        assert!(buffer
            .font_resources
            .iter()
            .any(|resource| resource.family == "Caveat"));
        assert!(buffer.glyph_runs.iter().any(|run| run.text == "Caveat"));
    }
}
