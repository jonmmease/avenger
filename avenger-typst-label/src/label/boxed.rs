//! A label's drawing items in its box, which anchors place.

use super::bounds::TextBounds;
use super::frame::{Size, Transform};
use crate::typst_pdf::{PdfItem, PdfLabel, PdfText};
use crate::typst_svg::{ImageItem, PathItem, SvgItem, SvgLabel, TextRun};

/// A label's SVG items moved down into its box, whose top is `top` above the label's frame,
/// and sized to the box.
pub(super) fn svg_in_box(svg: SvgLabel, bounds: &TextBounds, top: f32) -> SvgLabel {
    let offset = Transform::translate(0.0, top);
    let items = svg
        .items
        .into_iter()
        .map(|item| match item {
            SvgItem::Path(path) => SvgItem::Path(PathItem {
                transform: offset.pre_concat(path.transform),
                ..path
            }),
            SvgItem::Image(image) => SvgItem::Image(ImageItem {
                transform: offset.pre_concat(image.transform),
                ..image
            }),
            SvgItem::Text(run) => {
                SvgItem::Text(TextRun { baseline: run.baseline + top, ..run })
            }
        })
        .collect();
    SvgLabel {
        size: Size::new(bounds.width, bounds.height),
        items,
    }
}

/// A label's PDF items moved down into its box, whose top is `top` above the label's frame,
/// and sized to the box.
pub(super) fn pdf_in_box(pdf: PdfLabel, bounds: &TextBounds, top: f32) -> PdfLabel {
    let offset = Transform::translate(0.0, top);
    let items = pdf
        .items
        .into_iter()
        .map(|item| match item {
            PdfItem::Text(run) => PdfItem::Text(PdfText {
                transform: offset.pre_concat(run.transform),
                ..run
            }),
            PdfItem::Path(path) => PdfItem::Path(PathItem {
                transform: offset.pre_concat(path.transform),
                ..path
            }),
        })
        .collect();
    PdfLabel {
        size: Size::new(bounds.width, bounds.height),
        items,
        ..pdf
    }
}
