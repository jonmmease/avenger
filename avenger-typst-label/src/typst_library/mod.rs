//! Ported from crates/typst-library/src/lib.rs @ v0.15.1, modified for Avenger.
//!
//! Typst's standard library: the subset a single label line uses.

pub mod diag;
pub mod engine;
pub mod foundations;
pub mod layout;
pub mod math;
pub mod model;
pub mod routines;
pub mod symbols;
pub mod text;
pub mod visualize;

use std::sync::LazyLock;

use crate::typst_library::foundations::{Module, Scope};
use crate::typst_library::layout::{Alignment, Dir};
use crate::typst_library::text::{Font, FontBook};
use crate::typst_library::visualize::Color;
use crate::typst_syntax::FileId;

/// The environment in which typesetting occurs.
///
/// All loading functions (`main`, `source`, `file`, `font`) should perform
/// internal caching so that they are relatively cheap on repeated invocations
/// with the same argument. [`Source`], [`Bytes`], and [`Font`] are
/// all reference-counted and thus cheap to clone.
///
/// The compiler doesn't do the caching itself because the world has much more
/// information on when something can change. For example, fonts typically don't
/// change and can thus even be cached across multiple compilations (for
/// long-running applications like `typst watch`). Source files on the other
/// hand can change and should thus be cleared after each compilation. Advanced
/// clients like language servers can also retain the source files and
/// [edit](Source::edit) them in-place to benefit from better incremental
/// performance.
// avenger: fonts and the label's source text, since a label has no other files, library,
// packages or dates to load.
pub trait World: Send + Sync {
    /// Metadata about all known fonts.
    fn book(&self) -> &FontBook;

    /// Try to access the specified source file.
    // avenger: the source text, since there is no `Source` type.
    fn source(&self, id: FileId) -> Option<&str>;

    /// Try to access the font with the given index in the font book.
    ///
    /// Note that the index is not guaranteed to be in bounds of the font book
    /// returned by this world's `book()` function. This is the case because
    /// this function may be invoked with indices from an outdated or different
    /// font book during incremental compilation validation.
    fn font(&self, index: usize) -> Option<Font>;
}

/// Definition of Typst's standard library.
///
/// To create and configure the standard library, use the `LibraryExt` trait
/// and call
/// - `Library::default()` for a standard configuration
/// - `Library::builder().build()` if you want to customize the library
// avenger: the definitions a label can use, built once: no `std` module, inputs, features,
// default styles or show rules.
#[derive(Debug, Clone)]
pub struct Library {
    /// The module that contains the definitions that are available everywhere.
    pub global: Module,
    /// The module that contains the definitions available in math mode.
    pub math: Module,
}

impl Library {
    /// The label library.
    pub fn get() -> &'static Library {
        static LIBRARY: LazyLock<Library> = LazyLock::new(|| {
            let math = math::module();
            let global = global(math.clone());
            Library { global, math }
        });
        &LIBRARY
    }
}

/// Construct the module with global definitions.
// avenger: the strong and emph model elements, the text elements and functions, symbols,
// math, and Avenger's formatting functions.
fn global(math: Module) -> Module {
    let mut global = Scope::deduplicating();

    self::model::define(&mut global);
    self::text::define(&mut global);
    self::symbols::define(&mut global);

    global.define("math", math);

    prelude(&mut global);

    Module::new("global", global)
}

/// Defines scoped values that are globally available, too.
// avenger: the CSS named colors in place of Typst's (D22); no `oklab`, `oklch`, `cmyk` or
// `range`.
fn prelude(global: &mut Scope) {
    for (name, rgba) in avenger_color::css_named_colors() {
        global.define(name, Color::from_rgba(rgba));
    }
    global.define_func::<self::visualize::luma>();
    global.define_func::<self::visualize::rgb>();
    global.define("ltr", Dir::LTR);
    global.define("rtl", Dir::RTL);
    global.define("ttb", Dir::TTB);
    global.define("btt", Dir::BTT);
    global.define("start", Alignment::START);
    global.define("left", Alignment::LEFT);
    global.define("center", Alignment::CENTER);
    global.define("right", Alignment::RIGHT);
    global.define("end", Alignment::END);
    global.define("top", Alignment::TOP);
    global.define("horizon", Alignment::HORIZON);
    global.define("bottom", Alignment::BOTTOM);
}

#[cfg(test)]
mod tests;
