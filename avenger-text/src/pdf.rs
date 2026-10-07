use avenger_typst_label::{FontRef, PdfItem, PdfText, Transform};

use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    path::{text_path_item, TextPathItem},
    types::{FontStyle, FontWeight, TextSyntaxMode},
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
    /// Positive finite width in logical pixels. Plain text uses grapheme-safe
    /// ellipsis; Typst markup is compiled intact and clipped at this width.
    /// Other values leave the label unconstrained.
    pub limit: f32,
    pub syntax_mode: TextSyntaxMode,
    pub params: &'a avenger_typst_label::LabelParams,
    pub number_format: Option<&'a std::sync::Arc<dyn crate::NumberFormatProvider>>,
    pub datetime_format: Option<&'a std::sync::Arc<dyn crate::DateTimeFormatProvider>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPdfDrawItem {
    GlyphRun(usize),
    PathItem(usize),
}

#[derive(Debug, Clone)]
pub struct TextPdfBuffer {
    /// When set, clip every draw item to x <= this cutoff in label coordinates
    /// before applying placement. Semantic text retains the complete label.
    pub clip_width: Option<f32>,
    pub bounds: TextBounds,
    pub semantic_text: String,
    /// The fonts the glyph runs index.
    pub fonts: Vec<FontRef>,
    /// Runs of glyphs, in label coordinates. Bitmap glyphs draw from their fonts.
    pub glyph_runs: Vec<PdfText>,
    /// Shapes, such as fraction lines and text decorations.
    pub items: Vec<TextPathItem>,
    pub draw_items: Vec<TextPdfDrawItem>,
}

impl TextPdfBuffer {
    pub fn new(bounds: TextBounds, semantic_text: String) -> Self {
        Self {
            clip_width: None,
            bounds,
            semantic_text,
            fonts: Vec::new(),
            glyph_runs: Vec::new(),
            items: Vec::new(),
            draw_items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TextPdfExtractorImpl {
    typst: avenger_typst_label::LabelEngine,
    math: TextMarkupConfig,
}

impl TextPdfExtractorImpl {
    pub(crate) fn new(typst: avenger_typst_label::LabelEngine, math: TextMarkupConfig) -> Self {
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
        let math = self.math.with_syntax_mode(config.syntax_mode);
        let text = crate::measurement::prepare_text_to_limit_with(
            config.text,
            config.syntax_mode,
            config.limit,
            |candidate| {
                let measurement = TextMeasurementConfig {
                    text: candidate,
                    font: config.font,
                    font_size: config.font_size,
                    font_weight: config.font_weight,
                    font_style: config.font_style,
                    syntax_mode: config.syntax_mode,
                    params: config.params,
                    number_format: config.number_format,
                    datetime_format: config.datetime_format,
                };
                self.measure_text_bounds(&measurement)
                    .map(|bounds| bounds.width)
            },
        )?;
        let result = typeset_line(
            &self.typst,
            &math,
            &text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            config.color,
            config.params,
            config.number_format,
            config.datetime_format,
        )?;
        let tight_bounds = tight_bounds_from_metrics(&result.label.metrics);
        let mut bounds = bounds_from_metrics(
            &result.label.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let clip_width =
            crate::measurement::apply_text_limit(&mut bounds, config.syntax_mode, config.limit);
        let y_offset = bounds.ascent - tight_bounds.ascent;
        let mut output = TextPdfBuffer::new(bounds, text);
        output.clip_width = clip_width;
        let pdf = avenger_typst_label::pdf_items(
            &result.label,
            &avenger_typst_label::PdfOptions::default(),
        );
        output.semantic_text = pdf.semantic_text;
        output.fonts = pdf.fonts;

        for item in pdf.items {
            match item {
                PdfItem::Text(mut run) => {
                    run.transform = Transform::translate(0.0, y_offset).pre_concat(run.transform);
                    output
                        .draw_items
                        .push(TextPdfDrawItem::GlyphRun(output.glyph_runs.len()));
                    output.glyph_runs.push(run);
                }
                PdfItem::Path(path) => {
                    output
                        .draw_items
                        .push(TextPdfDrawItem::PathItem(output.items.len()));
                    output.items.push(text_path_item(&path, y_offset));
                }
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
pub(crate) fn validate_glyph_run_text_ranges(glyph_runs: &[PdfText]) -> bool {
    glyph_runs.iter().all(|run| {
        run.glyphs.iter().all(|glyph| {
            glyph.range.start <= glyph.range.end
                && glyph.range.end <= run.text.len()
                && run.text.is_char_boundary(glyph.range.start)
                && run.text.is_char_boundary(glyph.range.end)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::TextPathKind;
    use crate::types::{FontStyle, FontWeight, FontWeightNameSpec, TextSyntaxMode};

    static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
    static STYLE: FontStyle = FontStyle::Normal;
    static COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn engine() -> crate::TextEngine {
        crate::TextEngine::with_default_config()
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
            syntax_mode: TextSyntaxMode::TypstMarkup,
            params: crate::empty_label_params(),
            number_format: None,
            datetime_format: None,
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
            syntax_mode: TextSyntaxMode::TypstMarkup,
            params: crate::empty_label_params(),
            number_format: None,
            datetime_format: None,
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
        let run = &buffer.glyph_runs[0];
        let first_glyph_y = run.transform.apply(run.glyphs[0].position).y;

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
        let rule = buffer
            .items
            .iter()
            .find(|item| item.kind == TextPathKind::Shape)
            .unwrap();
        assert!(matches!(
            rule.path.iter().last(),
            Some(lyon_path::Event::End { close: false, .. })
        ));
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
        let engine = crate::TextEngine::with_fonts(&crate::FontOptions {
            extra_font_dirs: vec![caveat_dir],
            ..Default::default()
        });

        let buffer = engine
            .extract_pdf(&pdf_config_with_font("Caveat", "Caveat"))
            .unwrap();

        assert!(buffer.fonts.iter().any(|font| font.family() == "Caveat"));
        assert!(buffer.glyph_runs.iter().any(|run| run.text == "Caveat"));
    }
}
