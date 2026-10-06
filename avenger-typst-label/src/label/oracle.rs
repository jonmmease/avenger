//! The upstream reference fixtures for the crate's own tests: the reader and comparison that
//! the integration tests share, the flattening of the pipeline's internal frames, and cases
//! built from hand-made evaluated content.

#[path = "../../tests/common/oracle.rs"]
mod common;

pub(crate) use self::common::*;

use std::ops::Range;
use std::sync::LazyLock;

use ecow::EcoString;

use super::fixtures::{self, WithSource};
use crate::typst_layout::inline::layout_label_line;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{
    Content, NativeElement, Smart, StyleChain, Styles,
};
use crate::typst_library::layout::{Abs, Dir, Frame, FrameItem};
use crate::typst_library::routines::{Arenas, RealizationKind};
use crate::typst_library::text::{
    FontFamily, FontList, FontWeight, Lang, Region, SmartQuoteElem, SpaceElem, TextDir,
    TextElem, TextSize,
};
use crate::typst_library::visualize::{
    Color, CurveItem, FixedStroke, Geometry, Paint, Shape,
};
use crate::typst_realize::realize;
use crate::typst_syntax::{FileId, Span};

const SUITE: &str = "upstream_frames";

static MANIFEST: LazyLock<Manifest> = LazyLock::new(|| Manifest::load(SUITE));

/// Positions and metrics must agree within this many points.
pub(crate) const TOLERANCE: f64 = 1e-3;

/// Flattens a frame as [`Reference::flat`] flattens a reference. Every glyph carries its
/// source range, so the comparison checks the range wherever the reference has one: at the
/// glyphs of nodes that were laid out verbatim.
pub(crate) fn flatten(frame: &Frame) -> Flat {
    let mut flat = Flat {
        width: frame.width().to_pt(),
        height: frame.height().to_pt(),
        baseline: frame.baseline().to_pt(),
        ..Flat::default()
    };
    flatten_into(frame, Affine::IDENTITY, &mut flat);
    flat
}

fn flatten_into(frame: &Frame, transform: Affine, flat: &mut Flat) {
    for (pos, item) in frame.items() {
        let at = transform.then(Affine::translate(pos.x.to_pt(), pos.y.to_pt()));
        match item {
            FrameItem::Group(group) => {
                let t = group.transform;
                let inner = at.then(Affine::new([
                    t.sx.get(),
                    t.ky.get(),
                    t.kx.get(),
                    t.sy.get(),
                    t.tx.to_pt(),
                    t.ty.to_pt(),
                ]));
                flatten_into(&group.frame, inner, flat);
            }
            FrameItem::Text(text) => {
                let font = text.font.font();
                let name =
                    font.post_script_name().unwrap_or_else(|| font.info().family.clone());
                let size = text.size.to_pt();
                let fill = paint(&text.fill);
                let mut pen = 0.0;
                for glyph in &text.glyphs {
                    let (x, y) = at.apply(
                        pen + glyph.x_offset.get() * size,
                        -glyph.y_offset.get() * size,
                    );
                    pen += glyph.x_advance.get() * size;
                    flat.glyphs.push(FlatGlyph {
                        font: name.clone(),
                        id: glyph.id,
                        size,
                        x,
                        y,
                        fill: fill.clone(),
                        source: Some([glyph.source.start, glyph.source.end]),
                    });
                }
            }
            FrameItem::Shape(shape, _) => {
                if let Some(rule) = shape_rule(&ref_shape(shape), at) {
                    flat.rules.push(rule);
                }
            }
        }
    }
}

/// A shape as the probe writes it.
fn ref_shape(shape: &Shape) -> RefShape {
    let geometry = match &shape.geometry {
        Geometry::Line(to) => RefGeometry::Line([to.x.to_pt(), to.y.to_pt()]),
        Geometry::Rect(size) => RefGeometry::Rect([size.x.to_pt(), size.y.to_pt()]),
        Geometry::Curve(curve) => {
            let pt = |p: &crate::typst_library::layout::Point| {
                serde_json::json!([p.x.to_pt(), p.y.to_pt()])
            };
            let items = curve.0.iter().map(|item| match item {
                CurveItem::Move(p) => serde_json::json!(["move", pt(p)]),
                CurveItem::Line(p) => serde_json::json!(["line", pt(p)]),
                CurveItem::Cubic(a, b, c) => {
                    serde_json::json!(["cubic", pt(a), pt(b), pt(c)])
                }
                CurveItem::Close => serde_json::json!(["close"]),
            });
            RefGeometry::Curve(items.collect())
        }
    };
    RefShape {
        geometry,
        fill: shape.fill.as_ref().map(paint),
        fill_rule: format!("{:?}", shape.fill_rule),
        stroke: shape.stroke.as_ref().map(stroke),
        span: None,
    }
}

