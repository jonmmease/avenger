//! Frame-oracle cases through realization and inline layout, and realization invariants.
//!
//! Each case realizes the content that evaluation produces for a reference's source, with the
//! same spans, lays it out as a label line, and compares the line with upstream's frame. The
//! content is built by hand until the pipeline evaluates sources.

use ecow::eco_vec;

use super::realize;
use super::spaces::collapse_spaces;
use crate::label::fixtures::{self, WithSource};
use crate::label::oracle::{Case, check, check_plain};
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{
    Content, NativeElement, Smart, StyleChain, Styles, SymbolElem,
};
use crate::typst_library::layout::{Abs, Corners, Dir, Em, Length, Rel, Sides};
use crate::typst_library::math::EquationElem;
use crate::typst_library::model::{EmphElem, StrongElem};
use crate::typst_library::routines::{Arenas, Pair, RealizationKind};
use crate::typst_library::text::{
    BottomEdge, BottomEdgeMetric, Case as TextCase, FontFamily, FontFeatures, FontList,
    FontStyle, FontWeight, HighlightElem, LinebreakElem, OverlineElem, RawContent,
    RawElem, SmallcapsElem, SpaceElem, StrikeElem, SubElem, SuperElem, Tag, TextDir,
    TextElem, TextSize, TopEdge, TopEdgeMetric, UnderlineElem,
};
use crate::typst_library::visualize::{Color, ColorExt, LineCap, Paint, Stroke};

// ---------------------------------------------------------------------------------------------
// Content as evaluation produces it

fn seq<const N: usize>(children: [Content; N]) -> Content {
    Content::sequence(children)
}

fn strong(body: Content) -> Content {
    StrongElem::new(body).pack()
}

fn emph(body: Content) -> Content {
    EmphElem::new(body).pack()
}

/// `upper` and `lower` on content.
fn case(body: Content, case: TextCase) -> Content {
    body.set(TextElem::case, Some(case))
}

fn symbol(c: &mut Case, text: &str, needle: &str) -> Content {
    SymbolElem::packed(text).spanned(c.span(needle))
}

/// Inline raw text, from the next `needle` between backticks.
fn raw(c: &mut Case, needle: &str) -> Content {
    let span = c.span(needle);
    RawElem::new(RawContent::Lines(eco_vec![(needle.into(), span)]))
        .pack()
        .spanned(span)
}

fn underline(body: Content) -> UnderlineElem {
    UnderlineElem::new(body)
}

fn rgb(hex: u32) -> Paint {
    let [r, g, b, a] = hex.to_be_bytes();
    Paint::Solid(Color::from_u8(r, g, b, a))
}

fn pt(value: f64) -> Length {
    Abs::pt(value).into()
}

fn em(value: f64) -> Length {
    Em::new(value).into()
}

fn stroke(paint: Option<Paint>, thickness: Option<Length>) -> Stroke {
    Stroke {
        paint: paint.map_or(Smart::Auto, Smart::Custom),
        thickness: thickness.map_or(Smart::Auto, Smart::Custom),
        ..Stroke::default()
    }
}

/// A case and its content.
fn case_with(
    id: &'static str,
    content: impl FnOnce(&mut Case) -> Content,
) -> (Case, Content) {
    let mut case = Case::new(id);
    let content = content(&mut case);
    (case, content)
}

// ---------------------------------------------------------------------------------------------
// Text

#[test]
fn plain_text() {
    check_plain([
        "plain-hello",
        "plain-kerning",
        "plain-ligatures",
        "plain-punctuation",
        "plain-digits",
        "space-thin-kept",
        "emph-in-word",
    ]);
}

#[test]
fn shorthands_and_escapes() {
    check([
        case_with("space-nbsp", |c| {
            seq([c.words("A"), c.text("\u{a0}", "~"), c.words("B")])
        }),
        case_with("shorthands", |c| {
            seq([
                c.words("a "),
                c.text("–", "--"),
                c.words(" b "),
                c.text("—", "---"),
                c.words(" c "),
                c.text("…", "..."),
                c.words(" d"),
            ])
        }),
        case_with("escapes", |c| {
            seq([
                c.text("#", "\\#"),
                c.words("a "),
                c.text("*", "\\*"),
                c.words("b"),
                c.text("*", "\\*"),
                c.words(" "),
                c.text("_", "\\_"),
                c.words("c"),
                c.text("_", "\\_"),
                c.words(" "),
                c.text("A", "\\u{41}"),
            ])
        }),
    ]);
}

