//! Ported from crates/typst-library/src/foundations/none.rs @ v0.15.1, modified for Avenger.

use std::fmt::{self, Debug, Formatter};

use ecow::EcoString;

use crate::typst_library::diag::HintedStrResult;
use crate::typst_library::foundations::{
    CastInfo, FromValue, IntoValue, Reflect, Repr, Type, Value, ty,
};

/// A value that indicates the absence of any other value.
///
/// The none type has exactly one value: `{none}`.
///
/// When inserted into the document, it is not visible. This is also the value
/// that is produced by empty code blocks. It can be
/// @reference:scripting:blocks[joined] with any value, yielding the other
/// value.
///
/// = Example <example>
/// ```example
/// Not visible: #none
/// ```
#[derive(Default, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NoneValue;

ty!(NoneValue, name = "none", title = "None", long = "none");

impl Reflect for NoneValue {
    fn input() -> CastInfo {
        CastInfo::Type(Type::of::<Self>())
    }

    fn output() -> CastInfo {
        CastInfo::Type(Type::of::<Self>())
    }

    fn castable(value: &Value) -> bool {
        matches!(value, Value::None)
    }
}

impl IntoValue for NoneValue {
    fn into_value(self) -> Value {
        Value::None
    }
}

impl FromValue for NoneValue {
    fn from_value(value: Value) -> HintedStrResult<Self> {
        match value {
            Value::None => Ok(Self),
            _ => Err(Self::error(&value)),
        }
    }
}

impl Debug for NoneValue {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.pad("None")
    }
}

impl Repr for NoneValue {
    fn repr(&self) -> EcoString {
        "none".into()
    }
}

impl<T: Reflect> Reflect for Option<T> {
    fn input() -> CastInfo {
        T::input() + NoneValue::input()
    }

    fn output() -> CastInfo {
        T::output() + NoneValue::output()
    }

    fn castable(value: &Value) -> bool {
        NoneValue::castable(value) || T::castable(value)
    }
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        match self {
            Some(v) => v.into_value(),
            None => Value::None,
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: Value) -> HintedStrResult<Self> {
        match value {
            Value::None => Ok(None),
            v if T::castable(&v) => Ok(Some(T::from_value(v)?)),
            _ => Err(Self::error(&value)),
        }
    }
}

impl<T: Repr> Repr for Option<T> {
    fn repr(&self) -> EcoString {
        match self {
            Some(r) => r.repr(),
            None => NoneValue.repr(),
        }
    }
}
