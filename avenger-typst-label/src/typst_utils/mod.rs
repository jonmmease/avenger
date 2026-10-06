//! Utilities from upstream `typst-utils`.
//!
//! The files in `upstream/` are generated from upstream by `tools/typst-sync`, like
//! `typst_syntax/upstream`. This module root is Avenger's. It declares them and keeps three
//! upstream macros (`crates/typst-utils/src/macros.rs`) as crate-private macros instead of
//! `#[macro_export]` ones, so they don't leak into the public API.

// upstream: crates/typst-utils/src/macros.rs::singleton @ v0.15.1
/// Create a lazy initialized, globally unique `'static` reference to a value.
macro_rules! singleton {
    ($ty:ty, $value:expr) => {{
        static VALUE: ::std::sync::LazyLock<$ty> = ::std::sync::LazyLock::new(|| $value);
        &*VALUE
    }};
}

// upstream: crates/typst-utils/src/macros.rs::sub_impl @ v0.15.1
/// Implement the `Sub` trait based on existing `Neg` and `Add` impls.
macro_rules! sub_impl {
    ($a:ident - $b:ident -> $c:ident) => {
        impl ::core::ops::Sub<$b> for $a {
            type Output = $c;

            fn sub(self, other: $b) -> $c {
                self + -other
            }
        }
    };
}

// upstream: crates/typst-utils/src/macros.rs::assign_impl @ v0.15.1
/// Implement an assign trait based on an existing non-assign trait.
macro_rules! assign_impl {
    ($a:ident += $b:ident) => {
        impl ::core::ops::AddAssign<$b> for $a {
            fn add_assign(&mut self, other: $b) {
                *self = *self + other;
            }
        }
    };

    ($a:ident -= $b:ident) => {
        impl ::core::ops::SubAssign<$b> for $a {
            fn sub_assign(&mut self, other: $b) {
                *self = *self - other;
            }
        }
    };

    ($a:ident *= $b:ident) => {
        impl ::core::ops::MulAssign<$b> for $a {
            fn mul_assign(&mut self, other: $b) {
                *self = *self * other;
            }
        }
    };

    ($a:ident /= $b:ident) => {
        impl ::core::ops::DivAssign<$b> for $a {
            fn div_assign(&mut self, other: $b) {
                *self = *self / other;
            }
        }
    };

    ($a:ident %= $b:ident) => {
        impl ::core::ops::RemAssign<$b> for $a {
            fn rem_assign(&mut self, other: $b) {
                *self = *self % other;
            }
        }
    };
}

pub(crate) use {assign_impl, singleton, sub_impl};

// Declared after the macros, which the generated files use through textual scope as upstream's
// `#[macro_use] mod macros` provides them.
#[rustfmt::skip]
#[path = "upstream/lib.rs"]
#[allow(dead_code, reason = "upstream typst-utils subset")]
#[expect(unused_imports, reason = "grouped upstream imports keep names of removed items")]
mod lib;
#[rustfmt::skip]
#[path = "upstream/round.rs"]
#[allow(dead_code, reason = "upstream typst-utils subset")]
mod round;
#[rustfmt::skip]
#[path = "upstream/scalar.rs"]
mod scalar;

pub use self::lib::*;
pub use self::round::round_with_precision;
pub use self::scalar::Scalar;
