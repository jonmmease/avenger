//! Ported from crates/typst-library/src/diag.rs @ v0.15.1, modified for Avenger.
//!
//! Diagnostics.

// We re-export these types from `ecow` so that the macros below can write
// `$crate::typst_library::diag::eco_format` instead of `::ecow::eco_format`. This allows
// downstream crates to use the macros without needing to include `ecow` as a
// direct dependency of the crate.
#[doc(hidden)]
pub use ecow::{EcoString, EcoVec, eco_format, eco_vec};

use crate::typst_library::engine::Engine;
use typst_syntax::{DiagSpan, Span, Spanned, SyntaxDiagnostic};

/// Early-return with an error for common result types used in Typst. If you
/// need to interact with the produced errors more, consider using `error!` or
/// `warning!` instead.
///
/// The main usage is `bail!(span, "message with {}", "formatting")`, which will
/// early-return an error for a [`SourceResult`]. If you leave out the span, it
/// will return an error for a [`StrResult`] or [`HintedStrResult`] instead.
///
/// You can also add hints by separating the initial message with a semicolon
/// and writing `hint: "..."`, see the example.
///
/// ```ignore
/// bail!("returning a {} error with no span", "formatted"); // StrResult (no span)
/// bail!(span, "returning a {} error", "formatted"); // SourceResult (has a span)
/// bail!(
///     span, "returning a {} error", "formatted";
///     hint: "with multiple hints";
///     hint[hint_span]: "hints can have custom spans and {}", "formatting";
/// ); // SourceResult
/// ```
#[clippy::format_args]
// See the comment below for why this is `__bail` and not `bail`.
macro_rules! __bail {
    // If we don't have a span, forward to `error!` to create a `StrResult` or
    // `HintedStrResult`.
    (
        $fmt:literal $(, $arg:expr)* $(,)?
        $(; hint: $hint:literal $(, $hint_arg:expr)*)*
        $(;)?
    ) => {
        return Err($crate::typst_library::diag::error!(
            $fmt $(, $arg)*
            $(; hint: $hint $(, $hint_arg)*)*
        ))
    };

    // Just early return for a `SourceResult`: `bail!(some_error)`.
    ($error:expr) => {
        return Err($crate::typst_library::diag::eco_vec![$error])
    };

    // For `bail(span, ...)`, we reuse `error!` and produce a `SourceResult`.
    ($($tts:tt)*) => {
        return Err($crate::typst_library::diag::eco_vec![$crate::typst_library::diag::error!($($tts)*)])
    };
}

