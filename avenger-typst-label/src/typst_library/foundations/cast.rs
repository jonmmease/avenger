//! Ported from crates/typst-library/src/foundations/cast.rs @ v0.15.1, modified for Avenger.
//!
//! The conversion traits and `CastInfo` diagnostics are upstream's. Upstream's `cast!` is a
//! procedural macro; Avenger's is the `macro_rules!` below, which generates the same `Reflect`,
//! `IntoValue` and `FromValue` impls for the same input syntax.

use std::fmt::Write;
use std::ops::Add;

use ecow::eco_format;
use smallvec::SmallVec;
use unicode_math_class::MathClass;

use crate::typst_library::diag::{
    At, HintedStrResult, HintedString, SourceResult, StrResult,
};
use crate::typst_library::foundations::{NativeElement, Packed, Repr, Type, Value, repr};
use crate::typst_syntax::{Span, Spanned};

/// Determine details of a type.
///
/// Type casting works as follows:
/// - [`Reflect for T`](Reflect) describes the possible Typst values for `T`
///   (for documentation and autocomplete).
/// - [`IntoValue for T`](IntoValue) is for conversion from `T -> Value`
///   (infallible)
/// - [`FromValue for T`](FromValue) is for conversion from `Value -> T`
///   (fallible).
///
/// We can't use `TryFrom<Value>` due to conflicting impls. We could use
/// `From<T> for Value`, but that inverses the impl and leads to tons of
/// `.into()` all over the place that become hard to decipher.
pub trait Reflect {
    /// Describe what can be cast into this value.
    fn input() -> CastInfo;

    /// Describe what this value can be cast into.
    fn output() -> CastInfo;

    /// Whether the given value can be converted to `T`.
    ///
    /// This exists for performance. The check could also be done through the
    /// [`CastInfo`], but it would be much more expensive (heap allocation +
    /// dynamic checks instead of optimized machine code for each type).
    fn castable(value: &Value) -> bool;

    /// Produce an error message for an unacceptable value type.
    ///
    /// ```ignore
    /// assert_eq!(
    ///   <i64 as Reflect>::error(&Value::None),
    ///   "expected integer, found none",
    /// );
    /// ```
    fn error(found: &Value) -> HintedString {
        Self::input().error(found)
    }
}

impl Reflect for Value {
    fn input() -> CastInfo {
        CastInfo::Any
    }

    fn output() -> CastInfo {
        CastInfo::Any
    }

    fn castable(_: &Value) -> bool {
        true
    }
}

impl<T: Reflect> Reflect for Spanned<T> {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }
}

impl<T: NativeElement + Reflect> Reflect for Packed<T> {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }
}

impl<T: Reflect> Reflect for StrResult<T> {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }
}

impl<T: Reflect> Reflect for HintedStrResult<T> {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }
}

impl<T: Reflect> Reflect for SourceResult<T> {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }
}

impl<T: Reflect> Reflect for &T {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }
}

impl<T: Reflect> Reflect for &mut T {
    fn input() -> CastInfo {
        T::input()
    }

    fn output() -> CastInfo {
        T::output()
    }

    fn castable(value: &Value) -> bool {
        T::castable(value)
    }
}

