//! Ported from crates/typst-library/src/foundations/func.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: functions are native functions and element constructors. Labels define no
//! closures, load no plugins and pre-apply no arguments, and functions have no scopes, so
//! `with` and `where` are gone. [`func!`] stands in for upstream's `#[func]` attribute.

use std::fmt::{self, Debug, Formatter};

use ecow::EcoString;

use crate::typst_library::diag::{SourceResult, StrResult, WarningSink, bail};
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{Args, Element, Repr, Value, cast, ty};
use typst_syntax::Span;

/// A mapping from argument values to a return value.
#[derive(Clone)]
pub struct Func {
    /// The internal representation.
    inner: FuncInner,
    /// The span with which errors are reported when this function is called.
    span: Span,
}

ty!(Func, name = "function", title = "Function", long = "function");

/// The different kinds of function representations.
#[derive(Clone, Copy)]
enum FuncInner {
    /// A native Rust function.
    Native(&'static NativeFuncData),
    /// A function for an element.
    Element(Element),
}

impl PartialEq for FuncInner {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(a), Self::Native(b)) => std::ptr::eq(*a, *b),
            (Self::Element(a), Self::Element(b)) => a == b,
            _ => false,
        }
    }
}

impl Func {
    /// The function's name (e.g. `min`).
    pub fn name(&self) -> Option<&str> {
        match &self.inner {
            FuncInner::Native(native) => Some(native.name),
            FuncInner::Element(elem) => Some(elem.name()),
        }
    }

    /// Get a field from this function's scope, if possible.
    // avenger: functions have no scopes.
    pub fn field(&self, field: &str, _: impl WarningSink) -> StrResult<&'static Value> {
        match self.name() {
            Some(name) => bail!("function `{name}` does not contain field `{field}`"),
            None => bail!("function does not contain field `{field}`"),
        }
    }

    /// Extract the element function, if it is one.
    pub fn to_element(&self) -> Option<Element> {
        match self.inner {
            FuncInner::Element(func) => Some(func),
            _ => None,
        }
    }

    /// Call the function with the given arguments.
    // avenger: no context, since labels have no introspection.
    pub fn call(&self, engine: &mut Engine, mut args: Args) -> SourceResult<Value> {
        match &self.inner {
            FuncInner::Native(native) => {
                let value = (native.function)(engine, &mut args)?;
                args.finish()?;
                Ok(value)
            }
            FuncInner::Element(func) => {
                let value = func.construct(engine, &mut args)?;
                args.finish()?;
                Ok(Value::Content(value))
            }
        }
    }

    /// The function's span.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Attach a span to this function if it doesn't already have one.
    pub fn spanned(mut self, span: Span) -> Self {
        if self.span.is_detached() {
            self.span = span;
        }
        self
    }
}

impl Debug for Func {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Func({})", self.name().unwrap_or(".."))
    }
}

impl Repr for Func {
    fn repr(&self) -> EcoString {
        match &self.inner {
            FuncInner::Native(native) => native.name.into(),
            FuncInner::Element(elem) => elem.name().into(),
        }
    }
}

impl PartialEq for Func {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl PartialEq<&'static NativeFuncData> for Func {
    fn eq(&self, other: &&'static NativeFuncData) -> bool {
        match &self.inner {
            FuncInner::Native(native) => std::ptr::eq(*native, *other),
            _ => false,
        }
    }
}

impl PartialEq<Element> for Func {
    fn eq(&self, other: &Element) -> bool {
        match &self.inner {
            FuncInner::Element(elem) => elem == other,
            _ => false,
        }
    }
}

impl From<FuncInner> for Func {
    fn from(inner: FuncInner) -> Self {
        Self { inner, span: Span::detached() }
    }
}

impl From<&'static NativeFuncData> for Func {
    fn from(data: &'static NativeFuncData) -> Self {
        FuncInner::Native(data).into()
    }
}

impl From<Element> for Func {
    fn from(func: Element) -> Self {
        FuncInner::Element(func).into()
    }
}

/// A Typst function that is defined by a native Rust type that shadows a
/// native Rust function.
pub trait NativeFunc {
    /// Get the function for the native Rust type.
    fn func() -> Func {
        Func::from(Self::data())
    }

    /// Get the function data for the native Rust function.
    fn data() -> &'static NativeFuncData;
}

/// Defines a native function.
// avenger: the name and the implementation; no documentation, scope or parameter metadata.
#[derive(Debug)]
pub struct NativeFuncData {
    /// The implementation of the function.
    pub function: NativeFuncPtr,
    /// The function's normal name (e.g. `align`), as exposed to Typst.
    pub name: &'static str,
}

