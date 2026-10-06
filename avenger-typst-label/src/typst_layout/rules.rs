//! Ported from crates/typst-layout/src/rules.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the built-in show rules for the elements a label can contain, on the paged target,
//! which is a label's only target. The equation rule arrives with math layout.

use ecow::EcoVec;
use smallvec::smallvec;

use crate::typst_library::diag::SourceResult;
use crate::typst_library::foundations::{
    Content, Element, NativeElement, NativeShowRule, ShowFn, Smart, StyleChain,
};
use crate::typst_library::layout::{Em, Length};
use crate::typst_library::model::{EmphElem, StrongElem};
use crate::typst_library::text::{
    DecoLine, Decoration, HighlightElem, ItalicToggle, LinebreakElem, OverlineElem,
    RawElem, RawLine, ScriptKind, ShiftSettings, Smallcaps, SmallcapsElem, StrikeElem,
    SubElem, SuperElem, TextElem, TextSize, UnderlineElem, WeightDelta,
};
use crate::typst_library::visualize::Stroke;

/// The built-in show rule for an element on the paged target.
// avenger: in place of `register` and its `NativeRuleMap`: a lookup of the rules `register`
// adds, whose type-erasing wrappers need no `unsafe`.
pub fn builtin_rule(elem: Element) -> Option<NativeShowRule> {
    macro_rules! rules {
        ($($elem:ty => $rule:ident),* $(,)?) => {
            $(if elem == <$elem>::ELEM {
                return Some(NativeShowRule::new::<$elem>(|content, engine, styles| {
                    $rule(content.to_packed::<$elem>().unwrap(), engine, styles)
                }));
            })*
        };
    }

    // Model.
    rules! {
        StrongElem => STRONG_RULE,
        EmphElem => EMPH_RULE,
    }

    // Text.
    rules! {
        SubElem => SUB_RULE,
        SuperElem => SUPER_RULE,
        UnderlineElem => UNDERLINE_RULE,
        OverlineElem => OVERLINE_RULE,
        StrikeElem => STRIKE_RULE,
        HighlightElem => HIGHLIGHT_RULE,
        SmallcapsElem => SMALLCAPS_RULE,
        RawElem => RAW_RULE,
        RawLine => RAW_LINE_RULE,
    }

    None
}

const STRONG_RULE: ShowFn<StrongElem> = |elem, _, styles| {
    Ok(elem
        .body
        .clone()
        .set(TextElem::delta, WeightDelta(elem.delta.get(styles))))
};

const EMPH_RULE: ShowFn<EmphElem> =
    |elem, _, _| Ok(elem.body.clone().set(TextElem::emph, ItalicToggle(true)));

const SUB_RULE: ShowFn<SubElem> = |elem, _, styles| {
    show_script(
        styles,
        elem.body.clone(),
        elem.typographic.get(styles),
        elem.baseline.get(styles),
        elem.size.get(styles),
        ScriptKind::Sub,
    )
};

const SUPER_RULE: ShowFn<SuperElem> = |elem, _, styles| {
    show_script(
        styles,
        elem.body.clone(),
        elem.typographic.get(styles),
        elem.baseline.get(styles),
        elem.size.get(styles),
        ScriptKind::Super,
    )
};

fn show_script(
    styles: StyleChain,
    body: Content,
    typographic: bool,
    baseline: Smart<Length>,
    size: Smart<TextSize>,
    kind: ScriptKind,
) -> SourceResult<Content> {
    let font_size = styles.resolve(TextElem::size);
    Ok(body.set(
        TextElem::shift_settings,
        Some(ShiftSettings {
            typographic,
            shift: baseline.map(|l| -Em::from_length(l, font_size)),
            size: size.map(|t| Em::from_length(t.0, font_size)),
            kind,
        }),
    ))
}

const UNDERLINE_RULE: ShowFn<UnderlineElem> = |elem, _, styles| {
    Ok(elem.body.clone().set(
        TextElem::deco,
        smallvec![Decoration {
            line: DecoLine::Underline {
                stroke: elem.stroke.resolve(styles).unwrap_or_default(),
                offset: elem.offset.resolve(styles),
                evade: elem.evade.get(styles),
                background: elem.background.get(styles),
            },
            extent: elem.extent.resolve(styles),
        }],
    ))
};

const OVERLINE_RULE: ShowFn<OverlineElem> = |elem, _, styles| {
    Ok(elem.body.clone().set(
        TextElem::deco,
        smallvec![Decoration {
            line: DecoLine::Overline {
                stroke: elem.stroke.resolve(styles).unwrap_or_default(),
                offset: elem.offset.resolve(styles),
                evade: elem.evade.get(styles),
                background: elem.background.get(styles),
            },
            extent: elem.extent.resolve(styles),
        }],
    ))
};

const STRIKE_RULE: ShowFn<StrikeElem> = |elem, _, styles| {
    Ok(elem.body.clone().set(
        TextElem::deco,
        smallvec![Decoration {
            // Note that we do not support evade option for strikethrough.
            line: DecoLine::Strikethrough {
                stroke: elem.stroke.resolve(styles).unwrap_or_default(),
                offset: elem.offset.resolve(styles),
                background: elem.background.get(styles),
            },
            extent: elem.extent.resolve(styles),
        }],
    ))
};

const HIGHLIGHT_RULE: ShowFn<HighlightElem> = |elem, _, styles| {
    Ok(elem.body.clone().set(
        TextElem::deco,
        smallvec![Decoration {
            line: DecoLine::Highlight {
                fill: elem.fill.get_cloned(styles),
                stroke: elem
                    .stroke
                    .resolve(styles)
                    .unwrap_or_default()
                    .map(|stroke| stroke.map(Stroke::unwrap_or_default)),
                top_edge: elem.top_edge.get(styles),
                bottom_edge: elem.bottom_edge.get(styles),
                radius: elem.radius.resolve(styles).unwrap_or_default(),
            },
            extent: elem.extent.resolve(styles),
        }],
    ))
};

const SMALLCAPS_RULE: ShowFn<SmallcapsElem> = |elem, _, styles| {
    let sc = if elem.all.get(styles) { Smallcaps::All } else { Smallcaps::Minuscules };
    Ok(elem.body.clone().set(TextElem::smallcaps, Some(sc)))
};

// avenger: no block raw, which spans several lines and so can't occur in a label.
const RAW_RULE: ShowFn<RawElem> = |elem, _, _| {
    let lines = elem.lines.as_deref().unwrap_or_default();

    let mut seq = EcoVec::with_capacity((2 * lines.len()).saturating_sub(1));
    for (i, line) in lines.iter().enumerate() {
        if i != 0 {
            seq.push(LinebreakElem::shared().clone());
        }

        seq.push(line.clone().pack());
    }

    Ok(Content::sequence(seq))
};

const RAW_LINE_RULE: ShowFn<RawLine> = |elem, _, _| Ok(elem.body.clone());
