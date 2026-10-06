//! Ported from crates/typst-library/src/foundations/symbol.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: labels use codex's symbols and cannot define their own, so there is no `symbol`
//! constructor and no symbol variant cast. Symbols are not serialized.

use std::collections::BTreeSet;
use std::fmt::{self, Debug, Display, Formatter};
use std::sync::Arc;

use codex::ModifierSet;
use ecow::{EcoString, eco_format};

use crate::typst_library::diag::{StrResult, WarningSink, bail};
use crate::typst_library::foundations::{Content, NativeElement, Repr, elem, ty};

/// A Unicode symbol.
///
/// Typst defines common symbols so that they can easily be written with
/// standard keyboards. The symbols are defined in modules, from which they can
/// be accessed using @reference:scripting:fields[field access notation]:
///
/// - General symbols are defined in the @sym[`sym` module] and are accessible
///   without the `sym.` prefix in math mode.
/// - Emoji are defined in the @emoji[`emoji` module]
///
/// Moreover, you can define custom symbols with this type's constructor
/// function.
///
/// ```example
/// #sym.arrow.r \
/// #sym.gt.eq.not \
/// $gt.eq.not$ \
/// #emoji.face.halo
/// ```
///
/// Many symbols have different variants, which can be selected by appending the
/// modifiers with dot notation. The order of the modifiers is not relevant.
/// Visit the documentation pages of the symbol modules and click on a symbol to
/// see its available variants.
///
/// ```example
/// $arrow.l$ \
/// $arrow.r$ \
/// $arrow.t.quad$
/// ```
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Symbol(SymbolInner);

ty!(Symbol, name = "symbol", title = "Symbol", long = "symbol");

/// The internal representation of a [`Symbol`].
#[derive(Clone, Eq, PartialEq, Hash)]
enum SymbolInner {
    /// A native symbol that has no named variant.
    Single(&'static str),
    /// A native symbol with multiple named variants.
    Complex(&'static [Variant<&'static str>]),
    /// A symbol that has modifiers applied.
    Modified(Arc<Modified>),
}

/// A symbol with multiple named variants, where some modifiers may have been
/// applied. Also used for symbols defined at runtime by the user with no
/// modifier applied.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct Modified {
    /// The full list of variants.
    list: List,
    /// The modifiers that are already applied.
    modifiers: ModifierSet<EcoString>,
    /// Whether we already emitted a deprecation warning for the currently
    /// applied modifiers.
    deprecated: bool,
}

/// A symbol variant, consisting of a set of modifiers, the variant's value, and an
/// optional deprecation message.
type Variant<S> = (ModifierSet<S>, S, Option<S>);

/// A collection of symbols.
#[derive(Clone, Eq, PartialEq, Hash)]
enum List {
    Static(&'static [Variant<&'static str>]),
    Runtime(Box<[Variant<EcoString>]>),
}

impl Symbol {
    /// Create a new symbol from a single value.
    pub const fn single(value: &'static str) -> Self {
        Self(SymbolInner::Single(value))
    }

    /// Create a symbol with a static variant list.
    #[track_caller]
    pub const fn list(list: &'static [Variant<&'static str>]) -> Self {
        debug_assert!(!list.is_empty());
        Self(SymbolInner::Complex(list))
    }

    /// Create a symbol from a runtime char.
    pub fn runtime_char(c: char) -> Self {
        Self::runtime(Box::new([(ModifierSet::default(), c.into(), None)]))
    }

    /// Create a symbol with a runtime variant list.
    #[track_caller]
    pub fn runtime(list: Box<[Variant<EcoString>]>) -> Self {
        debug_assert!(!list.is_empty());
        Self(SymbolInner::Modified(Arc::new(Modified {
            list: List::Runtime(list),
            modifiers: ModifierSet::default(),
            deprecated: false,
        })))
    }

    /// Get the symbol's value.
    pub fn get(&self) -> &str {
        match &self.0 {
            SymbolInner::Single(value) => value,
            SymbolInner::Complex(_) => ModifierSet::<&'static str>::default()
                .best_match_in(self.variants().map(|(m, v, _)| (m, v)))
                .unwrap(),
            SymbolInner::Modified(arc) => arc
                .modifiers
                .best_match_in(self.variants().map(|(m, v, _)| (m, v)))
                .unwrap(),
        }
    }

    // avenger: no `func`; evaluation calls accents and delimiters by symbol value, without
    // function values.

    /// Apply a modifier to the symbol.
    pub fn modified(
        mut self,
        mut sink: impl WarningSink,
        modifier: &str,
    ) -> StrResult<Self> {
        if let SymbolInner::Complex(list) = self.0 {
            self.0 = SymbolInner::Modified(Arc::new(Modified {
                list: List::Static(list),
                modifiers: ModifierSet::default(),
                deprecated: false,
            }));
        }

        if let SymbolInner::Modified(arc) = &mut self.0 {
            let modified = Arc::make_mut(arc);
            modified.modifiers.insert_raw(modifier);
            if let Some(deprecation) = modified
                .modifiers
                .best_match_in(modified.list.variants().map(|(m, _, d)| (m, d)))
            {
                // If we already emitted a deprecation warning during a previous
                // modification of the symbol, do not emit another one.
                if !modified.deprecated
                    && let Some(message) = deprecation
                {
                    modified.deprecated = true;
                    sink.emit(message.into());
                }
                return Ok(self);
            }
        }

        bail!("unknown symbol modifier")
    }

    /// The characters that are covered by this symbol.
    pub fn variants(&self) -> impl Iterator<Item = Variant<&str>> {
        match &self.0 {
            SymbolInner::Single(value) => Variants::Single(std::iter::once(*value)),
            SymbolInner::Complex(list) => Variants::Static(list.iter()),
            SymbolInner::Modified(arc) => arc.list.variants(),
        }
    }

    /// Possible modifiers.
    pub fn modifiers(&self) -> impl Iterator<Item = &str> + '_ {
        let modifiers = match &self.0 {
            SymbolInner::Modified(arc) => arc.modifiers.as_deref(),
            _ => ModifierSet::default(),
        };
        self.variants()
            .flat_map(|(m, _, _)| m)
            .filter(|modifier| !modifier.is_empty() && !modifiers.contains(modifier))
            .collect::<BTreeSet<_>>()
            .into_iter()
    }
}

