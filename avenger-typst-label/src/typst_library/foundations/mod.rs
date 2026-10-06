//! Ported from crates/typst-library/src/foundations/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Foundational types and functions.

pub mod ops;
pub mod repr;

mod args;
mod array;
mod auto;
mod bool;
mod cast;
mod content;
mod dict;
mod elem;
mod float;
mod int;
mod none;
mod str;
mod styles;
mod symbol;
mod ty;
mod value;

#[allow(unused_imports, reason = "evaluation constructs elements from arguments")]
pub use self::args::*;
pub use self::array::*;
pub use self::auto::*;
pub use self::cast::*;
pub use self::content::*;
pub use self::dict::*;
pub use self::none::*;
pub use self::repr::Repr;
pub use self::str::*;
pub use self::styles::*;
pub use self::symbol::*;
pub use self::ty::*;
pub use self::value::*;
pub(crate) use self::{
    array::array, cast::cast, cast::derive_cast, dict::dict, elem::elem, ty::ty,
};

#[doc(hidden)]
pub use self::elem::kebab_case;
#[doc(hidden)]
pub use {ecow::eco_vec, indexmap::IndexMap};
