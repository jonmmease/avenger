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
#[derive(Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
    Number(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathFontBytesId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum MathFontSpec {
    #[default]
    LeteSansMath,
    NewComputerModernMath,
    Family(String),
    FontBytes(MathFontBytesId),
}
