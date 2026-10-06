//! Ported from crates/typst-library/src/foundations/array.rs @ v0.15.1, modified for Avenger.
//!
//! The array type, its representation and its casts. Labels never call array methods, so the
//! scripting surface (`#[scope]` methods) is out of scope.

use std::fmt::{Debug, Formatter};
use std::ops::{Add, AddAssign};

use ecow::{EcoString, EcoVec, eco_format};
use smallvec::SmallVec;

use crate::typst_library::diag::{HintedStrResult, StrResult};
use crate::typst_library::foundations::{
    CastInfo, FromValue, IntoValue, Reflect, Repr, Value, repr, ty,
};

/// Create an array from a list of values.
// avenger: crate-private instead of `#[macro_export]`.
macro_rules! __array {
    ($value:expr; $count:expr) => {
        $crate::typst_library::foundations::Array::from($crate::typst_library::foundations::eco_vec![
            $crate::typst_library::foundations::IntoValue::into_value($value);
            $count
        ])
    };

    ($($value:expr),* $(,)?) => {
        $crate::typst_library::foundations::Array::from($crate::typst_library::foundations::eco_vec![$(
            $crate::typst_library::foundations::IntoValue::into_value($value)
        ),*])
    };
}

#[allow(unused_imports, reason = "used by the ported library as it is filled in")]
pub(crate) use __array as array;

/// A sequence of values.
///
/// You can construct an array by enclosing a comma-separated sequence of values
/// in parentheses. The values do not have to be of the same type.
///
/// You can access and update array items with the `.at()` method. Indices are
/// zero-based and negative indices wrap around to the end of the array. You can
/// iterate over an array using a @reference:scripting:loops[for loop]. Arrays
/// can be added together with the `+` operator,
/// @reference:scripting:blocks[joined together] and multiplied with integers.
///
/// *Note:* An array of length one needs a trailing comma, as in `{(1,)}`. This
/// is to disambiguate from a simple parenthesized expressions like `{(1 + 2) *
/// 3}`. An empty array is written as `{()}`.
///
/// = Example <example>
/// ```example
/// #let values = (1, 7, 4, -3, 2)
///
/// #values.at(0) \
/// #(values.at(0) = 3)
/// #values.at(-1) \
/// #values.find(calc.even) \
/// #values.filter(calc.odd) \
/// #values.map(calc.abs) \
/// #values.rev() \
/// #(1, (2, 3)).flatten() \
/// #(("A", "B", "C")
///     .join(", ", last: " and "))
/// ```
#[derive(Default, Clone, PartialEq)]
pub struct Array(EcoVec<Value>);

ty!(Array, name = "array", title = "Array", long = "array");

impl Array {
    /// Create a new, empty array.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new vec, with a known capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(EcoVec::with_capacity(capacity))
    }

    /// Return `true` if the length is 0.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Extract a slice of the whole array.
    pub fn as_slice(&self) -> &[Value] {
        self.0.as_slice()
    }

    /// Iterate over references to the contained values.
    pub fn iter(&self) -> std::slice::Iter<'_, Value> {
        self.0.iter()
    }

    // upstream: crates/typst-library/src/foundations/array.rs::Array::len @ v0.15.1
    /// The number of values in the array.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    // upstream: crates/typst-library/src/foundations/array.rs::Array::push @ v0.15.1
    /// Adds a value to the end of the array.
    pub fn push(&mut self, value: Value) {
        self.0.push(value);
    }

    /// Repeat this array `n` times.
    pub fn repeat(&self, n: usize) -> StrResult<Self> {
        let count = self
            .len()
            .checked_mul(n)
            .ok_or_else(|| format!("cannot repeat this array {n} times"))?;

        Ok(self.iter().cloned().cycle().take(count).collect())
    }

    // upstream: crates/typst-library/src/foundations/array.rs::Array::contains @ v0.15.1
    /// Whether the array contains the specified value.
    pub fn contains(&self, value: Value) -> bool {
        self.0.contains(&value)
    }
}

impl Debug for Array {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.debug_list().entries(&self.0).finish()
    }
}

impl Repr for Array {
    fn repr(&self) -> EcoString {
        let max = 40;
        let mut pieces: Vec<_> = self
            .iter()
            .take(max)
            .map(|value| eco_format!("{}", value.repr()))
            .collect();
        if self.len() > max {
            pieces.push(eco_format!(".. ({} items omitted)", self.len() - max));
        }
        repr::pretty_array_like(&pieces, self.len() == 1).into()
    }
}

impl Add for Array {
    type Output = Self;

    fn add(mut self, rhs: Array) -> Self::Output {
        self += rhs;
        self
    }
}

impl AddAssign for Array {
    fn add_assign(&mut self, rhs: Self) {
        self.0.extend(rhs.0);
    }
}

impl Extend<Value> for Array {
    fn extend<T: IntoIterator<Item = Value>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl FromIterator<Value> for Array {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Array {
    type Item = Value;
    type IntoIter = ecow::vec::IntoIter<Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Array {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<EcoVec<Value>> for Array {
    fn from(v: EcoVec<Value>) -> Self {
        Array(v)
    }
}

impl From<&[Value]> for Array {
    fn from(v: &[Value]) -> Self {
        Array(v.into())
    }
}

impl<T> Reflect for Vec<T> {
    fn input() -> CastInfo {
        Array::input()
    }

    fn output() -> CastInfo {
        Array::output()
    }

    fn castable(value: &Value) -> bool {
        Array::castable(value)
    }
}

impl<T: Reflect, const N: usize> Reflect for SmallVec<[T; N]> {
    fn input() -> CastInfo {
        Array::input()
    }

    fn output() -> CastInfo {
        Array::output()
    }

    fn castable(value: &Value) -> bool {
        Array::castable(value)
    }
}

impl<T: IntoValue + Copy> IntoValue for &[T] {
    fn into_value(self) -> Value {
        Value::Array(self.iter().copied().map(IntoValue::into_value).collect())
    }
}

impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        Value::Array(self.into_iter().map(IntoValue::into_value).collect())
    }
}

impl<T: IntoValue, const N: usize> IntoValue for SmallVec<[T; N]> {
    fn into_value(self) -> Value {
        Value::Array(self.into_iter().map(IntoValue::into_value).collect())
    }
}

impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: Value) -> HintedStrResult<Self> {
        value.cast::<Array>()?.into_iter().map(Value::cast).collect()
    }
}

impl<T: FromValue, const N: usize> FromValue for SmallVec<[T; N]> {
    fn from_value(value: Value) -> HintedStrResult<Self> {
        value.cast::<Array>()?.into_iter().map(Value::cast).collect()
    }
}
