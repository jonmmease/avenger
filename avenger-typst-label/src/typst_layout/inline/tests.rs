//! Frame-oracle cases built from hand-made realized content, and inline layout invariants.
//!
//! Each case lays out the children that evaluation and realization produce for a reference's
//! source, under the probe wrapper's root styles, and compares the line with upstream's frame.
//! Until the pipeline has evaluation and realization, the children are written out here, and
//! the functions under "Rules" restate the built-in show rules that style them. Flattened
//! frames of failing cases go to the gitignored `tests/output/inline/`.

use std::fmt::Debug;
use std::ops::Range;
use std::sync::LazyLock;

use ecow::EcoString;
use smallvec::smallvec;

use super::layout_label_line;
use crate::label::fixtures::{self, WithSource};
use crate::label::oracle::{
    self, Manifest, Reference, Settings, TOLERANCE, compare, output_dir,
};
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::{Engine, Sink};
use crate::typst_library::foundations::{
    Content, Field, NativeElement, SettableProperty, Smart, StyleChain, Styles,
};
use crate::typst_library::layout::{
    Abs, Corners, Dir, Em, Frame, FrameItem, Length, Rel, Sides,
};
use crate::typst_library::model::StrongElem;
use crate::typst_library::routines::Pair;
use crate::typst_library::text::{
    BottomEdge, BottomEdgeMetric, Case as TextCase, DecoLine, Decoration, FontFamily,
    FontFeatures, FontList, FontStyle, FontWeight, HighlightElem, ItalicToggle, Lang,
    LinebreakElem, OverlineElem, Region, Smallcaps, SmartQuoteElem, SpaceElem,
    StrikeElem, Tag, TextDir, TextElem, TextSize, TopEdge, TopEdgeMetric, UnderlineElem,
    WeightDelta, check_font_list, families,
};
use crate::typst_library::visualize::{Color, ColorExt, LineCap, Paint, Stroke};
use crate::typst_syntax::{FileId, Span, Spanned};

const SUITE: &str = "upstream_frames";

static MANIFEST: LazyLock<Manifest> = LazyLock::new(|| Manifest::load(SUITE));

/// A case under construction: its source, root styles, and realized children. Content and
/// styles live for the rest of the test run, as realization's arenas would hold them.
struct Case {
    id: &'static str,
    source: &'static str,
    root: StyleChain<'static>,
    children: Vec<Pair<'static>>,
    /// Where the next source lookup starts.
    cursor: usize,
}

impl Case {
    /// Starts the case with this id in the manifest, under the probe wrapper's root styles.
    fn new(id: &'static str) -> Self {
        let case = MANIFEST
            .cases
            .iter()
            .find(|case| case.id == id)
            .unwrap_or_else(|| panic!("no case {id}"));
        Self::with_settings(id, &case.source, MANIFEST.settings(case))
    }

