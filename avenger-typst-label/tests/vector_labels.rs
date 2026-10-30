//! A label's vector drawing items, in its box.

mod common;

use avenger_typst_label::{
    BoxAnchor, Label, LabelEngine, LabelOptions, LabelOutline, LabelSource, PdfItem,
    RunPlace, SvgItem, TextBounds,
};

fn label(source: LabelSource<'_>) -> Label<'_> {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = 14.0;
    Label { source, options, path: None }
}

/// The distance from a straight label's box's top down to its first baseline.
fn ascent(bounds: &TextBounds) -> f32 {
    let BoxAnchor::Lines { ascent, .. } = bounds.anchor else {
        panic!("a label in straight lines: {bounds:?}");
    };
    ascent
}

#[test]
fn svg_items_sit_in_the_labels_box() {
    let engine = LabelEngine::new(common::engine_options());
    let (bounds, svg) = engine.svg(&label(LabelSource::Text("Ag"))).unwrap();
    assert_eq!((svg.size.x, svg.size.y), (bounds.width, bounds.height));
    let [SvgItem::Text(run)] = svg.items.as_slice() else {
        panic!("{:?}", svg.items);
    };
    let RunPlace::Line { baseline, .. } = run.place else {
        panic!("a run in a straight line: {run:?}");
    };
    assert!((baseline - ascent(&bounds)).abs() < 1e-4, "{run:?} {bounds:?}");
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
    assert!((y - ascent(&bounds)).abs() < 1e-3, "{y}: {bounds:?}");
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

/// The extent of an outline's points: `[left, top, right, bottom]`.
fn extent(outline: &LabelOutline) -> [f32; 4] {
    outline.contours.iter().flatten().fold(
        [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
        |[x0, y0, x1, y1], &[x, y]| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
    )
}

/// The outline of a label covers its glyphs in its box, with the box's top left at 0, and its
/// shapes, such as an underline, as polygons.
#[test]
fn outlines_cover_glyphs_and_shapes_in_the_labels_box() {
    let engine = LabelEngine::new(common::engine_options());
    let text = label(LabelSource::Text("Ag"));
    let bounds = engine.bounds(&text).unwrap();
    let outline = engine.outline(&text).unwrap();
    // A and g each have an outer contour and a counter.
    assert!(outline.contours.len() >= 4, "{outline:?}");
    let [left, top, right, bottom] = extent(&outline);
    assert!(left >= 0.0 && right <= bounds.width, "{left} {right}: {bounds:?}");
    // The A rises to its cap height above the baseline, and the g falls below it.
    assert!(top < ascent(&bounds) - 9.0, "{top}: {bounds:?}");
    assert!(bottom > ascent(&bounds) + 2.0, "{bottom}: {bounds:?}");
    assert!(std::sync::Arc::ptr_eq(&outline, &engine.outline(&text).unwrap()));

    let underlined = label(LabelSource::Markup("#underline[x]"));
    let bounds = engine.bounds(&underlined).unwrap();
    let outline = engine.outline(&underlined).unwrap();
    let line = outline
        .contours
        .iter()
        .find(|contour| contour.len() == 4)
        .expect("the underline is a polygon");
    let [left, top, right, _] = extent(&LabelOutline { contours: vec![line.clone()] });
    assert_eq!((left, right), (0.0, bounds.width));
    assert!(top > ascent(&bounds), "{top}: {bounds:?}");

    assert!(
        engine
            .outline(&label(LabelSource::Text(" ")))
            .unwrap()
            .contours
            .is_empty()
    );
}

/// An expanded outline reaches its width past the outline on every side, with the holes of the
/// glyphs that the width fills gone, and memoizes per label and width.
#[test]
fn expanded_outlines_grow_the_outline_by_their_width() {
    let engine = LabelEngine::new(common::engine_options());
    let text = label(LabelSource::Text("Ag"));
    let [left, top, right, bottom] = extent(&engine.outline(&text).unwrap());
    let expanded = engine.expanded_outline(&text, 2.0).unwrap();
    let [x0, y0, x1, y1] = extent(&expanded);
    for (actual, expected) in
        [(x0, left - 2.0), (y0, top - 2.0), (x1, right + 2.0), (y1, bottom + 2.0)]
    {
        assert!((actual - expected).abs() < 0.05, "{actual} != {expected}");
    }
    // A 4 px reach closes the counters of the A and the g, which leaves one polygon per glyph
    // or fewer.
    let wide = engine.expanded_outline(&text, 4.0).unwrap();
    assert!(wide.contours.len() <= 2, "{wide:?}");
    assert!(std::sync::Arc::ptr_eq(
        &expanded,
        &engine.expanded_outline(&text, 2.0).unwrap()
    ));
    assert!(!std::sync::Arc::ptr_eq(&expanded, &wide));
    assert!(
        engine
            .expanded_outline(&label(LabelSource::Text(" ")), 2.0)
            .unwrap()
            .contours
            .is_empty()
    );
}

/// Strokes cover their width, with their caps and dashes, and a shape with only a stroke has no
/// fill inside it.
#[test]
fn outlines_cover_strokes_and_only_the_fills() {
    let engine = LabelEngine::new(common::engine_options());
    let markup = |source| label(LabelSource::Markup(source));
    // A stroked highlight reaches half its 2 pt stroke past the label's box, and a thin growth
    // keeps the glyph apart from the ring around it, which a filled highlight merges.
    let stroked = markup("#highlight(fill: none, stroke: 2pt)[x]");
    let width = engine.bounds(&stroked).unwrap().width;
    let [left, _, right, _] = extent(&engine.outline(&stroked).unwrap());
    assert!(
        (left + 1.0).abs() < 0.01 && (right - width - 1.0).abs() < 0.01,
        "{left} {right}"
    );
    assert!(engine.expanded_outline(&stroked, 0.3).unwrap().contours.len() >= 3);
    let filled = markup("#highlight(fill: black)[x]");
    assert_eq!(engine.expanded_outline(&filled, 0.3).unwrap().contours.len(), 1);

    // A round cap reaches past the end of the line, and a butt cap stops there.
    let round = markup("#underline(stroke: (thickness: 2pt, cap: \"round\"))[xx]");
    let butt = markup("#underline(stroke: 2pt)[xx]");
    assert!((extent(&engine.outline(&round).unwrap())[0] + 1.0).abs() < 0.01);
    assert_eq!(extent(&engine.outline(&butt).unwrap())[0], 0.0);

    // A dashed line is one polygon per dash.
    let solid = engine.outline(&markup("#underline(stroke: 2pt)[xxxxxx]")).unwrap();
    let dashed = markup("#underline(stroke: (thickness: 2pt, dash: \"dashed\"))[xxxxxx]");
    let dashed = engine.outline(&dashed).unwrap();
    assert!(
        dashed.contours.len() >= solid.contours.len() + 3,
        "{}",
        dashed.contours.len()
    );
}