/// Cast a Rust type into a Typst [`Value`].
///
/// See also: [`Reflect`].
pub trait IntoValue {
    /// Cast this type into a value.
    fn into_value(self) -> Value;
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl<T: NativeElement + IntoValue> IntoValue for Packed<T> {
    fn into_value(self) -> Value {
        Value::Content(self.pack())
    }
}

impl<T: IntoValue> IntoValue for Spanned<T> {
    fn into_value(self) -> Value {
        self.v.into_value()
    }
}

/// Cast a Rust type or result into a [`SourceResult<Value>`].
///
/// Converts `T`, [`StrResult<T>`], or [`SourceResult<T>`] into
/// [`SourceResult<Value>`] by `Ok`-wrapping or adding span information.
pub trait IntoResult {
    /// Cast this type into a value.
    fn into_result(self, span: Span) -> SourceResult<Value>;
}

impl<T: IntoValue> IntoResult for T {
    fn into_result(self, _: Span) -> SourceResult<Value> {
        Ok(self.into_value())
    }
}

impl<T: IntoValue> IntoResult for StrResult<T> {
    fn into_result(self, span: Span) -> SourceResult<Value> {
        self.map(IntoValue::into_value).at(span)
    }
}

impl<T: IntoValue> IntoResult for HintedStrResult<T> {
    fn into_result(self, span: Span) -> SourceResult<Value> {
        self.map(IntoValue::into_value).at(span)
    }
}

impl<T: IntoValue> IntoResult for SourceResult<T> {
    fn into_result(self, _: Span) -> SourceResult<Value> {
        self.map(IntoValue::into_value)
    }
}

impl<T: IntoValue> IntoValue for fn() -> T {
    fn into_value(self) -> Value {
        self().into_value()
    }
}

/// Try to cast a Typst [`Value`] into a Rust type.
///
/// See also: [`Reflect`].
pub trait FromValue<V = Value>: Sized + Reflect {
    /// Try to cast the value into an instance of `Self`.
    fn from_value(value: V) -> HintedStrResult<Self>;
}

impl FromValue for Value {
    fn from_value(value: Value) -> HintedStrResult<Self> {
        Ok(value)
    }
}

impl<T: NativeElement + FromValue> FromValue for Packed<T> {
    fn from_value(mut value: Value) -> HintedStrResult<Self> {
        let mut span = Span::detached();
        if let Value::Content(content) = value {
            match content.into_packed::<T>() {
                Ok(packed) => return Ok(packed),
                Err(content) => {
                    span = content.span();
                    value = Value::Content(content)
                }
            }
        }
        let val = T::from_value(value)?;
        Ok(Packed::new(val).spanned(span))
    }
}

impl<T: FromValue> FromValue<Spanned<Value>> for T {
    fn from_value(value: Spanned<Value>) -> HintedStrResult<Self> {
        T::from_value(value.v)
    }
}

impl<T: FromValue> FromValue<Spanned<Value>> for Spanned<T> {
    fn from_value(value: Spanned<Value>) -> HintedStrResult<Self> {
        let span = value.span;
        T::from_value(value.v).map(|t| Spanned::new(t, span))
    }
}

/// Describes a possible value for a cast.
#[derive(Debug, Clone, PartialEq)]
pub enum CastInfo {
    /// Any value is okay.
    Any,
    /// A specific value, plus short documentation for that value.
    Value(Value, &'static str),
    /// Any value of a type.
    Type(Type),
    /// Multiple alternatives.
    Union(Vec<Self>),
}

impl CastInfo {
    /// Produce an error message describing what was expected and what was
    /// found.
    pub fn error(&self, found: &Value) -> HintedString {
        let mut matching_type = false;
        let mut parts = vec![];

        self.walk(|info| match info {
            CastInfo::Any => parts.push("anything".into()),
            CastInfo::Value(value, _) => {
                parts.push(value.repr());
                if value.ty() == found.ty() {
                    matching_type = true;
                }
            }
            CastInfo::Type(ty) => parts.push(eco_format!("{ty}")),
            CastInfo::Union(_) => {}
        });

        let mut msg = String::from("expected ");
        if parts.is_empty() {
            msg.push_str(" nothing");
        }

        msg.push_str(&repr::separated_list(&parts, "or"));

        if !matching_type {
            msg.push_str(", found ");
            write!(msg, "{}", found.ty()).unwrap();
        }

        let mut msg: HintedString = msg.into();

        // avenger: no label or decimal hints, since labels and decimals are not values here. The
        // remaining branch is one `if`, as clippy asks.
        if let Value::Int(i) = found
            && !matching_type
            && parts.iter().any(|p| p == "length")
        {
            msg.hint(eco_format!("a length needs a unit - did you mean {i}pt?"));
        }

        msg
    }