/// Construct an [`EcoString`], [`HintedString`] or [`SourceDiagnostic`] with
/// severity `Error`.
///
/// If you just want to quickly return an error, consider the `bail!` macro.
/// If you want to create a warning, use the `warning!` macro.
///
/// You can also add hints by separating the initial message with a semicolon
/// and writing `hint: "..."`, see the example.
///
/// ```ignore
/// error!("a {} error with no span", "formatted"); // EcoString, same as `eco_format!`
/// error!(span, "an error with a {} message", "formatted"); // SourceDiagnostic
/// error!(
///     span, "an error with a {} message", "formatted";
///     hint: "with multiple hints";
///     hint[hint_span]: "hints can have custom spans and {}", "formatting";
/// ); // SourceDiagnostic
/// ```
#[clippy::format_args]
// See the comment below for why this is `__error` and not `error`.
macro_rules! __error {
    // For `error!("just a {}", "string")`.
    ($fmt:literal $(, $arg:expr)* $(,)?) => {
        $crate::typst_library::diag::eco_format!($fmt $(, $arg)*).into()
    };

    // For `error!("a hinted {}", "string"; hint: "some hint"; hint: "...")`
    (
        $fmt:literal $(, $arg:expr)* $(,)?
        $(; hint: $hint:literal $(, $hint_arg:expr)*)*
        $(;)?
    ) => {
        $crate::typst_library::diag::HintedString::new(
            $crate::typst_library::diag::eco_format!($fmt $(, $arg)*)
        ) $(.with_hint($crate::typst_library::diag::eco_format!($hint $(, $hint_arg)*)))*
    };

    // For `error!(span, ...)`
    // Hints may include a span inside brackets: `hint[span_expr]: "msg"`.
    (
        $span:expr, $fmt:literal $(, $arg:expr)* $(,)?
        $(; hint $([$hint_span:expr])? : $hint:literal $(, $hint_arg:expr)*)*
        $(;)?
    ) => {{
        #[allow(unused_mut)]
        let mut err = $crate::typst_library::diag::SourceDiagnostic::error(
            $span,
            $crate::typst_library::diag::eco_format!($fmt $(, $arg)*)
        );
        // We use a recursive macro for hints to allow for optional spans.
        $($crate::typst_library::diag::error!(hint$([$hint_span])?: err, $hint $(, $hint_arg)*);)*
        err
    }};

    // Internal recursive macro for adding hints with/without spans. Note that
    // recursive macros must generate full expressions, so we can't use
    // `.with_hint()` or `.with_spanned_hint()`.
    (hint: $err:ident, $hint:literal $(, $hint_arg:expr)*) => {
        $err.hint($crate::typst_library::diag::eco_format!($hint $(, $hint_arg)*))
    };
    (hint[$hint_span:expr]: $err:ident, $hint:literal $(, $hint_arg:expr)*) => {
        $err.spanned_hint($crate::typst_library::diag::eco_format!($hint $(, $hint_arg)*), $hint_span)
    };
}

/// Construct a [`SourceDiagnostic`] with severity `Warning`. To use the warning
/// you will need to add it to a sink, likely inside the [`Engine`], e.g.
/// `engine.sink.warn(warning!(...))`.
///
/// If you want to return early or construct an error, consider the `bail!` or
/// `error!` macros instead.
///
/// You can also add hints by separating the initial message with a semicolon
/// and writing `hint: "..."`, see the example.
///
/// ```ignore
/// warning!(span, "warning with a {} message", "formatted");
/// warning!(
///     span, "warning with a {} message", "formatted";
///     hint: "with multiple hints";
///     hint[hint_span]: "hints can have custom spans and {}", "formatting";
/// );
/// ```
#[clippy::format_args]
// See the comment below for why this is `__warning` and not `warning`.
macro_rules! __warning {
    (
        $span:expr, $fmt:literal $(, $arg:expr)* $(,)?
        $(; hint $([$hint_span:expr])? : $hint:literal $(, $hint_arg:expr)*)*
        $(;)?
    ) => {{
        #[allow(unused_mut)]
        let mut warning = $crate::typst_library::diag::SourceDiagnostic::warning(
            $span,
            $crate::typst_library::diag::eco_format!($fmt $(, $arg)*)
        );
        // We use a recursive macro for hints to allow for optional spans.
        $($crate::typst_library::diag::error!(hint$([$hint_span])?: warning, $hint $(, $hint_arg)*);)*
        warning
    }};
}

// We want the `bail`, `error`, and `warning` macros and their documentation to
// be scoped locally to this module and imported like normal items, including by
// modules within this crate. However Rust only allows public macro_rules macros
// to be exported at the root of the crate, and gives us no tools to avoid that.
// See the "Import and Export" chapter of "The Little Book of Rust Macros" for
// more: <https://lukaswirth.dev/tlborm/decl-macros/minutiae/import-export.html>
//
// Our solution is simple: the actual macros are named with two underscores, and
// while they are available at the root of the crate, we add `doc(hidden)` to
// hide their docs at the crate root. We then we re-export them here with new
// names and `doc(inline)` so the preferred names and their documentation are
// scoped to this module.
//
// Unfortunately, `__bail` is still available at the crate root here and in all
// importers, but its name should suggest that we prefer to use `bail` instead.
// Note that the `disallowed_macros` lint does not handle re-exports like this.
//
// avenger: the macros are crate-private, so they are neither exported at the crate root nor
// documented there; the `__` names are kept so this module matches upstream.
#[rustfmt::skip]
pub(crate) use {
    __bail as bail,
    __error as error,
    __warning as warning,
};