#[test]
fn space_collapsing() {
    check([
        case_with("space-collapsing-comments-1", |c| {
            seq([c.words("A"), c.words("B"), c.words("C")])
        }),
        case_with("space-collapsing-comments-2", |c| {
            seq([c.words("A"), c.space(" "), c.space(" "), c.words("B"), c.words("C")])
        }),
        case_with("space-collapsing-comments-3", |c| {
            seq([c.words("A"), c.space(" "), c.words("B"), c.space(" "), c.words("C")])
        }),
        case_with("space-collapse-runs", |c| c.words("A   B")),
        case_with("space-trim-ends", |c| c.words("  A B  ")),
        case_with("space-newline", |c| seq([c.words("A"), c.space("\n"), c.words("B")])),
    ]);
}

#[test]
fn scripts_fallback_and_bidi() {
    check_plain([
        "shaping-script-separation",
        "shaping-devanagari",
        "fallback-hebrew",
        "bidi-en-he-top-level-he",
        "bidi-en-he-top-level-en",
    ]);
    check([
        case_with("fallback-tofu", |c| {
            seq([c.words("A"), c.text("\u{E000}", "\\u{E000}"), c.words("B")])
        }),
        case_with("bidi-tofus", |c| {
            c.text("\u{590}\u{591}\u{592}\u{593}", "\"\\u{590}\\u{591}\\u{592}\\u{593}\"")
        }),
        case_with("emph-hebrew", |c| emph(c.words("שלום"))),
        case_with("bidi-consecutive-embedded-rtl-runs-he", |c| {
            seq([c.words("Aגֶ"), strong(c.words("שֶׁ")), c.words("םB")])
        }),
        case_with("bidi-consecutive-embedded-rtl-runs-en", |c| {
            seq([c.words("Aגֶ"), strong(c.words("שֶׁ")), c.words("םB")])
        }),
        case_with("bidi-nesting", |c| {
            seq([
                c.words("א"),
                c.text("\u{2066}", "\\u{2066}"),
                c.words("A"),
                c.text("\u{2067}", "\\u{2067}"),
                c.words("Bב"),
                c.text("\u{2069}", "\\u{2069}"),
                c.words("?"),
            ])
        }),
        case_with("bidi-explicit-dir", |c| {
            let ltr = TextDir(Smart::Custom(Dir::LTR));
            seq([
                c.text("8:00 - 9:00", "\"8:00 - 9:00\"").set(TextElem::dir, ltr),
                c.words(" בבוקר"),
            ])
        }),
    ]);
}

#[test]
fn text_properties() {
    let frac = Tag::from_bytes(b"frac");
    check([
        case_with("text-fill", |c| {
            seq([
                c.words("This is "),
                c.words("way more").set(TextElem::fill, rgb(0xFA644BFF)),
                c.words(" colorful."),
            ])
        }),
        case_with("text-fill-transparent", |c| {
            c.words("This text is transparent.")
                .set(TextElem::fill, rgb(0xFF000080))
        }),
        case_with("text-size", |c| {
            seq([
                c.words("A").set(TextElem::size, TextSize(pt(20.0))),
                c.words(" "),
                c.words("A").set(TextElem::size, TextSize(em(2.0))),
                c.words(" "),
                c.words("A").set(TextElem::size, TextSize(pt(15.0) + em(0.5))),
            ])
        }),
        case_with("text-size-relative", |c| {
            seq([
                c.words("Big").set(TextElem::size, TextSize(em(1.5))),
                c.words(" small"),
            ])
        }),
        case_with("text-style-italic", |c| {
            c.words("Italic").set(TextElem::style, FontStyle::Italic)
        }),
        case_with("text-weight-bold", |c| {
            c.words("Bold").set(TextElem::weight, FontWeight::BOLD)
        }),
        case_with("text-weight-regular", |c| {
            c.words("Regular").set(TextElem::weight, FontWeight::REGULAR)
        }),
        case_with("text-weight-semibold", |c| {
            c.words("Semibold")
                .set(TextElem::weight, FontWeight::from_number(600))
        }),
        case_with("text-tracking", |c| {
            c.words("Tracking").set(TextElem::tracking, pt(1.0))
        }),
        case_with("text-baseline", |c| {
            seq([
                c.words("A"),
                c.words("B").set(TextElem::baseline, pt(-3.0)),
                c.words("C"),
            ])
        }),
        case_with("text-features", |c| {
            let features = FontFeatures(smallvec::smallvec![(frac, 1)]);
            seq([c.words("1/2").set(TextElem::features, features), c.words(" 1/2")])
        }),
        case_with("text-unknown-family", |c| {
            let font = FontList(vec![FontFamily::new("nonexistent")]);
            c.words("but").set(TextElem::font, font)
        }),
        case_with("text-font-list", |c| {
            let families = ["nonexistent", "DejaVu Sans Mono"].map(FontFamily::new);
            c.words("mono").set(TextElem::font, FontList(families.into()))
        }),
    ]);
}

