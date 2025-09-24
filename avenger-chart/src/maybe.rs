//! Three-state enum for tracking configuration values
//!
//! The `Maybe<T>` enum is used to distinguish between:
//! - Fields that have not been set by the user (Unset)
//! - Fields that have been explicitly set by the user (Set)
//!
//! This allows configuration objects to act as "patches" that can be
//! applied to computed defaults during rendering.

use serde::{Deserialize, Serialize};
use erased_serde::Deserializer;

/// Three-state enum for tracking configuration values
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Maybe<T> {
    /// Field has not been set by user
    Unset,
    /// Field has been explicitly set (including to None for Option<T>)
    Set(T),
}

impl<T> Default for Maybe<T> {
    fn default() -> Self {
        Maybe::Unset
    }
}

impl<T> Maybe<T> {
    /// Check if the value has been set
    pub fn is_set(&self) -> bool {
        matches!(self, Maybe::Set(_))
    }

    /// Check if the value is unset
    pub fn is_unset(&self) -> bool {
        matches!(self, Maybe::Unset)
    }

    /// Get the value if set, otherwise return the provided default
    pub fn unwrap_or(self, default: T) -> T {
        match self {
            Maybe::Set(v) => v,
            Maybe::Unset => default,
        }
    }

    /// Get the value if set, otherwise compute it from a closure
    pub fn unwrap_or_else<F>(self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        match self {
            Maybe::Set(v) => v,
            Maybe::Unset => f(),
        }
    }

    /// Convert &Maybe<T> to Maybe<&T>
    pub fn as_ref(&self) -> Maybe<&T> {
        match self {
            Maybe::Set(v) => Maybe::Set(v),
            Maybe::Unset => Maybe::Unset,
        }
    }

    /// Convert &mut Maybe<T> to Maybe<&mut T>
    pub fn as_mut(&mut self) -> Maybe<&mut T> {
        match self {
            Maybe::Set(v) => Maybe::Set(v),
            Maybe::Unset => Maybe::Unset,
        }
    }

    /// Apply this maybe value to a default, returning the final value
    pub fn apply_to(self, default: T) -> T {
        match self {
            Maybe::Set(v) => v,
            Maybe::Unset => default,
        }
    }

    /// Map a function over the value if it's set
    pub fn map<U, F>(self, f: F) -> Maybe<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Maybe::Set(v) => Maybe::Set(f(v)),
            Maybe::Unset => Maybe::Unset,
        }
    }

    /// Get an Option<T> from Maybe<T>
    pub fn into_option(self) -> Option<T> {
        match self {
            Maybe::Set(v) => Some(v),
            Maybe::Unset => None,
        }
    }

    /// Get an Option<&T> from &Maybe<T>
    pub fn as_option(&self) -> Option<&T> {
        match self {
            Maybe::Set(v) => Some(v),
            Maybe::Unset => None,
        }
    }

    /// Create a Maybe from an Option
    pub fn from_option(opt: Option<T>) -> Self {
        match opt {
            Some(v) => Maybe::Set(v),
            None => Maybe::Unset,
        }
    }
}

impl<T> From<T> for Maybe<T> {
    fn from(value: T) -> Self {
        Maybe::Set(value)
    }
}

impl<T> From<Option<T>> for Maybe<T> {
    fn from(opt: Option<T>) -> Self {
        Maybe::from_option(opt)
    }
}

// Helper for Maybe<Option<T>> - common pattern for nullable configuration fields
impl<T> Maybe<Option<T>> {
    /// Set the value to Some(v)
    pub fn set_some(v: T) -> Self {
        Maybe::Set(Some(v))
    }

    /// Set the value to None (explicitly unset the optional)
    pub fn set_none() -> Self {
        Maybe::Set(None)
    }

    /// Flatten Maybe<Option<T>> to Option<T>
    pub fn flatten(self) -> Option<T> {
        match self {
            Maybe::Set(opt) => opt,
            Maybe::Unset => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maybe_basic() {
        let unset: Maybe<i32> = Maybe::Unset;
        assert!(!unset.is_set());
        assert!(unset.is_unset());

        let set = Maybe::Set(42);
        assert!(set.is_set());
        assert!(!set.is_unset());
    }

    #[test]
    fn test_maybe_unwrap() {
        let unset: Maybe<i32> = Maybe::Unset;
        assert_eq!(unset.unwrap_or(10), 10);

        let set = Maybe::Set(42);
        assert_eq!(set.unwrap_or(10), 42);
    }

    #[test]
    fn test_maybe_option() {
        let maybe_none: Maybe<Option<String>> = Maybe::set_none();
        assert!(maybe_none.is_set());
        assert_eq!(maybe_none.flatten(), None);

        let maybe_some = Maybe::set_some("hello".to_string());
        assert!(maybe_some.is_set());
        assert_eq!(maybe_some.flatten(), Some("hello".to_string()));

        let unset: Maybe<Option<String>> = Maybe::Unset;
        assert!(unset.is_unset());
        assert_eq!(unset.flatten(), None);
    }
}