/// A result that can carry multiple source errors. The recommended way to
/// create an error for this type is with the `bail!` macro.
pub type SourceResult<T> = Result<T, EcoVec<SourceDiagnostic>>;

/// An error or warning in a source or text file. The recommended way to create
/// one is with the `error!` or `warning!` macros.
///
/// The contained spans will only be detached if any of the input source files
/// were detached.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct SourceDiagnostic {
    /// Whether the diagnostic is an error or a warning.
    pub severity: Severity,
    /// The span of a relevant node in a Typst source file or the relevant byte
    /// range of an external file.
    pub span: DiagSpan,
    /// A diagnostic message describing the problem.
    pub message: EcoString,
    // avenger: no `trace`; labels have no function calls, show rules or imports to trace.
    /// Additional hints to the user.
    ///
    /// - When the span is `None`, these are generic hints. The CLI renders them
    ///   as a list at the bottom, each prefixed with `hint: `.
    ///
    /// - When a span is given, the hint is related to a secondary piece of code
    ///   and will be annotated at that code.
    pub hints: EcoVec<Spanned<EcoString, DiagSpan>>,
}

/// The severity of a [`SourceDiagnostic`].
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Severity {
    /// A fatal error.
    Error,
    /// A non-fatal warning.
    Warning,
}

impl SourceDiagnostic {
    /// Create a new, bare error.
    pub fn error(span: impl Into<DiagSpan>, message: impl Into<EcoString>) -> Self {
        Self {
            severity: Severity::Error,
            span: span.into(),
            message: message.into(),
            hints: eco_vec![],
        }
    }

    /// Create a new, bare warning.
    pub fn warning(span: impl Into<DiagSpan>, message: impl Into<EcoString>) -> Self {
        Self {
            severity: Severity::Warning,
            span: span.into(),
            message: message.into(),
            hints: eco_vec![],
        }
    }

    /// Adds a single hint to the diagnostic.
    pub fn hint(&mut self, hint: impl Into<EcoString>) {
        self.hints.push(Spanned::detached(hint.into()));
    }

    /// Adds a single hint specific to a source code location to the diagnostic.
    pub fn spanned_hint(
        &mut self,
        hint: impl Into<EcoString>,
        span: impl Into<DiagSpan>,
    ) {
        self.hints.push(Spanned::new(hint.into(), span.into()));
    }

    /// Adds a single hint to the diagnostic.
    pub fn with_hint(mut self, hint: impl Into<EcoString>) -> Self {
        self.hint(hint);
        self
    }

    /// Adds a single hint specific to a source code location to the diagnostic.
    pub fn with_spanned_hint(
        mut self,
        hint: impl Into<EcoString>,
        span: impl Into<DiagSpan>,
    ) -> Self {
        self.spanned_hint(hint, span);
        self
    }

    /// Adds multiple user-facing hints to the diagnostic.
    pub fn with_hints(mut self, hints: impl IntoIterator<Item = EcoString>) -> Self {
        self.hints.extend(hints.into_iter().map(Spanned::detached));
        self
    }
}

impl From<SyntaxDiagnostic> for SourceDiagnostic {
    fn from(syntax_diag: SyntaxDiagnostic) -> Self {
        let SyntaxDiagnostic { is_error, span, message, hints } = syntax_diag;
        Self {
            severity: if is_error { Severity::Error } else { Severity::Warning },
            span,
            message,
            hints,
        }
    }
}

/// Destination for a warning message.
pub trait WarningSink {
    /// Emits the message as a warning.
    fn emit(&mut self, message: HintedString);
}