#[test]
fn strong_and_emph() {
    check([
        case_with("strong-segments", |c| {
            seq([strong(c.words("a")), strong(c.words("b"))])
        }),
        case_with("strong-delta", |c| {
            seq([
                strong(c.words("Bold")),
                c.words(" and "),
                StrongElem::new(c.words("Medium")).with_delta(150).pack(),
            ])
        }),
        case_with("strong-nested", |c| {
            seq([
                strong(c.words("Medium")),
                c.words(" and "),
                strong(strong(c.words("Bold"))),
            ])
        }),
        case_with("strong-emph", |c| strong(emph(c.words("x")))),
        case_with("strong-emph-calls", |c| strong(emph(c.words("x")))),
        case_with("emph-strong", |c| emph(strong(c.words("x")))),
        case_with("emph-syntax", |c| {
            emph(seq([
                c.words("Emphasized and "),
                strong(c.words("strong")),
                c.words(" words!"),
            ]))
        }),
        case_with("emph-and-strong-call-in-word", |c| {
            seq([
                c.words("P"),
                strong(c.words("art")),
                c.words("ly em"),
                emph(c.words("phas")),
                c.words("ized."),
            ])
        }),
        // `**` is an empty strong element.
        case_with("strong-double-star-empty", |c| {
            seq([strong(Content::empty()), c.words("not bold"), strong(Content::empty())])
        }),
        case_with("underline-strong", |c| underline(strong(c.words("x"))).pack()),
    ]);
}

#[test]
fn cases_and_smallcaps() {
    check([
        case_with("cases-lower", |c| case(c.words("HI!"), TextCase::Lower)),
        case_with("cases-upper", |c| case(c.words("ArE mEmEs gReAt?"), TextCase::Upper)),
        case_with("cases-upper-greek", |c| case(c.words("Ελλάδα"), TextCase::Upper)),
        case_with("cases-symbol", |c| {
            case(seq([c.words("a "), symbol(c, "α", "sym.alpha")]), TextCase::Upper)
        }),
        case_with("cases-upper-strong", |c| {
            case(seq([strong(c.words("x")), c.words(" y")]), TextCase::Upper)
        }),
        case_with("smallcaps", |c| SmallcapsElem::new(c.words("Smallcaps")).pack()),
        case_with("smallcaps-all", |c| {
            SmallcapsElem::new(c.words("Test 012")).with_all(true).pack()
        }),
    ]);
}

#[test]
fn smart_quotes() {
    check_plain([
        "smartquote",
        "smartquote-apostrophe",
        "smartquote-contraction",
        "smartquote-slash-1",
        "smartquote-slash-2",
        "smartquote-slash-3",
        "smartquote-close-before-letter",
        "smartquote-prime-1",
        "smartquote-prime-2",
        "smartquote-prime-3",
        "smartquote-bracket-1",
        "smartquote-bracket-2",
        "smartquote-nesting-1",
        "smartquote-nesting-2",
        "smartquote-nesting-3",
        "smartquote-lang-de",
        "smartquote-lang-fr",
    ]);
    check([
        // `\'` and `\"` are plain quotes.
        case_with("smartquote-escape", |c| {
            seq([
                c.words("The 5"),
                c.text("'", "\\'"),
                c.words("11"),
                c.text("\"", "\\\""),
                c.words(" "),
                c.quote("'"),
                c.words("quick"),
                c.text("'", "\\'"),
                c.words(" brown fox jumps over the "),
                c.text("\"", "\\\""),
                c.words("lazy"),
                c.quote("'"),
                c.words(" dog"),
                c.text("'", "\\'"),
                c.words("s ear."),
            ])
        }),
        // Embedding characters don't count as the text before a quote.
        case_with("smartquote-with-embedding-chars", |c| {
            seq([
                c.quote("\""),
                c.text("\u{202A}", "\"\\u{202A}\""),
                c.words("bonjour"),
                c.text("\u{202C}", "\"\\u{202C}\""),
                c.quote("\""),
            ])
        }),
    ]);
}

