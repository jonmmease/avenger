//! Compares the public label pipeline with upstream Typst's frames.
//!
//! References live in `tests/fixtures/upstream_{frames,math}/ref` and are written by
//! `tools/upstream-typst-probe`. Each suite's `expected_failures.toml` lists the cases the
//! current pipeline fails, so the census shows progress; a case that starts passing must be
//! removed from the list. Flattened frames of failing cases go to the gitignored
//! `tests/output/{suite}/` for diffing.

mod common;

use std::{collections::BTreeSet, fs};

use avenger_typst_label::{
    CompiledLabel, FontWeight, LabelEngine, LabelFrameItem, LabelOptions, MathFontSpec,
    PathCommand, PathItem, PathKind, Point,
};
use common::oracle::{
    Census, ExpectedFailures, Flat, FlatGlyph, FlatRule, Manifest, Reference, Settings, compare,
    output_dir,
};

/// Positions and metrics must agree within this many points.
const TOLERANCE: f64 = 1e-3;

#[test]
fn upstream_frames_census() {
    census("upstream_frames");
}

#[test]
fn upstream_math_census() {
    census("upstream_math");
}

fn census(suite: &str) {
    let manifest = Manifest::load(suite);
    let expected = ExpectedFailures::load(suite, "expected_failures.toml");
    let mut engine_options = common::engine_options();
    engine_options.fonts.load_system_fonts = false;
    let engine = LabelEngine::new(engine_options).expect("label engine should initialize");

    let out_dir = output_dir(suite);
    fs::remove_dir_all(&out_dir).ok();

    let mut census = Census::default();
    for case in &manifest.cases {
        let reference = Reference::load(suite, &case.id).unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(
            reference.source, case.source,
            "{}: stale reference",
            case.id
        );
        let compiled = engine.compile(&case.source, &options(&manifest.settings(case)));

        let (failed, detail) = match (reference.flat(), compiled) {
            (Some(expected), Ok(label)) => {
                let actual = flatten(&label);
                let mismatches = compare(&expected, &actual, TOLERANCE);
                let failed = mismatches.failed_checks();
                if !failed.is_empty() {
                    fs::create_dir_all(&out_dir).unwrap();
                    for (name, flat) in [("expected", &expected), ("actual", &actual)] {
                        let path = out_dir.join(format!("{}.{name}.json", case.id));
                        fs::write(path, serde_json::to_string_pretty(flat).unwrap()).unwrap();
                    }
                }
                let failed = failed.into_iter().map(String::from).collect();
                (failed, mismatches.summary())
            }
            (Some(_), Err(err)) => (BTreeSet::from(["compile".into()]), format!("    {err}\n")),
            (None, Ok(_)) => {
                let errors = reference.errors.iter().map(|error| error.message.as_str());
                let detail = format!(
                    "    upstream errors: {}\n",
                    errors.collect::<Vec<_>>().join("; ")
                );
                (BTreeSet::from(["compile".into()]), detail)
            }
            (None, Err(_)) => (BTreeSet::new(), String::new()),
        };
        census.record(&case.id, failed, detail);
    }
    let file = format!("tests/fixtures/{suite}/expected_failures.toml");
    census.check(&expected, &file, &out_dir);
}

fn options(settings: &Settings) -> LabelOptions {
    let mut options = LabelOptions::default();
    options.text.font_family = settings.text_font.clone();
    options.text.font_size = settings.font_size as f32;
    options.text.font_weight = FontWeight::Number(settings.font_weight);
    options.math.font = match settings.math_font.as_str() {
        "Lete Sans Math" => MathFontSpec::LeteSansMath,
        family => MathFontSpec::Family(family.to_string()),
    };
    options.math.font_size = settings.font_size as f32;
    options.math.font_weight = FontWeight::Number(settings.font_weight);
    // The current pipeline has no `lang`, `region` or `dir` options; cases that set them run
    // with the defaults.
    options
}

