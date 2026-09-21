use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Distinguishes an omitted property from explicit JSON null and a supplied value.
///
/// Spec fields omit `Missing` when serialized. Serialized by itself, `Missing`
/// becomes null because JSON has no standalone missing value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MissingNullOrValue<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> MissingNullOrValue<T> {
    /// Whether the property was omitted.
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// Whether the property was explicitly null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Borrows the supplied value, if present and non-null.
    pub fn as_option(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Missing | Self::Null => None,
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for MissingNullOrValue<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

impl<T: Serialize> Serialize for MissingNullOrValue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(value) => value.serialize(serializer),
            Self::Missing | Self::Null => serializer.serialize_none(),
        }
    }
}

// Field defaults handle omission. Explicit null is rejected before it can
// become None on a non-nullable optional property.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)?
        .map(Some)
        .ok_or_else(|| serde::de::Error::custom("null is not allowed for this property"))
}