#[test]
fn symbols() {
    check([
        case_with("sym-arrow", |c| {
            seq([c.words("Flow "), symbol(c, "→", "sym.arrow.r"), c.words(" target")])
        }),
        case_with("sym-modifiers", |c| {
            seq([
                symbol(c, "⇔", "sym.arrow.l.r.double"),
                c.words(" "),
                symbol(c, "±", "sym.plus.minus"),
            ])
        }),
    ]);
}

// ---------------------------------------------------------------------------------------------
// Decorations and scripts

#[test]
fn decorations() {
    let red = rgb(0xFF4136FF);
    let blue = rgb(0x0074D9FF);
    let round = Stroke {
        cap: Smart::Custom(LineCap::Round),
        ..stroke(Some(red.clone()), Some(em(0.5)))
    };
    let radius = Corners::splat(Some(Rel::from(pt(3.0))));
    let highlight = |body| HighlightElem::new(body);
    check([
        case_with("underline-basic", |c| underline(c.words("Further below.")).pack()),
        case_with("underline-descenders", |c| underline(c.words("gyp jq")).pack()),
        case_with("underline-offset", |c| {
            underline(c.words("Further below."))
                .with_offset(Smart::Custom(pt(5.0)))
                .pack()
        }),
        case_with("underline-stroke-no-evade", |c| {
            underline(c.words("Critical information"))
                .with_stroke(Smart::Custom(stroke(Some(red.clone()), None)))
                .with_evade(false)
                .pack()
        }),
        case_with("underline-inherits-fill", |c| {
            underline(c.words("Change with the wind."))
                .pack()
                .set(TextElem::fill, red.clone())
        }),
        case_with("overline-underline", |c| {
            OverlineElem::new(underline(c.words("Running amongst the wolves.")).pack())
                .pack()
        }),
        case_with("strike-basic", |c| {
            StrikeElem::new(c.words("Statements dreamt up")).pack()
        }),
        case_with("strike-redact", |c| {
            StrikeElem::new(c.words("in secret"))
                .with_stroke(Smart::Custom(stroke(None, Some(pt(10.0)))))
                .with_extent(em(0.05))
                .pack()
        }),
        case_with("strike-transparent", |c| {
            StrikeElem::new(c.words("redacted"))
                .with_stroke(Smart::Custom(stroke(Some(rgb(0xABCDEF88)), Some(pt(10.0)))))
                .with_extent(em(0.05))
                .pack()
        }),
        case_with("underline-stroke-folding", |c| {
            let body = c.words("DANGER!").set(TextElem::fill, red.clone());
            underline(body)
                .with_stroke(Smart::Custom(stroke(None, Some(pt(2.0)))))
                .with_offset(Smart::Custom(pt(2.0)))
                .pack()
        }),
        case_with("underline-background", |c| {
            underline(c.words("This is in the background"))
                .with_background(true)
                .with_stroke(Smart::Custom(round.clone()))
                .pack()
        }),
        case_with("overline-background", |c| {
            OverlineElem::new(c.words("This is in the background"))
                .with_background(true)
                .with_stroke(Smart::Custom(round.clone()))
                .pack()
        }),
        case_with("strike-background", |c| {
            StrikeElem::new(c.words("This is in the background"))
                .with_background(true)
                .with_stroke(Smart::Custom(stroke(Some(red.clone()), Some(pt(5.0)))))
                .pack()
        }),
        case_with("highlight-default", |c| {
            seq([
                c.words("This is the built-in "),
                highlight(c.words("highlight with default color")).pack(),
                c.words("."),
            ])
        }),
        case_with("highlight-fill", |c| {
            highlight(c.words("to highlight"))
                .with_fill(Some(rgb(0xCCF5D6FF)))
                .pack()
        }),
        case_with("highlight-bounds", |c| {
            seq([
                highlight(c.words("ace")).pack(),
                c.words(", "),
                highlight(c.words("base")).pack(),
                c.words(", "),
                highlight(c.words("super")).pack(),
                c.words(", "),
                highlight(seq([c.words("phone "), symbol(c, "∫", "sym.integral")]))
                    .pack(),
            ])
        }),
        case_with("highlight-edges", |c| {
            let edges = |elem: HighlightElem| {
                elem.with_top_edge(TopEdge::Metric(TopEdgeMetric::XHeight))
                    .with_bottom_edge(BottomEdge::Metric(BottomEdgeMetric::Baseline))
                    .pack()
            };
            seq([
                edges(highlight(c.words("ace"))),
                c.words(", "),
                edges(highlight(seq([
                    c.words("phone "),
                    symbol(c, "∫", "sym.integral"),
                ]))),
            ])
        }),
        case_with("highlight-edges-bounds", |c| {
            highlight(seq([c.words("abc "), symbol(c, "∫", "sym.integral")]))
                .with_top_edge(TopEdge::Metric(TopEdgeMetric::Bounds))
                .with_bottom_edge(BottomEdge::Metric(BottomEdgeMetric::Bounds))
                .pack()
        }),
        case_with("highlight-radius", |c| {
            highlight(c.words("abc")).with_radius(radius).pack()
        }),
        case_with("highlight-stroke", |c| {
            let side = Some(Some(stroke(Some(blue.clone()), Some(pt(2.0)))));
            highlight(c.words("abc")).with_stroke(Sides::splat(side)).pack()
        }),
        case_with("highlight-stroke-sides", |c| {
            let side = |paint: Paint| Some(Some(stroke(Some(paint), None)));
            let sides = Sides::new(
                side(red.clone()),
                side(blue.clone()),
                side(rgb(0xFF851BFF)),
                side(rgb(0x2ECC40FF)),
            );
            highlight(c.words("abc")).with_stroke(sides).pack()
        }),
        case_with("highlight-stroke-radius", |c| {
            highlight(c.words("Lorem ipsum dolor"))
                .with_stroke(Sides::splat(Some(Some(stroke(None, Some(pt(1.0)))))))
                .with_radius(radius)
                .pack()
        }),
    ]);
}

