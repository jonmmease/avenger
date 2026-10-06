//! Ported from crates/typst-library/src/foundations/int.rs @ v0.15.1, modified for Avenger.
//!
//! The type, its representation, and the casts of Rust's integer types. The scripting methods
//! are out of scope.

use ecow::{EcoString, eco_format};

use crate::typst_library::foundations::{Repr, Value, cast, ty};

ty!(i64, name = "int", title = "Integer", long = "integer");

impl Repr for i64 {
    fn repr(&self) -> EcoString {
        eco_format!("{self:?}")
    }
}

macro_rules! signed_int {
    ($($ty:ty)*) => {
        $(cast! {
            $ty,
            self => {
                #[allow(irrefutable_let_patterns)]
                if let Ok(int) = i64::try_from(self) {
                    Value::Int(int)
                } else {
                    // Some numbers (i128) are too large to be cast as i64
                    // In that case, we accept that there may be a
                    // precision loss, and use a floating point number
                    Value::Float(self as _)
                }
            },
            v: i64 => v.try_into().map_err(|_| "number too large")?,
        })*
    }
}

macro_rules! unsigned_int {
    ($($ty:ty)*) => {
        $(cast! {
            $ty,
            self => {
                #[allow(irrefutable_let_patterns)]
                if let Ok(int) = i64::try_from(self) {
                    Value::Int(int)
                } else {
                    // Some numbers (u64, u128) are too large to be cast as i64
                    // In that case, we accept that there may be a
                    // precision loss, and use a floating point number
                    Value::Float(self as _)
                }
            },
            v: i64 => v.try_into().map_err(|_| {
                if v < 0 {
                    "number must be at least zero"
                } else {
                    "number too large"
                }
            })?,
        })*
    }
}

signed_int! { i8 i16 i32 i128 isize }
unsigned_int! { u8 u16 u32 u64 u128 usize }
