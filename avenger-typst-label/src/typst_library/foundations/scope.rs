//! Ported from crates/typst-library/src/foundations/scope.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's scopes are its parameters over the label library, so there is no stack of
//! nested scopes, no mutation, no closure captures and no binding categories.

use std::fmt::{self, Debug, Formatter};

use ecow::{EcoString, eco_format};
use indexmap::IndexMap;
use indexmap::map::Entry;
use rustc_hash::FxBuildHasher;

use crate::typst_library::Library;
use crate::typst_library::diag::{HintedStrResult, HintedString, WarningSink};
use crate::typst_library::foundations::{
    Func, IntoValue, NativeElement, NativeFunc, NativeFuncData, Value,
};
use typst_syntax::Span;

/// A stack of scopes.
// avenger: the label's parameters over the library.
#[derive(Debug, Clone)]
pub struct Scopes<'a> {
    /// The active scope.
    pub top: Scope,
    /// The standard library.
    pub base: Option<&'a Library>,
}

impl<'a> Scopes<'a> {
    /// Create a new, empty hierarchy of scopes.
    pub fn new(base: Option<&'a Library>) -> Self {
        Self { top: Scope::new(), base }
    }

    /// Try to access a binding immutably.
    pub fn get(&self, var: &str) -> HintedStrResult<&Binding> {
        self.top
            .get(var)
            .or_else(|| self.base.and_then(|base| base.global.scope().get(var)))
            .ok_or_else(|| unknown_variable(var))
    }

    /// Try to access a binding immutably in math.
    pub fn get_in_math(&self, var: &str) -> HintedStrResult<&Binding> {
        self.top
            .get(var)
            .or_else(|| self.base.and_then(|base| base.math.scope().get(var)))
            .ok_or_else(|| {
                unknown_variable_math(
                    var,
                    self.base.is_some_and(|base| base.global.scope().get(var).is_some()),
                )
            })
    }
}

/// A map from binding names to values.
#[derive(Default, Clone)]
pub struct Scope {
    map: IndexMap<EcoString, Binding, FxBuildHasher>,
    // avenger: only the debug-build duplicate check reads this; upstream also hashes it.
    #[cfg_attr(
        not(debug_assertions),
        expect(dead_code, reason = "read by the debug-build duplicate check")
    )]
    deduplicate: bool,
}

/// Scope construction.
impl Scope {
    /// Create a new empty scope.
    pub fn new() -> Self {
        Default::default()
    }

    /// Create a new scope with duplication prevention.
    pub fn deduplicating() -> Self {
        Self { deduplicate: true, ..Default::default() }
    }

    /// Define a native function through a Rust type that shadows the function.
    #[track_caller]
    pub fn define_func<T: NativeFunc>(&mut self) -> &mut Binding {
        let data = T::data();
        self.define(data.name, Func::from(data))
    }

    /// Define a native function with raw function data.
    #[track_caller]
    pub fn define_func_with_data(
        &mut self,
        data: &'static NativeFuncData,
    ) -> &mut Binding {
        self.define(data.name, Func::from(data))
    }

    /// Define a native element.
    #[track_caller]
    pub fn define_elem<T: NativeElement>(&mut self) -> &mut Binding {
        let elem = T::ELEM;
        self.define(elem.name(), Func::from(elem))
    }

    /// Define a built-in with compile-time known name and returns a mutable
    /// reference to it.
    #[track_caller]
    pub fn define(&mut self, name: &'static str, value: impl IntoValue) -> &mut Binding {
        #[cfg(debug_assertions)]
        if self.deduplicate && self.map.contains_key(name) {
            panic!("duplicate definition: {name}");
        }

        self.bind(name.into(), Binding::detached(value))
    }
}

/// Scope manipulation and access.
impl Scope {
    /// Inserts a binding into this scope and returns a mutable reference to it.
    pub fn bind(&mut self, name: EcoString, binding: Binding) -> &mut Binding {
        match self.map.entry(name) {
            Entry::Occupied(mut entry) => {
                entry.insert(binding);
                entry.into_mut()
            }
            Entry::Vacant(entry) => entry.insert(binding),
        }
    }

    /// Try to access a binding immutably.
    pub fn get(&self, var: &str) -> Option<&Binding> {
        self.map.get(var)
    }

    /// Iterate over all definitions.
    pub fn iter(&self) -> impl Iterator<Item = (&EcoString, &Binding)> {
        self.map.iter()
    }
}

impl Debug for Scope {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str("Scope ")?;
        f.debug_map()
            .entries(self.map.iter().map(|(k, v)| (k, v.read())))
            .finish()
    }
}

