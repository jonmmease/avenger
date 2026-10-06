//! Ported from crates/typst-library/src/foundations/content/mod.rs @ v0.15.1, modified for Avenger.
//!
//! A `Content` is an `Arc` of a type-erased [`Packed<T>`]: the element together with its span
//! and realization metadata. Downcasts go through `Any`, and capabilities through
//! [`Capability`], so no `unsafe` is needed. Labels, locations, recipes and field reflection are
//! out of scope.

mod element;
mod field;
mod packed;

pub use self::element::*;
pub use self::field::*;
pub use self::packed::Packed;

use std::any::Any;
use std::fmt::{self, Debug, Formatter};
use std::iter::Sum;
use std::ops::{Add, AddAssign};
use std::sync::{Arc, LazyLock};

use ecow::{EcoString, eco_format};

use crate::typst_library::foundations::{
    IntoValue, Property, Repr, Style, Styles, Value, elem, ty,
};
use crate::typst_library::math::Mathy;
use typst_syntax::Span;

/// A piece of document content.
///
/// This type is at the heart of Typst. All markup you write and most
/// @function[functions] you call produce content values. You can create a
/// content value by enclosing markup in square brackets. This is also how you
/// pass content to functions.
///
/// = Example <example>
/// ```example
/// Type of *Hello!* is
/// #type([*Hello!*])
/// ```
///
/// Content can be added with the `+` operator,
/// @reference:scripting:blocks[joined together] and multiplied with integers.
/// Wherever content is expected, you can also pass a @str[string] or `{none}`.
///
/// = Representation <representation>
/// Content consists of elements with fields. When constructing an element with
/// its _element function,_ you provide these fields as arguments and when you
/// have a content value, you can access its fields with
/// @reference:scripting:fields[field access syntax].
///
/// Some fields are required: These must be provided when constructing an
/// element and as a consequence, they are always available through field access
/// on content of that type. Required fields are marked as such in the
/// documentation.
///
/// Most fields are optional: Like required fields, they can be passed to the
/// element function to configure them for a single element. However, these can
/// also be configured with @reference:styling:set-rules[set rules] to apply
/// them to all elements within a scope. Optional fields are only available with
/// field access syntax when they were explicitly passed to the element
/// function, not when they result from a set rule.
///
/// Each element has a default appearance. However, you can also completely
/// customize its appearance with a @reference:styling:show-rules[show rule].
/// The show rule is passed the element. It can access the element's field and
/// produce arbitrary content from it.
///
/// In the web app, you can hover over a content variable to see exactly which
/// elements the content is composed of and what fields they have.
/// Alternatively, you can inspect the output of the @repr function.
#[derive(Clone)]
pub struct Content(pub(super) Arc<dyn Bounds>);

ty!(Content, name = "content", title = "Content", long = "content");

