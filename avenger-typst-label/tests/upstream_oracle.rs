//! Compares the public label pipeline with upstream Typst's frames.
//!
//! References live in `tests/fixtures/upstream_{frames,math}/ref` and are written by
//! `tools/upstream-typst-probe`. Every case must lay out to upstream's frame, except the
//! deliberate divergences listed here. Flattened frames of failing cases go to the gitignored
//! `tests/output/{suite}/` for diffing.

mod common;

use std::fs;

use avenger_typst_label::{
    CompiledLabel, CurveItem, FontWeight, FrameItem, Geometry, LabelEngine, LabelFrame,
    LabelOptions, Shape, TextDir,
};
use common::oracle::{
    Affine, Flat, FlatGlyph, Manifest, RefGeometry, RefShape, RefStroke, Reference,
    Settings, compare, output_dir, shape_rule,
};

/// Positions and metrics must agree within this many points.
const TOLERANCE: f64 = 1e-3;

/// Cases that deliberately differ from upstream, with the reason.
const DIVERGENT: &[(&str, &str)] = &[(
    "text-unknown-family",
    "an unknown family falls back to the engine's sans-serif family first, where upstream \
     falls back to Libertinus Serif and then to the first font that covers the text",
)];

#[test]
fn frames_match_upstream() {
    check("upstream_frames");
}

#[test]
fn math_matches_upstream() {
    check("upstream_math");
}

fn check(suite: &str) {
    let manifest = Manifest::load(suite);
    let mut engine_options = common::engine_options();
    engine_options.fonts.load_system_fonts = false;
    let engine = LabelEngine::new(engine_options);

    let out_dir = output_dir(suite);
    fs::remove_dir_all(&out_dir).ok();

    let mut failures = vec![];
    for case in &manifest.cases {
        let reference =
            Reference::load(suite, &case.id).unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(reference.source, case.source, "{}: stale reference", case.id);
        let divergent = DIVERGENT.iter().any(|(id, _)| *id == case.id);
        let compiled = engine.compile(&case.source, &options(&manifest.settings(case)));
        let matches = match (reference.flat(), compiled) {
            (Some(expected), Ok(label)) => {
                let actual = flatten(&label);
                let mismatches = compare(&expected, &actual, TOLERANCE);
                let matches = mismatches.failed_checks().is_empty();
                if !matches && !divergent {
                    fs::create_dir_all(&out_dir).unwrap();
                    for (name, flat) in [("expected", &expected), ("actual", &actual)] {
                        let path = out_dir.join(format!("{}.{name}.json", case.id));
                        fs::write(path, serde_json::to_string_pretty(flat).unwrap())
                            .unwrap();
                    }
                    failures.push(format!("{}:\n{}", case.id, mismatches.summary()));
                }
                matches
            }
            (Some(_), Err(err)) => {
                failures.push(format!("{}: {err}", case.id));
                false
            }
            (None, Ok(_)) => {
                failures.push(format!("{}: compiles, but upstream fails", case.id));
                false
            }
            // Upstream fails, and so does the label; the evaluator's tests check the error.
            (None, Err(_)) => true,
        };
        if matches && divergent {
            failures
                .push(format!("{}: matches upstream, so it isn't divergent", case.id));
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

fn options(settings: &Settings) -> LabelOptions {
    let mut options = LabelOptions::default();
    options.text.font_family = settings.text_font.clone();
    options.text.font_size = settings.font_size as f32;
    options.text.font_weight = FontWeight::from_number(settings.font_weight);
    if let Some(lang) = &settings.lang {
        options.text.lang = lang.parse().unwrap();
    }
    if let Some(region) = &settings.region {
        options.text.region = Some(region.parse().unwrap());
    }
    options.text.dir = match settings.dir.as_deref() {
        None => TextDir::Auto,
        Some("ltr") => TextDir::Ltr,
        Some("rtl") => TextDir::Rtl,
        Some(other) => panic!("unexpected direction {other}"),
    };
    // The wrapper's `#show math.equation: set text(font: .., weight: ..)`.
    options.math.font_family = settings.math_font.clone();
    options.math.font_weight = Some(FontWeight::from_number(settings.font_weight));
    options
}

/// Flattens the public frame as the references flatten: glyphs at their pen positions plus
/// offsets, and shapes as rules.
fn flatten(label: &CompiledLabel) -> Flat {
    let frame = &label.frame;
    let mut flat = Flat {
        width: f64::from(frame.size.x),
        height: f64::from(frame.size.y),
        baseline: f64::from(frame.baseline),
        ..Flat::default()
    };
    flatten_into(frame, Affine::IDENTITY, &mut flat);
    flat
}

fn flatten_into(frame: &LabelFrame, transform: Affine, flat: &mut Flat) {
    for (pos, item) in &frame.items {
        let at = transform.then(Affine::translate(f64::from(pos.x), f64::from(pos.y)));
        match item {
            FrameItem::Group(group) => {
                let t = group.transform;
                let inner = at.then(Affine::new(
                    [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f64::from),
                ));
                flatten_into(&group.frame, inner, flat);
            }
            FrameItem::Text(text) => {
                flat.push_text(&text.text);
                let font = text
                    .font
                    .postscript_name()
                    .unwrap_or_else(|| text.font.family().into());
                let size = f64::from(text.size);
                let fill = hex(text.fill.to_rgba8());
                let mut pen = 0.0;
                for glyph in &text.glyphs {
                    let (x, y) = at.apply(
                        pen + f64::from(glyph.x_offset) * size,
                        -f64::from(glyph.y_offset) * size,
                    );
                    pen += f64::from(glyph.x_advance) * size;
                    flat.glyphs.push(FlatGlyph {
                        font: font.clone(),
                        id: glyph.id,
                        size,
                        x,
                        y,
                        fill: fill.clone(),
                        cluster: text.text[glyph.range.clone()].into(),
                        source: Some([glyph.source.start, glyph.source.end]),
                    });
                }
            }
            FrameItem::Shape(shape) => flat.push_rule(shape_rule(&ref_shape(shape), at)),
        }
    }
}

/// A shape as the probe writes it.
fn ref_shape(shape: &Shape) -> RefShape {
    let geometry = match &shape.geometry {
        Geometry::Line(to) => RefGeometry::Line([to.x, to.y].map(f64::from)),
        Geometry::Rect(size) => RefGeometry::Rect([size.x, size.y].map(f64::from)),
        Geometry::Curve(curve) => {
            let pt = |p: avenger_typst_label::Point| serde_json::json!([p.x, p.y]);
            let items = curve.0.iter().map(|item| match *item {
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
        fill: shape.fill.map(|fill| hex(fill.to_rgba8())),
        fill_rule: format!("{:?}", shape.fill_rule),
        stroke: shape.stroke.as_ref().map(|stroke| RefStroke {
            paint: hex(stroke.paint.to_rgba8()),
            thickness: f64::from(stroke.thickness),
            cap: format!("{:?}", stroke.cap),
            join: format!("{:?}", stroke.join),
            dash: stroke.dash.as_ref().map(
                |dash| serde_json::json!({ "array": dash.array, "phase": dash.phase }),
            ),
            miter_limit: f64::from(stroke.miter_limit),
        }),
        span: None,
    }
}

fn hex([r, g, b, a]: [u8; 4]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}