impl Display for Symbol {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str(self.get())
    }
}

impl Debug for SymbolInner {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Single(value) => Debug::fmt(value, f),
            Self::Complex(list) => list.fmt(f),
            Self::Modified(lists) => lists.fmt(f),
        }
    }
}

impl Debug for List {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Static(list) => list.fmt(f),
            Self::Runtime(list) => list.fmt(f),
        }
    }
}

impl Repr for Symbol {
    fn repr(&self) -> EcoString {
        match &self.0 {
            SymbolInner::Single(value) => eco_format!("symbol({})", value.repr()),
            SymbolInner::Complex(variants) => {
                eco_format!(
                    "symbol{}",
                    repr_variants(variants.iter().copied(), ModifierSet::default())
                )
            }
            SymbolInner::Modified(arc) => {
                let Modified { list, modifiers, .. } = arc.as_ref();
                if modifiers.is_empty() {
                    eco_format!(
                        "symbol{}",
                        repr_variants(list.variants(), ModifierSet::default())
                    )
                } else {
                    eco_format!(
                        "symbol{}",
                        repr_variants(list.variants(), modifiers.as_deref())
                    )
                }
            }
        }
    }
}

fn repr_variants<'a>(
    variants: impl Iterator<Item = Variant<&'a str>>,
    applied_modifiers: ModifierSet<&str>,
) -> String {
    crate::typst_library::foundations::repr::pretty_array_like(
        &variants
            .filter(|(modifiers, _, _)| {
                // Only keep variants that can still be accessed, i.e., variants
                // that contain all applied modifiers.
                applied_modifiers.iter().all(|am| modifiers.contains(am))
            })
            .map(|(modifiers, value, _)| {
                let trimmed_modifiers =
                    modifiers.into_iter().filter(|&m| !applied_modifiers.contains(m));
                if trimmed_modifiers.clone().all(|m| m.is_empty()) {
                    value.repr()
                } else {
                    let trimmed_modifiers =
                        trimmed_modifiers.collect::<Vec<_>>().join(".");
                    eco_format!("({}, {})", trimmed_modifiers.repr(), value.repr())
                }
            })
            .collect::<Vec<_>>(),
        false,
    )
}

impl List {
    /// The characters that are covered by this list.
    fn variants(&self) -> Variants<'_> {
        match self {
            List::Static(list) => Variants::Static(list.iter()),
            List::Runtime(list) => Variants::Runtime(list.iter()),
        }
    }
}

/// Iterator over variants.
enum Variants<'a> {
    Single(std::iter::Once<&'static str>),
    Static(std::slice::Iter<'static, Variant<&'static str>>),
    Runtime(std::slice::Iter<'a, Variant<EcoString>>),
}

impl<'a> Iterator for Variants<'a> {
    type Item = Variant<&'a str>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Single(iter) => Some((ModifierSet::default(), iter.next()?, None)),
            Self::Static(list) => list.next().copied(),
            Self::Runtime(list) => {
                list.next().map(|(m, s, d)| (m.as_deref(), s.as_str(), d.as_deref()))
            }
        }
    }
}

elem! {
/// A single character.
#[elem(name = "symbol", Repr, PlainText)]
pub struct SymbolElem {
    /// The symbol's value.
    #[required]
    pub text: EcoString, // This is called `text` for consistency with `TextElem`.
}
}

impl SymbolElem {
    /// Creates a new symbol element and directly packs it into type-erased
    /// content.
    pub fn packed(text: impl Into<EcoString>) -> Content {
        Self::new(text.into()).pack()
    }
}

// avenger: no `PlainText`; only outlines, bibliographies, footnotes and links read plain text.

impl Repr for SymbolElem {
    /// Use a custom repr that matches normal content.
    fn repr(&self) -> EcoString {
        eco_format!("[{}]", self.text)
    }
}
