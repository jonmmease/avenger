//! Upstream reference fixtures: manifests, references, and the flat frame comparison.
//!
//! `tools/upstream-typst-probe` writes the references. This module reads them and compares a
//! label against them. It names neither `crate::` nor `avenger_typst_label::`, so it can also
//! be included into the crate's own tests with `#[path]`.

#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub fn fixtures_dir(suite: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(suite)
}

pub fn output_dir(suite: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/output")
        .join(suite)
}

// ---------------------------------------------------------------------------------------------
// Manifests

/// A fixture directory's `cases.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub defaults: Defaults,
    #[serde(rename = "case")]
    pub cases: Vec<Case>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    pub text_font: String,
    pub math_font: String,
    pub font_size: f64,
    pub font_weight: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub upstream_tests: Vec<String>,
    pub note: Option<String>,
    text_font: Option<String>,
    math_font: Option<String>,
    font_size: Option<f64>,
    font_weight: Option<u16>,
    lang: Option<String>,
    region: Option<String>,
    dir: Option<String>,
}

/// A case's text settings, with the manifest defaults applied.
#[derive(Debug, Clone)]
pub struct Settings {
    pub text_font: String,
    pub math_font: String,
    pub font_size: f64,
    pub font_weight: u16,
    pub lang: Option<String>,
    pub region: Option<String>,
    pub dir: Option<String>,
}

impl Manifest {
    pub fn load(suite: &str) -> Self {
        let path = fixtures_dir(suite).join("cases.toml");
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        toml::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }

