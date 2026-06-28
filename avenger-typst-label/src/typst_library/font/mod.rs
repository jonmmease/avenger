//! Retained Typst font model.
//!
//! Upstream Typst stores `FontWeight` as a numeric newtype in
//! `crates/typst-library/src/text/font/variant.rs`. Avenger labels keep
//! `Normal` and `Bold` variants in the public API for ergonomic chart theme
//! plumbing, then normalize to numeric weights at layout/font-selection
//! boundaries.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum FontWeight {
    Normal,
    Bold,
    Number(u16),
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

impl Default for FontStyle {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathFontBytesId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathFontSpec {
    LeteSansMath,
    NewComputerModernMath,
    Family(String),
    FontBytes(MathFontBytesId),
}

impl Default for MathFontSpec {
    fn default() -> Self {
        Self::LeteSansMath
    }
}
