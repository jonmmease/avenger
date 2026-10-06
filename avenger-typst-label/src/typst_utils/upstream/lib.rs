//! Utilities for Typst.

use std::fmt::{Debug, Display, Formatter};
use std::num::{NonZeroU32, NonZeroUsize};
use std::ops::{Add, Deref, DerefMut, Div, Mul, Neg, Sub};

use unicode_math_class::MathClass;

/// Turn a closure into a struct implementing [`Debug`].
pub fn debug<F>(f: F) -> impl Debug
where
    F: Fn(&mut Formatter) -> std::fmt::Result,
{
    struct Wrapper<F>(F);

    impl<F> Debug for Wrapper<F>
    where
        F: Fn(&mut Formatter) -> std::fmt::Result,
    {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            self.0(f)
        }
    }

    Wrapper(f)
}

/// An extra constant for [`NonZeroUsize`].
pub trait NonZeroExt {
    /// The number `1`.
    const ONE: Self;
}

impl NonZeroExt for NonZeroUsize {
    const ONE: Self = Self::new(1).unwrap();
}

impl NonZeroExt for NonZeroU32 {
    const ONE: Self = Self::new(1).unwrap();
}

/// Generic access to a structure's components.
pub trait Get<Index> {
    /// The structure's component type.
    type Component;

    /// Borrow the component for the specified index.
    fn get_ref(&self, index: Index) -> &Self::Component;

    /// Borrow the component for the specified index mutably.
    fn get_mut(&mut self, index: Index) -> &mut Self::Component;

    /// Convenience method for getting a copy of a component.
    fn get(self, index: Index) -> Self::Component
    where
        Self: Sized,
        Self::Component: Copy,
    {
        *self.get_ref(index)
    }

    /// Convenience method for setting a component.
    fn set(&mut self, index: Index, component: Self::Component) {
        *self.get_mut(index) = component;
    }

    /// Builder-style method for setting a component.
    fn with(mut self, index: Index, component: Self::Component) -> Self
    where
        Self: Sized,
    {
        self.set(index, component);
        self
    }
}

/// A numeric type.
pub trait Numeric:
    Sized
    + Debug
    + Copy
    + PartialEq
    + Neg<Output = Self>
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<f64, Output = Self>
    + Div<f64, Output = Self>
{
    /// The identity element for addition.
    fn zero() -> Self;

    /// Whether `self` is zero.
    fn is_zero(self) -> bool {
        self == Self::zero()
    }

    /// Whether `self` consists only of finite parts.
    fn is_finite(self) -> bool;
}

/// A marker trait for numeric lengths.
pub trait NumericLength: Numeric {}

/// Returns the default math class of a character in Typst, if it has one.
///
/// This is determined by the Unicode math class, with some manual overrides.
pub fn default_math_class(c: char) -> Option<MathClass> {
    match c {
        // Better spacing.
        // https://github.com/typst/typst/commit/2e039cb052fcb768027053cbf02ce396f6d7a6be
        ':' => Some(MathClass::Relation),

        // Better spacing when used alongside + PLUS SIGN.
        // https://github.com/typst/typst/pull/1726
        '⋯' | '⋱' | '⋰' | '⋮' => Some(MathClass::Normal),

        // Better spacing.
        // https://github.com/typst/typst/pull/1855
        '.' | '/' => Some(MathClass::Normal),

        // ⊥ UP TACK should not be a relation, contrary to ⟂ PERPENDICULAR.
        // https://github.com/typst/typst/pull/5714
        '\u{22A5}' => Some(MathClass::Normal),

        // Used as a binary connector in linear logic, where it is referred to
        // as "par".
        // https://github.com/typst/typst/issues/5764
        '⅋' => Some(MathClass::Binary),

        // Those overrides should become the default in the next revision of
        // MathClass.txt.
        // https://github.com/typst/typst/issues/5764#issuecomment-2632435247
        '⎰' | '⟅' => Some(MathClass::Opening),
        '⎱' | '⟆' => Some(MathClass::Closing),

        // Both ∨ and ⟑ are classified as Binary.
        // https://github.com/typst/typst/issues/5764
        '⟇' => Some(MathClass::Binary),

        // Arabic comma.
        // https://github.com/latex3/unicode-math/pull/633#issuecomment-2028936135
        '،' => Some(MathClass::Punctuation),

        c => unicode_math_class::class(c),
    }
}

/// Automatically calls a deferred function when the returned handle is dropped.
pub fn defer<T, F: FnOnce(&mut T)>(
    thing: &mut T,
    deferred: F,
) -> impl DerefMut<Target = T> {
    pub struct DeferHandle<'a, T, F: FnOnce(&mut T)> {
        thing: &'a mut T,
        deferred: Option<F>,
    }

    impl<'a, T, F: FnOnce(&mut T)> Drop for DeferHandle<'a, T, F> {
        fn drop(&mut self) {
            std::mem::take(&mut self.deferred).expect("deferred function")(self.thing);
        }
    }

    impl<T, F: FnOnce(&mut T)> std::ops::Deref for DeferHandle<'_, T, F> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            self.thing
        }
    }

    impl<T, F: FnOnce(&mut T)> std::ops::DerefMut for DeferHandle<'_, T, F> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.thing
        }
    }

    DeferHandle { thing, deferred: Some(deferred) }
}
