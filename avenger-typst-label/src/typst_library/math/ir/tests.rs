//! The math IR against upstream's: every equation in the math references resolves to the IR
//! the probe records, in the probe's format.

use std::collections::BTreeSet;

use serde_json::{Value as Json, json};

use super::*;
use crate::label::fixtures::{self, WithSource};
use crate::label::oracle::{
    Manifest, Reference, Settings, output_dir, paint, root_styles,
};
use crate::typst_eval::{eval_label, parse_label};
use crate::typst_layout::math::{get_font, style_for_script_scale};
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{Scope, StyleChain, Styles};
use crate::typst_library::layout::{Abs, Axis, Rel};
use crate::typst_library::math::EquationElem;
use crate::typst_library::routines::{Arenas, RealizationKind};
use crate::typst_library::text::{
    FontFamily, FontList, FontWeight, TextElem, families, variant,
};
use crate::typst_library::visualize::FixedStroke;
use crate::typst_realize::realize;
use crate::typst_syntax::{FileId, Span, SpanKind};

/// Numbers in the IR must agree within this much.
const TOLERANCE: f64 = 1e-6;

/// Cases whose IR deliberately differs from upstream's, with the reason.
const DIVERGENT: &[(&str, &str)] = &[(
    "issue-8261-string-as-empty",
    "empty text is an empty group, not a multiline item without rows: labels have no \
     multiline math, and the two differ only in the empty frame's baseline",
)];

#[test]
fn math_equations_resolve_like_upstream() {
    check_suite("upstream_math");
}

#[test]
fn equations_in_text_resolve_like_upstream() {
    check_suite("upstream_frames");
}

