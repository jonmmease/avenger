//! Ported from crates/typst-realize/src/lib.rs @ v0.15.1, modified for Avenger.
//!
//! Typst's realization subsystem.
//!
//! *Realization* is the process of recursively applying styling and, in
//! particular, show rules to produce well-known elements that can be processed
//! further.
//!
//! avenger: labels have no user-defined show rules, no introspection and no containers, so
//! realization applies the kind rules, the built-in show rules with element preparation, and
//! styling, then collapses spaces. It groups nothing: in paragraph and math realization,
//! upstream's textual grouping only serves regex show rules, and citations and lists don't
//! occur in labels.

use std::borrow::Cow;

use crate::typst_layout::rules::builtin_rule;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{
    Content, NativeElement, NativeShowRule, SequenceElem, ShowSet, StyleChain,
    StyledElem, Styles, SymbolElem, Synthesize,
};
use crate::typst_library::math::{EquationElem, Mathy};
use crate::typst_library::routines::{Arenas, Pair, RealizationKind};
use crate::typst_library::text::TextElem;

mod spaces;
#[cfg(test)]
mod tests;

use spaces::collapse_spaces;

/// Realize content into a flat list of well-known, styled items.
// avenger: no locator, since labels have no introspection.
pub fn realize<'a>(
    kind: RealizationKind,
    engine: &mut Engine,
    arenas: &'a Arenas<'a>,
    content: &'a Content,
    styles: StyleChain<'a>,
) -> SourceResult<Vec<Pair<'a>>> {
    let mut s = State { engine, arenas, sink: vec![], kind };

    visit(&mut s, content, styles)?;
    finish(&mut s);

    Ok(s.sink)
}

/// Mutable state for realization.
///
/// Sadly, we need that many lifetimes because &mut references are invariant and
/// it would force the lifetimes of e.g. engine and locator to be equal if they
/// shared a lifetime. We can get around it by enforcing the lifetimes on
/// `fn realize`, but that makes it less flexible on the call site, which isn't
/// worth it.
///
/// The only interesting lifetime is 'a, which is that of the content that comes
/// in and goes out. It's the same 'a as on `fn realize`.
// avenger: no locator, grouping rules, groupings, or flow state.
struct State<'a, 'x, 'y> {
    /// Defines what kind of realization we are performing.
    kind: RealizationKind,
    /// The engine.
    engine: &'x mut Engine<'y>,
    /// Temporary storage arenas for lifetime extension during realization.
    arenas: &'a Arenas<'a>,
    /// The output elements of well-known types.
    sink: Vec<Pair<'a>>,
}

/// What to do with an element when encountering it during realization.
struct Verdict {
    /// Whether the element is already prepared (i.e. things that should only
    /// happen once have happened).
    prepared: bool,
    /// A map of styles to apply to the element.
    map: Styles,
    /// An optional show rule transformation to apply to the element.
    step: Option<ShowStep>,
}

/// A show rule transformation to apply to the element.
// avenger: no user-defined recipes.
enum ShowStep {
    /// The built-in show rule.
    Builtin(NativeShowRule),
}

impl<'a> State<'a, '_, '_> {
    /// Lifetime-extends some content.
    fn store(&self, content: Content) -> &'a Content {
        self.arenas.content.alloc(content)
    }
}

/// Handles an arbitrary piece of content during realization.
// avenger: no tags, and no grouping or filter rules, which don't apply to paragraph and math
// realization in a label.
fn visit<'a>(
    s: &mut State<'a, '_, '_>,
    content: &'a Content,
    styles: StyleChain<'a>,
) -> SourceResult<()> {
    // Transformations for content based on the realization kind. Needs
    // to happen before show rules.
    if visit_kind_rules(s, content, styles)? {
        return Ok(());
    }

    // Apply show rules and preparation.
    if visit_show_rules(s, content, styles)? {
        return Ok(());
    }

    // Recurse into sequences. Styled elements and sequences can currently also
    // have labels, so this needs to happen before they are handled.
    if let Some(sequence) = content.to_packed::<SequenceElem>() {
        for elem in &sequence.children {
            visit(s, elem, styles)?;
        }
        return Ok(());
    }

    // Recurse into styled elements.
    if let Some(styled) = content.to_packed::<StyledElem>() {
        return visit_styled(s, &styled.child, Cow::Borrowed(&styled.styles), styles);
    }

    // No further transformations to apply, so we can finally just push it to
    // the output!
    s.sink.push((content, styles));

    Ok(())
}

