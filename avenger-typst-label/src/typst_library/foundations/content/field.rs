//! Ported from crates/typst-library/src/foundations/content/field.rs @ v0.15.1, modified for Avenger.
//!
//! The field accessors and settable-field machinery are upstream's. Avenger drops the field
//! vtables, since labels never access element fields from markup: `elem!` generates the field
//! names, equality and repr that upstream derives from them.

use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::sync::OnceLock;

use crate::typst_library::foundations::{
    Fold, FoldFn, NativeElement, Property, Resolve, StyleChain, Styles,
};

/// An accessor for the `I`-th field of the element `E`. Values of this type are
/// generated for each field of an element can be used to interact with this
/// field programmatically, for example to access the style chain, as in
/// `styles.get(TextElem::size)`.
#[derive(Copy, Clone)]
pub struct Field<E: NativeElement, const I: u8>(pub PhantomData<E>);

impl<E: NativeElement, const I: u8> Field<E, I> {
    /// Creates a new zero-sized accessor.
    pub const fn new() -> Self {
        Self(PhantomData)
    }

    /// The index of the projected field.
    pub const fn index(self) -> u8 {
        I
    }

    /// Creates a dynamic property instance for this field.
    ///
    /// Prefer [`Content::set`] or
    /// [`Styles::set`](crate::foundations::Styles::set) when working with
    /// existing content or style value.
    pub fn set(self, value: E::Type) -> Property
    where
        E: SettableProperty<I>,
        E::Type: Debug + Clone + Send + Sync + 'static,
    {
        Property::new(self, value)
    }
}

impl<E: NativeElement, const I: u8> Default for Field<E, I> {
    fn default() -> Self {
        Self::new()
    }
}

// avenger: no `RequiredField`, `SynthesizedField` or `ExternalField` metadata; they only fed the
// reflection vtables.

/// A field that has a default value and can be configured via a set rule, but
/// can also present on elements and be present in the constructor.
pub trait SettableField<const I: u8>: NativeElement {
    type Type: Clone;

    const FIELD: SettableFieldData<Self, I>;
}

/// Metadata and routines for a [`SettableField`].
// avenger: no field getters, which only fed the reflection vtables.
pub struct SettableFieldData<E: SettableField<I>, const I: u8> {
    property: SettablePropertyData<E, I>,
}

impl<E: SettableField<I>, const I: u8> SettableFieldData<E, I> {
    /// Creates the data from its parts. This is called in the `elem!` macro.
    pub const fn new(
        default: fn() -> E::Type,
        slot: fn() -> &'static OnceLock<E::Type>,
    ) -> Self {
        Self { property: SettablePropertyData::new(default, slot) }
    }

    /// Ensures that the property is folded on every access. See the
    /// documentation of the [`Fold`] trait for more details.
    pub const fn with_fold(mut self) -> Self
    where
        E::Type: Fold,
    {
        self.property.fold = Some(E::Type::fold);
        self
    }
}

/// A field that has a default value and can be configured via a set rule, but
/// is never present on elements.
///
/// This is provided for all `SettableField` impls through a blanket impl. In
/// the case of `#[ghost]` fields, which only live in the style chain and not in
/// elements, it is also implemented manually.
pub trait SettableProperty<const I: u8>: NativeElement {
    type Type: Clone;

    const FIELD: SettablePropertyData<Self, I>;
    const FOLD: Option<FoldFn<Self::Type>> = Self::FIELD.fold;

    /// Produces an instance of the property's default value.
    fn default() -> Self::Type {
        // Avoid recreating an expensive instance over and over, but also
        // avoid unnecessary lazy initialization for cheap types.
        if std::mem::needs_drop::<Self::Type>() {
            Self::default_ref().clone()
        } else {
            (Self::FIELD.default)()
        }
    }

    /// Produces a static reference to this property's default value.
    fn default_ref() -> &'static Self::Type {
        (Self::FIELD.slot)().get_or_init(Self::FIELD.default)
    }
}

impl<T, const I: u8> SettableProperty<I> for T
where
    T: SettableField<I>,
{
    type Type = <Self as SettableField<I>>::Type;

    const FIELD: SettablePropertyData<Self, I> =
        <Self as SettableField<I>>::FIELD.property;
}

/// Metadata and routines for a [`SettableProperty`].
pub struct SettablePropertyData<E: SettableProperty<I>, const I: u8> {
    default: fn() -> E::Type,
    slot: fn() -> &'static OnceLock<E::Type>,
    fold: Option<FoldFn<E::Type>>,
}

