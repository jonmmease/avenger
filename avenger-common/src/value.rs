use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex},
};

use avenger_image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::hash::hash_floats;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ScalarOrArray<T: Sync + Clone> {
    pub(crate) value: ScalarOrArrayValue<T>,
    pub(crate) hash_cache: Arc<Mutex<Option<u64>>>,
}

impl<T: Sync + Clone + PartialEq> PartialEq for ScalarOrArray<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScalarOrArrayValue<T: Sync + Clone> {
    Scalar(T),
    Array(Arc<Vec<T>>),
}

impl<T: Sync + Clone> ScalarOrArray<T> {
    pub fn new_scalar(value: T) -> Self {
        ScalarOrArray {
            value: ScalarOrArrayValue::Scalar(value),
            hash_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn new_array(values: Vec<T>) -> Self {
        ScalarOrArray {
            value: ScalarOrArrayValue::Array(Arc::new(values)),
            hash_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn value(&self) -> &ScalarOrArrayValue<T> {
        &self.value
    }

    pub fn to_scalar_if_len_one(self) -> Self {
        if self.len() == 1 {
            ScalarOrArray::new_scalar(self.as_vec(1, None)[0].clone())
        } else {
            self
        }
    }

    pub fn as_iter<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Arc<Vec<usize>>>,
    ) -> Box<dyn Iterator<Item = &'a T> + 'a> {
        match &self.value {
            ScalarOrArrayValue::Scalar(value) => Box::new(std::iter::repeat_n(value, scalar_len)),
            ScalarOrArrayValue::Array(values) => match indices {
                None => Box::new(values.iter()),
                Some(indices) => Box::new(indices.iter().map(|i| &values[*i])),
            },
        }
    }

    pub fn as_iter_owned<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Arc<Vec<usize>>>,
    ) -> Box<dyn Iterator<Item = T> + 'a> {
        match &self.value {
            ScalarOrArrayValue::Scalar(value) => {
                Box::new(std::iter::repeat_n(value.clone(), scalar_len))
            }
            ScalarOrArrayValue::Array(values) => match indices {
                None => Box::new(values.iter().cloned()),
                Some(indices) => Box::new(indices.iter().map(|i| values[*i].clone())),
            },
        }
    }

    pub fn first(&self) -> Option<&T> {
        match &self.value {
            ScalarOrArrayValue::Scalar(v) => Some(v),
            ScalarOrArrayValue::Array(arr) => {
                if arr.is_empty() {
                    None
                } else {
                    Some(&arr[0])
                }
            }
        }
    }

    pub fn as_vec(&self, scalar_len: usize, indices: Option<&Arc<Vec<usize>>>) -> Vec<T> {
        self.as_iter(scalar_len, indices)
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn map<U: Sync + Clone>(&self, f: impl Fn(&T) -> U) -> ScalarOrArray<U> {
        match &self.value {
            ScalarOrArrayValue::Scalar(value) => ScalarOrArray::new_scalar(f(value)),
            ScalarOrArrayValue::Array(values) => {
                ScalarOrArray::new_array(values.iter().map(f).collect())
            }
        }
    }

    pub fn len(&self) -> usize {
        match &self.value {
            ScalarOrArrayValue::Scalar(_) => 1,
            ScalarOrArrayValue::Array(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ScalarOrArray<f32> {
    pub fn equals_scalar(&self, v: f32) -> bool {
        match &self.value {
            ScalarOrArrayValue::Scalar(value) => v == *value,
            _ => false,
        }
    }
}

impl<T: Sync + Clone> From<Vec<T>> for ScalarOrArray<T> {
    fn from(values: Vec<T>) -> Self {
        ScalarOrArray {
            value: ScalarOrArrayValue::Array(Arc::new(values)),
            hash_cache: Arc::new(Mutex::new(None)),
        }
    }
}

impl<T: Sync + Clone> From<T> for ScalarOrArray<T> {
    fn from(value: T) -> Self {
        ScalarOrArray {
            value: ScalarOrArrayValue::Scalar(value),
            hash_cache: Arc::new(Mutex::new(None)),
        }
    }
}

impl From<&str> for ScalarOrArray<String> {
    fn from(value: &str) -> Self {
        ScalarOrArray {
            value: ScalarOrArrayValue::Scalar(value.to_string()),
            hash_cache: Arc::new(Mutex::new(None)),
        }
    }
}

impl From<Vec<&str>> for ScalarOrArray<String> {
    fn from(values: Vec<&str>) -> Self {
        ScalarOrArray {
            value: ScalarOrArrayValue::Array(Arc::new(
                values.into_iter().map(|s| s.to_string()).collect(),
            )),
            hash_cache: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScalarOrArrayRef<'a, T: Sync + Clone> {
    Scalar(T),
    Array(&'a [T]),
}

impl<'a, T: Sync + Clone> ScalarOrArrayRef<'a, T> {
    pub fn from_slice(values: &'a [T]) -> Self {
        ScalarOrArrayRef::Array(values)
    }

    pub fn to_owned(self) -> ScalarOrArray<T> {
        match self {
            ScalarOrArrayRef::Scalar(value) => ScalarOrArray::new_scalar(value.clone()),
            ScalarOrArrayRef::Array(values) => ScalarOrArray::new_array(values.to_vec()),
        }
    }

    pub fn map<U: Sync + Clone>(self, f: impl Fn(&T) -> U) -> ScalarOrArray<U> {
        match self {
            ScalarOrArrayRef::Scalar(value) => ScalarOrArray::new_scalar(f(&value)),
            ScalarOrArrayRef::Array(values) => {
                ScalarOrArray::new_array(values.iter().map(f).collect())
            }
        }
    }
}

impl<'a, T: Sync + Clone> From<&'a [T]> for ScalarOrArrayRef<'a, T> {
    fn from(values: &'a [T]) -> Self {
        ScalarOrArrayRef::Array(values)
    }
}

impl<'a, T: Sync + Clone> From<&'a Vec<T>> for ScalarOrArrayRef<'a, T> {
    fn from(values: &'a Vec<T>) -> Self {
        ScalarOrArrayRef::Array(values.as_slice())
    }
}

impl<'a, T: Sync + Clone> From<&'a T> for ScalarOrArrayRef<'a, T> {
    fn from(value: &'a T) -> Self {
        ScalarOrArrayRef::Scalar(value.clone())
    }
}

impl<T: Sync + Clone> From<T> for ScalarOrArrayRef<'_, T> {
    fn from(value: T) -> Self {
        ScalarOrArrayRef::Scalar(value)
    }
}

impl<T: Sync + Clone> ScalarOrArray<T> {
    /// Writes the hash of the values, which `hash_value` computes once and the cache keeps.
    pub(crate) fn hash_cached<H: Hasher>(
        &self,
        state: &mut H,
        hash_value: impl Fn(&T, &mut DefaultHasher),
    ) {
        let mut hash_cache = self.hash_cache.lock().unwrap();
        let hash = *hash_cache.get_or_insert_with(|| {
            let mut hasher = DefaultHasher::new();
            match &self.value {
                ScalarOrArrayValue::Scalar(value) => {
                    hasher.write_u8(0);
                    hash_value(value, &mut hasher);
                }
                ScalarOrArrayValue::Array(values) => {
                    hasher.write_u8(1);
                    hasher.write_usize(values.len());
                    for value in values.iter() {
                        hash_value(value, &mut hasher);
                    }
                }
            }
            hasher.finish()
        });
        state.write_u64(hash);
    }
}

impl Hash for ScalarOrArray<f32> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_cached(state, hash_floats);
    }
}

impl Hash for ScalarOrArray<Vec<f32>> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_cached(state, hash_floats);
    }
}

/// Hashes values whose type implements `Hash`, for value types whose crates can't implement
/// `Hash` for `ScalarOrArray`.
pub fn hash_values<T: Hash + Sync + Clone, H: Hasher>(values: &ScalarOrArray<T>, state: &mut H) {
    values.hash_cached(state, |value, hasher| value.hash(hasher));
}

#[macro_export]
macro_rules! impl_hash_for_scalar_or_array {
    ($type_name:ty) => {
        impl std::hash::Hash for $crate::value::ScalarOrArray<$type_name> {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                $crate::value::hash_values(self, state);
            }
        }
    };
}

impl_hash_for_scalar_or_array!(i32);
impl_hash_for_scalar_or_array!(i64);
impl_hash_for_scalar_or_array!(u32);
impl_hash_for_scalar_or_array!(u64);
impl_hash_for_scalar_or_array!(bool);
impl_hash_for_scalar_or_array!(usize);
impl_hash_for_scalar_or_array!(String);
impl_hash_for_scalar_or_array!(RgbaImage);

#[cfg(test)]
mod tests {
    use std::hash::{DefaultHasher, Hash, Hasher};

    use super::*;

    fn hash<T: Hash>(value: &T) -> u64 {
        let mut state = DefaultHasher::new();
        value.hash(&mut state);
        state.finish()
    }

    #[test]
    fn dash_arrays_hash_every_length() {
        let dash = |lengths: Vec<f32>| ScalarOrArray::new_scalar(lengths);
        assert_ne!(hash(&dash(vec![4.0, 2.0])), hash(&dash(vec![4.0, 8.0])));
        let dashes = |lengths: Vec<f32>| ScalarOrArray::new_array(vec![vec![1.0], lengths]);
        assert_ne!(hash(&dashes(vec![4.0, 2.0])), hash(&dashes(vec![4.0, 8.0])));
    }
}
