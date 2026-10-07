//! Upstream reference fixtures: manifests, references, and the flat frame comparison.
//!
//! `tools/typst-upstream/references` writes the references. This module reads them and compares a
//! label against them. It names neither `crate::` nor `avenger_typst_label::`, so it can also
//! be included into the crate's own tests with `#[path]`.

#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::PathBuf,
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
    width: Option<Width>,
}

/// A case's width, in points, as `LabelWidth` has it: `{ max = 120.0 }` or `{ fixed = 120.0 }`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Width {
    Max(f64),
    Fixed(f64),
}

/// A case's settings, with the manifest defaults applied.
#[derive(Debug, Clone)]
pub struct Settings {
    pub text_font: String,
    pub math_font: String,
    pub font_size: f64,
    pub font_weight: u16,
    pub lang: Option<String>,
    pub region: Option<String>,
    pub dir: Option<String>,
    pub width: Option<Width>,
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
            width: case.width,
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
    /// Upstream's repr of the label's evaluated markup; `None` when evaluation fails.
    pub repr: Option<String>,
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
                "missing reference {} ({err}); regenerate with tools/typst-upstream/references",
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

/// A frame reduced to what any pipeline can be compared on: metrics, positioned glyphs, the
/// ink rectangles of shapes, all in points relative to the label's top-left corner, and the
/// text and painter order of the items.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Flat {
    pub width: f64,
    pub height: f64,
    pub baseline: f64,
    pub glyphs: Vec<FlatGlyph>,
    pub rules: Vec<FlatRule>,
    /// The text items' texts, in painter order.
    pub texts: Vec<String>,
    /// The painter order of text items (`T`) and rules (`R`).
    pub order: String,
}

impl Flat {
    /// Records a text item.
    pub fn push_text(&mut self, text: &str) {
        self.texts.push(text.into());
        self.order.push('T');
    }

    /// Records a shape's rule, if it draws one.
    pub fn push_rule(&mut self, rule: Option<FlatRule>) {
        if let Some(rule) = rule {
            self.rules.push(rule);
            self.order.push('R');
        }
    }
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
    /// The text of the glyph's cluster.
    pub cluster: String,
    /// The source bytes the glyph's cluster came from. References give it only for glyphs of
    /// verbatim nodes, whose spans map exactly.
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
    /// A diagonal line's direction (`\\` or `/`), and a stroke's cap, join, miter limit and dash
    /// pattern; empty for fills.
    pub style: String,
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
        let mut nodes = BTreeMap::new();
        collect_nodes(frame, &self.source, &mut nodes);
        let verbatim = nodes
            .into_iter()
            .filter(|(span, (exact, covered))| {
                *exact && covered.len() == span[1] - span[0]
            })
            .map(|(span, _)| span)
            .collect();
        flatten_into(frame, Affine::IDENTITY, self, &verbatim, &mut flat);
        Some(flat)
    }
}

/// For each source node with glyphs: whether every glyph's cluster equals the source at the
/// glyph's offset, and the source bytes those clusters cover. A node is verbatim when its
/// clusters match and cover all of it; only its glyphs map to source exactly. A collapsed run
/// of spaces, for example, matches at its first byte but isn't verbatim.
fn collect_nodes(
    frame: &RefFrame,
    source: &str,
    nodes: &mut BTreeMap<[usize; 2], (bool, BTreeSet<usize>)>,
) {
    for item in &frame.items {
        match &item.kind {
            RefItemKind::Text(text) => {
                for RefGlyph(_, _, _, _, _, range, span, span_offset) in &text.glyphs {
                    let Some(span) = *span else { continue };
                    let cluster = &text.text[range[0]..range[1]];
                    let start = span[0] + span_offset;
                    let end = start + cluster.len();
                    let exact = end <= span[1] && source.get(start..end) == Some(cluster);
                    let node = nodes.entry(span).or_insert((true, BTreeSet::new()));
                    node.0 &= exact;
                    if exact {
                        node.1.extend(start..end);
                    }
                }
            }
            RefItemKind::Group(group) => collect_nodes(&group.frame, source, nodes),
            RefItemKind::Shape(_) | RefItemKind::Image(_) => {}
        }
    }
}

fn flatten_into(
    frame: &RefFrame,
    transform: Affine,
    reference: &Reference,
    verbatim: &BTreeSet<[usize; 2]>,
    flat: &mut Flat,
) {
    for item in &frame.items {
        let at = transform.then(Affine::translate(item.x, item.y));
        match &item.kind {
            RefItemKind::Text(text) => {
                flat.push_text(&text.text);
                let font = &reference.fonts[text.font];
                let font = font.postscript.clone().unwrap_or_else(|| font.family.clone());
                // The pen advances in both directions, as upstream's renderers move it.
                let (mut pen_x, mut pen_y) = (0.0, 0.0);
                for RefGlyph(
                    id,
                    x_advance,
                    x_offset,
                    y_advance,
                    y_offset,
                    range,
                    span,
                    span_offset,
                ) in &text.glyphs
                {
                    let (x, y) = at.apply(
                        pen_x + x_offset * text.size,
                        -(pen_y + y_offset * text.size),
                    );
                    pen_x += x_advance * text.size;
                    pen_y += y_advance * text.size;
                    let cluster = &text.text[range[0]..range[1]];
                    let source =
                        span.filter(|span| verbatim.contains(span)).map(|span| {
                            let start = span[0] + span_offset;
                            [start, start + cluster.len()]
                        });
                    flat.glyphs.push(FlatGlyph {
                        font: font.clone(),
                        id: *id,
                        size: text.size,
                        x,
                        y,
                        fill: text.fill.clone(),
                        cluster: cluster.into(),
                        source,
                    });
                }
            }
            RefItemKind::Shape(shape) => flat.push_rule(shape_rule(shape, at)),
            RefItemKind::Group(group) => {
                let inner = at.then(Affine::new(group.transform));
                flatten_into(&group.frame, inner, reference, verbatim, flat);
            }
            RefItemKind::Image(_) => {}
        }
    }
}

