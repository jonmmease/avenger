//! Ported from crates/typst-library/src/foundations/dict.rs @ v0.15.1, modified for Avenger.

use std::fmt::{Debug, Formatter};
use std::ops::{Add, AddAssign};
use std::sync::Arc;

use ecow::{EcoString, eco_format};
use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;
use typst_syntax::is_ident;

use crate::typst_library::diag::{Hint, HintedStrResult, StrResult};
use crate::typst_library::foundations::{Repr, Str, Value, repr, ty};

/// Create a new [`Dict`] from key-value pairs.
// avenger: crate-private, so not `#[macro_export]`.
#[doc(hidden)]
macro_rules! __dict {
    ($($key:expr => $value:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut map = $crate::typst_library::foundations::IndexMap::default();
        $(map.insert($key.into(), $crate::typst_library::foundations::IntoValue::into_value($value));)*
        $crate::typst_library::foundations::Dict::from(map)
    }};
}

pub(crate) use __dict as dict;

/// A map from string keys to values.
///
/// You can construct a dictionary by enclosing comma-separated `key: value`
/// pairs in parentheses. The values do not have to be of the same type. Since
/// empty parentheses already yield an empty array, you have to use the special
/// `(:)` syntax to create an empty dictionary.
///
/// A dictionary is conceptually similar to an @array[array], but it is indexed
/// by strings instead of integers. You can access and create dictionary entries
/// with the `.at()` method. If you know the key statically, you can
/// alternatively use @reference:scripting:fields[field access notation]
/// (`.key`) to access the value. To check whether a key is present in the
/// dictionary, use the `in` keyword.
///
/// You can iterate over the pairs in a dictionary using a
/// @reference:scripting:loops[for loop]. This will iterate in the order the
/// pairs were inserted / declared initially.
///
/// Dictionaries can be added with the `+` operator and
/// @reference:scripting:blocks[joined together]. They can also be
/// @arguments:spreading[spread] into a function call or another dictionary
/// #footnote[When spreading into a dictionary, if all items between the
/// parentheses are spread, you have to begin the container with `(:`, as in
/// `(: ..dict, ..other_dict)`. Otherwise the container is inferred to be an
/// array and an error is raised.]
/// with the `..spread` operator. In each case, if a key appears multiple times,
/// the last value will override the others.
///
/// = Example <example>
/// ```example
/// #let dict = (
///   name: "Typst",
///   born: 2019,
/// )
///
/// #dict.name \
/// #(dict.launch = 20)
/// #dict.len() \
/// #dict.keys() \
/// #dict.values() \
/// #dict.at("born") \
/// #dict.insert("city", "Berlin")
/// #("name" in dict)
/// ```
#[derive(Default, Clone, PartialEq)]
pub struct Dict(Arc<IndexMap<Str, Value, FxBuildHasher>>);

ty!(Dict, name = "dictionary", title = "Dictionary", long = "dictionary");

impl Dict {
    /// Create a new, empty dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the dictionary is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Borrow the value at the given key.
    pub fn get(&self, key: &str) -> StrResult<&Value> {
        self.0.get(key).ok_or_else(|| missing_key(key))
    }

    /// Mutably borrow the value the given `key` maps to.
    pub fn at_mut(&mut self, key: &str) -> HintedStrResult<&mut Value> {
        Arc::make_mut(&mut self.0)
            .get_mut(key)
            .ok_or_else(|| missing_key(key))
            .hint("use `insert` to add or update values")
    }

    /// Remove the value if the dictionary contains the given key.
    pub fn take(&mut self, key: &str) -> StrResult<Value> {
        Arc::make_mut(&mut self.0)
            .shift_remove(key)
            .ok_or_else(|| missing_key(key))
    }

    /// Whether the dictionary contains a specific key.
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Clear the dictionary.
    pub fn clear(&mut self) {
        if Arc::strong_count(&self.0) == 1 {
            Arc::make_mut(&mut self.0).clear();
        } else {
            *self = Self::new();
        }
    }

