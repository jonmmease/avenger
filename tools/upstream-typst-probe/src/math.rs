//! Captures the resolved math IR of inline equations during a real compile.
//!
//! Upstream resolves an equation inside `layout_equation_inline`, which the probe cannot call
//! into. Instead the probe replaces the equation's paged show rule: the replacement repeats the
//! steps `layout_equation_inline` takes before layout (pick the math font, add the script-scale
//! style, `resolve_equation`), records the IR, and then defers to upstream's own rule. The IR
//! therefore comes from the same element and style chain that upstream lays out.

use std::{
    num::NonZeroU64,
    sync::{Mutex, OnceLock},
};

use serde_json::{Value as Json, json};
use typst::{
    World,
    engine::Engine,
    foundations::{NativeRuleMap, Packed, ShowFn, StyleChain, Target},
    introspection::Locator,
    layout::{Abs, Axis, Rel},
    math::{
        EquationElem,
        ir::{
            FencedBody, MathComponent, MathItem, MathKind, MathProperties, Stretch,
            resolve_equation,
        },
    },
    routines::Arenas,
    syntax::Span,
    text::{FontInstance, TextElem, families, variant},
};

use crate::{frame, world::SpanMapper};

static ORIGINAL_RULES: OnceLock<NativeRuleMap> = OnceLock::new();
static CAPTURED: Mutex<Vec<Json>> = Mutex::new(Vec::new());

/// Keeps upstream's rules so the capturing rule can defer to the original.
pub fn remember_rules(rules: NativeRuleMap) {
    ORIGINAL_RULES.set(rules).ok();
}

pub fn begin_capture() {
    CAPTURED.lock().unwrap().clear();
}

pub fn end_capture() -> Vec<Json> {
    std::mem::take(&mut *CAPTURED.lock().unwrap())
}

/// Replaces span placeholders with label-relative ranges and keeps the last capture of each
/// equation (realization may run more than once per compile).
pub fn finish(captures: Vec<Json>, mapper: &SpanMapper) -> Vec<Json> {
    let mut equations: Vec<Json> = Vec::new();
    for mut capture in captures {
        map_spans(&mut capture, mapper);
        match equations
            .iter_mut()
            .find(|eq| eq["span"] == capture["span"])
        {
            Some(existing) => *existing = capture,
            None => equations.push(capture),
        }
    }
    equations
}

pub const CAPTURING_EQUATION_RULE: ShowFn<EquationElem> = |elem, engine, styles| {
    if !elem.block.get(styles) {
        capture(elem, engine, styles);
    }
    let rules = ORIGINAL_RULES.get().expect("upstream rules are remembered");
    let rule = rules
        .get(Target::Paged, elem.pack_ref())
        .expect("upstream has an equation rule");
    rule.apply(elem.pack_ref(), engine, styles)
};

// upstream: crates/typst-layout/src/math/mod.rs::layout_equation_inline @ v0.15.1, up to
// `resolve_equation`. Errors are left for the real layout to report.
fn capture(elem: &Packed<EquationElem>, engine: &mut Engine, styles: StyleChain) {
    let Some(font) = get_font(engine, styles) else {
        return;
    };
    let scale_style = EquationElem::script_scale
        .set((
            font.math().script_percent_scale_down,
            font.math().script_script_percent_scale_down,
        ))
        .wrap();
    let styles = styles.chain(&scale_style);
    let arenas = Arenas::default();
    let Ok(item) = resolve_equation(elem, engine, Locator::root(), &arenas, styles) else {
        return;
    };
    let capture = json!({ "span": span(elem.span()), "root": item_json(&item, &Json::Null) });
    CAPTURED.lock().unwrap().push(capture);
}

// upstream: crates/typst-layout/src/math/mod.rs::get_font @ v0.15.1
fn get_font(engine: &Engine, styles: StyleChain) -> Option<FontInstance> {
    let variant = variant(styles);
    let size = styles.resolve(TextElem::size);
    let variations = styles.get_cloned(TextElem::variations);
    families(styles).find_map(|family| {
        engine
            .world
            .book()
            .select(family.as_str(), variant)
            .and_then(|id| engine.world.font(id))
            .filter(|_| family.covers().is_none())
            .map(|font| font.instantiate(variant, size, &variations))
    })
}

/// Serializes an item. Default-valued properties are omitted, and `style` lists only the
/// properties that differ from the parent component's (`parent` is null at the root).
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
        MathItem::Tag(_) => json!({ "type": "tag" }),
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
            "shared": matches!(fenced.body, FencedBody::Shared { .. }),
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
            "stroke": frame::stroke(&cancel.stroke),
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
        MathKind::Number(number) => json!({ "type": "number", "text": number.text.as_str() }),
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
        MathKind::Multiline(_) => json!({ "type": "multiline" }),
        MathKind::Table(_) => json!({ "type": "table" }),
        MathKind::Box(_) => json!({ "type": "box" }),
        MathKind::Mathml(_) => json!({ "type": "mathml" }),
        MathKind::External(external) => {
            json!({ "type": "external", "elem": external.content.func().name() })
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
    // `limits`, `ignorant` and `spaced` are crate-private upstream; read them from `Debug`.
    let debug = format!("{props:?}");
    let mut value = json!({ "size": format!("{:?}", props.size), "span": span(props.span) });
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
    if props.align_form_infix {
        value["align_form_infix"] = json!(true);
    }
    let limits = debug_field(&debug, "limits");
    if limits != "Never" {
        value["limits"] = json!(limits);
    }
    for flag in ["ignorant", "spaced"] {
        if debug_field(&debug, flag) == "true" {
            value[flag] = json!(true);
        }
    }
    value
}

fn debug_field(debug: &str, name: &str) -> String {
    let start = debug
        .find(&format!(" {name}: "))
        .expect("field in Debug output")
        + name.len()
        + 3;
    debug[start..]
        .split([',', ' ', '}'])
        .next()
        .unwrap()
        .to_string()
}

/// The style properties math layout reads from a component's chain.
fn style(styles: StyleChain) -> Json {
    let variant = variant(styles);
    json!({
        "font": families(styles).next().map(|family| family.as_str().to_string()),
        "font_size": styles.resolve(TextElem::size).to_pt(),
        "weight": variant.weight.to_number(),
        "style": format!("{:?}", variant.style),
        "fill": frame::paint(&styles.get_cloned(TextElem::fill)),
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

fn rel(value: Rel<Abs>) -> Json {
    json!({ "rel": value.rel.get(), "abs": value.abs.to_pt() })
}

/// A placeholder that `finish` maps to a label-relative range.
fn span(span: Span) -> Json {
    json!({ "$span": span.into_raw().get() })
}

fn map_spans(value: &mut Json, mapper: &SpanMapper) {
    match value {
        Json::Object(map) => {
            if let Some(raw) = map.get("$span").and_then(Json::as_u64) {
                *value =
                    NonZeroU64::new(raw).map_or(Json::Null, |raw| mapper.json(Span::from_raw(raw)));
                return;
            }
            map.values_mut().for_each(|value| map_spans(value, mapper));
        }
        Json::Array(items) => items.iter_mut().for_each(|value| map_spans(value, mapper)),
        _ => {}
    }
}
