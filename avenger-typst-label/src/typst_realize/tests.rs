//! Realization invariants. The frame oracle runs realization on every reference case, from
//! its source (`typst_layout::math::tests`).

use super::realize;
use super::spaces::collapse_spaces;
use crate::label::fixtures::{self, WithSource};
use crate::label::oracle::{default_settings, layout_source};
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{
    Content, NativeElement, StyleChain, Styles, SymbolElem,
};
use crate::typst_library::layout::{Abs, FrameItem};
use crate::typst_library::math::EquationElem;
use crate::typst_library::model::{EmphElem, StrongElem};
use crate::typst_library::routines::{Arenas, Pair, RealizationKind};
use crate::typst_library::text::{
    LinebreakElem, RawContent, RawElem, SpaceElem, TextElem, TextSize,
};

fn seq<const N: usize>(children: [Content; N]) -> Content {
    Content::sequence(children)
}

fn strong(body: Content) -> Content {
    StrongElem::new(body).pack()
}

fn emph(body: Content) -> Content {
    EmphElem::new(body).pack()
}

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
    local.set(TextElem::size, TextSize(Abs::pt(20.0).into()));
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

    // Raw text of more than one line is an error, as raw markup is.
    let content = RawElem::new(RawContent::Text("a\nb".into())).pack();
    let arenas = Arenas::default();
    let world = WithSource { world: fixtures::shared(), source: "" };
    let mut sink = Sink::new();
    let mut engine = Engine { world: &world, sink: &mut sink };
    let errors = realize(RealizationKind::Par, &mut engine, &arenas, &content, styles)
        .unwrap_err();
    assert_eq!(errors[0].message, "raw text in a label must be a single line");
}

#[test]
fn raw_lines_map_to_their_source() {
    let frame = layout_source("a `b c` d", &default_settings()).unwrap();
    let mut sources = vec![];
    for (_, item) in frame.items() {
        if let FrameItem::Text(text) = item {
            for glyph in &text.glyphs {
                sources.push(glyph.source.clone());
            }
        }
    }
    assert_eq!(sources, [0..1, 1..2, 3..4, 4..5, 5..6, 7..8, 8..9]);
}
