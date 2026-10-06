//! Ported from crates/typst-library/src/layout/length.rs @ v0.15.1, modified for Avenger.

use std::cmp::Ordering;
use std::fmt::{self, Debug, Formatter};
use std::ops::{Add, Div, Mul, Neg};

use crate::typst_utils::{Numeric, NumericLength};
use ecow::{EcoString, eco_format};

use crate::typst_library::foundations::{Fold, Repr, Resolve, StyleChain, ty};
use crate::typst_library::layout::{Abs, Em};

/// A size or distance, possibly expressed with contextual units.
///
/// Typst supports the following length units:
///
/// - Points: `{72pt}`
/// - Millimeters: `{254mm}`
/// - Centimeters: `{2.54cm}`
/// - Inches: `{1in}`
/// - Relative to font size: `{2.5em}`
///
/// You can multiply lengths with and divide them by integers and floats.
///
/// = Example <example>
/// ```example
/// #rect(width: 20pt)
/// #rect(width: 2em)
/// #rect(width: 1in)
///
/// #(3em + 5pt).em \
/// #(20pt).em \
/// #(40em + 2pt).abs \
/// #(5em).abs
/// ```
///
/// = Fields <fields>
/// - `abs`: A length with just the absolute component of the current length
///   (that is, excluding the `em` component).
/// - `em`: The amount of `em` units in this length, as a @float[float].
#[derive(Default, Copy, Clone, Eq, PartialEq, Hash)]
pub struct Length {
    /// The absolute part.
    pub abs: Abs,
    /// The font-relative part.
    pub em: Em,
}

ty!(Length, name = "length", title = "Length", long = "length");

impl Length {
    /// The zero length.
    pub const fn zero() -> Self {
        Self { abs: Abs::zero(), em: Em::zero() }
    }

    /// Try to compute the absolute value of the length.
    pub fn try_abs(self) -> Option<Self> {
        (self.abs.is_zero() || self.em.is_zero())
            .then(|| Self { abs: self.abs.abs(), em: self.em.abs() })
    }

    /// Try to divide two lengths.
    pub fn try_div(self, other: Self) -> Option<f64> {
        if self.abs.is_zero() && other.abs.is_zero() {
            Some(self.em / other.em)
        } else if self.em.is_zero() && other.em.is_zero() {
            Some(self.abs / other.abs)
        } else {
            None
        }
    }

    /// Convert to an absolute length at the given font size.
    pub fn at(self, font_size: Abs) -> Abs {
        self.abs + self.em.at(font_size)
    }
}

impl Debug for Length {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match (self.abs.is_zero(), self.em.is_zero()) {
            (false, false) => write!(f, "{:?} + {:?}", self.abs, self.em),
            (true, false) => self.em.fmt(f),
            (_, true) => self.abs.fmt(f),
        }
    }
}

impl Repr for Length {
    fn repr(&self) -> EcoString {
        match (self.abs.is_zero(), self.em.is_zero()) {
            (false, false) => eco_format!("{} + {}", self.abs.repr(), self.em.repr()),
            (true, false) => self.em.repr(),
            (_, true) => self.abs.repr(),
        }
    }
}

impl NumericLength for Length {}

impl Numeric for Length {
    fn zero() -> Self {
        Self::zero()
    }

    fn is_finite(self) -> bool {
        self.abs.is_finite() && self.em.is_finite()
    }
}

impl PartialOrd for Length {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.em.is_zero() && other.em.is_zero() {
            self.abs.partial_cmp(&other.abs)
        } else if self.abs.is_zero() && other.abs.is_zero() {
            self.em.partial_cmp(&other.em)
        } else {
            None
        }
    }
}

impl From<Abs> for Length {
    fn from(abs: Abs) -> Self {
        Self { abs, em: Em::zero() }
    }
}

impl From<Em> for Length {
    fn from(em: Em) -> Self {
        Self { abs: Abs::zero(), em }
    }
}

impl Neg for Length {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self { abs: -self.abs, em: -self.em }
    }
}

impl Add for Length {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self { abs: self.abs + rhs.abs, em: self.em + rhs.em }
    }
}

crate::typst_utils::sub_impl!(Length - Length -> Length);

impl Mul<f64> for Length {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self { abs: self.abs * rhs, em: self.em * rhs }
    }
}

impl Mul<Length> for f64 {
    type Output = Length;

    fn mul(self, rhs: Length) -> Self::Output {
        rhs * self
    }
}

impl Div<f64> for Length {
    type Output = Self;

    fn div(self, rhs: f64) -> Self::Output {
        Self { abs: self.abs / rhs, em: self.em / rhs }
    }
}

crate::typst_utils::assign_impl!(Length += Length);
crate::typst_utils::assign_impl!(Length -= Length);
crate::typst_utils::assign_impl!(Length *= f64);
crate::typst_utils::assign_impl!(Length /= f64);

impl Resolve for Length {
    type Output = Abs;

    fn resolve(self, styles: StyleChain) -> Self::Output {
        self.abs + self.em.resolve(styles)
    }
}

impl Fold for Length {
    fn fold(self, _: Self) -> Self {
        self
    }
}
