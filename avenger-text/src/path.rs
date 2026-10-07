use std::ops::Range;

use avenger_typst_label::{
    Curve, CurveItem, FontRef, ImageItem, LineCap, LineJoin, PathItem, PathKind, SvgItem, TextRun,
    Transform,
};
use lyon_path::{geom::point, Path};

use crate::{
    error::AvengerTextError,
    measurement::TextBounds,
    types::{FontStyle, FontWeight, TextConfig},
    typeset::{bounds_from_metrics, first_baseline, typeset, LabelSettings},
};

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathDashPattern {
    pub array: Vec<f32>,
    pub phase: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathStroke {
    pub color: [f32; 4],
    pub width: f32,
    pub line_cap: TextPathLineCap,
    pub line_join: TextPathLineJoin,
    pub dash: Option<TextPathDashPattern>,
    pub miter_limit: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextPathLineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextPathLineJoin {
    Bevel,
    #[default]
    Miter,
    Round,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPathKind {
    /// A glyph's outline.
    Glyph {
        /// The glyph's range in the label source.
        source: Range<usize>,
    },
    /// A shape, such as a fraction line or a text decoration.
    Shape,
}

#[derive(Debug, Clone)]
pub struct TextPathItem {
    pub path: Path,
    pub fill: Option<[f32; 4]>,
    pub stroke: Option<TextPathStroke>,
    pub kind: TextPathKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPathImageFormat {
    Png,
}

/// A bitmap glyph.
#[derive(Debug, Clone, PartialEq)]
pub struct TextPathImageItem {
    pub data: Vec<u8>,
    pub format: TextPathImageFormat,
    pub width: f32,
    pub height: f32,
    pub transform: [f32; 6],
    /// The glyph's range in the label source.
    pub byte_range: Range<usize>,
    /// The glyph's text item, which a run of the same text item draws too.
    pub text_item: usize,
}

/// A run of glyphs that draws as native text: shaping its text with its face and the default
/// features gives its glyphs, so a viewer with the face draws the same glyphs in the same
/// places.
#[derive(Debug, Clone, PartialEq)]
pub struct PlainTextPathRun {
    pub text: String,
    /// The run's range in the label source.
    pub byte_range: Range<usize>,
    /// The face the run's glyphs come from.
    pub face: FontRef,
    /// Resolved foreground color.
    pub color: [f32; 4],
    /// Whether the run uses right-to-left text direction.
    pub is_rtl: bool,
    /// The face's family.
    pub font: String,
    pub font_size: f32,
    /// The face's weight.
    pub font_weight: FontWeight,
    /// The face's style.
    pub font_style: FontStyle,
    /// The run's left edge.
    pub x: f32,
    /// The run's baseline.
    pub baseline: f32,
    /// The run's advance width.
    pub width: f32,
    /// The run's text item, which its bitmap glyphs' images share.
    pub text_item: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPathDrawItem {
    PlainRun(usize),
    PathItem(usize),
    ImageItem(usize),
}

#[derive(Debug, Clone)]
pub struct TextPathBuffer {
    pub bounds: TextBounds,
    pub items: Vec<TextPathItem>,
    pub images: Vec<TextPathImageItem>,
    pub plain_runs: Vec<PlainTextPathRun>,
    pub draw_items: Vec<TextPathDrawItem>,
}

impl TextPathBuffer {
    pub fn new(bounds: TextBounds) -> Self {
        Self {
            bounds,
            items: Vec::new(),
            images: Vec::new(),
            plain_runs: Vec::new(),
            draw_items: Vec::new(),
        }
    }

    fn push_run(&mut self, run: PlainTextPathRun) {
        self.draw_items
            .push(TextPathDrawItem::PlainRun(self.plain_runs.len()));
        self.plain_runs.push(run);
    }

    fn push_path(&mut self, item: TextPathItem) {
        self.draw_items
            .push(TextPathDrawItem::PathItem(self.items.len()));
        self.items.push(item);
    }

    fn push_image(&mut self, image: TextPathImageItem) {
        self.draw_items
            .push(TextPathDrawItem::ImageItem(self.images.len()));
        self.images.push(image);
    }
}

/// A label's drawing items: native text runs where viewers draw text as the label does, outlines
/// for the rest, and bitmap glyphs as images.
pub(crate) fn extract_paths(
    typst: &avenger_typst_label::LabelEngine,
    settings: &LabelSettings,
    config: &TextConfig,
) -> Result<TextPathBuffer, AvengerTextError> {
    let label = typeset(typst, settings, config)?;
    let bounds = bounds_from_metrics(&label.metrics, config.font_size);
    let y_offset = bounds.ascent - first_baseline(&label.metrics);
    let mut output = TextPathBuffer::new(bounds);
    let svg = avenger_typst_label::svg_items(
        &label,
        &avenger_typst_label::SvgOptions { native_text: true },
    );
    for item in svg.items {
        match item {
            SvgItem::Text(run) => output.push_run(text_run(run, y_offset)),
            SvgItem::Path(path) => output.push_path(text_path_item(&path, y_offset)),
            SvgItem::Image(image) => output.push_image(text_path_image_item(&image, y_offset)),
        }
    }
    Ok(output)
}

/// A native text run in label coordinates, below `y_offset` of padding.
fn text_run(run: TextRun, y_offset: f32) -> PlainTextPathRun {
    PlainTextPathRun {
        font: run.font.family().to_string(),
        text: run.text,
        byte_range: run.source,
        face: run.font,
        color: run.fill.to_rgba(),
        is_rtl: run.rtl,
        font_size: run.size,
        font_weight: FontWeight::Number(f32::from(run.weight.to_number())),
        font_style: match run.style {
            avenger_typst_label::FontStyle::Normal => FontStyle::Normal,
            avenger_typst_label::FontStyle::Italic | avenger_typst_label::FontStyle::Oblique => {
                FontStyle::Italic
            }
        },
        x: run.x,
        baseline: run.baseline + y_offset,
        width: run.width,
        text_item: run.text_item,
    }
}

/// A drawing item's path in label coordinates, below `y_offset` of padding.
pub(crate) fn text_path_item(item: &PathItem, y_offset: f32) -> TextPathItem {
    let transform = Transform::translate(0.0, y_offset).pre_concat(item.transform);
    // The stroke is in the path's coordinates; its lengths scale with the transform.
    let scale = (transform.sx * transform.sy - transform.kx * transform.ky)
        .abs()
        .sqrt();
    TextPathItem {
        path: lyon_path(&item.path, transform),
        fill: item.fill.map(|fill| fill.to_rgba()),
        stroke: item.stroke.as_ref().map(|stroke| TextPathStroke {
            color: stroke.paint.to_rgba(),
            width: stroke.thickness * scale,
            line_cap: match stroke.cap {
                LineCap::Butt => TextPathLineCap::Butt,
                LineCap::Round => TextPathLineCap::Round,
                LineCap::Square => TextPathLineCap::Square,
            },
            line_join: match stroke.join {
                LineJoin::Miter => TextPathLineJoin::Miter,
                LineJoin::Round => TextPathLineJoin::Round,
                LineJoin::Bevel => TextPathLineJoin::Bevel,
            },
            dash: stroke.dash.as_ref().map(|dash| TextPathDashPattern {
                array: dash.array.iter().map(|length| length * scale).collect(),
                phase: dash.phase * scale,
            }),
            miter_limit: stroke.miter_limit,
        }),
        kind: match &item.kind {
            PathKind::Glyph(glyph) => TextPathKind::Glyph {
                source: glyph.source.clone(),
            },
            PathKind::Shape => TextPathKind::Shape,
        },
    }
}

/// A bitmap glyph in label coordinates, below `y_offset` of padding.
fn text_path_image_item(image: &ImageItem, y_offset: f32) -> TextPathImageItem {
    let ts = image.transform;
    TextPathImageItem {
        data: image.data.to_vec(),
        format: TextPathImageFormat::Png,
        width: image.size.x,
        height: image.size.y,
        transform: [ts.sx, ts.ky, ts.kx, ts.sy, ts.tx, ts.ty + y_offset],
        byte_range: image.glyph.source.clone(),
        text_item: image.glyph.text,
    }
}

fn lyon_path(curve: &Curve, transform: Transform) -> Path {
    let to_point = |p: avenger_typst_label::Point| {
        let p = transform.apply(p);
        point(p.x, p.y)
    };
    let mut builder = Path::builder().with_svg();
    for item in &curve.0 {
        match *item {
            CurveItem::Move(to) => {
                builder.move_to(to_point(to));
            }
            CurveItem::Line(to) => {
                builder.line_to(to_point(to));
            }
            CurveItem::Cubic(first, second, to) => {
                builder.cubic_bezier_to(to_point(first), to_point(second), to_point(to));
            }
            CurveItem::Close => builder.close(),
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FontStyle, FontWeight};

    fn config(text: &str) -> TextConfig<'_> {
        TextConfig {
            text,
            syntax_mode: crate::types::TextSyntaxMode::TypstMarkup,
            font_size: 10.0,
            color: [0.1, 0.2, 0.3, 1.0],
            ..Default::default()
        }
    }

    /// An engine with the bundled fonts only.
    fn engine() -> crate::TextEngine {
        crate::TextEngine::new(&crate::FontOptions {
            load_system_fonts: false,
            ..crate::default_font_options()
        })
    }

    #[test]
    fn plain_runs_carry_their_faces() {
        let buffer = engine()
            .extract_paths(&TextConfig {
                font: "Lato",
                ..config("Regular _Italic_ *Bold* $sqrt(x)$")
            })
            .unwrap();
        assert!(buffer
            .plain_runs
            .iter()
            .any(|run| run.text.contains("Bold")));
        for run in &buffer.plain_runs {
            let italic = run
                .face
                .postscript_name()
                .is_some_and(|name| name.contains("Italic"));
            assert_eq!(run.font, "Lato");
            assert_eq!(italic, run.font_style == FontStyle::Italic);
            if run.text.contains("Bold") {
                assert_eq!(run.font_weight, FontWeight::Number(700.0));
            }
        }
    }

    #[test]
    fn text_line_extractor_returns_plain_runs_and_math_paths() {
        let text = "speed $v^2$".to_string();
        let buffer = engine().extract_paths(&config(&text)).unwrap();

        assert_eq!(buffer.plain_runs.len(), 1);
        assert_eq!(buffer.plain_runs[0].text, "speed ");
        assert!(!buffer.items.is_empty());
        assert!(matches!(buffer.items[0].kind, TextPathKind::Glyph { .. }));
        assert!(matches!(
            buffer.draw_items.first(),
            Some(TextPathDrawItem::PlainRun(0))
        ));
        assert!(buffer
            .draw_items
            .iter()
            .skip(1)
            .all(|item| matches!(item, TextPathDrawItem::PathItem(_))));
    }

    #[test]
    fn text_line_extractor_returns_plain_runs_and_static_decoration_paths() {
        let text = "#underline[important]".to_string();
        let buffer = engine().extract_paths(&config(&text)).unwrap();

        assert_eq!(buffer.plain_runs.len(), 1);
        assert_eq!(buffer.plain_runs[0].text, "important");
        assert_eq!(buffer.items.len(), 1);
        assert_eq!(buffer.items[0].kind, TextPathKind::Shape);
        assert!(buffer.items[0].stroke.is_some());
        assert!(matches!(
            buffer.items[0].path.iter().last(),
            Some(lyon_path::Event::End { close: false, .. })
        ));
        assert!(matches!(
            buffer.draw_items.as_slice(),
            [TextPathDrawItem::PlainRun(0), TextPathDrawItem::PathItem(0)]
        ));
    }

    #[test]
    fn text_line_extractor_preserves_decoration_dash_phase_and_miter_limit() {
        let text = "#underline(stroke: (thickness: 1pt, dash: (array: (2pt, 1pt), phase: 0.5pt), miter-limit: 2))[important]".to_string();
        let buffer = engine().extract_paths(&config(&text)).unwrap();
        let stroke = buffer.items[0]
            .stroke
            .as_ref()
            .expect("decoration stroke should exist");
        let dash = stroke.dash.as_ref().expect("dash should resolve");

        assert_eq!(dash.array.as_slice(), [2.0, 1.0].as_slice());
        assert_eq!(dash.phase, 0.5);
        assert_eq!(stroke.miter_limit, 2.0);
    }
}
