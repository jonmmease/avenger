use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ScalarOrArray<T: Sync + Clone> {
    Scalar(T),
    Array(Arc<Vec<T>>),
}

impl<T: Sync + Clone> ScalarOrArray<T> {
    pub fn as_iter<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Arc<Vec<usize>>>,
    ) -> Box<dyn Iterator<Item = &T> + '_> {
        match self {
            ScalarOrArray::Scalar(value) => Box::new(std::iter::repeat(value).take(scalar_len)),
            ScalarOrArray::Array(values) => match indices {
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
        match self {
            ScalarOrArray::Scalar(value) => {
                Box::new(std::iter::repeat(value.clone()).take(scalar_len))
            }
            ScalarOrArray::Array(values) => match indices {
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
        match self {
            ScalarOrArray::Scalar(value) => ScalarOrArray::Scalar(f(value)),
            ScalarOrArray::Array(values) => {
                ScalarOrArray::Array(Arc::new(values.iter().map(f).collect()))
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            ScalarOrArray::Scalar(_) => 1,
            ScalarOrArray::Array(values) => values.len(),
        }
    }
}

impl ScalarOrArray<f32> {
    pub fn equals_scalar(&self, v: f32) -> bool {
        match self {
            ScalarOrArray::Scalar(value) => v == *value,
            _ => false,
        }
    }
}

impl<T: Sync + Clone> From<Vec<T>> for ScalarOrArray<T> {
    fn from(values: Vec<T>) -> Self {
        ScalarOrArray::Array(Arc::new(values))
    }
}

impl<T: Sync + Clone> From<T> for ScalarOrArray<T> {
    fn from(value: T) -> Self {
        ScalarOrArray::Scalar(value)
    }
}

impl From<&str> for ScalarOrArray<String> {
    fn from(value: &str) -> Self {
        ScalarOrArray::Scalar(value.to_string())
    }
}

impl From<Vec<&str>> for ScalarOrArray<String> {
    fn from(values: Vec<&str>) -> Self {
        ScalarOrArray::Array(Arc::new(
            values.into_iter().map(|s| s.to_string()).collect(),
        ))
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
            ScalarOrArrayRef::Scalar(value) => ScalarOrArray::Scalar(value.clone()),
            ScalarOrArrayRef::Array(values) => ScalarOrArray::Array(Arc::new(values.to_vec())),
        }
    }

    pub fn map<U: Sync + Clone>(self, f: impl Fn(&T) -> U) -> ScalarOrArray<U> {
        match self {
            ScalarOrArrayRef::Scalar(value) => ScalarOrArray::Scalar(f(&value)),
            ScalarOrArrayRef::Array(values) => {
                ScalarOrArray::Array(Arc::new(values.iter().map(f).collect()))
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
