//! Ported from crates/typst-library/src/foundations/ops.rs @ v0.15.1, modified for Avenger.
//!
//! Operations on values. Avenger starts with equality; the arithmetic operators arrive with
//! evaluation.

use crate::typst_utils::Numeric;

use crate::typst_library::foundations::Value;

/// Determine whether two values are equal.
pub fn equal(lhs: &Value, rhs: &Value) -> bool {
    use Value::*;
    match (lhs, rhs) {
        // Compare reflexively.
        (None, None) => true,
        (Auto, Auto) => true,
        (Bool(a), Bool(b)) => a == b,
        (Int(a), Int(b)) => a == b,
        (Float(a), Float(b)) => a == b,
        (Length(a), Length(b)) => a == b,
        (Angle(a), Angle(b)) => a == b,
        (Ratio(a), Ratio(b)) => a == b,
        (Relative(a), Relative(b)) => a == b,
        (Fraction(a), Fraction(b)) => a == b,
        (Color(a), Color(b)) => a == b,
        (Symbol(a), Symbol(b)) => a == b,
        (Str(a), Str(b)) => a == b,
        (Content(a), Content(b)) => a == b,
        (Array(a), Array(b)) => a == b,
        (Dict(a), Dict(b)) => a == b,
        (Dyn(a), Dyn(b)) => a == b,

        // Some technically different things should compare equal.
        (&Int(i), &Float(f)) | (&Float(f), &Int(i)) => i as f64 == f,
        (&Length(len), &Relative(rel)) | (&Relative(rel), &Length(len)) => {
            len == rel.abs && rel.rel.is_zero()
        }
        (&Ratio(rat), &Relative(rel)) | (&Relative(rel), &Ratio(rat)) => {
            rat == rel.rel && rel.abs.is_zero()
        }

        _ => false,
    }
}
