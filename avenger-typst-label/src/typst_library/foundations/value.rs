//! Ported from crates/typst-library/src/foundations/value.rs @ v0.15.1, modified for Avenger.
//!
//! The values a label's static evaluator produces. Values that labels cannot express (gradients,
//! tilings, versions, bytes, labels, decimals, durations, styles, functions, arguments, types
//! and modules) are omitted, as are serialization and hashing.

use std::any::Any;
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use ecow::EcoString;

use crate::typst_library::diag::HintedStrResult;
use crate::typst_library::foundations::{
    Array, AutoValue, CastInfo, Content, Dict, FromValue, IntoValue, NativeType,
    NoneValue, Reflect, Repr, Str, Type, ops,
};
use crate::typst_library::layout::{Abs, Angle, Em, Fr, Length, Ratio, Rel};
use crate::typst_library::text::TextElem;
use crate::typst_library::visualize::Color;
use crate::typst_syntax::{Span, ast};

/// A computational value.
#[derive(Default, Clone)]
pub enum Value {
    /// The value that indicates the absence of a meaningful value.
    #[default]
    None,
    /// A value that indicates some smart default behaviour.
    Auto,
    /// A boolean: `true, false`.
    Bool(bool),
    /// An integer: `120`.
    Int(i64),
    /// A floating-point number: `1.2`, `10e-4`.
    Float(f64),
    /// A length: `12pt`, `3cm`, `1.5em`, `1em - 2pt`.
    Length(Length),
    /// An angle: `1.5rad`, `90deg`.
    Angle(Angle),
    /// A ratio: `50%`.
    Ratio(Ratio),
    /// A relative length, combination of a ratio and a length: `20% + 5cm`.
    Relative(Rel<Length>),
    /// A fraction: `1fr`.
    Fraction(Fr),
    /// A color value: `#f79143ff`.
    Color(Color),
    /// A string: `"string"`.
    Str(Str),
    /// A content value: `[*Hi* there]`.
    Content(Content),
    /// An array of values: `(1, "hi", 12cm)`.
    Array(Array),
    /// A dictionary value: `(a: 1, b: "hi")`.
    Dict(Dict),
    /// A dynamic value.
    Dyn(Dynamic),
}

impl Value {
    /// Create a new dynamic value.
    pub fn dynamic<T>(any: T) -> Self
    where
        T: Debug + Repr + NativeType + PartialEq + Sync + Send + 'static,
    {
        Self::Dyn(Dynamic::new(any))
    }

    /// Create a numeric value from a number with a unit.
    pub fn numeric(pair: (f64, ast::Unit)) -> Self {
        let (v, unit) = pair;
        match unit {
            ast::Unit::Pt => Abs::pt(v).into_value(),
            ast::Unit::Mm => Abs::mm(v).into_value(),
            ast::Unit::Cm => Abs::cm(v).into_value(),
            ast::Unit::In => Abs::inches(v).into_value(),
            ast::Unit::Rad => Angle::rad(v).into_value(),
            ast::Unit::Deg => Angle::deg(v).into_value(),
            ast::Unit::Em => Em::new(v).into_value(),
            ast::Unit::Fr => Fr::new(v).into_value(),
            ast::Unit::Percent => Ratio::new(v / 100.0).into_value(),
        }
    }

    /// The type of this value.
    pub fn ty(&self) -> Type {
        match self {
            Self::None => Type::of::<NoneValue>(),
            Self::Auto => Type::of::<AutoValue>(),
            Self::Bool(_) => Type::of::<bool>(),
            Self::Int(_) => Type::of::<i64>(),
            Self::Float(_) => Type::of::<f64>(),
            Self::Length(_) => Type::of::<Length>(),
            Self::Angle(_) => Type::of::<Angle>(),
            Self::Ratio(_) => Type::of::<Ratio>(),
            Self::Relative(_) => Type::of::<Rel<Length>>(),
            Self::Fraction(_) => Type::of::<Fr>(),
            Self::Color(_) => Type::of::<Color>(),
            Self::Str(_) => Type::of::<Str>(),
            Self::Content(_) => Type::of::<Content>(),
            Self::Array(_) => Type::of::<Array>(),
            Self::Dict(_) => Type::of::<Dict>(),
            Self::Dyn(v) => v.ty(),
        }
    }

