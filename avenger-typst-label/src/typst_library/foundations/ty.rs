//! Ported from crates/typst-library/src/foundations/ty.rs @ v0.15.1, modified for Avenger.
//!
//! Avenger keeps a type's identity and names, which diagnostics print ("expected length, found
//! string"), and its scope, which holds constants such as `float.inf`. Constructors, methods and
//! documentation are out of scope. Upstream's `#[ty]` attribute becomes the [`ty!`] macro.

use std::cmp::Ordering;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use ecow::EcoString;

use crate::typst_library::diag::{StrResult, WarningSink, bail};
use crate::typst_library::foundations::{AutoValue, NoneValue, Repr, Scope, Value};

/// Describes a kind of value.
#[derive(Copy, Clone)]
pub struct Type(&'static NativeTypeData);

impl Type {
    /// Get the type for `T`.
    pub fn of<T: NativeType>() -> Self {
        T::ty()
    }

    /// The type's short name, how it is used in code (e.g. `str`).
    pub fn short_name(&self) -> &'static str {
        self.0.name
    }

    /// The type's long name, for use in diagnostics (e.g. `string`).
    pub fn long_name(&self) -> &'static str {
        self.0.long_name
    }

    /// The type's title case name, for use in documentation (e.g. `String`).
    pub fn title(&self) -> &'static str {
        self.0.title
    }

    /// The type's associated scope that holds sub-definitions.
    pub fn scope(&self) -> &'static Scope {
        &self.0.scope
    }

    /// Get a field from this type's scope, if possible.
    pub fn field(
        &self,
        field: &str,
        sink: impl WarningSink,
    ) -> StrResult<&'static Value> {
        match self.scope().get(field) {
            Some(binding) => Ok(binding.read_checked(sink)),
            None => bail!("type {self} does not contain field `{field}`"),
        }
    }
}

ty!(Type, name = "type", title = "Type", long = "type");

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Type {}

impl Hash for Type {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.0, state);
    }
}

impl Debug for Type {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Type({})", self.long_name())
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.pad(self.long_name())
    }
}

impl Repr for Type {
    fn repr(&self) -> EcoString {
        if *self == Type::of::<AutoValue>() {
            "type(auto)"
        } else if *self == Type::of::<NoneValue>() {
            "type(none)"
        } else {
            self.short_name()
        }
        .into()
    }
}

impl Ord for Type {
    fn cmp(&self, other: &Self) -> Ordering {
        self.long_name().cmp(other.long_name())
    }
}

impl PartialOrd for Type {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A type that is defined by a native Rust type.
pub trait NativeType {
    /// The type's name.
    ///
    /// In contrast to `data()`, this is usable in const contexts.
    const NAME: &'static str;

    /// Get the type for the native Rust type.
    fn ty() -> Type {
        Type(Self::data())
    }

    /// Get the type data for the native Rust type.
    fn data() -> &'static NativeTypeData;
}

/// Defines a native type.
#[derive(Debug)]
pub struct NativeTypeData {
    /// The type's normal name (e.g. `str`), as exposed to Typst.
    pub name: &'static str,
    /// The type's long name (e.g. `string`), for error messages.
    pub long_name: &'static str,
    /// The function's title case name (e.g. `String`).
    pub title: &'static str,
    /// The type's associated scope that holds sub-definitions.
    pub scope: LazyLock<Scope>,
}

/// Implements [`NativeType`], as upstream's `#[ty(name = .., title = ..)]` does. The long name is
/// the lowercased title, and the title defaults to the capitalized name. A type with a scope, as
/// upstream's `#[ty(scope)]` with its `#[scope]` block, names the function that builds it.
macro_rules! ty {
    ($ty:ty, name = $name:literal, title = $title:literal, long = $long:literal) => {
        $crate::typst_library::foundations::ty!(
            $ty,
            name = $name,
            title = $title,
            long = $long,
            scope = $crate::typst_library::foundations::Scope::new
        );
    };
    (
        $ty:ty,
        name = $name:literal,
        title = $title:literal,
        long = $long:literal,
        scope = $scope:path
    ) => {
        impl $crate::typst_library::foundations::NativeType for $ty {
            const NAME: &'static str = $name;

            fn data() -> &'static $crate::typst_library::foundations::NativeTypeData {
                static DATA: $crate::typst_library::foundations::NativeTypeData =
                    $crate::typst_library::foundations::NativeTypeData {
                        name: $name,
                        long_name: $long,
                        title: $title,
                        scope: ::std::sync::LazyLock::new($scope),
                    };
                &DATA
            }
        }
    };
}

pub(crate) use ty;
