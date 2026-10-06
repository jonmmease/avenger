//! Ported from crates/typst-library/src/math/style.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a partial port so far; the math library adds the styling functions.

/// The size of elements in an equation.
///
/// See the TeXbook p. 141.
///
/// In MathML Core the attributes `displaystyle` and `scriptlevel` correspond
/// to the CSS properties `math-style` and `math-depth`.
/// - `displaystyle="true"` is equivalent to `math-style: normal`
/// - `displaystyle="false"` is equivalent to `math-style: compact`
/// - `scriptlevel="n"` is equivalent to `math-depth: n`
// avenger: no `Cast`; the size is internal, never given in markup.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum MathSize {
    /// Second-level sub- and superscripts.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `false` and `2`.
    ScriptScript,
    /// Sub- and superscripts.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `false` and `1`.
    Script,
    /// Math in text.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `false` and `0`.
    Text,
    /// Math on its own line.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `true` and `0`.
    Display,
}