    /// Walk all contained non-union infos.
    pub fn walk<F>(&self, mut f: F)
    where
        F: FnMut(&Self),
    {
        fn inner<F>(info: &CastInfo, f: &mut F)
        where
            F: FnMut(&CastInfo),
        {
            if let CastInfo::Union(infos) = info {
                for child in infos {
                    inner(child, f);
                }
            } else {
                f(info);
            }
        }

        inner(self, &mut f)
    }
}

impl Add for CastInfo {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::Union(match (self, rhs) {
            (Self::Union(mut lhs), Self::Union(rhs)) => {
                for cast in rhs {
                    if !lhs.contains(&cast) {
                        lhs.push(cast);
                    }
                }
                lhs
            }
            (Self::Union(mut lhs), rhs) => {
                if !lhs.contains(&rhs) {
                    lhs.push(rhs);
                }
                lhs
            }
            (lhs, Self::Union(mut rhs)) => {
                if !rhs.contains(&lhs) {
                    rhs.insert(0, lhs);
                }
                rhs
            }
            (lhs, rhs) => vec![lhs, rhs],
        })
    }
}

/// A container for an argument.
pub trait Container {
    /// The contained type.
    type Inner;
}

impl<T> Container for Option<T> {
    type Inner = T;
}

impl<T> Container for Vec<T> {
    type Inner = T;
}

impl<T, const N: usize> Container for SmallVec<[T; N]> {
    type Inner = T;
}

/// An uninhabitable type.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Never {}

impl Reflect for Never {
    fn input() -> CastInfo {
        CastInfo::Union(vec![])
    }

    fn output() -> CastInfo {
        CastInfo::Union(vec![])
    }