#[test]
fn sub_and_superscripts() {
    let synthesized = |elem: SuperElem| {
        elem.with_typographic(false)
            .with_baseline(Smart::Custom(em(-0.25)))
            .with_size(Smart::Custom(TextSize(em(0.7))))
            .pack()
    };
    let scripted = |stroke: Stroke| {
        move |body: Content| {
            underline(body)
                .with_stroke(Smart::Custom(stroke.clone()))
                .with_offset(Smart::Custom(em(0.15)))
                .pack()
        }
    };
    let thin = scripted(stroke(None, Some(pt(0.5))));
    check([
        case_with("sub-super-typographic", |c| {
            seq([
                c.words("x"),
                SuperElem::new(c.words("123")).pack(),
                c.words(" x"),
                SubElem::new(c.words("123")).pack(),
            ])
        }),
        case_with("sub-super-synthesized", |c| {
            seq([
                c.words("x"),
                SuperElem::new(c.words("1,2,3")).pack(),
                c.words(" x"),
                SubElem::new(c.words("1,2,3")).pack(),
            ])
        }),
        case_with("sub-super-non-typographic", |c| {
            seq([
                c.words("n"),
                synthesized(SuperElem::new(c.words("1"))),
                c.words(", n"),
                SubElem::new(c.words("2")).pack(),
                c.words(", "),
                c.text("…", "..."),
                c.words(" n"),
                synthesized(SuperElem::new(c.words("N"))),
            ])
        }),
        case_with("super-underline-outer", |c| {
            seq([
                thin(seq([
                    c.words("A"),
                    SuperElem::new(c.words("4")).with_typographic(false).pack(),
                ])),
                c.words(" B"),
            ])
        }),
        case_with("super-underline-inner", |c| {
            seq([
                c.words("A"),
                SuperElem::new(thin(c.words("4"))).with_typographic(false).pack(),
                c.words(" B"),
            ])
        }),
        case_with("super-underline-typographic", |c| {
            seq([
                thin(seq([c.words("A"), SuperElem::new(c.words("4")).pack()])),
                c.words(" B"),
            ])
        }),
        case_with("super-highlight-outer", |c| {
            let body = seq([
                c.words("A"),
                SuperElem::new(c.words("4")).with_typographic(false).pack(),
            ]);
            seq([HighlightElem::new(body).pack(), c.words(" B")])
        }),
        case_with("super-highlight-inner", |c| {
            seq([
                c.words("A"),
                SuperElem::new(HighlightElem::new(c.words("4")).pack()).pack(),
                c.words(" B"),
            ])
        }),
        case_with("long-scripts-typographic", |c| {
            seq([
                c.words("|"),
                SuperElem::new(c.words("longscript")).with_typographic(true).pack(),
                c.words("| |"),
                SubElem::new(c.words("longscript")).with_typographic(true).pack(),
                c.words("|"),
            ])
        }),
        case_with("long-scripts-synthesized", |c| {
            seq([
                c.words("|"),
                SuperElem::new(c.words("longscript")).with_typographic(false).pack(),
                c.words("| |"),
                SubElem::new(c.words("longscript")).with_typographic(false).pack(),
                c.words("|"),
            ])
        }),
        case_with("script-metrics-lato", |c| {
            seq([
                c.words("Xx"),
                SuperElem::new(c.words("Xx")).with_typographic(false).pack(),
                SubElem::new(c.words("Xx")).with_typographic(false).pack(),
            ])
        }),
        case_with("script-metrics-mono", |c| {
            seq([
                c.words("Xx"),
                SuperElem::new(c.words("Xx")).with_typographic(false).pack(),
                SubElem::new(c.words("Xx")).with_typographic(false).pack(),
            ])
        }),
        case_with("script-nested", |c| {
            let (x, y) = (c.words("x"), c.words("y"));
            let inner = SuperElem::new(c.words("z")).pack();
            seq([x, SuperElem::new(seq([y, inner])).pack()])
        }),
    ]);
}

