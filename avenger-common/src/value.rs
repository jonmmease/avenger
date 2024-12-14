use std::sync::{Arc, Mutex};

use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ScalarOrArray<T: Sync + Clone> {
    pub(crate) value: ScalarOrArrayValue<T>,
    pub(crate) hash_cache: Arc<Mutex<Option<u64>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    pub fn as_iter<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Arc<Vec<usize>>>,
    ) -> Box<dyn Iterator<Item = &T> + '_> {
        match &self.value {
            ScalarOrArrayValue::Scalar(value) => {
                Box::new(std::iter::repeat(value).take(scalar_len))
            }
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
    ) -> Box<dyn Iterator<Item = T> + '_> {
        match &self.value {
            ScalarOrArrayValue::Scalar(value) => {
                Box::new(std::iter::repeat(value.clone()).take(scalar_len))
            }
            ScalarOrArrayValue::Array(values) => match indices {
                None => Box::new(values.iter().cloned()),
                Some(indices) => Box::new(indices.iter().map(|i| values[*i].clone())),
            },
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

impl<'a, T: Sync + Clone> From<T> for ScalarOrArrayRef<'a, T> {
    fn from(value: T) -> Self {
        ScalarOrArrayRef::Scalar(value)
    }
}

impl Hash for ScalarOrArray<f32> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut hash_cache = self.hash_cache.lock().unwrap();

        match &self.value {
            ScalarOrArrayValue::Scalar(value) => {
                let hash_value = hash_cache.get_or_insert_with(|| {
                    let mut inner_hasher = std::hash::DefaultHasher::new();
                    OrderedFloat::from(*value).hash(&mut inner_hasher);
                    inner_hasher.finish()
                });
                state.write_u64(*hash_value);
            }
            ScalarOrArrayValue::Array(values) => {
                let hash_value = hash_cache.get_or_insert_with(|| {
                    let mut inner_hasher = std::hash::DefaultHasher::new();
                    for value in values.iter() {
                        OrderedFloat::from(*value).hash(&mut inner_hasher);
                    }
                    inner_hasher.finish()
                });
                state.write_u64(*hash_value);
            }
        }
    }
}

#[macro_export]
macro_rules! impl_hash_for_scalar_or_array {
    ($t:ty) => {
        impl Hash for crate::value::ScalarOrArray<$t> {
            fn hash<H: Hasher>(&self, state: &mut H) {
                let mut hash_cache = self.hash_cache.lock().unwrap();

                match &self.value {
                    crate::value::ScalarOrArrayValue::Scalar(value) => {
                        let hash_value = hash_cache.get_or_insert_with(|| {
                            let mut inner_hasher = std::hash::DefaultHasher::new();
                            value.hash(&mut inner_hasher);
                            inner_hasher.finish()
                        });
                        state.write_u64(*hash_value);
                    }
                    crate::value::ScalarOrArrayValue::Array(values) => {
                        let hash_value = hash_cache.get_or_insert_with(|| {
                            let mut inner_hasher = std::hash::DefaultHasher::new();
                            for value in values.iter() {
                                value.hash(&mut inner_hasher);
                            }
                            inner_hasher.finish()
                        });
                        state.write_u64(*hash_value);
                    }
                }
            }
        }
    };
}

impl_hash_for_scalar_or_array!(i32);
impl_hash_for_scalar_or_array!(i64);
impl_hash_for_scalar_or_array!(usize);
impl_hash_for_scalar_or_array!(u32);
impl_hash_for_scalar_or_array!(u64);
impl_hash_for_scalar_or_array!(String);