/// A type-erased `Packed<T>`.
// avenger: stands in for upstream's `RawContent` and its vtable.
pub(super) trait Bounds: Debug + Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn dyn_clone(&self) -> Arc<dyn Bounds>;
    fn dyn_elem(&self) -> Element;
    fn dyn_span(&self) -> Span;
    fn dyn_span_mut(&mut self) -> &mut Span;
    fn dyn_is_prepared(&self) -> bool;
    fn dyn_mark_prepared(&mut self);
    fn dyn_eq(&self, other: &Content) -> bool;
    fn dyn_repr(&self) -> EcoString;
    fn dyn_show_set(&self) -> Option<&(dyn ShowSet + 'static)>;
    fn dyn_mathy(&self) -> Option<&(dyn Mathy + 'static)>;
    fn dyn_synthesize(&self) -> Option<&(dyn Synthesize + 'static)>;
    fn dyn_synthesize_mut(&mut self) -> Option<&mut (dyn Synthesize + 'static)>;
}

impl<T: NativeElement> Bounds for Packed<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn dyn_clone(&self) -> Arc<dyn Bounds> {
        Arc::new(self.clone())
    }

    fn dyn_elem(&self) -> Element {
        T::ELEM
    }

    fn dyn_span(&self) -> Span {
        self.span()
    }

    fn dyn_span_mut(&mut self) -> &mut Span {
        self.span_mut()
    }

    fn dyn_is_prepared(&self) -> bool {
        self.is_prepared()
    }

    fn dyn_mark_prepared(&mut self) {
        self.mark_prepared()
    }

    fn dyn_eq(&self, other: &Content) -> bool {
        other.to_packed::<T>().is_some_and(|other| self == other)
    }

    fn dyn_repr(&self) -> EcoString {
        <T as NativeElement>::repr(self)
    }

    fn dyn_show_set(&self) -> Option<&(dyn ShowSet + 'static)> {
        T::as_show_set(self)
    }

    fn dyn_mathy(&self) -> Option<&(dyn Mathy + 'static)> {
        T::as_mathy(self)
    }

    fn dyn_synthesize(&self) -> Option<&(dyn Synthesize + 'static)> {
        T::as_synthesize(self)
    }

    fn dyn_synthesize_mut(&mut self) -> Option<&mut (dyn Synthesize + 'static)> {
        T::as_synthesize_mut(self)
    }
}

impl Content {
    /// Creates a new content from an element.
    pub fn new<T: NativeElement>(elem: T) -> Self {
        Packed::new(elem).pack()
    }

    /// Creates a empty sequence content.
    pub fn empty() -> Self {
        static EMPTY: LazyLock<Content> =
            LazyLock::new(|| SequenceElem::default().pack());
        EMPTY.clone()
    }

    /// Get the element of this content.
    pub fn elem(&self) -> Element {
        self.0.dyn_elem()
    }

    /// Get the span of the content.
    pub fn span(&self) -> Span {
        self.0.dyn_span()
    }

    /// Set the span of the content.
    pub fn spanned(mut self, span: Span) -> Self {
        if self.span().is_detached() {
            *self.make_mut().dyn_span_mut() = span;
        }
        self
    }

    // avenger: no labels, locations or recipe guards.

    /// Whether this content has already been prepared.
    pub fn is_prepared(&self) -> bool {
        self.0.dyn_is_prepared()
    }

    /// Mark this content as prepared.
    pub fn mark_prepared(&mut self) {
        self.make_mut().dyn_mark_prepared();
    }

    /// Create a new sequence element from multiples elements.
    pub fn sequence(iter: impl IntoIterator<Item = Self>) -> Self {
        let vec: Vec<_> = iter.into_iter().collect();
        if vec.is_empty() {
            Self::empty()
        } else if vec.len() == 1 {
            vec.into_iter().next().unwrap()
        } else {
            SequenceElem::new(vec).into()
        }
    }

    /// Whether the contained element is of type `T`.
    pub fn is<T: NativeElement>(&self) -> bool {
        self.0.as_any().is::<Packed<T>>()
    }

    /// Downcasts the element to a packed value.
    pub fn to_packed<T: NativeElement>(&self) -> Option<&Packed<T>> {
        Packed::from_ref(self)
    }

    /// Downcasts the element to a mutable packed value.
    pub fn to_packed_mut<T: NativeElement>(&mut self) -> Option<&mut Packed<T>> {
        Packed::from_mut(self)
    }

    /// Downcasts the element into an owned packed value.
    pub fn into_packed<T: NativeElement>(self) -> Result<Packed<T>, Self> {
        Packed::from_owned(self)
    }

    /// Extract the raw underlying element.
    pub fn unpack<T: NativeElement>(self) -> Result<T, Self> {
        self.into_packed::<T>().map(Packed::unpack)
    }

    /// Whether the contained element has the given capability.
    pub fn can<C>(&self) -> bool
    where
        C: ?Sized + Capability,
    {
        self.with::<C>().is_some()
    }

    /// Cast to a trait object if the contained element has the given
    /// capability.
    pub fn with<C>(&self) -> Option<&C>
    where
        C: ?Sized + Capability,
    {
        C::of(self)
    }

    /// Cast to a mutable trait object if the contained element has the given
    /// capability.
    pub fn with_mut<C>(&mut self) -> Option<&mut C>
    where
        C: ?Sized + CapabilityMut,
    {
        C::of_mut(self)
    }

    /// Whether the content is an empty sequence.
    pub fn is_empty(&self) -> bool {
        let Some(sequence) = self.to_packed::<SequenceElem>() else {
            return false;
        };

        sequence.children.is_empty()
    }

    /// Also auto expands sequence of sequences into flat sequence
    pub fn sequence_recursive_for_each<'a>(&'a self, f: &mut impl FnMut(&'a Self)) {
        if let Some(sequence) = self.to_packed::<SequenceElem>() {
            for child in &sequence.children {
                child.sequence_recursive_for_each(f);
            }
        } else {
            f(self);
        }
    }

    /// Repeat this content `count` times.
    pub fn repeat(&self, count: usize) -> Self {
        Self::sequence(std::iter::repeat_with(|| self.clone()).take(count))
    }

    /// Sets a style property on the content.
    pub fn set<E, const I: u8>(self, field: Field<E, I>, value: E::Type) -> Self
    where
        E: SettableProperty<I>,
        E::Type: Debug + Clone + Send + Sync + 'static,
    {
        self.styled(Property::new(field, value))
    }

    /// Style this content with a style entry.
    pub fn styled(mut self, style: impl Into<Style>) -> Self {
        if let Some(style_elem) = self.to_packed_mut::<StyledElem>() {
            style_elem.styles.apply_one(style.into());
            self
        } else {
            self.styled_with_map(style.into().into())
        }
    }

    /// Style this content with a full style map.
    pub fn styled_with_map(mut self, styles: Styles) -> Self {
        if styles.is_empty() {
            return self;
        }

        if let Some(style_elem) = self.to_packed_mut::<StyledElem>() {
            style_elem.styles.apply(styles);
            self
        } else {
            StyledElem::new(self, styles).into()
        }
    }

    /// Style this content with a full style map in-place.
    pub fn style_in_place(&mut self, styles: Styles) {
        if styles.is_empty() {
            return;
        }

        if let Some(style_elem) = self.to_packed_mut::<StyledElem>() {
            style_elem.styles.apply(styles);
        } else {
            *self = StyledElem::new(std::mem::take(self), styles).into();
        }
    }

    // avenger: no `query_first`, `plain_text` or `traverse`; labels have no introspection, and
    // only outlines, bibliographies, footnotes and links read plain text.

    /// The payload, cloned first if it is shared.
    fn make_mut(&mut self) -> &mut dyn Bounds {
        if Arc::get_mut(&mut self.0).is_none() {
            self.0 = self.0.dyn_clone();
        }
        Arc::get_mut(&mut self.0).unwrap()
    }
}

impl Content {
    /// Get the element of this content.
    // avenger: only `func`; upstream's other reflection accessors (`has`, `at`, `fields`) are
    // out of scope.
    pub fn func(&self) -> Element {
        self.elem()
    }
}

impl Default for Content {
    fn default() -> Self {
        Self::empty()
    }
}

impl Debug for Content {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: NativeElement> From<T> for Content {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        // Additional short circuit for different elements.
        self.elem() == other.elem() && self.0.dyn_eq(other)
    }
}

impl Repr for Content {
    // avenger: the generic `name(field: value, ..)` fallback is generated per element by `elem!`.
    fn repr(&self) -> EcoString {
        self.0.dyn_repr()
    }
}

impl Add for Content {
    type Output = Self;

    fn add(self, mut rhs: Self) -> Self::Output {
        let mut lhs = self;
        match (lhs.to_packed_mut::<SequenceElem>(), rhs.to_packed_mut::<SequenceElem>()) {
            (Some(seq_lhs), Some(rhs)) => {
                seq_lhs.children.extend(rhs.children.iter().cloned());
                lhs
            }
            (Some(seq_lhs), None) => {
                seq_lhs.children.push(rhs);
                lhs
            }
            (None, Some(rhs_seq)) => {
                rhs_seq.children.insert(0, lhs);
                rhs
            }
            (None, None) => Self::sequence([lhs, rhs]),
        }
    }
}

impl<'a> Add<&'a Self> for Content {
    type Output = Self;

    fn add(self, rhs: &'a Self) -> Self::Output {
        let mut lhs = self;
        match (lhs.to_packed_mut::<SequenceElem>(), rhs.to_packed::<SequenceElem>()) {
            (Some(seq_lhs), Some(rhs)) => {
                seq_lhs.children.extend(rhs.children.iter().cloned());
                lhs
            }
            (Some(seq_lhs), None) => {
                seq_lhs.children.push(rhs.clone());
                lhs
            }
            (None, Some(_)) => {
                let mut rhs = rhs.clone();
                rhs.to_packed_mut::<SequenceElem>().unwrap().children.insert(0, lhs);
                rhs
            }
            (None, None) => Self::sequence([lhs, rhs.clone()]),
        }
    }
}

impl AddAssign for Content {
    fn add_assign(&mut self, rhs: Self) {
        *self = std::mem::take(self) + rhs;
    }
}

impl AddAssign<&Self> for Content {
    fn add_assign(&mut self, rhs: &Self) {
        *self = std::mem::take(self) + rhs;
    }
}

impl Sum for Content {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Self::sequence(iter)
    }
}

elem! {
/// A sequence of content.
#[elem(name = "sequence", Debug, Repr)]
pub struct SequenceElem {
    /// The elements.
    #[required]
    pub children: Vec<Content>,
}
}

impl Debug for SequenceElem {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Sequence ")?;
        f.debug_list().entries(&self.children).finish()
    }
}