    /// Starts a case for a source without a reference, under the manifest's default styles.
    fn custom(source: &str) -> Self {
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
            styles.set(TextElem::dir, TextDir(Smart::Custom(direction(dir))));
        }
        Self {
            id,
            source: leak(source.to_string()),
            root: StyleChain::new(leak(styles)),
            children: vec![],
            cursor: 0,
        }
    }

    /// The source text as markup without markup: words, spaces and smart quotes.
    fn plain(id: &'static str) -> Self {
        let mut case = Self::new(id);
        let (root, source) = (case.root, case.source);
        case.markup(root, source);
        case
    }

    /// The range of the next `needle` in the source.
    fn find(&mut self, needle: &str) -> Range<usize> {
        let Some(offset) = self.source[self.cursor..].find(needle) else {
            panic!("{}: no {needle:?} after byte {}", self.id, self.cursor);
        };
        let start = self.cursor + offset;
        self.cursor = start + needle.len();
        start..self.cursor
    }

    /// The next `needle` in the source as words and spaces.
    fn words(&mut self, styles: StyleChain<'static>, needle: &str) -> &mut Self {
        let range = self.find(needle);
        let mut start = range.start;
        for run in word_runs(&self.source[range]) {
            let span = span(start..start + run.len());
            let content = if run.starts_with(' ') {
                SpaceElem::shared().clone()
            } else {
                TextElem::new(run.into()).pack()
            };
            self.children.push((leak(content.spanned(span)), styles));
            start += run.len();
        }
        self
    }

    /// The next `needle` in the source as markup: words, spaces, and smart quotes for its
    /// straight quotes.
    fn markup(&mut self, styles: StyleChain<'static>, needle: &str) -> &mut Self {
        for run in quote_runs(needle) {
            if run == "\"" || run == "'" {
                self.quote(styles, run);
            } else {
                self.words(styles, run);
            }
        }
        self
    }

    /// A text element with `text`, made from the next `needle` in the source.
    fn text(
        &mut self,
        styles: StyleChain<'static>,
        text: &str,
        needle: &str,
    ) -> &mut Self {
        let span = span(self.find(needle));
        let content = TextElem::new(EcoString::from(text)).pack().spanned(span);
        self.children.push((leak(content), styles));
        self
    }

    /// A smart quote made from the next `needle`.
    fn quote(&mut self, styles: StyleChain<'static>, needle: &str) -> &mut Self {
        let span = span(self.find(needle));
        let content =
            SmartQuoteElem::new().with_double(needle == "\"").pack().spanned(span);
        self.children.push((leak(content), styles));
        self
    }

    /// Lays out the children and compares the line with the reference.
    fn compare(&self) -> Result<(), String> {
        let frame = self.layout().map_err(|err| format!("{}: {err:?}", self.id))?;
        let reference = Reference::load(SUITE, self.id)?;
        if reference.source != self.source {
            return Err(format!("{}: stale reference", self.id));
        }
        let expected = reference.flat().expect("upstream lays the case out");
        let actual = oracle::flatten(&frame);
        let mismatches = compare(&expected, &actual, TOLERANCE);
        if mismatches.failed_checks().is_empty() {
            return Ok(());
        }
        let dir = output_dir("inline");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, flat) in [("expected", &expected), ("actual", &actual)] {
            let path = dir.join(format!("{}.{name}.json", self.id));
            std::fs::write(path, serde_json::to_string_pretty(flat).unwrap()).unwrap();
        }
        Err(format!("{} differs from upstream:\n{}", self.id, mismatches.summary()))
    }

    fn layout(&self) -> SourceResult<Frame> {
        let world = WithSource { world: fixtures::shared(), source: self.source };
        let mut sink = Sink::new();
        let mut engine = Engine { world: &world, sink: &mut sink };
        layout_label_line(&mut engine, &self.children, self.root)
    }
}

/// Checks every case and reports all failures.
fn check(cases: impl IntoIterator<Item = Case>) {
    let failures: Vec<_> =
        cases.into_iter().filter_map(|case| case.compare().err()).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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

fn span(range: Range<usize>) -> Span {
    Span::from_range(FileId::LABEL, range)
}

fn direction(dir: &str) -> Dir {
    match dir {
        "ltr" => Dir::LTR,
        "rtl" => Dir::RTL,
        other => panic!("unexpected direction {other}"),
    }
}

fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}