impl<E: SettableProperty<I>, const I: u8> SettablePropertyData<E, I> {
    /// Creates the data from its parts. This is called in the `elem!` macro.
    pub const fn new(
        default: fn() -> E::Type,
        slot: fn() -> &'static OnceLock<E::Type>,
    ) -> Self {
        Self { default, slot, fold: None }
    }

    /// Ensures that the property is folded on every access. See the
    /// documentation of the [`Fold`] trait for more details.
    pub const fn with_fold(self) -> Self
    where
        E::Type: Fold,
    {
        Self { fold: Some(E::Type::fold), ..self }
    }
}

/// A settable property that can be accessed by reference (because it is not
/// folded).
pub trait RefableProperty<const I: u8>: SettableProperty<I> {}

/// A settable field of an element.
///
/// The field can be in two states: Unset or present.
///
/// See [`StyleChain`] for more details about the available accessor methods.
#[derive(Copy, Clone)]
pub struct Settable<E: NativeElement, const I: u8>(Option<E::Type>)
where
    E: SettableProperty<I>;

impl<E: NativeElement, const I: u8> Settable<E, I>
where
    E: SettableProperty<I>,
{
    /// Creates a new unset instance.
    pub fn new() -> Self {
        Self(None)
    }

    /// Sets the instance to a value.
    pub fn set(&mut self, value: E::Type) {
        self.0 = Some(value);
    }

    /// Clears the value from the instance.
    pub fn unset(&mut self) {
        self.0 = None;
    }

    /// Views the type as an [`Option`] which is `Some` if the type is set
    /// and `None` if it is unset.
    pub fn as_option(&self) -> &Option<E::Type> {
        &self.0
    }

    /// Views the type as a mutable [`Option`].
    pub fn as_option_mut(&mut self) -> &mut Option<E::Type> {
        &mut self.0
    }

    /// Whether the field is set.
    pub fn is_set(&self) -> bool {
        self.0.is_some()
    }

    /// Retrieves the value given styles. The styles are used if the value is
    /// unset.
    pub fn get(&self, styles: StyleChain) -> E::Type
    where
        E::Type: Copy,
    {
        self.get_cloned(styles)
    }

    /// Retrieves and clones the value given styles. The styles are used if the
    /// value is unset or if it needs folding.
    pub fn get_cloned(&self, styles: StyleChain) -> E::Type {
        if let Some(fold) = E::FOLD {
            let mut res = styles.get_cloned::<E, I>(Field::new());
            if let Some(value) = &self.0 {
                res = fold(value.clone(), res);
            }
            res
        } else if let Some(value) = &self.0 {
            value.clone()
        } else {
            styles.get_cloned::<E, I>(Field::new())
        }
    }

    /// Retrieves a reference to the value given styles. The styles are used if
    /// the value is unset.
    pub fn get_ref<'a>(&'a self, styles: StyleChain<'a>) -> &'a E::Type
    where
        E: RefableProperty<I>,
    {
        if let Some(value) = &self.0 {
            value
        } else {
            styles.get_ref::<E, I>(Field::new())
        }
    }

    /// Retrieves the value and then immediately [resolves](Resolve) it.
    pub fn resolve(&self, styles: StyleChain) -> <E::Type as Resolve>::Output
    where
        E::Type: Resolve,
    {
        self.get_cloned(styles).resolve(styles)
    }

    /// Copies the field (if any) into the given styles.
    pub fn copy_into(&self, styles: &mut Styles)
    where
        E::Type: Debug + Send + Sync + 'static,
    {
        if let Some(value) = &self.0 {
            styles.set(Field::<E, I>::new(), value.clone());
        }
    }
}

impl<E: NativeElement, const I: u8> Debug for Settable<E, I>
where
    E: SettableProperty<I>,
    E::Type: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<E: NativeElement, const I: u8> Default for Settable<E, I>
where
    E: SettableProperty<I>,
{
    fn default() -> Self {
        Self(None)
    }
}

impl<E: NativeElement, const I: u8> From<Option<E::Type>> for Settable<E, I>
where
    E: SettableProperty<I>,
{
    fn from(value: Option<E::Type>) -> Self {
        Self(value)
    }
}

// avenger: no `FieldAccessError`; labels never access element fields from markup.
