//! Ported from crates/typst-library/src/foundations/str.rs @ v0.15.1, modified for Avenger.
//!
//! The string type and its representation. Labels never call string methods, so the scripting
//! surface (`#[scope]` methods, regexes, string patterns) is out of scope.

use std::borrow::{Borrow, Cow};
use std::fmt::{self, Debug, Display, Formatter};
use std::ops::{Add, AddAssign, Deref};

use ecow::{EcoString, eco_format};

use crate::typst_library::diag::{StrResult, bail};
use crate::typst_library::foundations::{Repr, Value, cast, ty};

/// A sequence of Unicode codepoints.
///
/// You can iterate over the grapheme clusters of the string using a
/// @reference:scripting:loops[for loop]. Grapheme clusters are basically
/// characters but keep together things that belong together, e.g. multiple
/// codepoints that together form a flag emoji. Strings can be added with the
/// `+` operator, @reference:scripting:blocks[joined together] and multiplied
/// with integers.
///
/// Typst provides utility methods for string manipulation. Many of these
/// methods (e.g., @str.split[`split`], @str.trim[`trim`] and
/// @str.replace[`replace`]) operate on _patterns:_ A pattern can be either a
/// string or a @regex[regular expression]. This makes the methods quite
/// versatile.
///
/// All lengths and indices are expressed in terms of UTF-8 bytes. Indices are
/// zero-based and negative indices wrap around to the end of the string.
///
/// You can convert a value to a string with this type's constructor.
#[derive(Default, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Str(EcoString);

/// Create a new [`Str`] from a format string.
macro_rules! __format_str {
    ($($tts:tt)*) => {{
        $crate::typst_library::foundations::Str::from(::ecow::eco_format!($($tts)*))
    }};
}

pub(crate) use __format_str as format_str;

ty!(Str, name = "str", title = "String", long = "string");

impl Str {
    /// Create a new, empty string.
    pub fn new() -> Self {
        Self(EcoString::new())
    }

    /// Repeat the string a number of times.
    pub fn repeat(&self, n: usize) -> StrResult<Self> {
        if self.0.len().checked_mul(n).is_none() {
            return Err(eco_format!("cannot repeat this string {n} times"));
        }
        Ok(Self(self.0.repeat(n)))
    }

    /// Return `true` if the length is 0.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A string slice containing the entire string.
    pub fn as_str(&self) -> &str {
        self
    }
}

impl Deref for Str {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl Debug for Str {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Debug::fmt(self.as_str(), f)
    }
}

impl Display for Str {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(self.as_str(), f)
    }
}

impl Repr for Str {
    fn repr(&self) -> EcoString {
        self.as_str().repr()
    }
}

impl Repr for EcoString {
    fn repr(&self) -> EcoString {
        self.as_str().repr()
    }
}

impl Repr for str {
    fn repr(&self) -> EcoString {
        let mut r = EcoString::with_capacity(self.len() + 2);
        r.push('"');
        for c in self.chars() {
            match c {
                '\0' => r.push_str(r"\u{0}"),
                '\'' => r.push('\''),
                '"' => r.push_str(r#"\""#),
                _ => r.extend(c.escape_debug()),
            }
        }
        r.push('"');
        r
    }
}

impl Repr for char {
    fn repr(&self) -> EcoString {
        EcoString::from(*self).repr()
    }
}

impl Add for Str {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        self += rhs;
        self
    }
}

impl AddAssign for Str {
    fn add_assign(&mut self, rhs: Self) {
        self.0.push_str(rhs.as_str());
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self
    }
}

impl Borrow<str> for Str {
    fn borrow(&self) -> &str {
        self
    }
}

impl From<char> for Str {
    fn from(c: char) -> Self {
        Self(c.into())
    }
}

impl From<&str> for Str {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

impl From<EcoString> for Str {
    fn from(s: EcoString) -> Self {
        Self(s)
    }
}

impl From<String> for Str {
    fn from(s: String) -> Self {
        Self(s.into())
    }
}

impl From<Cow<'_, str>> for Str {
    fn from(s: Cow<str>) -> Self {
        Self(s.into())
    }
}

impl FromIterator<char> for Str {
    fn from_iter<T: IntoIterator<Item = char>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl From<Str> for EcoString {
    fn from(str: Str) -> Self {
        str.0
    }
}

impl From<Str> for String {
    fn from(s: Str) -> Self {
        s.0.into()
    }
}

cast! {
    char,
    self => Value::Str(self.into()),
    string: Str => {
        let mut chars = string.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => c,
            _ => bail!("expected exactly one character"),
        }
    },
}

cast! {
    &str,
    self => Value::Str(self.into()),
}

cast! {
    EcoString,
    self => Value::Str(self.into()),
    v: Str => v.into(),
}

cast! {
    String,
    self => Value::Str(self.into()),
    v: Str => v.into(),
}
