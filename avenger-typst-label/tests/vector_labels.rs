//! A label's vector drawing items, in its box.

mod common;

use avenger_typst_label::{
    Label, LabelEngine, LabelOptions, LabelSource, PdfItem, SvgItem,
};

fn label(source: LabelSource<'_>) -> Label<'_> {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = 14.0;
    Label { source, options }
}

#[test]
fn svg_items_sit_in_the_labels_box() {
    let engine = LabelEngine::new(common::engine_options());
    let (bounds, svg) = engine.svg(&label(LabelSource::Text("Ag"))).unwrap();
    assert_eq!((svg.size.x, svg.size.y), (bounds.width, bounds.height));
    let [SvgItem::Text(run)] = svg.items.as_slice() else {
        panic!("{:?}", svg.items);
    };
    assert!((run.baseline - bounds.ascent).abs() < 1e-4, "{run:?} {bounds:?}");
    assert_eq!(engine.bounds(&label(LabelSource::Text("Ag"))).unwrap(), bounds);
}

#[test]
fn pdf_glyph_runs_sit_on_the_boxs_baseline() {
    let engine = LabelEngine::new(common::engine_options());
    let (bounds, pdf) = engine.pdf(&label(LabelSource::Text("Hello"))).unwrap();
    assert_eq!((pdf.size.x, pdf.size.y), (bounds.width, bounds.height));
    let [PdfItem::Text(run)] = pdf.items.as_slice() else {
        panic!("{:?}", pdf.items);
    };
    assert_eq!(run.text, "Hello");
    // The baseline lies the box's ascent below its top.
    let y = run.transform.apply(run.glyphs[0].position).y;
    assert!((y - bounds.ascent).abs() < 1e-3, "{y}: {bounds:?}");
}

/// A copy of a single-face font whose `OS/2` table's embedding flags are `fs_type`, or the font
/// unchanged when it has no `OS/2` table.
fn with_embedding_flags(font: &[u8], fs_type: u16) -> Vec<u8> {
    let mut font = font.to_vec();
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    let Some(record) = (0..tables)
        .map(|table| 12 + table * 16)
        .find(|record| &font[*record..record + 4] == b"OS/2")
    else {
        return font;
    };
    let os2 =
        u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize;
    font[os2 + 8..os2 + 10].copy_from_slice(&fs_type.to_be_bytes());
    font
}

#[test]
fn fonts_report_how_their_licenses_let_documents_embed_them() {
    use avenger_typst_label::FontEmbedding;
    for (fs_type, expected) in [
        (0x0000, FontEmbedding::Allowed),      // Installable
        (0x0002, FontEmbedding::Forbidden),    // Restricted
        (0x0004, FontEmbedding::Allowed),      // Preview and print
        (0x0008, FontEmbedding::Allowed),      // Editable
        (0x0104, FontEmbedding::NoSubsetting), // Preview and print, without subsetting
        (0x0200, FontEmbedding::Forbidden),    // Bitmaps only
    ] {
        let mut options = common::engine_options();
        for font in &mut options.fonts.registered_fonts {
            font.data = with_embedding_flags(&font.data, fs_type).into();
        }
        let engine = LabelEngine::new(options);
        let (_, pdf) = engine.pdf(&label(LabelSource::Text("Hello"))).unwrap();
        assert_eq!(pdf.fonts[0].embedding(), expected, "{fs_type:#06x}");
    }
}

#[test]
fn invalid_markup_draws_its_source_as_text() {
    let engine = LabelEngine::new(common::engine_options());
    let invalid = label(LabelSource::Markup("a $b"));
    let (_, svg) = engine.svg(&invalid).unwrap();
    let svg_text: String = svg
        .items
        .iter()
        .filter_map(|item| match item {
            SvgItem::Text(run) => Some(run.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(svg_text, "a $b");
    let (_, pdf) = engine.pdf(&invalid).unwrap();
    assert_eq!(pdf.semantic_text, "a $b");
}