    /// Try to cast the value into a specific type.
    pub fn cast<T: FromValue>(self) -> HintedStrResult<T> {
        T::from_value(self)
    }

    // avenger: `field`, `scope`, `docs` and `display` arrive with evaluation, which owns the
    // values labels can access fields of and display.

    /// Attach a span to the value, if possible.
    pub fn spanned(self, span: Span) -> Self {
        match self {
            Value::Content(v) => Value::Content(v.spanned(span)),
            v => v,
        }
    }
}

impl Debug for Value {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::None => Debug::fmt(&NoneValue, f),
            Self::Auto => Debug::fmt(&AutoValue, f),
            Self::Bool(v) => Debug::fmt(v, f),
            Self::Int(v) => Debug::fmt(v, f),
            Self::Float(v) => Debug::fmt(v, f),
            Self::Length(v) => Debug::fmt(v, f),
            Self::Angle(v) => Debug::fmt(v, f),
            Self::Ratio(v) => Debug::fmt(v, f),
            Self::Relative(v) => Debug::fmt(v, f),
            Self::Fraction(v) => Debug::fmt(v, f),
            Self::Color(v) => Debug::fmt(v, f),
            Self::Str(v) => Debug::fmt(v, f),
            Self::Content(v) => Debug::fmt(v, f),
            Self::Array(v) => Debug::fmt(v, f),
            Self::Dict(v) => Debug::fmt(v, f),
            Self::Dyn(v) => Debug::fmt(v, f),
        }
    }
}

impl Repr for Value {
    fn repr(&self) -> EcoString {
        match self {
            Self::None => NoneValue.repr(),
            Self::Auto => AutoValue.repr(),
            Self::Bool(v) => v.repr(),
            Self::Int(v) => v.repr(),
            Self::Float(v) => v.repr(),
            Self::Length(v) => v.repr(),
            Self::Angle(v) => v.repr(),
            Self::Ratio(v) => v.repr(),
            Self::Relative(v) => v.repr(),
            Self::Fraction(v) => v.repr(),
            Self::Color(v) => v.repr(),
            Self::Str(v) => v.repr(),
            Self::Content(v) => v.repr(),
            Self::Array(v) => v.repr(),
            Self::Dict(v) => v.repr(),
            Self::Dyn(v) => v.repr(),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        ops::equal(self, other)
    }
}

/// A value that is not part of the built-in enum.
#[derive(Clone)]
pub struct Dynamic(Arc<dyn Bounds>);

impl Dynamic {
    /// Create a new instance from any value that satisfies the required bounds.
    pub fn new<T>(any: T) -> Self
    where
        T: Debug + Repr + NativeType + PartialEq + Sync + Send + 'static,
    {
        Self(Arc::new(any))
    }

    /// Whether the wrapped type is `T`.
    pub fn is<T: 'static>(&self) -> bool {
        let inner: &dyn Bounds = &*self.0;
        (inner as &dyn Any).is::<T>()
    }

    /// Try to downcast to a reference to a specific type.
    pub fn downcast<T: 'static>(&self) -> Option<&T> {
        let inner: &dyn Bounds = &*self.0;
        (inner as &dyn Any).downcast_ref()
    }

    /// The name of the stored value's type.
    pub fn ty(&self) -> Type {
        self.0.dyn_ty()
    }
}

impl Debug for Dynamic {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Repr for Dynamic {
    fn repr(&self) -> EcoString {
        self.0.repr()
    }
}

impl PartialEq for Dynamic {
    fn eq(&self, other: &Self) -> bool {
        self.0.dyn_eq(other)
    }
}

trait Bounds: Debug + Repr + Any + Sync + Send + 'static {
    fn dyn_eq(&self, other: &Dynamic) -> bool;
    fn dyn_ty(&self) -> Type;
}

impl<T> Bounds for T
where
    T: Debug + Repr + NativeType + PartialEq + Sync + Send + 'static,
{
    fn dyn_eq(&self, other: &Dynamic) -> bool {
        let Some(other) = other.downcast::<Self>() else {
            return false;
        };
        self == other
    }

    fn dyn_ty(&self) -> Type {
        Type::of::<T>()
    }
}