impl WarningSink for () {
    fn emit(&mut self, _: HintedString) {}
}

impl WarningSink for (&mut Engine<'_>, Span) {
    fn emit(&mut self, hinted: HintedString) {
        self.0.sink.warn(
            SourceDiagnostic::warning(self.1, hinted.message())
                .with_hints(hinted.hints().iter().cloned()),
        );
    }
}

/// A result type with a string error message. The recommended way to create an
/// error for this type is with the [`bail!`] macro.
pub type StrResult<T> = Result<T, EcoString>;

/// Convert a [`StrResult`] or [`HintedStrResult`] to a [`SourceResult`] by
/// adding span information.
pub trait At<T> {
    /// Add the span information.
    fn at(self, span: Span) -> SourceResult<T>;
}

impl<T, S> At<T> for Result<T, S>
where
    S: Into<EcoString>,
{
    fn at(self, span: Span) -> SourceResult<T> {
        self.map_err(|message| eco_vec![SourceDiagnostic::error(span, message)])
    }
}

/// A result type with a string error message and hints. The recommended way to
/// create an error for this type is with the `bail!` macro.
pub type HintedStrResult<T> = Result<T, HintedString>;

/// A string message with hints. The recommended way to create one is with the
/// `error!` macro.
///
/// This is internally represented by a vector of strings.
/// - The first element of the vector contains the message.
/// - The remaining elements are the hints.
/// - This is done to reduce the size of a HintedString.
/// - The vector is guaranteed to not be empty.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct HintedString(EcoVec<EcoString>);

impl HintedString {
    /// Creates a new hinted string with the given message.
    pub fn new(message: EcoString) -> Self {
        Self(eco_vec![message])
    }

    /// A diagnostic message describing the problem.
    pub fn message(&self) -> &EcoString {
        self.0.first().unwrap()
    }

    /// Additional hints to the user, indicating how this error could be avoided
    /// or worked around.
    pub fn hints(&self) -> &[EcoString] {
        self.0.get(1..).unwrap_or(&[])
    }

    /// Adds a single hint to the hinted string.
    pub fn hint(&mut self, hint: impl Into<EcoString>) {
        self.0.push(hint.into());
    }

    /// Adds a single hint to the hinted string.
    pub fn with_hint(mut self, hint: impl Into<EcoString>) -> Self {
        self.hint(hint);
        self
    }

    /// Adds user-facing hints to the hinted string.
    pub fn with_hints(mut self, hints: impl IntoIterator<Item = EcoString>) -> Self {
        self.0.extend(hints);
        self
    }
}

impl<S> From<S> for HintedString
where
    S: Into<EcoString>,
{
    fn from(value: S) -> Self {
        Self::new(value.into())
    }
}

impl<T> At<T> for HintedStrResult<T> {
    fn at(self, span: Span) -> SourceResult<T> {
        self.map_err(|err| {
            let mut components = err.0.into_iter();
            let message = components.next().unwrap();
            let diag = SourceDiagnostic::error(span, message).with_hints(components);
            eco_vec![diag]
        })
    }
}

/// Enrich a [`StrResult`] or [`HintedStrResult`] with a hint.
pub trait Hint<T> {
    /// Add the hint.
    fn hint(self, hint: impl Into<EcoString>) -> HintedStrResult<T>;
}

impl<T, S> Hint<T> for Result<T, S>
where
    S: Into<EcoString>,
{
    fn hint(self, hint: impl Into<EcoString>) -> HintedStrResult<T> {
        self.map_err(|message| HintedString::new(message.into()).with_hint(hint))
    }
}

impl<T> Hint<T> for HintedStrResult<T> {
    fn hint(self, hint: impl Into<EcoString>) -> HintedStrResult<T> {
        self.map_err(|mut error| {
            error.hint(hint.into());
            error
        })
    }
}
