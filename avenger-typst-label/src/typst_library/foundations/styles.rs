//! Ported from crates/typst-library/src/foundations/styles.rs @ v0.15.1, modified for Avenger.
//!
//! Avenger keeps style properties only: labels have no set or show rules, so there are no
//! recipes, revocations, or page-level lifting. Nothing memoizes over styles, so styles are not
//! hashed (`LazyHash` and the `Hash` bounds are gone). Pointer equality of style chains is kept;
//! realization depends on it.

use std::any::Any;
use std::fmt::{self, Debug, Formatter};
use std::{mem, ptr};

use ecow::{EcoVec, eco_vec};
use smallvec::SmallVec;

use crate::typst_library::foundations::{
    Element, Field, NativeElement, RefableProperty, SettableProperty,
};
use crate::typst_syntax::Span;

/// A list of style properties.
#[derive(Default, Clone)]
pub struct Styles(EcoVec<Style>);

impl Styles {
    /// Create a new, empty style list.
    pub const fn new() -> Self {
        Self(EcoVec::new())
    }

    /// Whether this contains no styles.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over the contained styles.
    pub fn iter(&self) -> impl Iterator<Item = &Style> {
        self.0.iter()
    }

    /// Iterate over the contained styles.
    pub fn as_slice(&self) -> &[Style] {
        self.0.as_slice()
    }

    /// Set an inner value for a style property.
    ///
    /// If the property needs folding and the value is already contained in the
    /// style map, `self` contributes the outer values and `value` is the inner
    /// one.
    pub fn set<E, const I: u8>(&mut self, field: Field<E, I>, value: E::Type)
    where
        E: SettableProperty<I>,
        E::Type: Debug + Clone + Send + Sync + 'static,
    {
        self.push(Property::new(field, value));
    }

    /// Add a new style to the list.
    pub fn push(&mut self, style: impl Into<Style>) {
        self.0.push(style.into());
    }

    /// Remove the style that was last set.
    pub fn unset(&mut self) {
        self.0.pop();
    }

    /// Apply outer styles. Like [`chain`](StyleChain::chain), but in-place.
    pub fn apply(&mut self, mut outer: Self) {
        outer.0.extend(mem::take(self).0);
        *self = outer;
    }

    /// Apply one outer styles.
    pub fn apply_one(&mut self, outer: Style) {
        self.0.insert(0, outer);
    }

    /// Add an origin span to all contained properties.
    pub fn spanned(mut self, span: Span) -> Self {
        for entry in self.0.make_mut() {
            let Style::Property(property) = entry;
            property.span = span;
        }
        self
    }
}

impl<const N: usize> From<[Style; N]> for Styles {
    fn from(arr: [Style; N]) -> Self {
        Self(arr.into())
    }
}

impl From<Style> for Styles {
    fn from(style: Style) -> Self {
        Self(eco_vec![style])
    }
}

impl IntoIterator for Styles {
    type Item = Style;
    type IntoIter = ecow::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<Style> for Styles {
    fn from_iter<T: IntoIterator<Item = Style>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Debug for Styles {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str("Styles ")?;
        f.debug_list().entries(&self.0).finish()
    }
}

/// A single style property or recipe.
#[derive(Clone)]
pub enum Style {
    /// A style property originating from a set rule or constructor.
    Property(Property),
    // avenger: no `Recipe` or `Revocation`; labels have no show rules.
}

impl Style {
    /// If this is a property, return it.
    pub fn property(&self) -> Option<&Property> {
        match self {
            Self::Property(property) => Some(property),
        }
    }

    /// The style's span, if any.
    pub fn span(&self) -> Span {
        match self {
            Self::Property(property) => property.span,
        }
    }

    /// Returns `Some(_)` with an optional span if this style is for
    /// the given element.
    pub fn element(&self) -> Option<Element> {
        match self {
            Style::Property(property) => Some(property.elem),
        }
    }

