//! Ported from crates/typst-library/src/text/case.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: no `lower` and `upper` functions; evaluation calls `case` for them.

use crate::typst_library::foundations::{Content, Str, cast, derive_cast};
use crate::typst_library::text::TextElem;

/// Change the case of text.
fn case(text: Caseable, case: Case) -> Caseable {
    match text {
        Caseable::Str(v) => Caseable::Str(case.apply(&v).into()),
        Caseable::Content(v) => Caseable::Content(v.set(TextElem::case, Some(case))),
    }
}

/// A value whose case can be changed.
pub enum Caseable {
    Str(Str),
    Content(Content),
}

cast! {
    Caseable,
    self => match self {
        Self::Str(v) => v.into_value(),
        Self::Content(v) => v.into_value(),
    },
    v: Str => Self::Str(v),
    v: Content => Self::Content(v),
}

/// A case transformation on text.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Case {
    /// Everything is lowercased.
    Lower,
    /// Everything is uppercased.
    Upper,
}

derive_cast!(Case { Lower, Upper });

impl Case {
    /// Apply the case to a string.
    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Lower => text.to_lowercase(),
            Self::Upper => text.to_uppercase(),
        }
    }
}
