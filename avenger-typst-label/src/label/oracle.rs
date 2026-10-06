//! The upstream reference fixtures for the crate's own tests: the reader and comparison that
//! the integration tests share, the flattening of the pipeline's internal frames, and the
//! layout of label sources through the pipeline.

#[path = "../../tests/common/oracle.rs"]
mod common;

pub(crate) use self::common::*;

use std::sync::LazyLock;

use ecow::EcoVec;

use super::fixtures::{self, WithSource};
use crate::typst_eval::{eval_label, parse_label};
use crate::typst_layout::inline::layout_label_line;
use crate::typst_library::diag::{SourceDiagnostic, SourceResult};
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Content, Scope, Smart, StyleChain, Styles};
use crate::typst_library::layout::{Abs, Dir, Frame, FrameItem};
use crate::typst_library::math::{EquationElem, LabelMathStyle};
use crate::typst_library::routines::{Arenas, RealizationKind};
use crate::typst_library::text::{
    FontFamily, FontList, FontWeight, Lang, Region, TextDir, TextElem, TextSize,
};
use crate::typst_library::visualize::{
    Color, CurveItem, FixedStroke, Geometry, Paint, Shape,
};
use crate::typst_realize::realize;

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
                flat.push_text(&text.text);
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
                        cluster: text.text[glyph.range()].into(),
                        source: Some([glyph.source.start, glyph.source.end]),
                    });
                }
            }
            FrameItem::Shape(shape, _) => {
                flat.push_rule(shape_rule(&ref_shape(shape), at))
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
        dash: stroke.dash.as_ref().map(|dash| {
            let array: Vec<_> = dash.array.iter().map(|length| length.to_pt()).collect();
            serde_json::json!({ "array": array, "phase": dash.phase.to_pt() })
        }),
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
// Layout

/// The frame suite's default settings, for sources that have no reference.
pub(crate) fn default_settings() -> Settings {
    let defaults = &MANIFEST.defaults;
    Settings {
        text_font: defaults.text_font.clone(),
        math_font: defaults.math_font.clone(),
        font_size: defaults.font_size,
        font_weight: defaults.font_weight,
        lang: None,
        region: None,
        dir: None,
    }
}

/// The probe wrapper's text and equation styles for a case, which live for the rest of the
/// test run.
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
    // The wrapper's `#show math.equation: set text(font: .., weight: ..)`.
    styles.set(
        EquationElem::label_style,
        LabelMathStyle {
            font: Some(FontList(vec![FontFamily::new(&settings.math_font)])),
            weight: Some(FontWeight::from_number(settings.font_weight)),
            ..LabelMathStyle::default()
        },
    );
    StyleChain::new(Box::leak(Box::new(styles)))
}

/// Evaluates a source as a label under a case's settings, realizes it and lays it out as a
/// label line.
pub(crate) fn layout_source(source: &str, settings: &Settings) -> SourceResult<Frame> {
    layout_source_in(source, root_styles(settings)).0
}

/// Evaluates a source as a label under root styles, realizes it and lays it out as a label
/// line, with the warnings.
pub(crate) fn layout_source_in(
    source: &str,
    root: StyleChain,
) -> (SourceResult<Frame>, EcoVec<SourceDiagnostic>) {
    let world = WithSource { world: fixtures::shared(), source };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    let frame = eval_label(&mut engine, &parse_label(source), Scope::new())
        .and_then(|content| layout(&mut engine, &content, root));
    (frame, sink.warnings())
}

/// Realizes content that evaluation can't produce under the default settings and lays it out
/// as a label line.
pub(crate) fn layout_content(content: &Content) -> SourceResult<Frame> {
    let world = WithSource { world: fixtures::shared(), source: "" };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    layout(&mut engine, content, root_styles(&default_settings()))
}

fn layout(
    engine: &mut Engine,
    content: &Content,
    root: StyleChain,
) -> SourceResult<Frame> {
    let arenas = Arenas::default();
    let children = realize(RealizationKind::Par, engine, &arenas, content, root)?;
    Ok(layout_label_line(engine, &children, root)?.frame)
}

/// Lays out every case of a suite from its source and compares each line with its reference,
/// except the cases that `divergent` lists with a reason, which must still differ. Flattened
/// frames of failing cases go to the gitignored `tests/output/internal/`.
pub(crate) fn check_suite(suite: &str, divergent: &[(&str, &str)]) {
    let manifest = Manifest::load(suite);
    let mut failures = vec![];
    for case in &manifest.cases {
        let reference = Reference::load(suite, &case.id).unwrap();
        let Some(expected) = reference.flat() else {
            // Upstream fails, which the evaluator's tests check.
            continue;
        };
        let is_divergent = divergent.iter().any(|(id, _)| *id == case.id);
        let frame = match layout_source(&case.source, &manifest.settings(case)) {
            Ok(frame) => frame,
            Err(errors) => {
                failures.push(format!("{}: fails with {}", case.id, errors[0].message));
                continue;
            }
        };
        let actual = flatten(&frame);
        let mismatches = compare(&expected, &actual, TOLERANCE);
        match (mismatches.failed_checks().is_empty(), is_divergent) {
            (true, true) => failures
                .push(format!("{}: matches upstream, so it isn't divergent", case.id)),
            (false, false) => {
                let dir = output_dir("internal");
                std::fs::create_dir_all(&dir).unwrap();
                for (name, flat) in [("expected", &expected), ("actual", &actual)] {
                    let path = dir.join(format!("{}.{name}.json", case.id));
                    let json = serde_json::to_string_pretty(flat).unwrap();
                    std::fs::write(path, json).unwrap();
                }
                failures.push(format!("{}:\n{}", case.id, mismatches.summary()));
            }
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
