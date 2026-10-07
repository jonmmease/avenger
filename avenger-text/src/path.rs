use std::ops::Range;

use avenger_typst_label::{
    Curve, CurveItem, FontRef, ImageItem, LineCap, LineJoin, PathItem, PathKind, SvgItem, TextItem,
    Transform,
};
use lyon_path::{geom::point, Path};
use rustybuzz::{ttf_parser, BufferFlags, Direction, UnicodeBuffer};

use crate::{
    error::AvengerTextError,
    math::TextMarkupConfig,
    measurement::{TextBounds, TextMeasurementConfig},
    text_line::{
        bounds_from_metrics, is_rtl, tight_bounds_from_metrics, typeset_line, TextLineMeasurer,
    },
    types::{FontStyle, FontWeight, TextSyntaxMode},
};

#[derive(Debug, Clone)]
pub struct TextPathExtractionConfig<'a> {
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
    /// The top of the run's bounds.
    pub y_offset: f32,
    /// The run's advance width, with the face's ascent and descent.
    pub bounds: TextBounds,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPathDrawItem {
    PlainRun(usize),
    PathItem(usize),
    ImageItem(usize),
}

#[derive(Debug, Clone)]
pub struct TextPathBuffer {
    /// When set, the consumer must clip every draw item to x <= this cutoff in
    /// label coordinates, before applying the label's placement transform.
    pub clip_width: Option<f32>,
    pub bounds: TextBounds,
    pub items: Vec<TextPathItem>,
    pub images: Vec<TextPathImageItem>,
    pub plain_runs: Vec<PlainTextPathRun>,
    pub draw_items: Vec<TextPathDrawItem>,
}