/// Every equation of every case in the suite resolves to upstream's IR. Both IRs of failing
/// cases go to the gitignored `tests/output/internal/ir/`.
fn check_suite(suite: &str) {
    let manifest = Manifest::load(suite);
    let mut failures = vec![];
    for case in &manifest.cases {
        let divergent = DIVERGENT.iter().any(|(id, _)| *id == case.id);
        let reference = Reference::load(suite, &case.id).unwrap();
        if reference.repr.is_none() {
            // Evaluation fails, which the evaluator's tests check.
            continue;
        }
        let actual = match equations(&case.source, &manifest.settings(case)) {
            Ok(equations) => Json::Array(equations),
            Err(errors) => {
                failures.push(format!("{}: fails with {}", case.id, errors[0].message));
                continue;
            }
        };
        let expected = Json::Array(reference.equations);
        let mut diffs = vec![];
        diff("equations", &expected, &actual, &mut diffs);
        if diffs.is_empty() {
            if divergent {
                failures.push(format!(
                    "{}: matches upstream, so it isn't divergent",
                    case.id
                ));
            }
            continue;
        }
        if divergent {
            continue;
        }
        let dir = output_dir("internal").join("ir");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, json) in [("expected", &expected), ("actual", &actual)] {
            let path = dir.join(format!("{}.{name}.json", case.id));
            std::fs::write(path, serde_json::to_string_pretty(json).unwrap()).unwrap();
        }
        diffs.truncate(8);
        failures.push(format!("{}:\n    {}", case.id, diffs.join("\n    ")));
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// Resolves a case's equations as the probe captures them: in each equation's style chain,
/// with the wrapper's show-set rule for equations and the math font's script scale.
fn equations(source: &str, settings: &Settings) -> SourceResult<Vec<Json>> {
    let world = WithSource { world: fixtures::shared(), source };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    let content = eval_label(&mut engine, &parse_label(source), Scope::new())?;
    let arenas = Arenas::default();
    let root = root_styles(settings);
    let pairs = realize(RealizationKind::Par, &mut engine, &arenas, &content, root)?;

    // The wrapper's `#show math.equation: set text(font: .., weight: ..)`.
    let mut show_set = Styles::new();
    show_set.set(TextElem::font, FontList(vec![FontFamily::new(&settings.math_font)]));
    show_set.set(TextElem::weight, FontWeight::from_number(settings.font_weight));
    let show_set = &*arenas.styles.alloc(show_set);

    let mut equations = vec![];
    for (elem, styles) in pairs {
        let Some(elem) = elem.to_packed::<EquationElem>() else { continue };
        let styles = arenas.chains.alloc(styles).chain(show_set);
        let font = get_font(engine.world, styles, elem.span())?;
        let scale = &*arenas.styles.alloc(style_for_script_scale(&font).into());
        let styles = arenas.chains.alloc(styles).chain(scale);
        let item = resolve_equation(elem, &mut engine, &arenas, styles)?;
        let root = item_json(&item, &Json::Null);
        equations.push(json!({ "span": span(elem.span()), "root": root }));
    }
    Ok(equations)
}

/// Serializes an item as the probe does. Default-valued properties are omitted, and `style`
/// lists only the properties that differ from the parent component's (`parent` is null at
/// the root).
fn item_json(item: &MathItem, parent: &Json) -> Json {
    match item {
        MathItem::Component(component) => component_json(component, parent),
        MathItem::Spacing(length, font_size, weak) => json!({
            "type": "spacing",
            "em": length.em.get(),
            "abs": length.abs.to_pt(),
            "font_size": font_size.to_pt(),
            "weak": weak,
        }),
        MathItem::Space => json!({ "type": "space" }),
    }
}

fn component_json(component: &MathComponent, parent: &Json) -> Json {
    let style = style(component.styles);
    let item = |item: &MathItem| item_json(item, &style);
    let opt = |slot: &Option<MathItem>| slot.as_ref().map_or(Json::Null, item);
    let mut value = match &component.kind {
        MathKind::Group(group) => json!({
            "type": "group",
            "items": group.items.iter().map(item).collect::<Vec<_>>(),
        }),
        MathKind::Radical(radical) => json!({
            "type": "radical",
            "radicand": item(&radical.radicand),
            "index": opt(&radical.index),
            "sqrt": item(&radical.sqrt),
        }),
        MathKind::Fenced(fenced) => json!({
            "type": "fenced",
            "open": opt(&fenced.open),
            "close": opt(&fenced.close),
            "body": item(&fenced.body),
            "shared": false,
            "balanced": fenced.balanced,
        }),
        MathKind::Fraction(fraction) => json!({
            "type": "fraction",
            "numerator": item(&fraction.numerator),
            "denominator": item(&fraction.denominator),
            "line": fraction.line,
            "padding": fraction.padding.get(),
        }),
        MathKind::SkewedFraction(fraction) => json!({
            "type": "skewed_fraction",
            "numerator": item(&fraction.numerator),
            "denominator": item(&fraction.denominator),
            "slash": item(&fraction.slash),
        }),
        MathKind::Scripts(scripts) => json!({
            "type": "scripts",
            "base": item(&scripts.base),
            "t": opt(&scripts.top),
            "b": opt(&scripts.bottom),
            "tl": opt(&scripts.top_left),
            "bl": opt(&scripts.bottom_left),
            "tr": opt(&scripts.top_right),
            "br": opt(&scripts.bottom_right),
        }),
        MathKind::Accent(accent) => json!({
            "type": "accent",
            "base": item(&accent.base),
            "accent": item(&accent.accent),
            "position": format!("{:?}", accent.position),
            "dotless": accent.dotless,
            "exact_frame_width": accent.exact_frame_width,
        }),
        MathKind::Cancel(cancel) => json!({
            "type": "cancel",
            "base": item(&cancel.base),
            "length": rel(cancel.length),
            "stroke": stroke(&cancel.stroke),
            "cross": cancel.cross,
            "invert_first_line": cancel.invert_first_line,
            "angle": format!("{:?}", cancel.angle),
        }),
        MathKind::Line(line) => json!({
            "type": "line",
            "base": item(&line.base),
            "position": format!("{:?}", line.position),
        }),
        MathKind::Primes(primes) => json!({ "type": "primes", "count": primes.count }),
        MathKind::Text(text) => json!({ "type": "text", "text": text.text.as_str() }),
        MathKind::Number(number) => {
            json!({ "type": "number", "text": number.text.as_str() })
        }
        MathKind::Glyph(glyph) => {
            let mut value = json!({
                "type": "glyph",
                "text": glyph.text.as_str(),
                "glyph_class": format!("{:?}", glyph.class),
            });
            let (x, y) = (
                stretch(glyph.stretch.get(), Axis::X),
                stretch(glyph.stretch.get(), Axis::Y),
            );
            if !x.is_null() || !y.is_null() {
                value["stretch"] = json!({ "x": x, "y": y });
            }
            if let Some(mid) = glyph.mid_stretched.get() {
                value["mid_stretched"] = json!(mid);
            }
            if glyph.flac.get() {
                value["flac"] = json!(true);
            }
            value
        }
    };
    value["props"] = props(&component.props);
    let delta = style_delta(&style, parent);
    if !delta.as_object().is_some_and(|map| map.is_empty()) {
        value["style"] = delta;
    }
    value
}

/// The fields of `style` that differ from `parent`, or all of them at the root.
fn style_delta(style: &Json, parent: &Json) -> Json {
    let (Some(style), Some(parent)) = (style.as_object(), parent.as_object()) else {
        return style.clone();
    };
    style
        .iter()
        .filter(|(key, value)| parent.get(*key) != Some(value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// The component's properties; `size` and `span` always, the rest only when not default.
fn props(props: &MathProperties) -> Json {
    let mut value =
        json!({ "size": format!("{:?}", props.size), "span": span(props.span) });
    if let Some(class) = props.class {
        value["class"] = json!(format!("{class:?}"));
    }
    if props.cramped {
        value["cramped"] = json!(true);
    }
    if let Some(lspace) = props.lspace {
        value["lspace"] = json!(lspace.get());
    }
    if let Some(rspace) = props.rspace {
        value["rspace"] = json!(rspace.get());
    }
    let limits = format!("{:?}", props.limits);
    if limits != "Never" {
        value["limits"] = json!(limits);
    }
    if props.spaced {
        value["spaced"] = json!(true);
    }
    value
}

/// The style properties math layout reads from a component's chain.
fn style(styles: StyleChain) -> Json {
    let variant = variant(styles);
    json!({
        "font": families(styles).next().map(|family| family.as_str().to_string()),
        "font_size": styles.resolve(TextElem::size).to_pt(),
        "weight": variant.weight.to_number(),
        "style": format!("{:?}", variant.style),
        "fill": paint(&styles.get_cloned(TextElem::fill)),
        "math_variant": styles.get(EquationElem::variant).map(|v| format!("{v:?}")),
        "bold": styles.get(EquationElem::bold),
        "italic": styles.get(EquationElem::italic),
    })
}

fn stretch(stretch: Stretch, axis: Axis) -> Json {
    let Some(info) = stretch.resolve(axis) else {
        return Json::Null;
    };
    json!({
        "target": rel(info.target),
        "short_fall": info.short_fall.get(),
        "explicit": stretch.is_explicit(axis),
        "requested": stretch.resolve_requested(axis).map(|target| json!({
            "rel": target.rel.get(),
            "abs": target.abs.abs.to_pt(),
            "em": target.abs.em.get(),
        })),
    })
}

fn stroke(stroke: &FixedStroke) -> Json {
    json!({
        "paint": paint(&stroke.paint),
        "thickness": stroke.thickness.to_pt(),
        "cap": format!("{:?}", stroke.cap),
        "join": format!("{:?}", stroke.join),
        "dash": stroke.dash.as_ref().map(|dash| json!({
            "array": dash.array.iter().map(|len| len.to_pt()).collect::<Vec<_>>(),
            "phase": dash.phase.to_pt(),
        })),
        "miter_limit": stroke.miter_limit.get(),
    })
}

fn rel(value: Rel<Abs>) -> Json {
    json!({ "rel": value.rel.get(), "abs": value.abs.to_pt() })
}

/// A span's range in the label, or null.
fn span(span: Span) -> Json {
    match span.get() {
        SpanKind::Range { id, range } if id == FileId::LABEL => {
            json!([range.start, range.end])
        }
        _ => Json::Null,
    }
}

/// Collects the differences between `expected` and `actual` at `path`. Numbers agree within
/// the tolerance; the probe rounds them to nine decimals.
fn diff(path: &str, expected: &Json, actual: &Json, out: &mut Vec<String>) {
    match (expected, actual) {
        (Json::Number(e), Json::Number(a)) => {
            let (e, a) = (e.as_f64().unwrap(), a.as_f64().unwrap());
            if (e - a).abs() > TOLERANCE {
                out.push(format!("{path}: {a}, expected {e}"));
            }
        }
        (Json::Array(e), Json::Array(a)) if e.len() == a.len() => {
            for (i, (e, a)) in e.iter().zip(a).enumerate() {
                diff(&format!("{path}[{i}]"), e, a, out);
            }
        }
        (Json::Object(e), Json::Object(a)) => {
            for key in e.keys().chain(a.keys()).collect::<BTreeSet<_>>() {
                let path = format!("{path}.{key}");
                match (e.get(key), a.get(key)) {
                    (Some(e), Some(a)) => diff(&path, e, a, out),
                    (Some(e), None) => out.push(format!("{path}: missing, expected {e}")),
                    (None, Some(a)) => out.push(format!("{path}: {a}, expected nothing")),
                    (None, None) => {}
                }
            }
        }
        _ if expected == actual => {}
        _ => out.push(format!("{path}: {actual}, expected {expected}")),
    }
}