    /// Turn this style into prehashed style.
    // avenger: styles are not hashed, so this is the identity.
    pub fn wrap(self) -> Style {
        self
    }
}

impl Debug for Style {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::Property(property) => property.fmt(f),
        }
    }
}

impl From<Property> for Style {
    fn from(property: Property) -> Self {
        Self::Property(property)
    }
}

/// A style property originating from a set rule or constructor.
#[derive(Clone)]
pub struct Property {
    /// The element the property belongs to.
    elem: Element,
    /// The property's ID.
    id: u8,
    /// The property's value.
    value: Block,
    /// The span of the set rule the property stems from.
    span: Span,
    // avenger: no `liftable` or `outside`; labels have no pages to lift styles to.
}

impl Property {
    /// Create a new property from a key-value pair.
    pub fn new<E, const I: u8>(_: Field<E, I>, value: E::Type) -> Self
    where
        E: SettableProperty<I>,
        E::Type: Debug + Clone + Send + Sync + 'static,
    {
        Self {
            elem: E::ELEM,
            id: I,
            value: Block::new(value),
            span: Span::detached(),
        }
    }

    /// Whether this property is the given one.
    pub fn is(&self, elem: Element, id: u8) -> bool {
        self.elem == elem && self.id == id
    }

    /// Whether this property belongs to the given element.
    pub fn is_of(&self, elem: Element) -> bool {
        self.elem == elem
    }

    /// Turn this property into prehashed style.
    // avenger: styles are not hashed, so this only wraps the property.
    pub fn wrap(self) -> Style {
        Style::Property(self).wrap()
    }
}

impl Debug for Property {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "Set({}.{}: ",
            self.elem.name(),
            self.elem.field_name(self.id).unwrap_or("internal")
        )?;
        self.value.fmt(f)?;
        write!(f, ")")
    }
}

/// A block storage for storing style values.
///
/// We're using a `Box` since values will either be contained in an `Arc` and
/// therefore already on the heap or they will be small enough that we can just
/// clone them.
struct Block(Box<dyn Blockable>);

impl Block {
    /// Creates a new block.
    fn new<T: Blockable>(value: T) -> Self {
        Self(Box::new(value))
    }

    /// Downcasts the block to the specified type.
    fn downcast<T: 'static>(&self, func: Element, id: u8) -> &T {
        let inner: &dyn Blockable = &*self.0;
        (inner as &dyn Any)
            .downcast_ref()
            .unwrap_or_else(|| block_wrong_type(func, id, self))
    }
}

impl Debug for Block {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Clone for Block {
    fn clone(&self) -> Self {
        self.0.dyn_clone()
    }
}

/// A value that can be stored in a block.
///
/// Auto derived for all types that implement [`Any`], [`Clone`], [`Debug`],
/// [`Send`] and [`Sync`].
// avenger: no `Hash` bound and no `dyn_hash`; nothing hashes styles.
trait Blockable: Debug + Any + Send + Sync + 'static {
    /// Equivalent to [`Clone`] for the block.
    fn dyn_clone(&self) -> Block;
}

impl<T: Debug + Clone + Send + Sync + 'static> Blockable for T {
    fn dyn_clone(&self) -> Block {
        Block(Box::new(self.clone()))
    }
}

/// A chain of styles, similar to a linked list.
///
/// A style chain allows to combine properties from multiple style lists in a
/// element hierarchy in a non-allocating way. Rather than eagerly merging the
/// lists, each access walks the hierarchy from the innermost to the outermost
/// map, trying to find a match and then folding it with matches further up the
/// chain.
#[derive(Default, Copy, Clone)]
pub struct StyleChain<'a> {
    /// The first link of this chain.
    head: &'a [Style],
    /// The remaining links in the chain.
    tail: Option<&'a Self>,
}

impl<'a> StyleChain<'a> {
    /// Start a new style chain with root styles.
    pub fn new(root: &'a Styles) -> Self {
        Self { head: &root.0, tail: None }
    }