    pub fn settings(&self, case: &Case) -> Settings {
        Settings {
            text_font: case
                .text_font
                .clone()
                .unwrap_or_else(|| self.defaults.text_font.clone()),
            math_font: case
                .math_font
                .clone()
                .unwrap_or_else(|| self.defaults.math_font.clone()),
            font_size: case.font_size.unwrap_or(self.defaults.font_size),
            font_weight: case.font_weight.unwrap_or(self.defaults.font_weight),
            lang: case.lang.clone(),
            region: case.region.clone(),
            dir: case.dir.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// References

/// `ref/{id}.json`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub source: String,
    /// The fonts text items reference by index.
    #[serde(default)]
    pub fonts: Vec<RefFont>,
    pub frame: Option<RefFrame>,
    #[serde(default)]
    pub equations: Vec<serde_json::Value>,
    #[serde(default)]
    pub errors: Vec<RefDiagnostic>,
    pub warnings: Vec<RefDiagnostic>,
}

impl Reference {
    pub fn load(suite: &str, id: &str) -> Result<Self, String> {
        let path = fixtures_dir(suite).join("ref").join(format!("{id}.json"));
        let text = fs::read_to_string(&path).map_err(|err| {
            format!(
                "missing reference {} ({err}); regenerate with tools/upstream-typst-probe",
                path.display()
            )
        })?;
        serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefDiagnostic {
    pub message: String,
    pub range: Option<[usize; 2]>,
    pub hints: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefFrame {
    pub width: f64,
    pub height: f64,
    pub baseline: f64,
    pub kind: String,
    pub items: Vec<RefItem>,
}

#[derive(Debug, Deserialize)]
pub struct RefItem {
    pub x: f64,
    pub y: f64,
    #[serde(flatten)]
    pub kind: RefItemKind,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefItemKind {
    Text(RefText),
    Shape(RefShape),
    Group(RefGroup),
    Image(RefImage),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefText {
    /// An index into `Reference::fonts`.
    pub font: usize,
    pub size: f64,
    pub fill: String,
    #[serde(default)]
    pub stroke: Option<RefStroke>,
    pub lang: String,
    #[serde(default)]
    pub region: Option<String>,
    pub text: String,
    pub glyphs: Vec<RefGlyph>,
}

/// `[id, x_advance, x_offset, y_advance, y_offset, range, span, span_offset]`, with advances and
/// offsets in em, `range` into the item's text, and `span` the label-relative range of the
/// glyph's source node.
#[derive(Debug, Deserialize)]
pub struct RefGlyph(
    pub u16,
    pub f64,
    pub f64,
    pub f64,
    pub f64,
    pub [usize; 2],
    pub Option<[usize; 2]>,
    pub usize,
);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefFont {
    pub family: String,
    pub postscript: Option<String>,
    pub index: u32,
    pub variations: Vec<(String, f32)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefShape {
    pub geometry: RefGeometry,
    pub fill: Option<String>,
    pub fill_rule: String,
    pub stroke: Option<RefStroke>,
    pub span: Option<[usize; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefGeometry {
    Line([f64; 2]),
    Rect([f64; 2]),
    Curve(Vec<serde_json::Value>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefStroke {
    pub paint: String,
    pub thickness: f64,
    pub cap: String,
    pub join: String,
    pub dash: Option<serde_json::Value>,
    pub miter_limit: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefGroup {
    pub transform: [f64; 6],
    pub clip: Option<serde_json::Value>,
    pub frame: RefFrame,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefImage {
    pub width: f64,
    pub height: f64,
    pub span: Option<[usize; 2]>,
}

// ---------------------------------------------------------------------------------------------
// Flat frames

/// A frame reduced to what any pipeline can be compared on: metrics, positioned glyphs, and
/// the ink rectangles of shapes, all in points relative to the label's top-left corner.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Flat {
    pub width: f64,
    pub height: f64,
    pub baseline: f64,
    pub glyphs: Vec<FlatGlyph>,
    pub rules: Vec<FlatRule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlatGlyph {
    /// The PostScript name of the glyph's font.
    pub font: String,
    pub id: u16,
    pub size: f64,
    /// The glyph origin, on its baseline.
    pub x: f64,
    pub y: f64,
    /// `#rrggbbaa`.
    pub fill: String,
    /// The source bytes the glyph's cluster came from. References give it only for verbatim
    /// clusters, whose span maps exactly.
    pub source: Option<[usize; 2]>,
}

/// A shape's ink bounds: the stroke-inflated geometry for strokes, the geometry for fills.
#[derive(Debug, Clone, Serialize)]
pub struct FlatRule {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub paint: String,
}

/// A 2D affine transform `[sx, ky, kx, sy, tx, ty]`, as in Typst.
#[derive(Debug, Clone, Copy)]
pub struct Affine([f64; 6]);

impl Affine {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(x: f64, y: f64) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, x, y])
    }

    pub fn new(values: [f64; 6]) -> Self {
        Self(values)
    }

    /// `self ∘ other`: apply `other` first.
    pub fn then(self, other: Self) -> Self {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Self([
            a1 * a2 + c1 * b2,
            b1 * a2 + d1 * b2,
            a1 * c2 + c1 * d2,
            b1 * c2 + d1 * d2,
            a1 * e2 + c1 * f2 + e1,
            b1 * e2 + d1 * f2 + f1,
        ])
    }

    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        let [sx, ky, kx, sy, tx, ty] = self.0;
        (sx * x + kx * y + tx, ky * x + sy * y + ty)
    }

    /// The transformed bounds of an axis-aligned rectangle.
    pub fn bounds(self, x0: f64, y0: f64, x1: f64, y1: f64) -> [f64; 4] {
        let corners = [
            self.apply(x0, y0),
            self.apply(x1, y0),
            self.apply(x0, y1),
            self.apply(x1, y1),
        ];
        let xs = corners.iter().map(|c| c.0);
        let ys = corners.iter().map(|c| c.1);
        [
            xs.clone().fold(f64::INFINITY, f64::min),
            ys.clone().fold(f64::INFINITY, f64::min),
            xs.fold(f64::NEG_INFINITY, f64::max),
            ys.fold(f64::NEG_INFINITY, f64::max),
        ]
    }
}

impl Reference {
    /// The label frame, flattened. `None` when upstream rejected the source.
    pub fn flat(&self) -> Option<Flat> {
        let frame = self.frame.as_ref()?;
        let mut flat = Flat {
            width: frame.width,
            height: frame.height,
            baseline: frame.baseline,
            ..Flat::default()
        };
        flatten_into(frame, Affine::IDENTITY, self, &mut flat);
        Some(flat)
    }
}

fn flatten_into(frame: &RefFrame, transform: Affine, reference: &Reference, flat: &mut Flat) {
    let source = reference.source.as_str();
    for item in &frame.items {
        let at = transform.then(Affine::translate(item.x, item.y));
        match &item.kind {
            RefItemKind::Text(text) => {
                let font = &reference.fonts[text.font];
                let font = font
                    .postscript
                    .clone()
                    .unwrap_or_else(|| font.family.clone());
                let mut pen = 0.0;
                for RefGlyph(id, x_advance, x_offset, _, y_offset, range, span, span_offset) in
                    &text.glyphs
                {
                    let (x, y) = at.apply(pen + x_offset * text.size, -y_offset * text.size);
                    pen += x_advance * text.size;
                    let cluster = &text.text[range[0]..range[1]];
                    let source = span.and_then(|span| {
                        let start = span[0] + span_offset;
                        let end = start + cluster.len();
                        (end <= span[1] && source.get(start..end) == Some(cluster))
                            .then_some([start, end])
                    });
                    flat.glyphs.push(FlatGlyph {
                        font: font.clone(),
                        id: *id,
                        size: text.size,
                        x,
                        y,
                        fill: text.fill.clone(),
                        source,
                    });
                }
            }
            RefItemKind::Shape(shape) => {
                if let Some(rule) = shape_rule(shape, at) {
                    flat.rules.push(rule);
                }
            }
            RefItemKind::Group(group) => {
                let inner = at.then(Affine::new(group.transform));
                flatten_into(&group.frame, inner, reference, flat);
            }
            RefItemKind::Image(_) => {}
        }
    }
}

fn shape_rule(shape: &RefShape, at: Affine) -> Option<FlatRule> {
    let (x0, y0, x1, y1) = match &shape.geometry {
        RefGeometry::Line([dx, dy]) => (dx.min(0.0), dy.min(0.0), dx.max(0.0), dy.max(0.0)),
        RefGeometry::Rect([w, h]) => (0.0, 0.0, *w, *h),
        RefGeometry::Curve(items) => {
            let points = items
                .iter()
                .filter_map(|item| item.as_array())
                .flat_map(|item| item.iter().skip(1))
                .filter_map(|point| {
                    let point = point.as_array()?;
                    Some((point.first()?.as_f64()?, point.get(1)?.as_f64()?))
                })
                .collect::<Vec<_>>();
            if points.is_empty() {
                return None;
            }
            points.iter().fold(
                (
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ),
                |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            )
        }
    };
    let (inflate, paint) = match (&shape.stroke, &shape.fill) {
        (Some(stroke), _) => (stroke.thickness / 2.0, stroke.paint.clone()),
        (None, Some(fill)) => (0.0, fill.clone()),
        (None, None) => return None,
    };
    // Lines are stroked across their direction only: a horizontal rule grows vertically.
    let (ix, iy) = match &shape.geometry {
        RefGeometry::Line([_, dy]) if *dy == 0.0 => (0.0, inflate),
        RefGeometry::Line([dx, _]) if *dx == 0.0 => (inflate, 0.0),
        _ => (inflate, inflate),
    };
    let [x0, y0, x1, y1] = at.bounds(x0 - ix, y0 - iy, x1 + ix, y1 + iy);
    Some(FlatRule {
        x0,
        y0,
        x1,
        y1,
        paint,
    })
}

// ---------------------------------------------------------------------------------------------
// Comparison

/// The checks a case runs. A case passes when each is empty.
#[derive(Debug, Default)]
pub struct Mismatches {
    pub metrics: Vec<String>,
    pub glyphs: Vec<String>,
    pub rules: Vec<String>,
    pub source: Vec<String>,
}

impl Mismatches {
    pub fn failed_checks(&self) -> BTreeSet<&'static str> {
        [
            ("metrics", &self.metrics),
            ("glyphs", &self.glyphs),
            ("rules", &self.rules),
            ("source", &self.source),
        ]
        .into_iter()
        .filter(|(_, list)| !list.is_empty())
        .map(|(name, _)| name)
        .collect()
    }

    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (name, list) in [
            ("metrics", &self.metrics),
            ("glyphs", &self.glyphs),
            ("rules", &self.rules),
            ("source", &self.source),
        ] {
            for line in list.iter().take(4) {
                let _ = writeln!(out, "    {name}: {line}");
            }
            if list.len() > 4 {
                let _ = writeln!(out, "    {name}: … {} more", list.len() - 4);
            }
        }
        out
    }
}

/// Compares an actual flat frame with the expected one. Glyphs and rules are matched
/// order-independently: each expected glyph takes the nearest unmatched actual glyph with the
/// same font and id.
pub fn compare(expected: &Flat, actual: &Flat, tolerance: f64) -> Mismatches {
    let mut out = Mismatches::default();
    for (name, e, a) in [
        ("width", expected.width, actual.width),
        ("height", expected.height, actual.height),
        ("baseline", expected.baseline, actual.baseline),
    ] {
        if (e - a).abs() > tolerance {
            out.metrics.push(format!("{name} {a:.4}, expected {e:.4}"));
        }
    }

    let mut used = vec![false; actual.glyphs.len()];
    for glyph in &expected.glyphs {
        let candidate = actual
            .glyphs
            .iter()
            .enumerate()
            .filter(|(i, other)| !used[*i] && other.font == glyph.font && other.id == glyph.id)
            .min_by(|(_, a), (_, b)| {
                distance(glyph.x, glyph.y, a.x, a.y)
                    .total_cmp(&distance(glyph.x, glyph.y, b.x, b.y))
            });
        let Some((index, other)) = candidate else {
            out.glyphs.push(format!(
                "missing {} #{} at ({:.3}, {:.3})",
                glyph.font, glyph.id, glyph.x, glyph.y
            ));
            continue;
        };
        used[index] = true;
        if distance(glyph.x, glyph.y, other.x, other.y) > tolerance {
            out.glyphs.push(format!(
                "{} #{} at ({:.3}, {:.3}), expected ({:.3}, {:.3})",
                glyph.font, glyph.id, other.x, other.y, glyph.x, glyph.y
            ));
        }
        if (glyph.size - other.size).abs() > tolerance {
            out.glyphs.push(format!(
                "{} #{} size {:.3}, expected {:.3}",
                glyph.font, glyph.id, other.size, glyph.size
            ));
        }
        if glyph.fill != other.fill {
            out.glyphs.push(format!(
                "{} #{} fill {}, expected {}",
                glyph.font, glyph.id, other.fill, glyph.fill
            ));
        }
        if let (Some(e), Some(a)) = (glyph.source, other.source)
            && e != a
        {
            out.source.push(format!(
                "{} #{} source {a:?}, expected {e:?}",
                glyph.font, glyph.id
            ));
        }
    }
    for (glyph, _) in actual.glyphs.iter().zip(&used).filter(|(_, used)| !**used) {
        out.glyphs.push(format!(
            "extra {} #{} at ({:.3}, {:.3})",
            glyph.font, glyph.id, glyph.x, glyph.y
        ));
    }

    let mut used = vec![false; actual.rules.len()];
    for rule in &expected.rules {
        let candidate = actual
            .rules
            .iter()
            .enumerate()
            .filter(|(i, _)| !used[*i])
            .min_by(|(_, a), (_, b)| rule_distance(rule, a).total_cmp(&rule_distance(rule, b)));
        let Some((index, other)) = candidate else {
            out.rules.push(format!("missing rule {}", rule_text(rule)));
            continue;
        };
        used[index] = true;
        if rule_distance(rule, other) > tolerance || rule.paint != other.paint {
            out.rules.push(format!(
                "rule {}, expected {}",
                rule_text(other),
                rule_text(rule)
            ));
        }
    }
    for (rule, _) in actual.rules.iter().zip(&used).filter(|(_, used)| !**used) {
        out.rules.push(format!("extra rule {}", rule_text(rule)));
    }
    out
}

fn distance(x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    (x0 - x1).abs().max((y0 - y1).abs())
}

fn rule_distance(a: &FlatRule, b: &FlatRule) -> f64 {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1]
        .into_iter()
        .map(f64::abs)
        .fold(0.0, f64::max)
}

fn rule_text(rule: &FlatRule) -> String {
    format!(
        "[{:.3}, {:.3}]–[{:.3}, {:.3}] {}",
        rule.x0, rule.y0, rule.x1, rule.y1, rule.paint
    )
}

// ---------------------------------------------------------------------------------------------
// Census

/// `expected_failures.toml`: the cases a pipeline is known to fail, and which checks.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedFailures {
    #[serde(default, rename = "case")]
    pub cases: Vec<ExpectedFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedFailure {
    pub id: String,
    /// `metrics`, `glyphs`, `rules`, `source`, or `compile` when the label fails to compile.
    pub fails: BTreeSet<String>,
    pub reason: String,
    /// The plan phase expected to fix it.
    pub fixed_by: String,
}

impl ExpectedFailures {
    pub fn load(suite: &str, file: &str) -> Self {
        let path = fixtures_dir(suite).join(file);
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }
}

/// Collects case outcomes and checks them against the expected failures.
#[derive(Default)]
pub struct Census {
    outcomes: BTreeMap<String, (BTreeSet<String>, String)>,
}

impl Census {
    pub fn record(&mut self, id: &str, failed: BTreeSet<String>, detail: String) {
        self.outcomes.insert(id.to_string(), (failed, detail));
    }

    /// Panics with every difference between the outcomes and the expectation file.
    ///
    /// Also writes the outcomes as `census.toml` in `out_dir`, in the expectation file's format:
    /// reasons are kept for cases already listed, and new failures get their first mismatch.
    /// Accept a new census by copying it over the expectation file and editing the reasons.
    pub fn check(self, expected: &ExpectedFailures, file: &str, out_dir: &Path) {
        let mut problems = Vec::new();
        let expected_by_id = expected
            .cases
            .iter()
            .map(|case| (case.id.as_str(), case))
            .collect::<BTreeMap<_, _>>();
        for id in expected_by_id.keys() {
            if !self.outcomes.contains_key(*id) {
                problems.push(format!("{id}: listed in {file} but not a case"));
            }
        }
        let mut passed = 0;
        let mut census = String::from(
            "# Generated by the census test; see `Census::check` in tests/common/oracle.rs.\n",
        );
        for (id, (failed, detail)) in &self.outcomes {
            let listed = expected_by_id.get(id.as_str());
            if failed.is_empty() {
                passed += 1;
            } else {
                let first = detail.lines().next().unwrap_or_default().trim();
                let reason = listed.map_or(first, |case| case.reason.as_str());
                let fixed_by = listed.map_or("?", |case| case.fixed_by.as_str());
                let fails = failed.iter().map(|check| format!("{check:?}"));
                let _ = write!(
                    census,
                    "\n[[case]]\nid = {id:?}\nfails = [{}]\nreason = {}\nfixed_by = {fixed_by:?}\n",
                    fails.collect::<Vec<_>>().join(", "),
                    toml_string(reason),
                );
            }
            match listed.map(|case| &case.fails) {
                None if failed.is_empty() => {}
                None => problems.push(format!("{id}: fails {failed:?}\n{detail}")),
                Some(listed) if listed == failed => {}
                Some(listed) if failed.is_empty() => problems.push(format!(
                    "{id}: passes now; remove it from {file} (listed {listed:?})"
                )),
                Some(listed) => problems.push(format!(
                    "{id}: fails {failed:?}, but {file} lists {listed:?}\n{detail}"
                )),
            }
        }
        fs::create_dir_all(out_dir).unwrap();
        fs::write(out_dir.join("census.toml"), census).unwrap();
        eprintln!("{passed} of {} cases pass", self.outcomes.len());
        if !problems.is_empty() {
            panic!(
                "{} census differences (outcomes in {}):\n{}",
                problems.len(),
                out_dir.join("census.toml").display(),
                problems.join("\n")
            );
        }
    }
}

fn toml_string(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
