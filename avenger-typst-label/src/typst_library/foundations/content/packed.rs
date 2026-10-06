//! Ported from crates/typst-library/src/foundations/content/packed.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: upstream's `Packed<T>` is a `repr(transparent)` wrapper around `Content` that is
//! cast to and from it unsafely. Avenger's is the payload a `Content` holds: the element with the
//! span and realization metadata. `Content::to_packed` therefore downcasts with `Any` instead of
//! transmuting, and returns a reference into the content as upstream's does. A `Packed<T>`
//! clones its element where upstream clones an `Arc`.

use std::fmt::{self, Debug, Formatter};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::typst_library::foundations::{Content, NativeElement};
use typst_syntax::Span;

/// A packed element of a static type.
#[derive(Clone)]
pub struct Packed<T: NativeElement> {
    /// The element.
    elem: T,
    /// The span of the element.
    span: Span,
    /// Whether realization has already prepared the element.
    prepared: bool,
}

impl<T: NativeElement> Packed<T> {
    /// Pack element while retaining its static type.
    pub fn new(element: T) -> Self {
        Self {
            elem: element,
            span: Span::detached(),
            prepared: false,
        }
    }

    /// Try to cast type-erased content into a statically known packed element.
    pub fn from_ref(content: &Content) -> Option<&Self> {
        content.0.as_any().downcast_ref()
    }

    /// Try to cast mutably type-erased content into a statically known packed
    /// element.
    // avenger: clones the element first if the content is shared, like `Arc::make_mut`.
    pub fn from_mut(content: &mut Content) -> Option<&mut Self> {
        if !content.is::<T>() {
            return None;
        }
        if Arc::get_mut(&mut content.0).is_none() {
            content.0 = content.0.dyn_clone();
        }
        Arc::get_mut(&mut content.0)?.as_any_mut().downcast_mut()
    }

    /// Try to cast type-erased content into a statically known packed element.
    pub fn from_owned(content: Content) -> Result<Self, Content> {
        match Packed::<T>::from_ref(&content) {
            Some(packed) => Ok(packed.clone()),
            None => Err(content),
        }
    }

    /// Pack back into content.
    pub fn pack(self) -> Content {
        Content(Arc::new(self))
    }

    // avenger: no `pack_ref`/`pack_mut`; a `Packed<T>` is inside its content, not a view of it.

    /// Extract the raw underlying element.
    pub fn unpack(self) -> T {
        self.elem
    }

    /// The element's span.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Set the span of the element.
    pub fn spanned(mut self, span: Span) -> Self {
        if self.span.is_detached() {
            self.span = span;
        }
        self
    }

    // avenger: no `label`, `location` or `set_location`; labels have no introspection.

    /// Mutable access to the span, for `Content::spanned`.
    pub(super) fn span_mut(&mut self) -> &mut Span {
        &mut self.span
    }

    /// Whether realization has already prepared the element.
    pub(super) fn is_prepared(&self) -> bool {
        self.prepared
    }

    /// Marks the element as prepared.
    pub(super) fn mark_prepared(&mut self) {
        self.prepared = true;
    }
}

impl<T: NativeElement> AsRef<T> for Packed<T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: NativeElement> AsMut<T> for Packed<T> {
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: NativeElement> Deref for Packed<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.elem
    }
}

impl<T: NativeElement> DerefMut for Packed<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.elem
    }
}

// avenger: compares the elements, as upstream's content comparison does; spans are ignored.
impl<T: NativeElement> PartialEq for Packed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.elem == other.elem
    }
}

impl<T: NativeElement + Debug> Debug for Packed<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.elem.fmt(f)
    }
}