    /// Retrieves the value of the given field from the style chain.
    ///
    /// A `Field` value is a zero-sized value that specifies which field of an
    /// element you want to retrieve on the type-system level. It also ensures
    /// that Rust can infer the correct return type.
    ///
    /// Should be preferred over [`get_cloned`](Self::get_cloned) or
    /// [`get_ref`](Self::get_ref), but is only available for [`Copy`] types.
    /// For other types an explicit decision needs to be made whether cloning is
    /// necessary.
    pub fn get<E, const I: u8>(self, field: Field<E, I>) -> E::Type
    where
        E: SettableProperty<I>,
        E::Type: Copy,
    {
        self.get_cloned(field)
    }

    /// Retrieves and clones the value from the style chain.
    ///
    /// Prefer [`get`](Self::get) if the type is `Copy` and
    /// [`get_ref`](Self::get_ref) if a reference suffices.
    pub fn get_cloned<E, const I: u8>(self, _: Field<E, I>) -> E::Type
    where
        E: SettableProperty<I>,
    {
        if let Some(fold) = E::FOLD {
            self.get_folded::<E::Type>(E::ELEM, I, fold, E::default())
        } else {
            self.get_unfolded::<E::Type>(E::ELEM, I)
                .cloned()
                .unwrap_or_else(E::default)
        }
    }

    /// Retrieves a reference to the value of the given field from the style
    /// chain.
    ///
    /// Not possible if the value needs folding.
    pub fn get_ref<E, const I: u8>(self, _: Field<E, I>) -> &'a E::Type
    where
        E: RefableProperty<I>,
    {
        self.get_unfolded(E::ELEM, I).unwrap_or_else(|| E::default_ref())
    }

    /// Retrieves the value and then immediately [resolves](Resolve) it.
    pub fn resolve<E, const I: u8>(
        self,
        field: Field<E, I>,
    ) -> <E::Type as Resolve>::Output
    where
        E: SettableProperty<I>,
        E::Type: Resolve,
    {
        self.get_cloned(field).resolve(self)
    }

    /// Whether there is a style for the given field of the given element.
    pub fn has<E: NativeElement, const I: u8>(&self, _: Field<E, I>) -> bool {
        let elem = E::ELEM;
        self.entries()
            .filter_map(|style| style.property())
            .any(|property| property.is_of(elem) && property.id == I)
    }

    /// Retrieves a reference to a field, also taking into account the
    /// instance's value if any.
    fn get_unfolded<T: 'static>(self, func: Element, id: u8) -> Option<&'a T> {
        self.find(func, id).map(|block| block.downcast(func, id))
    }

