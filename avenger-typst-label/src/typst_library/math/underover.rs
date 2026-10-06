//! Ported from crates/typst-library/src/math/underover.rs @ v0.15.1, modified for Avenger.

use crate::typst_library::foundations::{Content, elem};

elem! {
/// A horizontal line under content.
///
/// ```example
/// $ underline(1 + 2 + ... + 5) $
/// ```
#[elem(name = "underline", Mathy)]
pub struct UnderlineElem {
    /// The content above the line.
    #[required]
    pub body: Content,
}
}

elem! {
/// A horizontal line over content.
///
/// ```example
/// $ overline(1 + 2 + ... + 5) $
/// ```
#[elem(name = "overline", Mathy)]
pub struct OverlineElem {
    /// The content below the line.
    #[required]
    pub body: Content,
}
}

elem! {
/// A horizontal brace under content, with an optional annotation below.
///
/// ```example
/// $ underbrace(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "underbrace", Mathy)]
pub struct UnderbraceElem {
    /// The content above the brace.
    #[required]
    pub body: Content,

    /// The optional content below the brace.
    #[positional]
    pub annotation: Option<Content>,
}
}

elem! {
/// A horizontal brace over content, with an optional annotation above.
///
/// ```example
/// $ overbrace(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "overbrace", Mathy)]
pub struct OverbraceElem {
    /// The content below the brace.
    #[required]
    pub body: Content,

    /// The optional content above the brace.
    #[positional]
    pub annotation: Option<Content>,
}
}

elem! {
/// A horizontal bracket under content, with an optional annotation below.
///
/// ```example
/// $ underbracket(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "underbracket", Mathy)]
pub struct UnderbracketElem {
    /// The content above the bracket.
    #[required]
    pub body: Content,

    /// The optional content below the bracket.
    #[positional]
    pub annotation: Option<Content>,
}
}

elem! {
/// A horizontal bracket over content, with an optional annotation above.
///
/// ```example
/// $ overbracket(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "overbracket", Mathy)]
pub struct OverbracketElem {
    /// The content below the bracket.
    #[required]
    pub body: Content,

    /// The optional content above the bracket.
    #[positional]
    pub annotation: Option<Content>,
}
}

elem! {
/// A horizontal parenthesis under content, with an optional annotation below.
///
/// ```example
/// $ underparen(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "underparen", Mathy)]
pub struct UnderparenElem {
    /// The content above the parenthesis.
    #[required]
    pub body: Content,

    /// The optional content below the parenthesis.
    #[positional]
    pub annotation: Option<Content>,
}
}

elem! {
/// A horizontal parenthesis over content, with an optional annotation above.
///
/// ```example
/// $ overparen(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "overparen", Mathy)]
pub struct OverparenElem {
    /// The content below the parenthesis.
    #[required]
    pub body: Content,

    /// The optional content above the parenthesis.
    #[positional]
    pub annotation: Option<Content>,
}
}

elem! {
/// A horizontal tortoise shell bracket under content, with an optional
/// annotation below.
///
/// ```example
/// $ undershell(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "undershell", Mathy)]
pub struct UndershellElem {
    /// The content above the tortoise shell bracket.
    #[required]
    pub body: Content,

    /// The optional content below the tortoise shell bracket.
    #[positional]
    pub annotation: Option<Content>,
}
}

elem! {
/// A horizontal tortoise shell bracket over content, with an optional
/// annotation above.
///
/// ```example
/// $ overshell(0 + 1 + dots.c + n, n + 1 "numbers") $
/// ```
#[elem(name = "overshell", Mathy)]
pub struct OvershellElem {
    /// The content below the tortoise shell bracket.
    #[required]
    pub body: Content,

    /// The optional content above the tortoise shell bracket.
    #[positional]
    pub annotation: Option<Content>,
}
}