/// A shape's rule, placed by `at`. `None` for shapes that draw nothing.
pub fn shape_rule(shape: &RefShape, at: Affine) -> Option<FlatRule> {
    let (x0, y0, x1, y1) = match &shape.geometry {
        RefGeometry::Line([dx, dy]) => {
            (dx.min(0.0), dy.min(0.0), dx.max(0.0), dy.max(0.0))
        }
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
                (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
                |(x0, y0, x1, y1), &(x, y)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            )
        }
    };
    let (inflate, paint, style) = match (&shape.stroke, &shape.fill) {
        (Some(stroke), _) => {
            (stroke.thickness / 2.0, stroke.paint.clone(), stroke_style(stroke))
        }
        (None, Some(fill)) => (0.0, fill.clone(), String::new()),
        (None, None) => return None,
    };
    // Lines are stroked across their direction only: a horizontal rule grows vertically.
    let (ix, iy) = match &shape.geometry {
        RefGeometry::Line([_, dy]) if *dy == 0.0 => (0.0, inflate),
        RefGeometry::Line([dx, _]) if *dx == 0.0 => (inflate, 0.0),
        _ => (inflate, inflate),
    };
    let [x0, y0, x1, y1] = at.bounds(x0 - ix, y0 - iy, x1 + ix, y1 + iy);
    // A diagonal line's direction, which its bounds don't show.
    let style = match &shape.geometry {
        RefGeometry::Line([dx, dy]) => {
            let (start, end) = (at.apply(0.0, 0.0), at.apply(*dx, *dy));
            match (end.0 - start.0) * (end.1 - start.1) {
                slope if slope > 1e-6 => format!("\\ {style}"),
                slope if slope < -1e-6 => format!("/ {style}"),
                _ => style,
            }
        }
        _ => style,
    };
    Some(FlatRule { x0, y0, x1, y1, paint, style })
}

/// A stroke's cap, join, miter limit and dash pattern, with lengths to a thousandth of a point.
fn stroke_style(stroke: &RefStroke) -> String {
    let dash = match &stroke.dash {
        None => "solid".to_string(),
        Some(dash) => {
            let lengths = |value: &serde_json::Value| -> Vec<String> {
                let lengths = value.as_array().map(Vec::as_slice).unwrap_or_default();
                lengths
                    .iter()
                    .map(|length| format!("{:.3}", length.as_f64().unwrap()))
                    .collect()
            };
            let phase = dash["phase"].as_f64().unwrap_or_default();
            format!("dash [{}] @ {phase:.3}", lengths(&dash["array"]).join(", "))
        }
    };
    format!("{} {} miter {:.3} {dash}", stroke.cap, stroke.join, stroke.miter_limit)
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
    pub text: Vec<String>,
    pub order: Vec<String>,
}

impl Mismatches {
    fn checks(&self) -> [(&'static str, &Vec<String>); 6] {
        [
            ("metrics", &self.metrics),
            ("glyphs", &self.glyphs),
            ("rules", &self.rules),
            ("source", &self.source),
            ("text", &self.text),
            ("order", &self.order),
        ]
    }

    pub fn failed_checks(&self) -> BTreeSet<&'static str> {
        self.checks()
            .into_iter()
            .filter(|(_, list)| !list.is_empty())
            .map(|(name, _)| name)
            .collect()
    }

    pub fn summary(&self) -> String {
        let mut out = String::new();
        for (name, list) in self.checks() {
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
            .filter(|(i, other)| {
                !used[*i] && other.font == glyph.font && other.id == glyph.id
            })
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
        if glyph.cluster != other.cluster {
            out.text.push(format!(
                "{} #{} cluster {:?}, expected {:?}",
                glyph.font, glyph.id, other.cluster, glyph.cluster
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
        let candidate =
            actual.rules.iter().enumerate().filter(|(i, _)| !used[*i]).min_by(
                |(_, a), (_, b)| {
                    rule_distance(rule, a).total_cmp(&rule_distance(rule, b))
                },
            );
        let Some((index, other)) = candidate else {
            out.rules.push(format!("missing rule {}", rule_text(rule)));
            continue;
        };
        used[index] = true;
        if rule_distance(rule, other) > tolerance
            || rule.paint != other.paint
            || rule.style != other.style
        {
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

    if actual.texts != expected.texts {
        out.text
            .push(format!("{:?}, expected {:?}", actual.texts, expected.texts));
    }
    if actual.order != expected.order {
        out.order
            .push(format!("{}, expected {}", actual.order, expected.order));
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
        "[{:.3}, {:.3}]–[{:.3}, {:.3}] {} {}",
        rule.x0, rule.y0, rule.x1, rule.y1, rule.paint, rule.style
    )
}
