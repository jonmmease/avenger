//! Hash functions for fields whose types don't implement `Hash`, for the scene's
//! `#[educe(Hash(method(...)))]` field attributes.

use std::hash::{Hash, Hasher};

use ordered_float::OrderedFloat;

/// Values made of floats, which hash so that equal values hash the same.
pub trait HashFloats {
    fn hash_floats<H: Hasher>(&self, state: &mut H);
}

impl HashFloats for f32 {
    fn hash_floats<H: Hasher>(&self, state: &mut H) {
        OrderedFloat(*self).hash(state);
    }
}

impl<T: HashFloats, const N: usize> HashFloats for [T; N] {
    fn hash_floats<H: Hasher>(&self, state: &mut H) {
        for value in self {
            value.hash_floats(state);
        }
    }
}

impl<T: HashFloats> HashFloats for Option<T> {
    fn hash_floats<H: Hasher>(&self, state: &mut H) {
        match self {
            None => state.write_u8(0),
            Some(value) => {
                state.write_u8(1);
                value.hash_floats(state);
            }
        }
    }
}

impl<T: HashFloats> HashFloats for Vec<T> {
    fn hash_floats<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.len());
        for value in self {
            value.hash_floats(state);
        }
    }
}

/// Hashes floats by value, so that `-0.0` and `0.0`, which compare equal, hash the same.
pub fn hash_floats<T: HashFloats, H: Hasher>(value: &T, state: &mut H) {
    value.hash_floats(state);
}

#[cfg(test)]
mod tests {
    use std::hash::DefaultHasher;

    use super::*;

    fn hash<T: HashFloats>(value: &T) -> u64 {
        let mut state = DefaultHasher::new();
        hash_floats(value, &mut state);
        state.finish()
    }

    #[test]
    fn equal_floats_hash_the_same_and_shapes_stay_distinct() {
        assert_eq!(hash(&0.0f32), hash(&-0.0f32));
        assert_ne!(hash(&None::<f32>), hash(&Some(0.0f32)));
        assert_ne!(hash(&vec![1.0f32]), hash(&vec![1.0f32, 0.0]));
        assert_ne!(hash(&[1.0f32, 2.0]), hash(&[2.0f32, 1.0]));
    }
}
