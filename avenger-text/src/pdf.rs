use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    path::{typst_path_item_to_text_path_item, TextPathItem, TextPathKind},
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
    pub number_locale: Option<&'a str>,
    pub number_locale_specs: Option<&'a crate::NumberLocaleSpecs>,
    pub datetime_locale: Option<&'a str>,
    pub datetime_timezone: Option<&'a str>,
    pub datetime_locale_specs: Option<&'a crate::DateTimeLocaleSpecs>,
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
    pub font_resources: Vec<avenger_typst_label::FontResource>,
    pub glyph_runs: Vec<avenger_typst_label::PdfGlyphRun>,
    pub items: Vec<TextPathItem>,
    pub draw_items: Vec<TextPdfDrawItem>,
}

impl TextPdfBuffer {
    pub fn new(bounds: TextBounds, semantic_text: String) -> Self {
        Self {
            clip_width: None,
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
                    number_locale: config.number_locale,
                    number_locale_specs: config.number_locale_specs,
                    datetime_locale: config.datetime_locale,
                    datetime_timezone: config.datetime_timezone,
                    datetime_locale_specs: config.datetime_locale_specs,
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
            config.number_locale,
            config.number_locale_specs,
            config.datetime_locale,
            config.datetime_timezone,
            config.datetime_locale_specs,
        )?;
        let tight_bounds = tight_bounds_from_metrics(result.label.metrics);
        let mut bounds = bounds_from_metrics(
            result.label.metrics,
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
        )?;
        output.semantic_text = pdf.semantic_text;
        output.font_resources = pdf.font_resources;

        for item in pdf.draw_items {
            match item {
                avenger_typst_label::PdfDrawItem::GlyphRun(index) => {
                    let mut run = pdf.glyph_runs[index].clone();
                    for glyph in &mut run.glyphs {
                        glyph.transform.ty += y_offset;
                    }
                    let index = output.glyph_runs.len();
                    output.glyph_runs.push(run);
                    output.draw_items.push(TextPdfDrawItem::GlyphRun(index));
                }
                avenger_typst_label::PdfDrawItem::PathItem(index) => {
                    let path = &pdf.path_items[index];
                    let item = typst_path_item_to_text_path_item(
                        path.item.clone(),
                        path.byte_range.clone(),
                        0.0,
                        y_offset,
                    );
                    if item.kind != TextPathKind::MathShape {
                        continue;
                    }
                    let index = output.items.len();
                    output.items.push(item);
                    output.draw_items.push(TextPdfDrawItem::PathItem(index));
                }
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
pub(crate) fn validate_glyph_run_text_ranges(
    glyph_runs: &[avenger_typst_label::PdfGlyphRun],
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
    use crate::types::{FontStyle, FontWeight, FontWeightNameSpec, TextSyntaxMode};

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
            syntax_mode: TextSyntaxMode::TypstMarkup,
            params: crate::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
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
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
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
        let first_glyph_y = buffer.glyph_runs[0].glyphs[0].transform.ty;

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