    /// Retrieves a reference to a field, also taking into account the
    /// instance's value if any.
    fn get_folded<T: 'static + Clone>(
        self,
        func: Element,
        id: u8,
        fold: fn(T, T) -> T,
        default: T,
    ) -> T {
        let iter = self
            .properties(func, id)
            .map(|block| block.downcast::<T>(func, id).clone());

        if let Some(folded) = iter.reduce(fold) { fold(folded, default) } else { default }
    }

    /// Iterate over all values for the given property in the chain.
    fn find(self, func: Element, id: u8) -> Option<&'a Block> {
        self.properties(func, id).next()
    }

    /// Iterate over all values for the given property in the chain.
    fn properties(self, func: Element, id: u8) -> impl Iterator<Item = &'a Block> {
        self.entries()
            .filter_map(|style| style.property())
            .filter(move |property| property.is(func, id))
            .map(|property| &property.value)
    }

    /// Make the given chainable the first link of this chain.
    ///
    /// The resulting style chain contains styles from `local` as well as
    /// `self`. The ones from `local` take precedence over the ones from
    /// `self`. For folded properties `local` contributes the inner value.
    pub fn chain<'b, C>(&'b self, local: &'b C) -> StyleChain<'b>
    where
        C: Chainable + ?Sized,
    {
        Chainable::chain(local, self)
    }

    /// Iterate over the entries of the chain.
    pub fn entries(self) -> Entries<'a> {
        Entries { inner: [].as_slice().iter(), links: self.links() }
    }

    /// Iterate over the links of the chain.
    pub fn links(self) -> Links<'a> {
        Links(Some(self))
    }

    /// Convert to a style map.
    pub fn to_map(self) -> Styles {
        let mut styles: EcoVec<_> = self.entries().cloned().collect();
        styles.make_mut().reverse();
        Styles(styles)
    }

    /// Build owned styles from the suffix (all links beyond the `len`) of the
    /// chain.
    pub fn suffix(self, len: usize) -> Styles {
        let mut styles = EcoVec::new();
        let take = self.links().count().saturating_sub(len);
        for link in self.links().take(take) {
            styles.extend(link.iter().cloned().rev());
        }
        styles.make_mut().reverse();
        Styles(styles)
    }

    /// Remove the last link from the chain.
    pub fn pop(&mut self) {
        *self = self.tail.copied().unwrap_or_default();
    }

    /// Determine the shared trunk of a collection of style chains.
    pub fn trunk(iter: impl IntoIterator<Item = Self>) -> Option<Self> {
        // Determine shared style depth and first span.
        let mut iter = iter.into_iter();
        let mut trunk = iter.next()?;
        let mut depth = trunk.links().count();

        for mut chain in iter {
            let len = chain.links().count();
            if len < depth {
                for _ in 0..depth - len {
                    trunk.pop();
                }
                depth = len;
            } else if len > depth {
                for _ in 0..len - depth {
                    chain.pop();
                }
            }

            while depth > 0 && chain != trunk {
                trunk.pop();
                chain.pop();
                depth -= 1;
            }
        }

        Some(trunk)
    }
}

impl Debug for StyleChain<'_> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str("StyleChain ")?;
        f.debug_list()
            .entries(self.entries().collect::<Vec<_>>().into_iter().rev())
            .finish()
    }
}

impl PartialEq for StyleChain<'_> {
    fn eq(&self, other: &Self) -> bool {
        ptr::eq(self.head, other.head)
            && match (self.tail, other.tail) {
                (Some(a), Some(b)) => ptr::eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// Things that can be attached to a style chain.
pub trait Chainable {
    /// Attach `self` as the first link of the chain.
    fn chain<'a>(&'a self, outer: &'a StyleChain<'_>) -> StyleChain<'a>;
}

impl Chainable for Style {
    fn chain<'a>(&'a self, outer: &'a StyleChain<'_>) -> StyleChain<'a> {
        StyleChain {
            head: std::slice::from_ref(self),
            tail: Some(outer),
        }
    }
}

impl Chainable for [Style] {
    fn chain<'a>(&'a self, outer: &'a StyleChain<'_>) -> StyleChain<'a> {
        if self.is_empty() {
            *outer
        } else {
            StyleChain { head: self, tail: Some(outer) }
        }
    }
}

impl<const N: usize> Chainable for [Style; N] {
    fn chain<'a>(&'a self, outer: &'a StyleChain<'_>) -> StyleChain<'a> {
        Chainable::chain(self.as_slice(), outer)
    }
}

impl Chainable for Styles {
    fn chain<'a>(&'a self, outer: &'a StyleChain<'_>) -> StyleChain<'a> {
        Chainable::chain(self.0.as_slice(), outer)
    }
}

/// An iterator over the entries in a style chain.
pub struct Entries<'a> {
    inner: std::slice::Iter<'a, Style>,
    links: Links<'a>,
}

impl<'a> Iterator for Entries<'a> {
    type Item = &'a Style;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(entry) = self.inner.next_back() {
                return Some(entry);
            }

            match self.links.next() {
                Some(next) => self.inner = next.iter(),
                None => return None,
            }
        }
    }
}