/// `styles` on top of `outer`.
fn styled(outer: StyleChain<'static>, styles: Styles) -> StyleChain<'static> {
    leak(outer).chain(leak(styles))
}

/// One text property on top of `outer`.
fn text<const I: u8>(
    outer: StyleChain<'static>,
    field: Field<TextElem, I>,
    value: <TextElem as SettableProperty<I>>::Type,
) -> StyleChain<'static>
where
    TextElem: SettableProperty<I>,
    <TextElem as SettableProperty<I>>::Type: Debug + Clone + Send + Sync + 'static,
{
    let mut styles = Styles::new();
    styles.set(field, value);
    styled(outer, styles)
}

fn rgb(hex: u32) -> Paint {
    let [r, g, b, a] = hex.to_be_bytes();
    Paint::Solid(Color::from_u8(r, g, b, a))
}

// ---------------------------------------------------------------------------------------------
// Rules: what upstream's built-in show rules (`typst-layout/src/rules.rs`) put on the bodies
// of these elements.

fn strong(outer: StyleChain<'static>, elem: StrongElem) -> StyleChain<'static> {
    text(outer, TextElem::delta, WeightDelta(elem.delta.get(outer)))
}

fn emph(outer: StyleChain<'static>) -> StyleChain<'static> {
    text(outer, TextElem::emph, ItalicToggle(true))
}

fn deco(outer: StyleChain<'static>, line: DecoLine, extent: Abs) -> StyleChain<'static> {
    text(outer, TextElem::deco, smallvec![Decoration { line, extent }])
}

fn underline(outer: StyleChain<'static>, elem: UnderlineElem) -> StyleChain<'static> {
    let line = DecoLine::Underline {
        stroke: elem.stroke.resolve(outer).unwrap_or_default(),
        offset: elem.offset.resolve(outer),
        evade: elem.evade.get(outer),
        background: elem.background.get(outer),
    };
    deco(outer, line, elem.extent.resolve(outer))
}

fn overline(outer: StyleChain<'static>, elem: OverlineElem) -> StyleChain<'static> {
    let line = DecoLine::Overline {
        stroke: elem.stroke.resolve(outer).unwrap_or_default(),
        offset: elem.offset.resolve(outer),
        evade: elem.evade.get(outer),
        background: elem.background.get(outer),
    };
    deco(outer, line, elem.extent.resolve(outer))
}

fn strike(outer: StyleChain<'static>, elem: StrikeElem) -> StyleChain<'static> {
    let line = DecoLine::Strikethrough {
        stroke: elem.stroke.resolve(outer).unwrap_or_default(),
        offset: elem.offset.resolve(outer),
        background: elem.background.get(outer),
    };
    deco(outer, line, elem.extent.resolve(outer))
}

fn highlight(outer: StyleChain<'static>, elem: HighlightElem) -> StyleChain<'static> {
    let line = DecoLine::Highlight {
        fill: elem.fill.get_cloned(outer),
        stroke: elem
            .stroke
            .resolve(outer)
            .unwrap_or_default()
            .map(|stroke| stroke.map(Stroke::unwrap_or_default)),
        top_edge: elem.top_edge.get(outer),
        bottom_edge: elem.bottom_edge.get(outer),
        radius: elem.radius.resolve(outer).unwrap_or_default(),
    };
    deco(outer, line, elem.extent.resolve(outer))
}

fn smallcaps(outer: StyleChain<'static>, all: bool) -> StyleChain<'static> {
    let smallcaps = if all { Smallcaps::All } else { Smallcaps::Minuscules };
    text(outer, TextElem::smallcaps, Some(smallcaps))
}

fn case(outer: StyleChain<'static>, case: TextCase) -> StyleChain<'static> {
    text(outer, TextElem::case, Some(case))
}

fn body() -> Content {
    Content::empty()
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

// ---------------------------------------------------------------------------------------------
// Cases

#[test]
fn plain_text() {
    check(
        [
            "plain-hello",
            "plain-kerning",
            "plain-ligatures",
            "plain-punctuation",
            "plain-digits",
            "space-thin-kept",
            "emph-in-word",
        ]
        .map(Case::plain),
    );
}

#[test]
fn shorthands_and_escapes() {
    let mut nbsp = Case::new("space-nbsp");
    let root = nbsp.root;
    nbsp.words(root, "A").text(root, "\u{a0}", "~").words(root, "B");

    let mut shorthands = Case::new("shorthands");
    let root = shorthands.root;
    shorthands
        .words(root, "a ")
        .text(root, "–", "--")
        .words(root, " b ")
        .text(root, "—", "---")
        .words(root, " c ")
        .text(root, "…", "...")
        .words(root, " d");

    let mut escapes = Case::new("escapes");
    let root = escapes.root;
    escapes
        .text(root, "#", "\\#")
        .words(root, "a ")
        .text(root, "*", "\\*")
        .words(root, "b")
        .text(root, "*", "\\*")
        .words(root, " ")
        .text(root, "_", "\\_")
        .words(root, "c")
        .text(root, "_", "\\_")
        .words(root, " ")
        .text(root, "A", "\\u{41}");

    check([nbsp, shorthands, escapes]);
}

#[test]
fn scripts_fallback_and_bidi() {
    let plain = [
        "shaping-script-separation",
        "shaping-devanagari",
        "fallback-hebrew",
        "bidi-en-he-top-level-he",
        "bidi-en-he-top-level-en",
    ]
    .map(Case::plain);

    let mut tofu = Case::new("fallback-tofu");
    let root = tofu.root;
    tofu.words(root, "A")
        .text(root, "\u{E000}", "\\u{E000}")
        .words(root, "B");

    let mut tofus = Case::new("bidi-tofus");
    let root = tofus.root;
    tofus.text(
        root,
        "\u{590}\u{591}\u{592}\u{593}",
        "\"\\u{590}\\u{591}\\u{592}\\u{593}\"",
    );

    let mut emph_hebrew = Case::new("emph-hebrew");
    let e = emph(emph_hebrew.root);
    emph_hebrew.words(e, "שלום");

    let consecutive = [
        "bidi-consecutive-embedded-rtl-runs-he",
        "bidi-consecutive-embedded-rtl-runs-en",
    ]
    .map(|id| {
        let mut case = Case::new(id);
        let root = case.root;
        let s = strong(root, StrongElem::new(body()));
        case.words(root, "Aגֶ").words(s, "שֶׁ").words(root, "םB");
        case
    });

    let mut nesting = Case::new("bidi-nesting");
    let root = nesting.root;
    nesting
        .words(root, "א")
        .text(root, "\u{2066}", "\\u{2066}")
        .words(root, "A")
        .text(root, "\u{2067}", "\\u{2067}")
        .words(root, "Bב")
        .text(root, "\u{2069}", "\\u{2069}")
        .words(root, "?");

    let mut explicit = Case::new("bidi-explicit-dir");
    let root = explicit.root;
    let ltr = text(root, TextElem::dir, TextDir(Smart::Custom(Dir::LTR)));
    explicit
        .text(ltr, "8:00 - 9:00", "\"8:00 - 9:00\"")
        .words(root, " בבוקר");

    check(
        plain
            .into_iter()
            .chain([tofu, tofus, emph_hebrew])
            .chain(consecutive)
            .chain([nesting, explicit]),
    );
}

#[test]
fn text_properties() {
    let mut cases = vec![];

    let mut fill = Case::new("text-fill");
    let root = fill.root;
    let red = text(root, TextElem::fill, rgb(0xFA644BFF));
    fill.words(root, "This is ")
        .words(red, "way more")
        .words(root, " colorful.");
    cases.push(fill);

    let mut transparent = Case::new("text-fill-transparent");
    let f = text(transparent.root, TextElem::fill, rgb(0xFF000080));
    transparent.words(f, "This text is transparent.");
    cases.push(transparent);

    let mut size = Case::new("text-size");
    let root = size.root;
    let s20 = text(root, TextElem::size, TextSize(pt(20.0)));
    let s2em = text(root, TextElem::size, TextSize(em(2.0)));
    let mixed = text(root, TextElem::size, TextSize(pt(15.0) + em(0.5)));
    size.words(s20, "A")
        .words(root, " ")
        .words(s2em, "A")
        .words(root, " ")
        .words(mixed, "A");
    cases.push(size);

    let mut relative = Case::new("text-size-relative");
    let root = relative.root;
    let big = text(root, TextElem::size, TextSize(em(1.5)));
    relative.words(big, "Big").words(root, " small");
    cases.push(relative);

    let mut italic = Case::new("text-style-italic");
    let styles = text(italic.root, TextElem::style, FontStyle::Italic);
    italic.words(styles, "Italic");
    cases.push(italic);

    for (id, word, weight) in [
        ("text-weight-bold", "Bold", FontWeight::BOLD),
        ("text-weight-regular", "Regular", FontWeight::REGULAR),
        ("text-weight-semibold", "Semibold", FontWeight::from_number(600)),
    ] {
        let mut case = Case::new(id);
        let styles = text(case.root, TextElem::weight, weight);
        case.words(styles, word);
        cases.push(case);
    }

    let mut tracking = Case::new("text-tracking");
    let t = text(tracking.root, TextElem::tracking, pt(1.0));
    tracking.words(t, "Tracking");
    cases.push(tracking);

    let mut baseline = Case::new("text-baseline");
    let root = baseline.root;
    let b = text(root, TextElem::baseline, pt(-3.0));
    baseline.words(root, "A").words(b, "B").words(root, "C");
    cases.push(baseline);

    let mut features = Case::new("text-features");
    let root = features.root;
    let frac = Tag::from_bytes(b"frac");
    let f = text(root, TextElem::features, FontFeatures(smallvec![(frac, 1)]));
    features.words(f, "1/2").words(root, " 1/2");
    cases.push(features);

    let mut unknown = Case::new("text-unknown-family");
    let f = text(
        unknown.root,
        TextElem::font,
        FontList(vec![FontFamily::new("nonexistent")]),
    );
    unknown.words(f, "but");
    cases.push(unknown);

    let mut list = Case::new("text-font-list");
    let families = ["nonexistent", "DejaVu Sans Mono"].map(FontFamily::new);
    let f = text(list.root, TextElem::font, FontList(families.into()));
    list.words(f, "mono");
    cases.push(list);

    check(cases);
}

#[test]
fn strong_and_emph() {
    let mut cases = vec![];

    // Two strong elements style separately, so their text shapes as two segments.
    let mut segments = Case::new("strong-segments");
    let root = segments.root;
    let a = strong(root, StrongElem::new(body()));
    let b = strong(root, StrongElem::new(body()));
    segments.words(a, "a").words(b, "b");
    cases.push(segments);

    let mut delta = Case::new("strong-delta");
    let root = delta.root;
    let bold = strong(root, StrongElem::new(body()));
    let medium = strong(root, StrongElem::new(body()).with_delta(150));
    delta.words(bold, "Bold").words(root, " and ").words(medium, "Medium");
    cases.push(delta);

    let mut nested = Case::new("strong-nested");
    let root = nested.root;
    let medium = strong(root, StrongElem::new(body()));
    let outer = strong(root, StrongElem::new(body()));
    let bold = strong(outer, StrongElem::new(body()));
    nested
        .words(medium, "Medium")
        .words(root, " and ")
        .words(bold, "Bold");
    cases.push(nested);

    for id in ["strong-emph", "strong-emph-calls"] {
        let mut case = Case::new(id);
        let styles = emph(strong(case.root, StrongElem::new(body())));
        case.words(styles, "x");
        cases.push(case);
    }

    let mut emph_strong = Case::new("emph-strong");
    let styles = strong(emph(emph_strong.root), StrongElem::new(body()));
    emph_strong.words(styles, "x");
    cases.push(emph_strong);

    let mut syntax = Case::new("emph-syntax");
    let e = emph(syntax.root);
    let s = strong(e, StrongElem::new(body()));
    syntax
        .words(e, "Emphasized and ")
        .words(s, "strong")
        .words(e, " words!");
    cases.push(syntax);

    let mut in_word = Case::new("emph-and-strong-call-in-word");
    let root = in_word.root;
    let s = strong(root, StrongElem::new(body()));
    let e = emph(root);
    in_word
        .words(root, "P")
        .words(s, "art")
        .words(root, "ly em")
        .words(e, "phas")
        .words(root, "ized.");
    cases.push(in_word);

    // `**` is an empty strong element, which realizes to nothing.
    let mut empty = Case::new("strong-double-star-empty");
    let root = empty.root;
    empty.words(root, "not bold");
    cases.push(empty);

    let mut underlined = Case::new("underline-strong");
    let u = underline(underlined.root, UnderlineElem::new(body()));
    let s = strong(u, StrongElem::new(body()));
    underlined.words(s, "x");
    cases.push(underlined);

    check(cases);
}

#[test]
fn cases_and_smallcaps() {
    let mut cases = vec![];
    for (id, word, which) in [
        ("cases-lower", "HI!", TextCase::Lower),
        ("cases-upper", "ArE mEmEs gReAt?", TextCase::Upper),
        ("cases-upper-greek", "Ελλάδα", TextCase::Upper),
    ] {
        let mut c = Case::new(id);
        let styles = case(c.root, which);
        c.words(styles, word);
        cases.push(c);
    }

    let mut symbol = Case::new("cases-symbol");
    let up = case(symbol.root, TextCase::Upper);
    symbol.words(up, "a ").text(up, "α", "sym.alpha");
    cases.push(symbol);

    let mut upper_strong = Case::new("cases-upper-strong");
    let up = case(upper_strong.root, TextCase::Upper);
    let s = strong(up, StrongElem::new(body()));
    upper_strong.words(s, "x").words(up, " y");
    cases.push(upper_strong);

    for (id, word, all) in
        [("smallcaps", "Smallcaps", false), ("smallcaps-all", "Test 012", true)]
    {
        let mut c = Case::new(id);
        let styles = smallcaps(c.root, all);
        c.words(styles, word);
        cases.push(c);
    }

    check(cases);
}

#[test]
fn smart_quotes() {
    let mut cases: Vec<_> = [
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
    ]
    .into_iter()
    .map(Case::plain)
    .collect();

    // `\'` and `\"` are plain quotes.
    let mut escape = Case::new("smartquote-escape");
    let root = escape.root;
    escape
        .words(root, "The 5")
        .text(root, "'", "\\'")
        .words(root, "11")
        .text(root, "\"", "\\\"")
        .words(root, " ")
        .quote(root, "'")
        .words(root, "quick")
        .text(root, "'", "\\'")
        .words(root, " brown fox jumps over the ")
        .text(root, "\"", "\\\"")
        .words(root, "lazy")
        .quote(root, "'")
        .words(root, " dog")
        .text(root, "'", "\\'")
        .words(root, "s ear.");
    cases.push(escape);

    // Embedding characters don't count as the text before a quote.
    let mut embedding = Case::new("smartquote-with-embedding-chars");
    let root = embedding.root;
    embedding
        .quote(root, "\"")
        .text(root, "\u{202A}", "\"\\u{202A}\"")
        .words(root, "bonjour")
        .text(root, "\u{202C}", "\"\\u{202C}\"")
        .quote(root, "\"");
    cases.push(embedding);

    check(cases);
}

#[test]
fn decorations() {
    let mut cases = vec![];
    let red = rgb(0xFF4136FF);

    for (id, words) in
        [("underline-basic", "Further below."), ("underline-descenders", "gyp jq")]
    {
        let mut c = Case::new(id);
        let u = underline(c.root, UnderlineElem::new(body()));
        c.words(u, words);
        cases.push(c);
    }

    let mut offset = Case::new("underline-offset");
    let elem = UnderlineElem::new(body()).with_offset(Smart::Custom(pt(5.0)));
    let u = underline(offset.root, elem);
    offset.words(u, "Further below.");
    cases.push(offset);

    let mut struck = Case::new("strike-basic");
    let s = strike(struck.root, StrikeElem::new(body()));
    struck.words(s, "Statements dreamt up");
    cases.push(struck);

    let mut no_evade = Case::new("underline-stroke-no-evade");
    let u = underline(
        no_evade.root,
        UnderlineElem::new(body())
            .with_stroke(Smart::Custom(stroke(Some(red.clone()), None)))
            .with_evade(false),
    );
    no_evade.words(u, "Critical information");
    cases.push(no_evade);

    let mut inherits = Case::new("underline-inherits-fill");
    let f = text(inherits.root, TextElem::fill, red.clone());
    let u = underline(f, UnderlineElem::new(body()));
    inherits.words(u, "Change with the wind.");
    cases.push(inherits);

    let mut both = Case::new("overline-underline");
    let o = overline(both.root, OverlineElem::new(body()));
    let u = underline(o, UnderlineElem::new(body()));
    both.words(u, "Running amongst the wolves.");
    cases.push(both);

    for (id, words, paint) in [
        ("strike-redact", "in secret", None),
        ("strike-transparent", "redacted", Some(rgb(0xABCDEF88))),
    ] {
        let mut c = Case::new(id);
        let styles = strike(
            c.root,
            StrikeElem::new(body())
                .with_stroke(Smart::Custom(stroke(paint, Some(pt(10.0)))))
                .with_extent(em(0.05)),
        );
        c.words(styles, words);
        cases.push(c);
    }

    let mut folding = Case::new("underline-stroke-folding");
    let u = underline(
        folding.root,
        UnderlineElem::new(body())
            .with_stroke(Smart::Custom(stroke(None, Some(pt(2.0)))))
            .with_offset(Smart::Custom(pt(2.0))),
    );
    let f = text(u, TextElem::fill, red.clone());
    folding.words(f, "DANGER!");
    cases.push(folding);

    let round = Stroke {
        cap: Smart::Custom(LineCap::Round),
        ..stroke(Some(red.clone()), Some(em(0.5)))
    };
    let mut under_bg = Case::new("underline-background");
    let u = underline(
        under_bg.root,
        UnderlineElem::new(body())
            .with_background(true)
            .with_stroke(Smart::Custom(round.clone())),
    );
    under_bg.words(u, "This is in the background");
    cases.push(under_bg);

    let mut over_bg = Case::new("overline-background");
    let o = overline(
        over_bg.root,
        OverlineElem::new(body())
            .with_background(true)
            .with_stroke(Smart::Custom(round)),
    );
    over_bg.words(o, "This is in the background");
    cases.push(over_bg);

    let mut strike_bg = Case::new("strike-background");
    let s = strike(
        strike_bg.root,
        StrikeElem::new(body())
            .with_background(true)
            .with_stroke(Smart::Custom(stroke(Some(red.clone()), Some(pt(5.0))))),
    );
    strike_bg.words(s, "This is in the background");
    cases.push(strike_bg);

    let mut default = Case::new("highlight-default");
    let root = default.root;
    let h = highlight(root, HighlightElem::new(body()));
    default
        .words(root, "This is the built-in ")
        .words(h, "highlight with default color")
        .words(root, ".");
    cases.push(default);

    let mut fill = Case::new("highlight-fill");
    let h =
        highlight(fill.root, HighlightElem::new(body()).with_fill(Some(rgb(0xCCF5D6FF))));
    fill.words(h, "to highlight");
    cases.push(fill);

    let mut bounds = Case::new("highlight-bounds");
    let root = bounds.root;
    for (i, word) in ["ace", "base", "super", "phone "].into_iter().enumerate() {
        if i > 0 {
            bounds.words(root, ", ");
        }
        let h = highlight(root, HighlightElem::new(body()));
        bounds.words(h, word);
        if word == "phone " {
            bounds.text(h, "∫", "sym.integral");
        }
    }
    cases.push(bounds);

    let mut edges = Case::new("highlight-edges");
    let root = edges.root;
    for (i, word) in ["ace", "phone "].into_iter().enumerate() {
        if i > 0 {
            edges.words(root, ", ");
        }
        let elem = HighlightElem::new(body())
            .with_top_edge(TopEdge::Metric(TopEdgeMetric::XHeight))
            .with_bottom_edge(BottomEdge::Metric(BottomEdgeMetric::Baseline));
        let h = highlight(root, elem);
        edges.words(h, word);
        if word == "phone " {
            edges.text(h, "∫", "sym.integral");
        }
    }
    cases.push(edges);

    let mut edge_bounds = Case::new("highlight-edges-bounds");
    let elem = HighlightElem::new(body())
        .with_top_edge(TopEdge::Metric(TopEdgeMetric::Bounds))
        .with_bottom_edge(BottomEdge::Metric(BottomEdgeMetric::Bounds));
    let h = highlight(edge_bounds.root, elem);
    edge_bounds.words(h, "abc ").text(h, "∫", "sym.integral");
    cases.push(edge_bounds);

    let radius = Corners::splat(Some(Rel::from(pt(3.0))));
    let mut rounded = Case::new("highlight-radius");
    let h = highlight(rounded.root, HighlightElem::new(body()).with_radius(radius));
    rounded.words(h, "abc");
    cases.push(rounded);

    let blue = rgb(0x0074D9FF);
    let mut stroked = Case::new("highlight-stroke");
    let elem = HighlightElem::new(body())
        .with_stroke(Sides::splat(Some(Some(stroke(Some(blue.clone()), Some(pt(2.0)))))));
    let h = highlight(stroked.root, elem);
    stroked.words(h, "abc");
    cases.push(stroked);

    let mut sides = Case::new("highlight-stroke-sides");
    let side = |paint: Paint| Some(Some(stroke(Some(paint), None)));
    let elem = HighlightElem::new(body()).with_stroke(Sides::new(
        side(red.clone()),
        side(blue),
        side(rgb(0xFF851BFF)),
        side(rgb(0x2ECC40FF)),
    ));
    let h = highlight(sides.root, elem);
    sides.words(h, "abc");
    cases.push(sides);

    let mut stroke_radius = Case::new("highlight-stroke-radius");
    let elem = HighlightElem::new(body())
        .with_stroke(Sides::splat(Some(Some(stroke(None, Some(pt(1.0)))))))
        .with_radius(radius);
    let h = highlight(stroke_radius.root, elem);
    stroke_radius.words(h, "Lorem ipsum dolor");
    cases.push(stroke_radius);

    check(cases);
}

#[test]
fn symbols() {
    let mut arrow = Case::new("sym-arrow");
    let root = arrow.root;
    arrow
        .words(root, "Flow ")
        .text(root, "→", "sym.arrow.r")
        .words(root, " target");

    let mut modifiers = Case::new("sym-modifiers");
    let root = modifiers.root;
    modifiers
        .text(root, "⇔", "sym.arrow.l.r.double")
        .words(root, " ")
        .text(root, "±", "sym.plus.minus");

    check([arrow, modifiers]);
}

// ---------------------------------------------------------------------------------------------
// Invariants

/// The text of each text item in a frame, in order.
fn item_texts(frame: &Frame) -> Vec<String> {
    frame
        .items()
        .filter_map(|(_, item)| match item {
            FrameItem::Text(text) => Some(text.text.to_string()),
            _ => None,
        })
        .collect()
}

/// Each glyph's cluster text and source range, in visual order.
fn glyph_sources(frame: &Frame) -> Vec<(String, Range<usize>)> {
    frame
        .items()
        .filter_map(|(_, item)| match item {
            FrameItem::Text(text) => Some(text),
            _ => None,
        })
        .flat_map(|text| {
            text.glyphs
                .iter()
                .map(|glyph| (text.text[glyph.range()].to_string(), glyph.source.clone()))
        })
        .collect()
}

#[test]
fn style_chains_split_segments_by_identity() {
    // `*a**b*`: two strong elements give two chains with equal styles, which shape apart.
    let mut apart = Case::new("strong-segments");
    let a = strong(apart.root, StrongElem::new(body()));
    let b = strong(apart.root, StrongElem::new(body()));
    apart.words(a, "a").words(b, "b");
    assert_eq!(item_texts(&apart.layout().unwrap()), ["a", "b"]);

    // Children under one chain shape together.
    let mut together = Case::new("strong-segments");
    let s = strong(together.root, StrongElem::new(body()));
    together.words(s, "a").words(s, "b");
    assert_eq!(item_texts(&together.layout().unwrap()), ["ab"]);
}

#[test]
fn glyphs_map_to_their_source() {
    let sources = |case: &Case| glyph_sources(&case.layout().unwrap());
    let owned = |expected: &[(&str, Range<usize>)]| {
        expected
            .iter()
            .map(|(text, range)| (text.to_string(), range.clone()))
            .collect::<Vec<_>>()
    };

    // Escapes and shorthands map to their whole node, verbatim text byte for byte.
    let mut escapes = Case::custom("\\#a -- \\u{41}");
    let root = escapes.root;
    escapes
        .text(root, "#", "\\#")
        .words(root, "a ")
        .text(root, "–", "--")
        .words(root, " ")
        .text(root, "A", "\\u{41}");
    assert_eq!(
        sources(&escapes),
        owned(&[
            ("#", 0..2),
            ("a", 2..3),
            (" ", 3..4),
            ("–", 4..6),
            (" ", 6..7),
            ("A", 7..13)
        ])
    );

    // A ligature maps to all of its characters.
    let mut ligature = Case::custom("office");
    let root = ligature.root;
    ligature.words(root, "office");
    assert_eq!(
        sources(&ligature),
        owned(&[("o", 0..1), ("ffi", 1..4), ("c", 4..5), ("e", 5..6)])
    );

    // A case change maps byte for byte while characters keep their lengths. `ß` becomes `SS`,
    // so its text maps to the whole node.
    let mut upper = Case::custom("#upper[ab] #upper[ß]");
    let root = upper.root;
    let up = case(root, TextCase::Upper);
    let up2 = case(root, TextCase::Upper);
    upper.words(up, "ab").words(root, " ").words(up2, "ß");
    assert_eq!(
        sources(&upper),
        owned(&[("A", 7..8), ("B", 8..9), (" ", 10..11), ("S", 18..20), ("S", 18..20)])
    );

    // A smart quote maps to its straight quote.
    let mut quotes = Case::custom("\"I'm\"");
    let root = quotes.root;
    quotes.markup(root, "\"I'm\"");
    assert_eq!(
        sources(&quotes),
        owned(&[("“", 0..1), ("I", 1..2), ("’", 2..3), ("m", 3..4), ("”", 4..5)])
    );
}

#[test]
fn families_end_with_the_fallback_tail() {
    let case = Case::custom("");
    let names =
        |styles| families(styles).map(|family| family.as_str()).collect::<Vec<_>>();
    assert_eq!(
        names(case.root),
        [
            "lato",
            "libertinus serif",
            "twitter color emoji",
            "noto color emoji",
            "apple color emoji",
            "segoe ui emoji",
        ]
    );
    let no_fallback = text(case.root, TextElem::fallback, false);
    assert_eq!(names(no_fallback), ["lato"]);
}

#[test]
fn unknown_font_families_warn() {
    let world = fixtures::shared();
    let mut sink = Sink::new();
    let mut engine = Engine { world, sink: &mut sink };
    let list = FontList(
        ["Lato", "Nonexistent", "DejaVu Sans Mono"]
            .map(FontFamily::new)
            .into(),
    );
    check_font_list(&mut engine, &Spanned::new(list, Span::detached()));
    let warnings: Vec<_> = sink
        .warnings()
        .iter()
        .map(|warning| warning.message.to_string())
        .collect();
    assert_eq!(warnings, ["unknown font family: nonexistent"]);
}

#[test]
fn a_label_is_one_line() {
    let mut case = Case::custom("a b");
    let root = case.root;
    case.words(root, "a");
    case.children.push((LinebreakElem::shared(), root));
    case.words(root, "b");
    let errors = case.layout().unwrap_err();
    assert_eq!(errors[0].message, "a label must be a single line");

    // A break at the end leaves one line.
    let mut trailing = Case::custom("a");
    let root = trailing.root;
    trailing.words(root, "a");
    trailing.children.push((LinebreakElem::shared(), root));
    assert_eq!(item_texts(&trailing.layout().unwrap()), ["a"]);
}