// Handles transformations based on the realization kind.
// avenger: no regex show rules.
fn visit_kind_rules<'a>(
    s: &mut State<'a, '_, '_>,
    content: &'a Content,
    styles: StyleChain<'a>,
) -> SourceResult<bool> {
    if let RealizationKind::Math = s.kind {
        // Transparently recurse into equations nested in math, so that things
        // like this work:
        // ```
        // #let my = $pi$
        // $ my r^2 $
        // ```
        if let Some(elem) = content.to_packed::<EquationElem>() {
            visit(s, &elem.body, styles)?;
            return Ok(true);
        }
    } else {
        // Transparently wrap mathy content into equations.
        if content.can::<dyn Mathy>() && !content.is::<EquationElem>() {
            let eq = EquationElem::new(content.clone()).pack().spanned(content.span());
            visit(s, s.store(eq), styles)?;
            return Ok(true);
        }

        // Symbols in non-math content transparently convert to `TextElem` so we
        // don't have to handle them in non-math layout.
        if let Some(elem) = content.to_packed::<SymbolElem>() {
            let text = TextElem::packed(elem.text.clone()).spanned(elem.span());
            visit(s, s.store(text), styles)?;
            return Ok(true);
        }
    }

    Ok(false)
}

/// Tries to apply show rules to or prepare content. Returns `true` if the
/// element was handled.
fn visit_show_rules<'a>(
    s: &mut State<'a, '_, '_>,
    content: &'a Content,
    styles: StyleChain<'a>,
) -> SourceResult<bool> {
    // Determines whether and how to proceed with show rule application.
    let Some(Verdict { prepared, mut map, step }) = verdict(content) else {
        return Ok(false);
    };

    // Create a fresh copy that we can mutate.
    let mut output = Cow::Borrowed(content);

    // If the element isn't yet prepared (we're seeing it for the first time),
    // prepare it.
    if !prepared {
        prepare(s.engine, output.to_mut(), &mut map, styles)?;
    }

    // Apply a show rule step, if there is one.
    if let Some(step) = step {
        let chained = styles.chain(&map);
        let result = match step {
            // Apply a built-in show rule.
            ShowStep::Builtin(rule) => rule
                .apply(&output, s.engine, chained)
                .map(|content| content.spanned(output.span())),
        };

        // avenger: an error ends realization. Upstream delays it to the end of the
        // introspection loop, which labels don't have.
        output = Cow::Owned(result?);
    }

    // Lifetime-extend the realized content if necessary.
    let realized = match output {
        Cow::Borrowed(realized) => realized,
        Cow::Owned(realized) => s.store(realized),
    };

    // avenger: no tags, page-style lifting or show-rule depth: built-in rules don't recurse
    // into themselves.
    visit_styled(s, realized, Cow::Owned(map), styles)?;

    Ok(true)
}

/// Inspects an element and the current styles and determines how to proceed
/// with the styling.
// avenger: no pre-synthesis and no recipes, since labels have no user-defined show rules,
// and so no engine or styles.
fn verdict(elem: &Content) -> Option<Verdict> {
    let prepared = elem.is_prepared();
    let map = Styles::new();

    // The built-in show rule.
    let step = builtin_rule(elem.elem()).map(ShowStep::Builtin);

    // If there's no nothing to do, there is also no verdict.
    if step.is_none()
        && map.is_empty()
        && (prepared || !elem.can::<dyn ShowSet>() && !elem.can::<dyn Synthesize>())
    {
        return None;
    }

    Some(Verdict { prepared, map, step })
}

/// This is only executed the first time an element is visited.
// avenger: no locations or tags, since labels have no introspection, and no materialization,
// which only makes fields available to queries.
fn prepare(
    engine: &mut Engine,
    elem: &mut Content,
    map: &mut Styles,
    styles: StyleChain,
) -> SourceResult<()> {
    // Apply built-in show-set rules. User-defined show-set rules are already
    // considered in the map built while determining the verdict.
    if let Some(show_settable) = elem.with::<dyn ShowSet>() {
        map.apply(show_settable.show_set(styles));
    }

    // If necessary, generated "synthesized" fields (which are derived from
    // other fields or queries). Do this after show-set so that show-set styles
    // are respected.
    if let Some(synthesizable) = elem.with_mut::<dyn Synthesize>() {
        synthesizable.synthesize(engine, styles.chain(map))?;
    }

    // Ensure that this preparation only runs once by marking the element as
    // prepared.
    elem.mark_prepared();

    Ok(())
}

/// Handles a styled element.
// avenger: no document or page styles, which labels can't set, and no groupings to
// interrupt.
fn visit_styled<'a>(
    s: &mut State<'a, '_, '_>,
    content: &'a Content,
    local: Cow<'a, Styles>,
    outer: StyleChain<'a>,
) -> SourceResult<()> {
    // Nothing to do if the styles are actually empty.
    if local.is_empty() {
        return visit(s, content, outer);
    }

    // Lifetime-extend the styles if necessary.
    let outer: &'a StyleChain<'a> = s.arenas.chains.alloc(outer);
    let local = match local {
        Cow::Borrowed(map) => map,
        Cow::Owned(owned) => &*s.arenas.styles.alloc(owned),
    };

    visit(s, content, outer.chain(local))
}

/// Finishes all grouping.
// avenger: there are no groupings, so this only collapses spaces.
fn finish(s: &mut State) {
    // In paragraph and math realization, spaces are top-level.
    collapse_spaces(&mut s.sink, 0);
}