/// An iterator over the links of a style chain.
pub struct Links<'a>(Option<StyleChain<'a>>);

impl<'a> Iterator for Links<'a> {
    type Item = &'a [Style];

    fn next(&mut self) -> Option<Self::Item> {
        let StyleChain { head, tail } = self.0?;
        self.0 = tail.copied();
        Some(head)
    }
}

/// A property that is resolved with other properties from the style chain.
pub trait Resolve {
    /// The type of the resolved output.
    type Output;

    /// Resolve the value using the style chain.
    fn resolve(self, styles: StyleChain) -> Self::Output;
}

impl<T: Resolve> Resolve for Option<T> {
    type Output = Option<T::Output>;

    fn resolve(self, styles: StyleChain) -> Self::Output {
        self.map(|v| v.resolve(styles))
    }
}

/// A property that is folded to determine its final value.
///
/// In the example below, the chain of stroke values is folded into a single
/// value: `4pt + red`.
///
/// ```example
/// #set rect(stroke: red)
/// #set rect(stroke: 4pt)
/// #rect()
/// ```
///
/// Note: Folding must be associative, i.e. any implementation must satisfy
/// `fold(fold(a, b), c) == fold(a, fold(b, c))`.
pub trait Fold {
    /// Fold this inner value with an outer folded value.
    fn fold(self, outer: Self) -> Self;
}

impl Fold for bool {
    fn fold(self, _: Self) -> Self {
        self
    }
}

impl<T: Fold> Fold for Option<T> {
    fn fold(self, outer: Self) -> Self {
        match (self, outer) {
            (Some(inner), Some(outer)) => Some(inner.fold(outer)),
            // An explicit `None` should be respected, thus we don't do
            // `inner.or(outer)`.
            (inner, _) => inner,
        }
    }
}

impl<T> Fold for Vec<T> {
    fn fold(self, mut outer: Self) -> Self {
        outer.extend(self);
        outer
    }
}

impl<T, const N: usize> Fold for SmallVec<[T; N]> {
    fn fold(self, mut outer: Self) -> Self {
        outer.extend(self);
        outer
    }
}

/// A [folding](Fold) function.
pub type FoldFn<T> = fn(T, T) -> T;

/// A variant of fold for foldable optional (`Option<T>`) values where an inner
/// `None` value isn't respected (contrary to `Option`'s usual `Fold`
/// implementation, with which folding with an inner `None` always returns
/// `None`). Instead, when either of the `Option` objects is `None`, the other
/// one is necessarily returned by `fold_or`. Normal folding still occurs when
/// both values are `Some`, using `T`'s `Fold` implementation.
///
/// This is useful when `None` in a particular context means "unspecified"
/// rather than "absent", in which case a specified value (`Some`) is chosen
/// over an unspecified one (`None`), while two specified values are folded
/// together.
pub trait AlternativeFold {
    /// Attempts to fold this inner value with an outer value. However, if
    /// either value is `None`, returns the other one instead of folding.
    fn fold_or(self, outer: Self) -> Self;
}

impl<T: Fold> AlternativeFold for Option<T> {
    fn fold_or(self, outer: Self) -> Self {
        match (self, outer) {
            (Some(inner), Some(outer)) => Some(inner.fold(outer)),
            // If one of values is `None`, return the other one instead of
            // folding.
            (inner, outer) => inner.or(outer),
        }
    }
}

/// A type that accumulates depth when folded.
#[derive(Debug, Default, Copy, Clone, PartialEq, Hash)]
pub struct Depth(pub usize);

impl Fold for Depth {
    fn fold(self, outer: Self) -> Self {
        Self(outer.0 + self.0)
    }
}

#[cold]
fn block_wrong_type(func: Element, id: u8, value: &Block) -> ! {
    panic!(
        "attempted to read a value of a different type than was written {}.{}: {:?}",
        func.name(),
        func.field_name(id).unwrap(),
        value
    )
}