impl TextPathBuffer {
    pub fn new(bounds: TextBounds) -> Self {
        Self {
            clip_width: None,
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

#[derive(Debug, Clone)]
pub(crate) struct TextPathExtractorImpl {
    typst: avenger_typst_label::LabelEngine,
    math: TextMarkupConfig,
}

impl TextPathExtractorImpl {
    pub(crate) fn new(typst: avenger_typst_label::LabelEngine, math: TextMarkupConfig) -> Self {
        Self { typst, math }
    }

    fn measure_text_bounds(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        TextLineMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    pub(crate) fn extract_text_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
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
        let mut output = TextPathBuffer::new(bounds);
        output.clip_width = clip_width;

        // Text items that read as native text draw as runs, in the place of their first glyph;
        // the others draw as outlines. Bitmap glyphs draw as images either way. Drawing items
        // come in text item order, so a run without glyph items, such as a run of spaces,
        // draws before the next item's first.
        let mut runs: Vec<_> = result
            .label
            .frame
            .text_items()
            .into_iter()
            .map(|(ts, item)| RunState::new(plain_run(ts, item, y_offset)))
            .collect();
        let mut placed = 0;
        let mut place = |output: &mut TextPathBuffer, text: usize| {
            let end = text.min(runs.len());
            for run in &mut runs[placed.min(end)..end] {
                run.draw(output);
            }
            placed = placed.max(end);
            runs.get_mut(text).is_some_and(|run| run.draw(output))
        };
        let svg = avenger_typst_label::svg_items(
            &result.label,
            &avenger_typst_label::SvgOptions::default(),
        );
        for item in svg.items {
            match item {
                SvgItem::Path(path) => match &path.kind {
                    PathKind::Glyph(glyph) => {
                        if !place(&mut output, glyph.text) {
                            output.push_path(text_path_item(&path, y_offset));
                        }
                    }
                    PathKind::Shape => output.push_path(text_path_item(&path, y_offset)),
                },
                SvgItem::Image(image) => {
                    place(&mut output, image.glyph.text);
                    output.push_image(text_path_image_item(&image, y_offset));
                }
                // Without native text, the label's text lowers to outlines.
                SvgItem::Text(_) => {}
            }
        }
        place(&mut output, usize::MAX);

        Ok(output)
    }
}

/// Whether a text item draws as a native run.
enum RunState {
    Outlined,
    Pending(PlainTextPathRun),
    Drawn,
}

impl RunState {
    fn new(run: Option<PlainTextPathRun>) -> Self {
        run.map_or(Self::Outlined, Self::Pending)
    }

    /// Draws the run if it is pending. Returns whether the item is a native run.
    fn draw(&mut self, output: &mut TextPathBuffer) -> bool {
        if let Self::Pending(_) = self {
            if let Self::Pending(run) = std::mem::replace(self, Self::Drawn) {
                output.push_run(run);
            }
        }
        !matches!(self, Self::Outlined)
    }
}

/// A text item as a native run, if it reads as one: it is unrotated, its face is static and
/// not a math face, and shaping its text with the default features reproduces its glyphs.
/// Math faces stay outlines, so that documents don't embed them for a few glyphs.
fn plain_run(ts: Transform, item: &TextItem, y_offset: f32) -> Option<PlainTextPathRun> {
    if (ts.sx, ts.ky, ts.kx, ts.sy) != (1.0, 0.0, 0.0, 1.0) || !item.font.variations().is_empty() {
        return None;
    }
    let face = rustybuzz::Face::from_slice(item.font.data(), item.font.index())?;
    if face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"MATH"))
        .is_some()
    {
        return None;
    }
    let is_rtl = is_rtl(item);
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(&item.text);
    buffer.set_direction(if is_rtl {
        Direction::RightToLeft
    } else {
        Direction::LeftToRight
    });
    buffer.guess_segment_properties();
    // As the label engine shapes: default ignorables draw nothing.
    buffer.set_flags(BufferFlags::REMOVE_DEFAULT_IGNORABLES);
    let shaped = rustybuzz::shape(&face, &[], buffer);
    if shaped.glyph_infos().len() != item.glyphs.len() {
        return None;
    }

    // Advances and vertical offsets must match. Horizontal offsets may differ by a constant,
    // as synthesized scripts' do, which moves the run.
    let units = item.font.units_per_em();
    let em = |units_value: i32| units_value as f32 / units;
    let mut shift = None;
    for ((glyph, info), position) in item
        .glyphs
        .iter()
        .zip(shaped.glyph_infos())
        .zip(shaped.glyph_positions())
    {
        let offset = glyph.x_offset - em(position.x_offset);
        if u32::from(glyph.id) != info.glyph_id
            || glyph.range.start != info.cluster as usize
            || (glyph.x_advance - em(position.x_advance)).abs() > EM_TOLERANCE
            || position.y_offset != 0
            || position.y_advance != 0
            || (offset - *shift.get_or_insert(offset)).abs() > EM_TOLERANCE
        {
            return None;
        }
    }

    let size = item.size;
    let ascent = em(i32::from(face.ascender())) * size;
    let descent = -em(i32::from(face.descender())) * size;
    let baseline = ts.ty + y_offset;
    Some(PlainTextPathRun {
        text: item.text.clone(),
        byte_range: item.source.clone(),
        face: item.font.clone(),
        color: item.fill.to_rgba(),
        is_rtl,
        font: item.font.family().to_string(),
        font_size: size,
        font_weight: FontWeight::Number(f32::from(face.weight().to_number())),
        font_style: match face.style() {
            ttf_parser::Style::Normal => FontStyle::Normal,
            ttf_parser::Style::Italic | ttf_parser::Style::Oblique => FontStyle::Italic,
        },
        x: ts.tx + shift.unwrap_or(0.0) * size,
        y_offset: baseline - ascent,
        bounds: TextBounds {
            width: item.width(),
            height: ascent + descent,
            ascent,
            descent,
            line_height: ascent + descent,
        },
    })
}

/// How far a run's advances and offsets may stray from shaping's, in ems.
const EM_TOLERANCE: f32 = 1e-4;

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
    use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};

    fn config(text: &str) -> TextPathExtractionConfig<'_> {
        static COLOR: [f32; 4] = [0.1, 0.2, 0.3, 1.0];
        static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
        static STYLE: FontStyle = FontStyle::Normal;

        TextPathExtractionConfig {
            text,
            color: COLOR,
            font: "",
            font_size: 10.0,
            font_weight: WEIGHT,
            font_style: STYLE,
            limit: f32::INFINITY,
            syntax_mode: TextSyntaxMode::TypstMarkup,
            params: crate::empty_label_params(),
            number_format: None,
            datetime_format: None,
        }
    }

    /// An engine with the bundled fonts only.
    fn engine() -> crate::TextEngine {
        crate::TextEngine::with_fonts(&crate::FontOptions {
            load_system_fonts: false,
            ..crate::default_font_options()
        })
    }

    fn lato_runs(source: &str) -> Vec<(Transform, TextItem)> {
        use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine, LabelOptions};
        let engine = LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                registered_fonts: crate::fonts::registered_default_fonts(),
                ..Default::default()
            },
        });
        let mut options = LabelOptions::default();
        options.text.font_family = "Lato".into();
        options.text.font_size = 40.0;
        let label = engine.compile(source, &options).unwrap();
        label
            .frame
            .text_items()
            .into_iter()
            .map(|(ts, item)| (ts, item.clone()))
            .collect()
    }

    #[test]
    fn typographic_scripts_keep_parent_size_and_baseline() {
        for function in ["sub", "super"] {
            // Explicit size and baseline affect only synthesized scripts.
            for arguments in ["", "(size: 0.25em, baseline: 0.8em)"] {
                let runs = lato_runs(&format!("H#{function}{arguments}[2]O"));
                assert_eq!(runs.len(), 3);
                let (parent_transform, parent) = &runs[0];
                let (script_transform, script) = &runs[1];
                assert_eq!(script.text, "2");
                assert_eq!(script.size, 40.0);
                assert_eq!(script_transform.ty, parent_transform.ty);
                // The font's script glyph isn't the digit's.
                assert!(plain_run(*script_transform, script, 0.0).is_none());
                assert!(script.width() < parent.width());
            }
        }
    }

    #[test]
    fn incomplete_script_features_synthesize_the_entire_run() {
        // Lato provides script digits but no script at sign.
        for function in ["sub", "super"] {
            let runs = lato_runs(&format!("H#{function}(size: 0.5em)[2@]O"));
            let (transform, script) = runs.iter().find(|(_, run)| run.text == "2@").unwrap();
            assert_eq!(script.size, 20.0);
            assert_eq!(script.font.family(), "Lato");
            assert!(plain_run(*transform, script, 0.0).is_some());
        }
    }

    #[test]
    fn svg_extraction_outlines_runs_with_substituted_glyphs() {
        // Lato's typographic scripts substitute script glyphs. Lato has no small capitals,
        // so small caps keep the ordinary glyphs and stay native.
        for (source, native, outlined) in [
            ("H#sub[2]O", "HO", true),
            ("H#super[2]O", "HO", true),
            ("#smallcaps[Smallcaps]", "Smallcaps", false),
            ("H#smallcaps(all: true)[CAPS]O", "HCAPSO", false),
        ] {
            let buffer = engine()
                .extract_paths(&TextPathExtractionConfig {
                    font: "Lato",
                    font_size: 40.0,
                    ..config(source)
                })
                .unwrap();
            assert_eq!(!buffer.items.is_empty(), outlined, "{source}");
            assert!(buffer
                .items
                .iter()
                .all(|item| matches!(item.kind, TextPathKind::Glyph { .. })));
            let text: String = buffer
                .plain_runs
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            assert_eq!(text, native);
        }
    }

    #[test]
    fn plain_runs_carry_their_faces() {
        let buffer = engine()
            .extract_paths(&TextPathExtractionConfig {
                font: "Lato",
                ..config("Regular _Italic_ *Bold* $sqrt(x)$")
            })
            .unwrap();
        assert!(buffer
            .plain_runs
            .iter()
            .any(|run| run.text.contains("Bold")));
        for run in &buffer.plain_runs {
            let face = ttf_parser::Face::parse(run.face.data(), run.face.index()).unwrap();
            assert_eq!(run.font, "Lato");
            assert_eq!(face.is_italic(), run.font_style == FontStyle::Italic);
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

    #[test]
    fn runs_keep_their_order_in_the_line() {
        let text = "#underline[Decorations] _Italic_".to_string();
        let buffer = engine().extract_paths(&config(&text)).unwrap();
        let runs: Vec<_> = buffer
            .draw_items
            .iter()
            .filter_map(|item| match item {
                TextPathDrawItem::PlainRun(index) => Some(buffer.plain_runs[*index].text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(runs, ["Decorations", " ", "Italic"]);
    }

    #[test]
    fn text_line_extractor_returns_synthesized_script_runs_with_smaller_style() {
        let text = "H#sub(typographic: false)[2]O #super(typographic: false)[\\*]".to_string();
        let buffer = engine().extract_paths(&config(&text)).unwrap();

        assert_eq!(buffer.plain_runs.len(), 4);
        assert_eq!(buffer.plain_runs[0].text, "H");
        assert_eq!(buffer.plain_runs[1].text, "2");
        assert_eq!(buffer.plain_runs[2].text, "O ");
        assert_eq!(buffer.plain_runs[3].text, "*");
        assert!(buffer.plain_runs[1].font_size < buffer.plain_runs[0].font_size);
        assert!(buffer.plain_runs[3].font_size < buffer.plain_runs[0].font_size);
        assert!(buffer.plain_runs[1].y_offset > buffer.plain_runs[0].y_offset);
        assert!(buffer.plain_runs[3].y_offset < buffer.plain_runs[0].y_offset);
    }
}