/// Implements traits for primitives (Value enum variants).
macro_rules! primitive {
    (
        $ty:ty: $name:literal, $variant:ident
        $(, $other:ident$(($binding:ident))? => $out:expr)*
    ) => {
        impl Reflect for $ty {
            fn input() -> CastInfo {
                CastInfo::Type(Type::of::<Self>())
            }

            fn output() -> CastInfo {
                CastInfo::Type(Type::of::<Self>())
            }

            fn castable(value: &Value) -> bool {
                matches!(value, Value::$variant(_)
                    $(|  primitive!(@$other $(($binding))?))*)
            }
        }

        impl IntoValue for $ty {
            fn into_value(self) -> Value {
                Value::$variant(self)
            }
        }

        impl FromValue for $ty {
            fn from_value(value: Value) -> HintedStrResult<Self> {
                match value {
                    Value::$variant(v) => Ok(v),
                    $(Value::$other$(($binding))? => Ok($out),)*
                    v => Err(<Self as Reflect>::error(&v)),
                }
            }
        }
    };

    (@$other:ident($binding:ident)) => { Value::$other(_) };
    (@$other:ident) => { Value::$other };
}

primitive! { bool: "boolean", Bool }
primitive! { i64: "integer", Int }
primitive! { f64: "float", Float, Int(v) => v as f64 }
primitive! { Length: "length", Length }
primitive! { Angle: "angle", Angle }
primitive! { Ratio: "ratio", Ratio }
primitive! { Rel<Length>:  "relative length",
    Relative,
    Length(v) => v.into(),
    Ratio(v) => v.into()
}
primitive! { Fr: "fraction", Fraction }
primitive! { Color: "color", Color }
primitive! { Str: "string", Str }
primitive! { Content: "content",
    Content,
    None => Content::empty(),
    Str(v) => TextElem::packed(v)
}
primitive! { Array: "array", Array }
primitive! { Dict: "dictionary", Dict }

impl<T: Reflect> Reflect for Arc<T> {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }

    fn error(found: &Value) -> crate::typst_library::diag::HintedString {
        T::error(found)
    }
}

impl<T: Clone + IntoValue> IntoValue for Arc<T> {
    fn into_value(self) -> Value {
        // avenger: std's `unwrap_or_clone` in place of `typst_utils::ArcExt::take`.
        Arc::unwrap_or_clone(self).into_value()
    }
}

impl<T: FromValue> FromValue for Arc<T> {
    fn from_value(value: Value) -> HintedStrResult<Self> {
        match value {
            v if T::castable(&v) => Ok(Arc::new(T::from_value(v)?)),
            _ => Err(Self::error(&value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typst_library::foundations::{array, dict};

    #[track_caller]
    fn test(value: impl IntoValue, exp: &str) {
        assert_eq!(value.into_value().repr(), exp);
    }

    #[test]
    fn test_value_size() {
        assert!(std::mem::size_of::<Value>() <= 32);
    }

    #[test]
    fn test_value_debug() {
        // Primitives.
        test(Value::None, "none");
        test(Value::Auto, "auto");
        // avenger: no `type(none)` or `type(auto)`; types are not values here.
        test(false, "false");
        test(12i64, "12");
        test(3.24, "3.24");
        test(Abs::pt(5.5), "5.5pt");
        test(Angle::deg(90.0), "90deg");
        test(Ratio::one() / 2.0, "50%");
        test(Ratio::new(0.3) + Length::from(Abs::cm(2.0)), "30% + 56.69pt");
        test(Fr::one() * 7.55, "7.55fr");

        // Collections.
        test("hello", r#""hello""#);
        test("\n", r#""\n""#);
        test("\\", r#""\\""#);
        test("\"", r#""\"""#);
        test(array![], "()");
        test(array![Value::None], "(none,)");
        test(array![1, 2], "(1, 2)");
        test(dict![], "(:)");
        test(dict!["one" => 1], "(one: 1)");
        test(dict!["two" => false, "one" => 1], "(two: false, one: 1)");
    }
}
