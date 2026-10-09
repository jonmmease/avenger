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
mod datetime;
mod dict;
mod elem;
mod fields;
mod float;
mod func;
mod int;
mod module;
mod none;
mod scope;
mod str;
mod styles;
mod symbol;
mod ty;
mod value;

pub use self::args::*;
pub use self::array::*;
pub use self::auto::*;
pub use self::cast::*;
pub use self::content::*;
pub use self::datetime::*;
pub use self::dict::*;
pub use self::func::*;
pub use self::module::*;
pub use self::none::*;
pub use self::repr::Repr;
pub use self::scope::*;
pub use self::str::*;
pub use self::styles::*;
pub use self::symbol::*;
pub use self::ty::*;
pub use self::value::*;
pub(crate) use self::{
    array::array, cast::cast, cast::derive_cast, dict::dict, elem::elem, func::func,
    str::format_str, ty::ty,
};

#[doc(hidden)]
pub use self::elem::kebab_case;

/// Hook up all `foundations` definitions.
// avenger: only the float type, whose scope holds `inf` and `nan`.
pub(super) fn define(global: &mut Scope) {
    global.define_type::<f64>();
}
#[doc(hidden)]
pub use {ecow::eco_vec, indexmap::IndexMap};