cast! {
    &'static NativeFuncData,
    self => Func::from(self).into_value(),
}

/// A pointer to a native function's implementation.
// avenger: a function pointer, since `func!` wrappers capture nothing.
pub type NativeFuncPtr = fn(&mut Engine, &mut Args) -> SourceResult<Value>;

/// Defines a native function, as upstream's `#[func]` attribute does.
///
/// Takes the function as upstream writes it, with `#[func(..)]` and its parameter attributes,
/// and generates the function without those attributes, plus a type of the same name that
/// implements [`NativeFunc`] by parsing the arguments as upstream's attribute does:
///
/// - a parameter named `engine` receives the engine, one named `args` the arguments (and then
///   the arguments aren't checked for leftovers), and one named `span` the call's span;
/// - `#[external]` parameters only document arguments the function parses itself;
/// - `#[named]` parameters come from named arguments, `#[variadic]` ones from all remaining
///   positional arguments, ones with `#[default(..)]` from an optional positional argument,
///   and all others from a required positional argument.
macro_rules! func {
    (
        $(#[doc = $doc:literal])*
        #[func $(($($meta:tt)*))?]
        $vis:vis fn $name:ident ($($params:tt)*) -> $ret:ty $body:block
    ) => {
        // The wrapper's `engine` and `args` are made here, once, so that every rule below names
        // the same variables.
        $crate::typst_library::foundations::func! { @params [engine args]
            [$name [$vis] [$(#[doc = $doc])*] [$ret] $body]
            [] [] [] [no]
            [$($params)*]
        }
    };

    // The state is `[ids] [header] [signature] [parsers] [call arguments] [takes args]
    // [rest]`. Each parameter is classified by its name, keeping the invocation's token.
    (@params $ids:tt $h:tt $sig:tt $parsers:tt $call:tt $takes:tt
        [$(#[$($attr:tt)*])* $pname:ident : $ty:ty $(, $($rest:tt)*)?]
    ) => {
        $crate::typst_library::foundations::func! { @classify $ids
            [$h $sig $parsers $call $takes [$($($rest)*)?]]
            [$pname] $pname [$ty] [$([$($attr)*])*]
        }
    };
    // All parameters are done.
    (@params [$engine:ident $args:ident]
        [$name:ident [$vis:vis] [$($docs:tt)*] [$ret:ty] $body:block]
        [$($sig:tt)*] [$($parsers:tt)*] [$($call:tt)*] [$takes:ident] []
    ) => {
        $($docs)*
        $vis fn $name($($sig)*) -> $ret $body

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        $vis enum $name {}

        impl $crate::typst_library::foundations::NativeFunc for $name {
            fn data() -> &'static $crate::typst_library::foundations::NativeFuncData {
                static DATA: $crate::typst_library::foundations::NativeFuncData =
                    $crate::typst_library::foundations::NativeFuncData {
                        function: |$engine, $args| {
                            let _ = &$engine;
                            $($parsers)*
                            $crate::typst_library::foundations::func!(@finish $takes $args);
                            let output = $name($($call)*);
                            $crate::typst_library::foundations::IntoResult::into_result(
                                output,
                                $args.span,
                            )
                        },
                        name: $crate::typst_library::foundations::func!(@name $name),
                    };
                &DATA
            }
        }
    };

    // Special parameters: the engine, the arguments themselves, and the call's span.
    (@classify [$engine:ident $args:ident]
        [$h:tt [$($sig:tt)*] $parsers:tt [$($call:tt)*] $takes:tt $rest:tt]
        [$p:ident] engine [$ty:ty] $attrs:tt
    ) => {
        $crate::typst_library::foundations::func! { @params [$engine $args] $h
            [$($sig)* $p: $ty,] $parsers [$($call)* $engine,] $takes $rest }
    };
    (@classify [$engine:ident $args:ident]
        [$h:tt [$($sig:tt)*] $parsers:tt [$($call:tt)*] $takes:tt $rest:tt]
        [$p:ident] args [$ty:ty] $attrs:tt
    ) => {
        $crate::typst_library::foundations::func! { @params [$engine $args] $h
            [$($sig)* $p: $ty,] $parsers [$($call)* $args,] [yes] $rest }
    };
    (@classify [$engine:ident $args:ident]
        [$h:tt [$($sig:tt)*] $parsers:tt [$($call:tt)*] $takes:tt $rest:tt]
        [$p:ident] span [$ty:ty] $attrs:tt
    ) => {
        $crate::typst_library::foundations::func! { @params [$engine $args] $h
            [$($sig)* $p: $ty,] $parsers [$($call)* $args.span,] $takes $rest }
    };
    // Other parameters: work through their attributes. The state after the parameter is
    // `[mode] [default]`.
    (@classify $ids:tt $state:tt [$p:ident] $other:ident [$ty:ty] $attrs:tt) => {
        $crate::typst_library::foundations::func! { @attrs $ids $state [$p [$ty]]
            [positional] [] $attrs }
    };

    (@attrs $ids:tt $state:tt $param:tt $mode:tt $default:tt [[doc = $d:literal] $($more:tt)*]) => {
        $crate::typst_library::foundations::func! { @attrs $ids $state $param $mode $default
            [$($more)*] }
    };
    (@attrs $ids:tt $state:tt $param:tt $mode:tt $default:tt [[named] $($more:tt)*]) => {
        $crate::typst_library::foundations::func! { @attrs $ids $state $param [named] $default
            [$($more)*] }
    };
    (@attrs $ids:tt $state:tt $param:tt $mode:tt $default:tt [[variadic] $($more:tt)*]) => {
        $crate::typst_library::foundations::func! { @attrs $ids $state $param [variadic]
            $default [$($more)*] }
    };
    (@attrs $ids:tt $state:tt $param:tt $mode:tt $default:tt [[external] $($more:tt)*]) => {
        $crate::typst_library::foundations::func! { @attrs $ids $state $param [external]
            $default [$($more)*] }
    };
    (@attrs $ids:tt $state:tt $param:tt $mode:tt $default:tt [[default] $($more:tt)*]) => {
        $crate::typst_library::foundations::func! { @attrs $ids $state $param $mode
            [::std::default::Default::default()] [$($more)*] }
    };
    (@attrs $ids:tt $state:tt $param:tt $mode:tt $default:tt
        [[default($($value:tt)*)] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::func! { @attrs $ids $state $param $mode
            [$($value)*] [$($more)*] }
    };

    // An external parameter is only documentation.
    (@attrs $ids:tt [$h:tt $sig:tt $parsers:tt $call:tt $takes:tt $rest:tt] $param:tt
        [external] $default:tt []
    ) => {
        $crate::typst_library::foundations::func! { @params $ids $h $sig $parsers $call
            $takes $rest }
    };
    // Other parameters are parsed from the arguments.
    (@attrs [$engine:ident $args:ident]
        [$h:tt [$($sig:tt)*] [$($parsers:tt)*] [$($call:tt)*] $takes:tt $rest:tt]
        [$pname:ident [$ty:ty]] [$mode:ident] $default:tt []
    ) => {
        $crate::typst_library::foundations::func! { @params [$engine $args] $h
            [$($sig)* $pname: $ty,]
            [$($parsers)*
                let $pname: $ty = $crate::typst_library::foundations::func!(
                    @parse $mode $default $args $pname
                );]
            [$($call)* $pname,]
            $takes $rest }
    };

    (@parse named [] $args:ident $pname:ident) => {
        $args.named($crate::typst_library::foundations::func!(@name $pname))?
    };
    (@parse named [$($default:tt)+] $args:ident $pname:ident) => {
        $args.named($crate::typst_library::foundations::func!(@name $pname))?
            .unwrap_or_else(|| $($default)+)
    };
    (@parse variadic $default:tt $args:ident $pname:ident) => {
        $args.all()?
    };
    (@parse positional [] $args:ident $pname:ident) => {
        $args.expect($crate::typst_library::foundations::func!(@name $pname))?
    };
    (@parse positional [$($default:tt)+] $args:ident $pname:ident) => {
        $args.eat()?.unwrap_or_else(|| $($default)+)
    };

    (@finish no $args:ident) => { $args.take().finish()? };
    (@finish yes $args:ident) => { () };

    // Upstream exposes names in kebab case.
    (@name $name:ident) => {{
        const NAME: &str = {
            const BYTES: [u8; stringify!($name).len()] =
                $crate::typst_library::foundations::kebab_case(stringify!($name));
            match ::std::str::from_utf8(&BYTES) {
                ::std::result::Result::Ok(name) => name,
                ::std::result::Result::Err(_) => panic!("names are ASCII"),
            }
        };
        NAME
    }};
}

pub(crate) use func;