#[test]
fn raw_text() {
    check([
        case_with("raw-consecutive", |c| seq([raw(c, "A"), raw(c, "B")])),
        case_with("raw-inline", |c| {
            let start = seq([c.words("Use "), raw(c, "x # y"), c.words(" and ")]);
            let call = c.span("raw(\"z * w\")");
            let text =
                RawElem::new(RawContent::Text("z * w".into())).pack().spanned(call);
            seq([start, text])
        }),
        case_with("raw-in-text", |c| seq([c.words("a "), raw(c, "b"), c.words(" c")])),
        case_with("bidi-raw", |c| {
            seq([c.words("לדוג. "), raw(c, "if a == b:"), c.words(" זה תנאי")])
        }),
    ]);
}

// ---------------------------------------------------------------------------------------------
// Invariants

/// Realizes content under root styles.
fn realized<'a>(
    kind: RealizationKind,
    arenas: &'a Arenas<'a>,
    content: &'a Content,
    styles: StyleChain<'a>,
) -> Vec<Pair<'a>> {
    let world = WithSource { world: fixtures::shared(), source: "" };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    realize(kind, &mut engine, arenas, content, styles).unwrap()
}

/// The element names of realized pairs, with each text's text.
fn names(pairs: &[Pair]) -> Vec<String> {
    pairs
        .iter()
        .map(|(content, _)| match content.to_packed::<TextElem>() {
            Some(text) => format!("text {:?}", text.text),
            None => content.elem().name().into(),
        })
        .collect()
}

