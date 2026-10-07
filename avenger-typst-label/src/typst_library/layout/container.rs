//! Ported from crates/typst-library/src/layout/container.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: only the inline element, which lays out equations in a line of text. Labels have
//! no boxes or blocks. The inline element's layouter takes no locator or region, since labels
//! have no introspection and an equation uses the region only for boxes and external content.

use crate::typst_library::diag::{SourceResult, bail};
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{
    Args, Construct, Content, NativeElement, Packed, StyleChain, elem,
};
use crate::typst_library::layout::{Abs, Frame};

elem! {
/// An inline-level container that can produce arbitrary items that can break
/// across lines.
#[elem(name = "inline", Construct)]
pub struct InlineElem {
    /// A callback that is invoked with the regions to produce arbitrary inline
    /// items.
    #[required]
    #[internal]
    body: callbacks::InlineCallback,
}
}

impl Construct for InlineElem {
    fn construct(_: &mut Engine, args: &mut Args) -> SourceResult<Content> {
        bail!(args.span, "cannot be constructed manually");
    }
}

impl InlineElem {
    /// Create an inline-level item with a custom layouter.
    // avenger: the callback takes the captured element as content and downcasts it, where
    // upstream transmutes a callback over `Packed<T>`, so no `unsafe` is needed.
    #[allow(clippy::type_complexity)]
    pub fn layouter<T: NativeElement>(
        captured: Packed<T>,
        callback: fn(
            content: &Content,
            engine: &mut Engine,
            styles: StyleChain,
        ) -> SourceResult<Vec<InlineItem>>,
    ) -> Self {
        Self::new(callbacks::InlineCallback::new(captured, callback))
    }
}

impl Packed<InlineElem> {
    /// Layout the element.
    pub fn layout(
        &self,
        engine: &mut Engine,
        styles: StyleChain,
    ) -> SourceResult<Vec<InlineItem>> {
        self.body.call(engine, styles)
    }

    /// The element the layouter was created for.
    // avenger: for tests that inspect the equation an inline element lays out.
    #[cfg(test)]
    pub fn captured(&self) -> &Content {
        self.body.captured()
    }
}

/// Layouted items suitable for placing in a paragraph.
#[derive(Debug, Clone)]
pub enum InlineItem {
    /// Absolute spacing between other items, and whether it is weak.
    Space(Abs, bool),
    /// Layouted inline-level content.
    Frame(Frame),
}

/// Callbacks for inline layout.
// avenger: only the inline callback, without `unsafe`.
mod callbacks {
    use super::*;

    /// A callback that lays out captured content into inline items.
    #[derive(Debug, Clone)]
    pub struct InlineCallback {
        captured: Content,
        f: fn(&Content, &mut Engine, StyleChain) -> SourceResult<Vec<InlineItem>>,
    }

    impl InlineCallback {
        pub fn new<T: NativeElement>(
            captured: Packed<T>,
            f: fn(&Content, &mut Engine, StyleChain) -> SourceResult<Vec<InlineItem>>,
        ) -> Self {
            Self { captured: captured.pack(), f }
        }

        pub fn call(
            &self,
            engine: &mut Engine,
            styles: StyleChain,
        ) -> SourceResult<Vec<InlineItem>> {
            (self.f)(&self.captured, engine, styles)
        }

        #[cfg(test)]
        pub fn captured(&self) -> &Content {
            &self.captured
        }
    }

    impl PartialEq for InlineCallback {
        fn eq(&self, other: &Self) -> bool {
            // Comparing function pointers is problematic. Since for
            // each type of content, there is typically just one
            // callback, we skip it. It barely matters anyway since
            // getting into a comparison codepath for inline & block
            // elements containing callback bodies is close to
            // impossible (as these are generally generated in show
            // rules).
            self.captured.eq(&other.captured)
        }
    }
}
