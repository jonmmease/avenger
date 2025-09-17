//! Selector implementation for chart elements

use cssparser::ToCss;
use precomputed_hash::PrecomputedHash;
use selectors::parser::{NonTSPseudoClass, SelectorImpl};
use std::borrow::Borrow;
use std::fmt;

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

impl PrecomputedHash for ChartString {
    fn precomputed_hash(&self) -> u32 {
        let mut hash = 0u32;
        for byte in self.0.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
        }
        hash
    }
}

/// Pseudo-classes for chart elements
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChartPseudoClass {
    FirstChild,
    LastChild,
    NthChild(i32),
    Hover,
    Active,
}

impl NonTSPseudoClass for ChartPseudoClass {
    type Impl = ChartSelectors;

    fn is_active_or_hover(&self) -> bool {
        matches!(*self, ChartPseudoClass::Active | ChartPseudoClass::Hover)
    }

    fn is_user_action_state(&self) -> bool {
        matches!(*self, ChartPseudoClass::Active | ChartPseudoClass::Hover)
    }
}

impl ToCss for ChartPseudoClass {
    fn to_css<W>(&self, dest: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        match self {
            ChartPseudoClass::FirstChild => dest.write_str(":first-child"),
            ChartPseudoClass::LastChild => dest.write_str(":last-child"),
            ChartPseudoClass::NthChild(n) => write!(dest, ":nth-child({})", n),
            ChartPseudoClass::Hover => dest.write_str(":hover"),
            ChartPseudoClass::Active => dest.write_str(":active"),
        }
    }
}

/// Pseudo-elements (none for charts)
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