    fn castable(_: &Value) -> bool {
        false
    }
}

impl IntoValue for Never {
    fn into_value(self) -> Value {
        match self {}
    }
}

impl FromValue for Never {
    fn from_value(value: Value) -> HintedStrResult<Self> {
        Err(Self::error(&value))
    }
}

// avenger: no `SyntaxMode` cast; labels have no `eval`.

cast! {
    MathClass,
    self => IntoValue::into_value(match self {
        MathClass::Normal => "normal",
        MathClass::Alphabetic => "alphabetic",
        MathClass::Binary => "binary",
        MathClass::Closing => "closing",
        MathClass::Diacritic => "diacritic",
        MathClass::Fence => "fence",
        MathClass::GlyphPart => "glyph-part",
        MathClass::Large => "large",
        MathClass::Opening => "opening",
        MathClass::Punctuation => "punctuation",
        MathClass::Relation => "relation",
        MathClass::Space => "space",
        MathClass::Unary => "unary",
        MathClass::Vary => "vary",
        MathClass::Special => "special",
    }),
    /// The default class for non-special things.
    "normal" => MathClass::Normal,
    /// Punctuation, e.g. a comma.
    "punctuation" => MathClass::Punctuation,
    /// An opening delimiter, e.g. `(`.
    "opening" => MathClass::Opening,
    /// A closing delimiter, e.g. `)`.
    "closing" => MathClass::Closing,
    /// A delimiter that is the same on both sides, e.g. `|`.
    "fence" => MathClass::Fence,
    /// A large operator like `sum`.
    ///
    /// If the body is a single glyph, this class vertically centers it on the
    /// math axis (where the fraction line sits) and stretches it vertically
    /// when in @math.display[`display`] style.
    "large" => MathClass::Large,
    /// A relation like `=` or `prec`.
    "relation" => MathClass::Relation,
    /// A unary operator like `not`.
    "unary" => MathClass::Unary,
    /// A binary operator like `times`.
    "binary" => MathClass::Binary,
    /// An operator that can be both unary or binary like `+`.
    "vary" => MathClass::Vary,
}

// avenger: no `Derived`; it serves raw syntax themes.

/// Implements [`Reflect`], [`IntoValue`] and [`FromValue`] for a type, as upstream's
/// `cast!` procedural macro does (`crates/typst-macros/src/cast.rs`).
///
/// ```ignore
/// cast! {
///     [type] Target,                  // `type`: also castable from `Value::Dyn`
///     self => into_value_expression,  // optional
///     "string" => from_string_expr,   // any number of alternatives,
///     [mut] binding: Type => from_type_expr,
/// }
/// ```
// avenger: `self` is matched as an identifier and reused as the generated method's receiver,
// since hygiene keeps a `self` written in the invocation from naming a macro-made parameter.
macro_rules! cast {
    (type $ty:ty $(,)?) => {
        $crate::typst_library::foundations::cast! { @munch [$ty] [dynamic] [] infos [] strs [] casts [] rest [] }
    };
    (type $ty:ty, $self:ident => $into:expr $(, $($rest:tt)*)?) => {
        $crate::typst_library::foundations::cast! { @munch [$ty] [dynamic] [$self $into] infos [] strs [] casts [] rest [$($($rest)*)?] }
    };
    (type $ty:ty, $($rest:tt)+) => {
        $crate::typst_library::foundations::cast! { @munch [$ty] [dynamic] [] infos [] strs [] casts [] rest [$($rest)+] }
    };
    ($ty:ty, $self:ident => $into:expr $(, $($rest:tt)*)?) => {
        $crate::typst_library::foundations::cast! { @munch [$ty] [static] [$self $into] infos [] strs [] casts [] rest [$($($rest)*)?] }
    };
    ($ty:ty, $($rest:tt)+) => {
        $crate::typst_library::foundations::cast! { @munch [$ty] [static] [] infos [] strs [] casts [] rest [$($rest)+] }
    };

    // A string alternative.
    (@munch $ty:tt $kind:tt $into:tt infos [$($infos:tt)*] strs [$($strs:tt)*] casts $casts:tt
        rest [$(#[doc = $doc:literal])* $lit:literal => $expr:expr $(, $($rest:tt)*)?]) => {
        $crate::typst_library::foundations::cast! { @munch $ty $kind $into
            infos [$($infos)* ($crate::typst_library::foundations::CastInfo::Value(
                $crate::typst_library::foundations::IntoValue::into_value($lit),
                concat!($($doc, "\n",)* ""),
            ))]
            strs [$($strs)* ([$lit] => $expr)]
            casts $casts
            rest [$($($rest)*)?] }
    };
    // A string alternative whose string is a constant expression, from `derive_cast!`.
    (@munch $ty:tt $kind:tt $into:tt infos [$($infos:tt)*] strs [$($strs:tt)*] casts $casts:tt
        rest [@str $name:expr => $expr:expr $(, $($rest:tt)*)?]) => {
        $crate::typst_library::foundations::cast! { @munch $ty $kind $into
            infos [$($infos)* ($crate::typst_library::foundations::CastInfo::Value(
                $crate::typst_library::foundations::IntoValue::into_value($name),
                "",
            ))]
            strs [$($strs)* ([$name] => $expr)]
            casts $casts
            rest [$($($rest)*)?] }
    };
    // A typed alternative with a wildcard binding.
    (@munch $ty:tt $kind:tt $into:tt infos [$($infos:tt)*] strs $strs:tt casts [$($casts:tt)*]
        rest [$(#[doc = $doc:literal])* _ : $cty:ty => $expr:expr $(, $($rest:tt)*)?]) => {
        $crate::typst_library::foundations::cast! { @munch $ty $kind $into
            infos [$($infos)* (<$cty as $crate::typst_library::foundations::Reflect>::input())]
            strs $strs
            casts [$($casts)* ([_unused], $cty, $expr)]
            rest [$($($rest)*)?] }
    };
    // A typed alternative with a mutable binding.
    (@munch $ty:tt $kind:tt $into:tt infos [$($infos:tt)*] strs $strs:tt casts [$($casts:tt)*]
        rest [$(#[doc = $doc:literal])* mut $bind:ident : $cty:ty => $expr:expr $(, $($rest:tt)*)?]) => {
        $crate::typst_library::foundations::cast! { @munch $ty $kind $into
            infos [$($infos)* (<$cty as $crate::typst_library::foundations::Reflect>::input())]
            strs $strs
            casts [$($casts)* ([mut $bind], $cty, $expr)]
            rest [$($($rest)*)?] }
    };
    // A typed alternative.
    (@munch $ty:tt $kind:tt $into:tt infos [$($infos:tt)*] strs $strs:tt casts [$($casts:tt)*]
        rest [$(#[doc = $doc:literal])* $bind:ident : $cty:ty => $expr:expr $(, $($rest:tt)*)?]) => {
        $crate::typst_library::foundations::cast! { @munch $ty $kind $into
            infos [$($infos)* (<$cty as $crate::typst_library::foundations::Reflect>::input())]
            strs $strs
            casts [$($casts)* ([$bind], $cty, $expr)]
            rest [$($($rest)*)?] }
    };
    // All alternatives are parsed.
    (@munch [$ty:ty] [$kind:ident] [$($into:tt)*] infos [$($infos:tt)*] strs [$(([$($name:tt)*] => $sexpr:expr))*]
        casts [$(([$($bind:tt)*], $cty:ty, $cexpr:expr))*] rest []) => {
        $crate::typst_library::foundations::cast! { @reflect [$ty] [$kind] [$($infos)*] [$([$($name)*])*] [$($cty)*] }
        $crate::typst_library::foundations::cast! { @into [$ty] [$kind] [$($into)*] }
        $crate::typst_library::foundations::cast! { @from [$ty] [$kind] [$(([$($name)*] => $sexpr))*] [$(([$($bind)*], $cty, $cexpr))*] }
    };

    (@reflect [$ty:ty] [static] [] [] []) => {};
    (@reflect [$ty:ty] [$kind:ident] [$($info:tt)*] [$([$($name:tt)*])*] [$($cty:ty)*]) => {
        impl $crate::typst_library::foundations::Reflect for $ty {
            fn input() -> $crate::typst_library::foundations::CastInfo {
                let infos: ::std::vec::Vec<::std::option::Option<$crate::typst_library::foundations::CastInfo>> = vec![
                    $(::std::option::Option::Some($info),)*
                    $crate::typst_library::foundations::cast!(@dynamic_info [$kind])
                ];
                infos
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| a + b)
                    .unwrap_or($crate::typst_library::foundations::CastInfo::Union(vec![]))
            }

            fn output() -> $crate::typst_library::foundations::CastInfo {
                $crate::typst_library::foundations::cast!(@output [$kind])
            }

            fn castable(value: &$crate::typst_library::foundations::Value) -> bool {
                $crate::typst_library::foundations::cast!(@dynamic_check [$kind] value);
                $crate::typst_library::foundations::cast!(@str_castable value [$([$($name)*])*]);
                $(
                    if <$cty as $crate::typst_library::foundations::Reflect>::castable(value) {
                        return true;
                    }
                )*
                false
            }
        }
    };

    (@str_castable $value:ident []) => {};
    (@str_castable $value:ident [$([$($name:tt)*])+]) => {
        if let $crate::typst_library::foundations::Value::Str(string) = &$value {
            $(if string.as_str() == $($name)* {
                return true;
            })+
        }
    };

    (@dynamic_info [dynamic]) => {
        ::std::option::Option::Some($crate::typst_library::foundations::CastInfo::Type(
            $crate::typst_library::foundations::Type::of::<Self>(),
        ))
    };
    (@dynamic_info [static]) => { ::std::option::Option::None };

    (@output [dynamic]) => {
        $crate::typst_library::foundations::CastInfo::Type($crate::typst_library::foundations::Type::of::<Self>())
    };
    (@output [static]) => { <Self as $crate::typst_library::foundations::Reflect>::input() };

    (@dynamic_check [dynamic] $value:ident) => {
        if let $crate::typst_library::foundations::Value::Dyn(dynamic) = &$value {
            if dynamic.is::<Self>() {
                return true;
            }
        }
    };
    (@dynamic_check [static] $value:ident) => {};

    (@into [$ty:ty] [static] []) => {};
    (@into [$ty:ty] [$kind:ident] []) => {
        impl $crate::typst_library::foundations::IntoValue for $ty {
            fn into_value(self) -> $crate::typst_library::foundations::Value {
                $crate::typst_library::foundations::Value::dynamic(self)
            }
        }
    };
    (@into [$ty:ty] [$kind:ident] [$self:ident $into:expr]) => {
        impl $crate::typst_library::foundations::IntoValue for $ty {
            fn into_value($self) -> $crate::typst_library::foundations::Value {
                $into
            }
        }
    };

    (@from [$ty:ty] [static] [] []) => {};
    (@from [$ty:ty] [$kind:ident] [$(([$($name:tt)*] => $sexpr:expr))*] [$(([$($bind:tt)*], $cty:ty, $cexpr:expr))*]) => {
        impl $crate::typst_library::foundations::FromValue for $ty {
            fn from_value(
                value: $crate::typst_library::foundations::Value,
            ) -> $crate::typst_library::diag::HintedStrResult<Self> {
                $crate::typst_library::foundations::cast!(@dynamic_from [$kind] value);
                $crate::typst_library::foundations::cast!(@str_from value [$(([$($name)*] => $sexpr))*]);
                $(
                    if <$cty as $crate::typst_library::foundations::Reflect>::castable(&value) {
                        #[allow(unused_variables)]
                        let $($bind)* = <$cty as $crate::typst_library::foundations::FromValue>::from_value(value)?;
                        return Ok($cexpr);
                    }
                )*
                Err(<Self as $crate::typst_library::foundations::Reflect>::error(&value))
            }
        }
    };

    (@str_from $value:ident []) => {};
    (@str_from $value:ident [$(([$($name:tt)*] => $sexpr:expr))+]) => {
        if let $crate::typst_library::foundations::Value::Str(string) = &$value {
            $(if string.as_str() == $($name)* {
                return Ok($sexpr);
            })+
        }
    };

    (@dynamic_from [dynamic] $value:ident) => {
        if let $crate::typst_library::foundations::Value::Dyn(dynamic) = &$value {
            if let Some(concrete) = dynamic.downcast::<Self>() {
                return Ok(concrete.clone());
            }
        }
    };
    (@dynamic_from [static] $value:ident) => {};
}

pub(crate) use cast;

/// Implements the casts that upstream's `#[derive(Cast)]` generates for a fieldless enum
/// (`crates/typst-macros/src/cast.rs`): each variant casts from and to its name in kebab case,
/// or the string given with `#[string(..)]`.
///
/// ```ignore
/// derive_cast!(LineCap { Butt, Round, Square });
/// ```
// avenger: a `macro_rules!` stand-in for the derive, listing the variants once more. The
// enum's variant docs are not repeated, since only documentation reads them.
macro_rules! derive_cast {
    ($ty:ident { $($(#[string($string:literal)])? $variant:ident),* $(,)? }) => {
        $crate::typst_library::foundations::cast! {
            $ty,
            self => $crate::typst_library::foundations::IntoValue::into_value(match self {
                $($ty::$variant => $crate::typst_library::foundations::derive_cast!(
                    @name $variant $($string)?
                ),)*
            }),
            $(@str $crate::typst_library::foundations::derive_cast!(@name $variant $($string)?)
                => Self::$variant,)*
        }
    };
    (@name $variant:ident $string:literal) => { $string };
    (@name $variant:ident) => {{
        const NAME: &str = {
            const LEN: usize = $crate::typst_library::foundations::kebab_case_len(stringify!($variant));
            const BYTES: [u8; LEN] =
                $crate::typst_library::foundations::camel_to_kebab_case(stringify!($variant));
            match ::std::str::from_utf8(&BYTES) {
                ::std::result::Result::Ok(name) => name,
                ::std::result::Result::Err(_) => panic!("variant names are ASCII"),
            }
        };
        NAME
    }};
}

pub(crate) use derive_cast;

/// The length of a camel case variant name in kebab case.
#[doc(hidden)]
pub const fn kebab_case_len(name: &str) -> usize {
    let bytes = name.as_bytes();
    let mut len = bytes.len();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            len += 1;
        }
        i += 1;
    }
    len
}

/// Converts a camel case variant name to kebab case at compile time, as upstream's derive
/// does (`ScriptScript` becomes `script-script`).
#[doc(hidden)]
pub const fn camel_to_kebab_case<const N: usize>(name: &str) -> [u8; N] {
    let bytes = name.as_bytes();
    let mut out = [0; N];
    let (mut i, mut j) = (0, 0);
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_uppercase() {
            if i > 0 {
                out[j] = b'-';
                j += 1;
            }
            out[j] = b.to_ascii_lowercase();
        } else {
            out[j] = b;
        }
        i += 1;
        j += 1;
    }
    out
}
