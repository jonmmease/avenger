use avenger_typst_label::{FontRef, PdfItem, PdfText, Transform};

use crate::{
    error::AvengerTextError,
    measurement::TextBounds,
    path::{text_path_item, TextPathItem},
    types::TextConfig,
    typeset::{bounds_from_metrics, first_baseline, typeset, LabelSettings},
};

#[derive(Debug, Clone, PartialEq)]
pub enum TextPdfDrawItem {
    GlyphRun(usize),
    PathItem(usize),
}

#[derive(Debug, Clone)]
pub struct TextPdfBuffer {
    pub bounds: TextBounds,
    /// The label's text in reading order, as it shows: with a newline where an explicit break
    /// ends a line, and the ellipses of cut text.
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
            bounds,
            semantic_text,
            fonts: Vec::new(),
            glyph_runs: Vec::new(),
            items: Vec::new(),
            draw_items: Vec::new(),
        }
    }
}

/// A label's glyph runs in their fonts, and its shapes as paths.
pub(crate) fn extract_pdf(
    typst: &avenger_typst_label::LabelEngine,
    settings: &LabelSettings,
    config: &TextConfig,
) -> Result<TextPdfBuffer, AvengerTextError> {
    let label = typeset(typst, settings, config)?;
    let bounds = bounds_from_metrics(&label.metrics, config.font_size);
    let y_offset = bounds.ascent - first_baseline(&label.metrics);
    let pdf = avenger_typst_label::pdf_items(&label, &avenger_typst_label::PdfOptions::default());
    let mut output = TextPdfBuffer::new(bounds, pdf.semantic_text);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::TextPathKind;
    use crate::types::TextSyntaxMode;

    fn engine() -> crate::TextEngine {
        crate::TextEngine::default()
    }

    fn pdf_config(text: &str) -> TextConfig<'_> {
        TextConfig {
            text,
            syntax_mode: TextSyntaxMode::TypstMarkup,
            font_size: 14.0,
            ..Default::default()
        }
    }

    #[test]
    fn extracts_plain_text_pdf_glyph_runs() {
        let buffer = engine().extract_pdf(&pdf_config("Hello")).unwrap();

        assert!(buffer.bounds.width > 0.0);
        assert_eq!(buffer.glyph_runs.len(), 1);
        assert_eq!(buffer.glyph_runs[0].text, "Hello");
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
    }

    #[test]
    fn configured_extra_font_dirs_drive_pdf_glyph_extraction() {
        let caveat_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-vega-test-data/fonts/Caveat/static");
        let engine = crate::TextEngine::new(&crate::FontOptions {
            extra_font_dirs: vec![caveat_dir],
            ..Default::default()
        });

        let buffer = engine
            .extract_pdf(&TextConfig {
                font: "Caveat",
                ..pdf_config("Caveat")
            })
            .unwrap();

        assert!(buffer.fonts.iter().any(|font| font.family() == "Caveat"));
        assert!(buffer.glyph_runs.iter().any(|run| run.text == "Caveat"));
    }
}