fn stroke(stroke: &FixedStroke) -> RefStroke {
    RefStroke {
        paint: paint(&stroke.paint),
        thickness: stroke.thickness.to_pt(),
        cap: format!("{:?}", stroke.cap),
        join: format!("{:?}", stroke.join),
        dash: None,
        miter_limit: stroke.miter_limit.get(),
    }
}

/// `#rrggbbaa`, rounded to bytes as upstream's `Color::to_vec4_u8` rounds.
pub(crate) fn paint(paint: &Paint) -> String {
    let Paint::Solid(color) = paint;
    hex(color)
}

fn hex(color: &Color) -> String {
    let [r, g, b, a] = color.to_rgba().map(|c| (c * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}

// ---------------------------------------------------------------------------------------------
// Cases

/// A frame reference case: its source, the probe wrapper's root styles, and a cursor for
/// finding the source of hand-made content. Until the pipeline evaluates sources, tests build
/// the content that evaluation would produce, with the same spans.
pub(crate) struct Case {
    pub id: &'static str,
    pub source: &'static str,
    /// The root styles, which live for the rest of the test run.
    pub root: StyleChain<'static>,
    /// Where the next source lookup starts.
    cursor: usize,
}

impl Case {
    /// The case with this id in the manifest.
    pub fn new(id: &'static str) -> Self {
        let case = MANIFEST
            .cases
            .iter()
            .find(|case| case.id == id)
            .unwrap_or_else(|| panic!("no case {id}"));
        Self::with_settings(id, &case.source, MANIFEST.settings(case))
    }

    /// A case for a source without a reference, under the manifest's default settings.
    pub fn custom(source: &str) -> Self {
        let defaults = &MANIFEST.defaults;
        let settings = Settings {
            text_font: defaults.text_font.clone(),
            math_font: defaults.math_font.clone(),
            font_size: defaults.font_size,
            font_weight: defaults.font_weight,
            lang: None,
            region: None,
            dir: None,
        };
        Self::with_settings("custom", source, settings)
    }

    fn with_settings(id: &'static str, source: &str, settings: Settings) -> Self {
        Self {
            id,
            source: Box::leak(source.into()),
            root: root_styles(&settings),
            cursor: 0,
        }
    }

    /// The range of the next `needle` in the source.
    pub fn find(&mut self, needle: &str) -> Range<usize> {
        let Some(offset) = self.source[self.cursor..].find(needle) else {
            panic!("{}: no {needle:?} after byte {}", self.id, self.cursor);
        };
        let start = self.cursor + offset;
        self.cursor = start + needle.len();
        start..self.cursor
    }

    /// The span of the next `needle` in the source.
    pub fn span(&mut self, needle: &str) -> Span {
        span(self.find(needle))
    }

    /// The next `needle` in the source as text and spaces.
    pub fn words(&mut self, needle: &str) -> Content {
        let range = self.find(needle);
        let mut start = range.start;
        let mut children = vec![];
        for run in word_runs(&self.source[range]) {
            let span = span(start..start + run.len());
            children.push(if run.starts_with(' ') {
                SpaceElem::shared().clone().spanned(span)
            } else {
                TextElem::packed(run).spanned(span)
            });
            start += run.len();
        }
        Content::sequence(children)
    }

    /// The next `needle` in the source as markup without markup: text, spaces, and smart
    /// quotes for its straight quotes.
    pub fn markup(&mut self, needle: &str) -> Content {
        let children: Vec<_> = quote_runs(needle)
            .map(|run| match run {
                "\"" | "'" => self.quote(run),
                _ => self.words(run),
            })
            .collect();
        Content::sequence(children)
    }

    /// A text element with `text`, made from the next `needle` in the source.
    pub fn text(&mut self, text: &str, needle: &str) -> Content {
        TextElem::packed(EcoString::from(text)).spanned(self.span(needle))
    }

    /// A space made from the next `needle` in the source.
    pub fn space(&mut self, needle: &str) -> Content {
        SpaceElem::shared().clone().spanned(self.span(needle))
    }

    /// A smart quote made from the next `needle`.
    pub fn quote(&mut self, needle: &str) -> Content {
        let span = self.span(needle);
        SmartQuoteElem::new().with_double(needle == "\"").pack().spanned(span)
    }

    /// Realizes the content under the root styles and lays it out as a label line.
    pub fn layout(&self, content: &Content) -> SourceResult<Frame> {
        let world = WithSource { world: fixtures::shared(), source: self.source };
        let mut sink = Sink::new();
        let mut engine = Engine { world: &world, sink: &mut sink };
        let arenas = Arenas::default();
        let children =
            realize(RealizationKind::Par, &mut engine, &arenas, content, self.root)?;
        layout_label_line(&mut engine, &children, self.root)
    }

    /// Lays out the content and compares the line with the reference.
    pub fn compare(&self, content: &Content) -> Result<(), String> {
        let frame =
            self.layout(content).map_err(|err| format!("{}: {err:?}", self.id))?;
        let reference = Reference::load(SUITE, self.id)?;
        if reference.source != self.source {
            return Err(format!("{}: stale reference", self.id));
        }
        let expected = reference.flat().expect("upstream lays the case out");
        let actual = flatten(&frame);
        let mismatches = compare(&expected, &actual, TOLERANCE);
        if mismatches.failed_checks().is_empty() {
            return Ok(());
        }
        let dir = output_dir("internal");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, flat) in [("expected", &expected), ("actual", &actual)] {
            let path = dir.join(format!("{}.{name}.json", self.id));
            std::fs::write(path, serde_json::to_string_pretty(flat).unwrap()).unwrap();
        }
        Err(format!("{} differs from upstream:\n{}", self.id, mismatches.summary()))
    }
}

/// The probe wrapper's text styles for a case, which live for the rest of the test run.
pub(crate) fn root_styles(settings: &Settings) -> StyleChain<'static> {
    let mut styles = Styles::new();
    styles.set(TextElem::font, FontList(vec![FontFamily::new(&settings.text_font)]));
    styles.set(TextElem::size, TextSize(Abs::pt(settings.font_size).into()));
    styles.set(TextElem::weight, FontWeight::from_number(settings.font_weight));
    if let Some(lang) = &settings.lang {
        styles.set(TextElem::lang, lang.parse::<Lang>().unwrap());
    }
    if let Some(region) = &settings.region {
        styles.set(TextElem::region, Some(region.parse::<Region>().unwrap()));
    }
    if let Some(dir) = &settings.dir {
        let dir = match dir.as_str() {
            "ltr" => Dir::LTR,
            "rtl" => Dir::RTL,
            other => panic!("unexpected direction {other}"),
        };
        styles.set(TextElem::dir, TextDir(Smart::Custom(dir)));
    }
    StyleChain::new(Box::leak(Box::new(styles)))
}