/// Flattens the current public frame. Glyph positions come from the PDF glyph runs, whose
/// transforms are absolute; outlines of glyphs are skipped and other paths become rules.
fn flatten(label: &CompiledLabel) -> Flat {
    let mut flat = Flat {
        width: f64::from(label.frame.size.x),
        height: f64::from(label.frame.size.y),
        baseline: f64::from(label.frame.baseline),
        ..Flat::default()
    };
    flatten_items(&label.frame.items, &mut flat);
    flat
}

fn flatten_items(items: &[(Point, LabelFrameItem)], flat: &mut Flat) {
    for (_, item) in items {
        match item {
            LabelFrameItem::Text(text) => {
                let Some(pdf) = &text.pdf_text else { continue };
                for run in &pdf.glyph_runs {
                    let font = text
                        .font_resources
                        .iter()
                        .find(|resource| resource.id == run.font)
                        .map(|resource| {
                            resource
                                .postscript_name
                                .clone()
                                .unwrap_or_else(|| resource.family.clone())
                        })
                        .unwrap_or_default();
                    let c = run.fill;
                    let fill = hex([c.r, c.g, c.b, c.a]);
                    for glyph in &run.glyphs {
                        let t = glyph.transform;
                        let (gx, gy) = (f64::from(glyph.x), f64::from(glyph.y));
                        let x = f64::from(t.sx) * gx + f64::from(t.kx) * gy + f64::from(t.tx);
                        let y = f64::from(t.ky) * gx + f64::from(t.sy) * gy + f64::from(t.ty);
                        flat.glyphs.push(FlatGlyph {
                            font: font.clone(),
                            id: glyph.glyph_id,
                            size: f64::from(run.font_size),
                            x,
                            y,
                            fill: fill.clone(),
                            source: Some([glyph.text_range.start, glyph.text_range.end]),
                        });
                    }
                }
            }
            LabelFrameItem::Shape(shape) => {
                if let Some(rule) = path_rule(&shape.item) {
                    flat.rules.push(rule);
                }
            }
            LabelFrameItem::Group(group) => flatten_items(&group.items, flat),
            LabelFrameItem::Image(_) => {}
        }
    }
}

fn path_rule(item: &PathItem) -> Option<FlatRule> {
    if matches!(item.kind, PathKind::GlyphOutline { .. }) {
        return None;
    }
    let mut points = Vec::new();
    for command in &item.path.commands {
        match *command {
            PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => points.push((x, y)),
            PathCommand::QuadTo { x1, y1, x, y } => points.extend([(x1, y1), (x, y)]),
            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => points.extend([(x1, y1), (x2, y2), (x, y)]),
            PathCommand::Close => {}
        }
    }
    let (x0, y0, x1, y1) = points.iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(x0, y0, x1, y1), &(x, y)| {
            let (x, y) = (f64::from(x), f64::from(y));
            (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
        },
    );
    if points.is_empty() {
        return None;
    }
    let (inflate, paint) = match (&item.stroke, &item.fill) {
        (Some(stroke), _) => {
            let c = stroke.color;
            (f64::from(stroke.width) / 2.0, hex([c.r, c.g, c.b, c.a]))
        }
        (None, Some(c)) => (0.0, hex([c.r, c.g, c.b, c.a])),
        (None, None) => return None,
    };
    let (ix, iy) = match (item.stroke.is_some(), y0 == y1, x0 == x1) {
        (true, true, _) => (0.0, inflate),
        (true, _, true) => (inflate, 0.0),
        _ => (inflate, inflate),
    };
    let t = item.transform;
    let at = common::oracle::Affine::new([t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f64::from));
    let [x0, y0, x1, y1] = at.bounds(x0 - ix, y0 - iy, x1 + ix, y1 + iy);
    Some(FlatRule {
        x0,
        y0,
        x1,
        y1,
        paint,
    })
}

fn hex(rgba: [f32; 4]) -> String {
    let [r, g, b, a] = rgba.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}
