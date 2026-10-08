pub use avenger_typst_label::{FontRef, PdfText};
use avenger_typst_label::{PdfItem, Transform};

use crate::{
    error::AvengerTextError,
    measurement::TextBounds,
    path::{text_shape, TextShape},
    types::TextConfig,
    typeset::{bounds_from_metrics, first_baseline, typeset, LabelSettings},
};

/// What a label draws in a PDF.
#[derive(Debug, Clone)]
pub enum TextPdfItem {
    /// A run of glyphs in one of the label's fonts. Bitmap glyphs draw from their fonts.
    Glyphs(PdfText),
    /// A shape, such as a fraction line or a text decoration.
    Shape(TextShape),
}

#[derive(Debug, Clone)]
pub struct TextPdfBuffer {
    pub bounds: TextBounds,
    /// The label's text in reading order, as it shows: with a newline where an explicit break
    /// ends a line, and the ellipses of cut text.
    pub semantic_text: String,
    /// The fonts that glyph runs index.
    pub fonts: Vec<FontRef>,
    /// The label's drawing items, in drawing order, in its box's coordinates.
    pub items: Vec<TextPdfItem>,
}

/// A label's glyph runs in their fonts, and its shapes as paths.
pub(crate) fn extract_pdf(
    typst: &avenger_typst_label::LabelEngine,
    settings: &LabelSettings,
    config: &TextConfig,
) -> Result<TextPdfBuffer, AvengerTextError> {
    let label = typeset(typst, settings, config)?;
    let bounds = bounds_from_metrics(&label.metrics, config.font_size);
    // The label's frame starts below the padding at the top of its box.
    let offset = Transform::translate(0.0, bounds.ascent - first_baseline(&label.metrics));
    let pdf = avenger_typst_label::pdf_items(&label, &avenger_typst_label::PdfOptions::default());
    let items = pdf
        .items
        .into_iter()
        .map(|item| match item {
            PdfItem::Text(run) => TextPdfItem::Glyphs(PdfText {
                transform: offset.pre_concat(run.transform),
                ..run
            }),
            PdfItem::Path(path) => TextPdfItem::Shape(text_shape(&path, offset)),
        })
        .collect();
    Ok(TextPdfBuffer {
        bounds,
        semantic_text: pdf.semantic_text,
        fonts: pdf.fonts,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_common::types::TextSyntaxMode;

    fn config(text: &str) -> TextConfig<'_> {
        TextConfig {
            text,
            syntax_mode: TextSyntaxMode::TypstMarkup,
            font_size: 14.0,
            ..Default::default()
        }
    }

    fn glyph_runs(buffer: &TextPdfBuffer) -> Vec<&PdfText> {
        buffer
            .items
            .iter()
            .filter_map(|item| match item {
                TextPdfItem::Glyphs(run) => Some(run),
                TextPdfItem::Shape(_) => None,
            })
            .collect()
    }

    #[test]
    fn plain_text_is_one_glyph_run_on_the_box_baseline() {
        let buffer = crate::default_text_engine()
            .extract_pdf(&config("Hello"))
            .unwrap();
        let runs = glyph_runs(&buffer);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "Hello");
        // The baseline lies the box's ascent below its top.
        let y = runs[0].transform.apply(runs[0].glyphs[0].position).y;
        assert!(
            (y - buffer.bounds.ascent).abs() < 0.001,
            "{y}: {:?}",
            buffer.bounds
        );
    }

    #[test]
    fn math_draws_glyph_runs_and_its_fraction_line_as_an_open_shape() {
        let buffer = crate::default_text_engine()
            .extract_pdf(&config("$a / b$"))
            .unwrap();
        assert!(glyph_runs(&buffer).len() >= 2);
        let Some(TextPdfItem::Shape(line)) = buffer
            .items
            .iter()
            .find(|item| matches!(item, TextPdfItem::Shape(_)))
        else {
            panic!("{:?}", buffer.items);
        };
        assert!(matches!(
            line.path.iter().last(),
            Some(lyon_path::Event::End { close: false, .. })
        ));
    }

    #[test]
    fn extra_font_dirs_supply_glyph_runs() {
        let caveat_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-vega-test-data/fonts/Caveat/static");
        let engine = crate::TextEngine::new(&crate::FontOptions {
            extra_font_dirs: vec![caveat_dir],
            ..Default::default()
        });
        let buffer = engine
            .extract_pdf(&TextConfig {
                font: "Caveat",
                ..config("Caveat")
            })
            .unwrap();
        assert!(buffer.fonts.iter().any(|font| font.family() == "Caveat"));
        assert!(glyph_runs(&buffer).iter().any(|run| run.text == "Caveat"));
    }
}