    /// Iterate over pairs of references to the contained keys and values.
    pub fn iter(&self) -> indexmap::map::Iter<'_, Str, Value> {
        self.0.iter()
    }

    // upstream: crates/typst-library/src/foundations/dict.rs::Dict::len @ v0.15.1
    /// The number of pairs in the dictionary.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    // upstream: crates/typst-library/src/foundations/dict.rs::Dict::insert @ v0.15.1
    /// Inserts a new pair into the dictionary. If the dictionary already
    /// contains this key, the value is updated.
    pub fn insert(&mut self, key: Str, value: Value) {
        Arc::make_mut(&mut self.0).insert(key, value);
    }

    /// Check if there is any remaining pair, and if so return an
    /// "unexpected key" error.
    pub fn finish(&self, expected: &[&str]) -> StrResult<()> {
        let mut iter = self.iter().peekable();
        if iter.peek().is_none() {
            return Ok(());
        }
        let unexpected: Vec<&str> = iter.map(|kv| kv.0.as_str()).collect();

        Err(Self::unexpected_keys(unexpected, Some(expected)))
    }

    // Return an "unexpected key" error string.
    pub fn unexpected_keys(
        unexpected: Vec<&str>,
        hint_expected: Option<&[&str]>,
    ) -> EcoString {
        let format_as_list = |arr: &[&str]| {
            repr::separated_list(
                &arr.iter().map(|s| eco_format!("\"{s}\"")).collect::<Vec<_>>(),
                "and",
            )
        };

        let mut msg = String::from(match unexpected.len() {
            1 => "unexpected key ",
            _ => "unexpected keys ",
        });

        msg.push_str(&format_as_list(&unexpected[..]));

        if let Some(expected) = hint_expected {
            msg.push_str(", valid keys are ");
            msg.push_str(&format_as_list(expected));
        }

        msg.into()
    }
}

impl Debug for Dict {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}

impl Repr for Dict {
    fn repr(&self) -> EcoString {
        if self.is_empty() {
            return "(:)".into();
        }

        let max = 40;
        let mut pieces: Vec<_> = self
            .iter()
            .take(max)
            .map(|(key, value)| {
                if is_ident(key) {
                    eco_format!("{key}: {}", value.repr())
                } else {
                    eco_format!("{}: {}", key.repr(), value.repr())
                }
            })
            .collect();

        if self.len() > max {
            pieces.push(eco_format!(".. ({} pairs omitted)", self.len() - max));
        }

        repr::pretty_array_like(&pieces, false).into()
    }
}

impl Add for Dict {
    type Output = Self;

    fn add(mut self, rhs: Dict) -> Self::Output {
        self += rhs;
        self
    }
}

impl AddAssign for Dict {
    fn add_assign(&mut self, rhs: Dict) {
        match Arc::try_unwrap(rhs.0) {
            Ok(map) => self.extend(map),
            Err(rc) => self.extend(rc.iter().map(|(k, v)| (k.clone(), v.clone()))),
        }
    }
}

impl Extend<(Str, Value)> for Dict {
    fn extend<T: IntoIterator<Item = (Str, Value)>>(&mut self, iter: T) {
        Arc::make_mut(&mut self.0).extend(iter);
    }
}

impl FromIterator<(Str, Value)> for Dict {
    fn from_iter<T: IntoIterator<Item = (Str, Value)>>(iter: T) -> Self {
        Self(Arc::new(iter.into_iter().collect()))
    }
}

impl IntoIterator for Dict {
    type Item = (Str, Value);
    type IntoIter = indexmap::map::IntoIter<Str, Value>;

    fn into_iter(self) -> Self::IntoIter {
        // avenger: std's `unwrap_or_clone` in place of `typst_utils::ArcExt::take`.
        Arc::unwrap_or_clone(self.0).into_iter()
    }
}

impl<'a> IntoIterator for &'a Dict {
    type Item = (&'a Str, &'a Value);
    type IntoIter = indexmap::map::Iter<'a, Str, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<IndexMap<Str, Value, FxBuildHasher>> for Dict {
    fn from(map: IndexMap<Str, Value, FxBuildHasher>) -> Self {
        Self(Arc::new(map))
    }
}

/// The missing key access error message.
#[cold]
fn missing_key(key: &str) -> EcoString {
    eco_format!("dictionary does not contain key {}", key.repr())
}