/// A bound value with metadata.
// avenger: no binding kinds or categories, since labels capture and mutate nothing.
#[derive(Debug, Clone)]
pub struct Binding {
    /// The bound value.
    value: Value,
    /// A span associated with the binding.
    span: Span,
    /// The deprecation information if this item is deprecated.
    deprecation: Option<Box<Deprecation>>,
}

impl Binding {
    /// Create a new binding with a span marking its definition site.
    pub fn new(value: impl IntoValue, span: Span) -> Self {
        Self { value: value.into_value(), span, deprecation: None }
    }

    /// Create a binding without a span.
    pub fn detached(value: impl IntoValue) -> Self {
        Self::new(value, Span::detached())
    }

    /// Marks this binding as deprecated, with the given `message`.
    pub fn deprecated(&mut self, deprecation: Deprecation) -> &mut Self {
        self.deprecation = Some(Box::new(deprecation));
        self
    }

    /// Read the value.
    pub fn read(&self) -> &Value {
        &self.value
    }

    /// Read the value, checking for deprecation.
    ///
    /// As the `sink`
    /// - pass `()` to ignore the message.
    /// - pass `(&mut engine, span)` to emit a warning into the engine.
    pub fn read_checked(&self, mut sink: impl WarningSink) -> &Value {
        if let Some(message) = &self.deprecation {
            sink.emit((**message).into());
        }
        &self.value
    }

    /// A span associated with the stored value.
    pub fn span(&self) -> Span {
        self.span
    }

    /// A deprecation message for the value, if any.
    pub fn deprecation(&self) -> Option<&Deprecation> {
        self.deprecation.as_deref()
    }
}

/// Information about a deprecated binding.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct Deprecation {
    /// A deprecation message for the definition.
    message: &'static str,
    /// A version in which the deprecated binding is planned to be removed.
    until: Option<&'static str>,
}

impl Deprecation {
    /// Creates new deprecation info with a default message to display when
    /// emitting the deprecation warning.
    pub fn new() -> Self {
        Self { message: "item is deprecated", until: None }
    }

    /// Set the message to display when emitting the deprecation warning.
    pub fn with_message(mut self, message: &'static str) -> Self {
        self.message = message;
        self
    }

    /// Set the version in which the binding is planned to be removed.
    pub fn with_until(mut self, version: &'static str) -> Self {
        self.until = Some(version);
        self
    }

    /// The message to display when emitting the deprecation warning.
    pub fn message(&self) -> &'static str {
        self.message
    }

    /// The version in which the binding is planned to be removed.
    pub fn until(&self) -> Option<&'static str> {
        self.until
    }
}

impl Default for Deprecation {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Deprecation> for HintedString {
    fn from(deprecation: Deprecation) -> Self {
        HintedString::new(deprecation.message.into()).with_hints(
            deprecation
                .until
                .map(|v| eco_format!("it will be removed in Typst {v}")),
        )
    }
}

/// The error message when a variable wasn't found.
#[cold]
fn unknown_variable(var: &str) -> HintedString {
    let mut res = HintedString::new(eco_format!("unknown variable: {var}"));

    if var.contains('-') {
        res.hint(eco_format!(
            "if you meant to use subtraction, \
             try adding spaces around the minus sign{}: `{}`",
            if var.matches('-').count() > 1 { "s" } else { "" },
            var.replace('-', " - ")
        ));
    }

    res
}

/// The error message when a variable wasn't found it math.
#[cold]
fn unknown_variable_math(var: &str, in_global: bool) -> HintedString {
    let mut res = HintedString::new(eco_format!("unknown variable: {var}"));

    if matches!(var, "none" | "auto" | "false" | "true") {
        res.hint(eco_format!(
            "if you meant to use a literal, \
             try adding a hash before it: `#{var}`",
        ));
    } else if in_global {
        res.hint(eco_format!(
            "`{var}` is not available directly in math, but is in the standard library",
        ));
        res.hint(eco_format!(
            "to access `{var}` in code mode you can add a hash: `#{var}`",
        ));
        res.hint(eco_format!(
            "or access `{var}` in math mode by using the `std` module: `std.{var}`",
        ));
    } else {
        res.hint(eco_format!(
            "if you meant to display multiple letters as is, \
             try adding spaces between each letter: `{}`",
            var.chars().flat_map(|c| [' ', c]).skip(1).collect::<EcoString>()
        ));
        res.hint(eco_format!(
            "or if you meant to display this as text, \
             try placing it in quotes: `\"{var}\"`"
        ));
    }

    res
}
