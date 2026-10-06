//! Ported from crates/typst-library/src/math/ir/process.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's math has no line breaks, alignment points or tags, so processing a
//! group only spaces its items, and nothing is ignorant: there are no rows, table cells or
//! `MathBuffer`.

use smallvec::SmallVec;
use unicode_math_class::MathClass;

use super::item::MathItem;
use crate::typst_library::math::{MEDIUM, MathSize, THICK, THIN};

/// Processes items for grouping.
///
/// The `closing` parameter indicates whether a closing delimiter follows the
/// items.
// upstream: crates/typst-library/src/math/ir/process.rs::process_group @ v0.15.1
pub(crate) fn process_group<'a, I>(items: I, closing: bool) -> Vec<MathItem<'a>>
where
    I: IntoIterator<Item = MathItem<'a>>,
    I::IntoIter: ExactSizeIterator,
{
    preprocess(items, closing).into_vec()
}

/// Takes the given [`MathItem`]s and processes the spacing between them.
///
/// The `closing` parameter indicates whether a closing delimiter follows the
/// items.
// upstream: crates/typst-library/src/math/ir/process.rs::preprocess @ v0.15.1
fn preprocess<'a, I>(items: I, closing: bool) -> SmallVec<[MathItem<'a>; 8]>
where
    I: IntoIterator<Item = MathItem<'a>>,
    I::IntoIter: ExactSizeIterator,
{
    let iter = items.into_iter();
    let mut resolved = SmallVec::<[MathItem<'a>; 8]>::with_capacity(iter.len());

    let mut last: Option<usize> = None;
    let mut space: Option<MathItem> = None;

    for mut item in iter {
        match item {
            // Keep space only if supported by spaced items.
            MathItem::Space => {
                if last.is_some() {
                    space = Some(item);
                }
                continue;
            }

            // Explicit spacing disables automatic spacing.
            MathItem::Spacing(width, font_size, weak) => {
                last = None;
                space = None;

                if weak {
                    let Some(resolved_last) = resolved.last_mut() else {
                        continue;
                    };
                    if let MathItem::Spacing(prev_width, prev_font_size, true) =
                        resolved_last
                    {
                        if prev_width.at(*prev_font_size) < width.at(font_size) {
                            *prev_width = width;
                            *prev_font_size = font_size;
                        }
                        continue;
                    }
                }

                resolved.push(item);
                continue;
            }

            _ => {}
        }

        // Convert variable operators into binary operators if something
        // precedes them and they are not preceded by a operator or comparator.
        if item.class() == MathClass::Vary
            && let Some(prev) = last.map(|i| &resolved[i])
            && matches!(
                prev.class(),
                MathClass::Normal
                    | MathClass::Alphabetic
                    | MathClass::Closing
                    | MathClass::Fence
            )
        {
            item.set_class(MathClass::Binary);
        }

        // Insert spacing between the last and this item.
        if let Some(i) = last
            && let Some(s) = spacing(&mut resolved[i], space.take(), &mut item)
        {
            resolved.insert(i + 1, s);
        }

        last = Some(resolved.len());
        resolved.push(item);
    }

    // Apply closing punctuation spacing if applicable.
    if closing
        && let Some(item) = resolved.last_mut()
        && item.rclass() == MathClass::Punctuation
        && item.size().is_none_or(|s| s > MathSize::Script)
    {
        item.set_rspace(Some(THIN))
    } else if let Some(MathItem::Spacing(_, _, true)) = resolved.last() {
        resolved.pop();
    }

    resolved
}

/// Computes the spacing between two adjacent math items.
fn spacing<'a>(
    l: &mut MathItem,
    space: Option<MathItem<'a>>,
    r: &mut MathItem,
) -> Option<MathItem<'a>> {
    use MathClass::*;

    let script = |f: &MathItem| f.size().is_some_and(|s| s <= MathSize::Script);

    match (l.rclass(), r.lclass()) {
        // No spacing before punctuation; thin spacing after punctuation, unless
        // in script size.
        (_, Punctuation) => {}
        (Punctuation, _) if !script(l) => l.set_rspace(Some(THIN)),

        // No spacing after opening delimiters and before closing delimiters.
        (Opening, _) | (_, Closing) => {}

        // Thick spacing around relations, unless followed by a another relation
        // or in script size.
        (Relation, Relation) => {}
        (Relation, _) if !script(l) => l.set_rspace(Some(THICK)),
        (_, Relation) if !script(r) => r.set_lspace(Some(THICK)),

        // Medium spacing around binary operators, unless in script size.
        (Binary, _) if !script(l) => l.set_rspace(Some(MEDIUM)),
        (_, Binary) if !script(r) => r.set_lspace(Some(MEDIUM)),

        // Thin spacing around large operators, unless to the left of
        // an opening delimiter. TeXBook, p170
        (Large, Opening | Fence) => {}
        (Large, _) => l.set_rspace(Some(THIN)),

        (_, Large) => r.set_lspace(Some(THIN)),

        // Spacing around spaced frames.
        _ if (l.is_spaced() || r.is_spaced()) => return space,

        _ => {}
    };

    None
}