// Derive is currently incompatible with `elem` macro.
#[allow(clippy::derivable_impls)]
impl Default for SequenceElem {
    fn default() -> Self {
        Self { children: Default::default() }
    }
}

impl Repr for SequenceElem {
    fn repr(&self) -> EcoString {
        if self.children.is_empty() {
            "[]".into()
        } else {
            let elements = crate::typst_library::foundations::repr::pretty_array_like(
                &self.children.iter().map(|c| c.repr()).collect::<Vec<_>>(),
                false,
            );
            eco_format!("sequence{elements}")
        }
    }
}

elem! {
/// Content alongside styles.
#[elem(name = "styled", Debug, Repr, PartialEq)]
pub struct StyledElem {
    /// The content.
    #[required]
    pub child: Content,
    /// The styles.
    #[required]
    pub styles: Styles,
}
}

impl Debug for StyledElem {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        for style in self.styles.iter() {
            writeln!(f, "#{style:?}")?;
        }
        self.child.fmt(f)
    }
}

impl PartialEq for StyledElem {
    fn eq(&self, other: &Self) -> bool {
        self.child == other.child
    }
}

impl Repr for StyledElem {
    fn repr(&self) -> EcoString {
        eco_format!("styled(child: {}, ..)", self.child.repr())
    }
}

impl<T: NativeElement> IntoValue for T {
    fn into_value(self) -> Value {
        Value::Content(self.pack())
    }
}