#[test]
fn spaces_collapse_like_upstream() {
    let text = |t: &str| TextElem::packed(t);
    let space = || SpaceElem::shared().clone();
    let linebreak = || LinebreakElem::shared().clone();
    let root = Styles::new();
    let styles = StyleChain::new(&root);
    let run = |children: Vec<Content>| {
        let mut buf: Vec<_> = children.iter().map(|c| (c, styles)).collect();
        collapse_spaces(&mut buf, 0);
        names(&buf)
    };
    // Spaces at the edges go, and adjacent spaces collapse into the first.
    assert_eq!(
        run(vec![space(), text("a"), space(), space(), text("b"), space()]),
        ["text \"a\"", "space", "text \"b\""]
    );
    // Spaces next to a line break go.
    assert_eq!(
        run(vec![text("a"), space(), linebreak(), space(), text("b")]),
        ["text \"a\"", "linebreak", "text \"b\""]
    );
    // The first of adjacent spaces keeps its styles.
    let mut local = Styles::new();
    local.set(TextElem::size, TextSize(pt(20.0)));
    let first = space();
    let second = space();
    let (a, b) = (text("a"), text("b"));
    let mut buf: Vec<Pair> = vec![
        (&a, styles),
        (&first, styles.chain(&local)),
        (&second, styles),
        (&b, styles),
    ];
    collapse_spaces(&mut buf, 0);
    assert_eq!(buf.len(), 3);
    assert_eq!(buf[1].1.resolve(TextElem::size), Abs::pt(20.0));
}

#[test]
fn symbols_are_text_outside_of_math() {
    let root = Styles::new();
    let styles = StyleChain::new(&root);
    let content = seq([SymbolElem::packed("α"), TextElem::packed("x")]);
    let arenas = Arenas::default();
    assert_eq!(
        names(&realized(RealizationKind::Par, &arenas, &content, styles)),
        ["text \"α\"", "text \"x\""]
    );
    // In math, symbols stay symbols, and nested equations give way to their bodies.
    let nested = EquationElem::new(seq([SymbolElem::packed("α"), TextElem::packed("x")]));
    let arenas = Arenas::default();
    let content = nested.pack();
    assert_eq!(
        names(&realized(RealizationKind::Math, &arenas, &content, styles)),
        ["symbol", "text \"x\""]
    );
}

#[test]
fn built_in_rules_style_their_bodies() {
    let root = Styles::new();
    let styles = StyleChain::new(&root);
    let content = strong(emph(TextElem::packed("x")));
    let arenas = Arenas::default();
    let pairs = realized(RealizationKind::Par, &arenas, &content, styles);
    assert_eq!(names(&pairs), ["text \"x\""]);
    let (_, styles) = pairs[0];
    assert_eq!(styles.get(TextElem::delta).0, 300);
    assert!(styles.get(TextElem::emph).0);
}

#[test]
fn raw_text_takes_its_show_set_and_lines() {
    let root = Styles::new();
    let styles = StyleChain::new(&root);
    let content = RawElem::new(RawContent::Text("a\tb".into())).pack();
    let arenas = Arenas::default();
    let pairs = realized(RealizationKind::Par, &arenas, &content, styles);
    // Tabs align to the default tab size, 2.
    assert_eq!(names(&pairs), ["text \"a b\""]);
    let (_, raw) = pairs[0];
    assert_eq!(
        raw.get_ref(TextElem::font)
            .into_iter()
            .map(|f| f.as_str())
            .collect::<Vec<_>>(),
        ["dejavu sans mono"]
    );
    assert_eq!(raw.resolve(TextElem::size), Abs::pt(11.0 * 0.8));
    assert!(!raw.get(TextElem::overhang));

    // Lines are separated by line breaks, which a label then rejects.
    let content = RawElem::new(RawContent::Text("a\nb".into())).pack();
    let arenas = Arenas::default();
    assert_eq!(
        names(&realized(RealizationKind::Par, &arenas, &content, styles)),
        ["text \"a\"", "linebreak", "text \"b\""]
    );
}

#[test]
fn raw_lines_map_to_their_source() {
    let mut c = Case::custom("a `b c` d");
    let content = seq([c.words("a "), raw(&mut c, "b c"), c.words(" d")]);
    let frame = c.layout(&content).unwrap();
    let mut sources = vec![];
    for (_, item) in frame.items() {
        if let crate::typst_library::layout::FrameItem::Text(text) = item {
            for glyph in &text.glyphs {
                sources.push(glyph.source.clone());
            }
        }
    }
    assert_eq!(sources, [0..1, 1..2, 3..4, 4..5, 5..6, 7..8, 8..9]);
}
