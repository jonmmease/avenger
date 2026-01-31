//! Selector implementation for chart elements

use std::{borrow::Borrow, fmt};

use cssparser::ToCss;
use precomputed_hash::PrecomputedHash;
use selectors::parser::{NonTSPseudoClass, SelectorImpl};

/// Chart selector implementation
#[derive(Debug, Clone)]
pub struct ChartSelectors;

/// String wrapper that implements required traits
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ChartString(pub String);

impl From<&str> for ChartString {
    fn from(s: &str) -> Self {
        ChartString(s.to_string())
    }
}

impl From<String> for ChartString {
    fn from(s: String) -> Self {
        ChartString(s)
    }
}

impl ToCss for ChartString {
    fn to_css<W>(&self, dest: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        dest.write_str(&self.0)
    }
}

impl Borrow<str> for ChartString {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ChartString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PrecomputedHash for ChartString {
    fn precomputed_hash(&self) -> u32 {
        let mut hash = 0u32;
        for byte in self.0.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
        }
        hash
    }
}

/// Pseudo-classes (none supported for charts)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChartPseudoClass {}

impl NonTSPseudoClass for ChartPseudoClass {
    type Impl = ChartSelectors;

    fn is_active_or_hover(&self) -> bool {
        match *self {}
    }

    fn is_user_action_state(&self) -> bool {
        match *self {}
    }
}

impl ToCss for ChartPseudoClass {
    fn to_css<W>(&self, _dest: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        match *self {}
    }
}

/// Pseudo-elements (none supported for charts)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChartPseudoElement {}

impl selectors::parser::PseudoElement for ChartPseudoElement {
    type Impl = ChartSelectors;
}

impl ToCss for ChartPseudoElement {
    fn to_css<W>(&self, _dest: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        match *self {}
    }
}

impl SelectorImpl for ChartSelectors {
    type ExtraMatchingData<'a> = ();
    type AttrValue = ChartString;
    type Identifier = ChartString;
    type LocalName = ChartString;
    type NamespacePrefix = ChartString;
    type NamespaceUrl = ChartString;
    type BorrowedNamespaceUrl = str;
    type BorrowedLocalName = str;
    type NonTSPseudoClass = ChartPseudoClass;
    type PseudoElement = ChartPseudoElement;
}