/// Compares every case with its reference and reports all failures. Flattened frames of
/// failing cases go to the gitignored `tests/output/internal/`.
pub(crate) fn check(cases: impl IntoIterator<Item = (Case, Content)>) {
    let failures: Vec<_> = cases
        .into_iter()
        .filter_map(|(case, content)| case.compare(&content).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Checks cases whose source is plain markup.
pub(crate) fn check_plain(ids: impl IntoIterator<Item = &'static str>) {
    check(ids.into_iter().map(|id| {
        let mut case = Case::new(id);
        let source = case.source;
        let content = case.markup(source);
        (case, content)
    }));
}

/// A span into the label source.
pub(crate) fn span(range: Range<usize>) -> Span {
    Span::from_range(FileId::LABEL, range)
}

/// `text` in maximal runs of spaces and of other characters.
fn word_runs(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let space = rest.starts_with(' ');
        let end = rest.find(|c: char| (c == ' ') != space).unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        rest = tail;
        Some(run)
    })
}

/// `text` with each straight quote split off on its own.
fn quote_runs(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let end = if matches!(first, '"' | '\'') {
            1
        } else {
            rest.find(['"', '\'']).unwrap_or(rest.len())
        };
        let (run, tail) = rest.split_at(end);
        rest = tail;
        Some(run)
    })
}
