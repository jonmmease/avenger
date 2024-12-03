use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct IdentityScale {}

impl IdentityScale {
    pub fn new() -> Self {
        Self {}
    }

    pub fn scale<'a, T>(&self, values: impl Into<ScalarOrArrayRef<'a, T>>) -> ScalarOrArray<T>
    where
        T: Clone + Debug + Sync + 'static,
    {
        values.into().to_owned()
    }

    pub fn invert<'a, T>(&self, values: impl Into<ScalarOrArrayRef<'a, T>>) -> ScalarOrArray<T>
    where
        T: Clone + Debug + Sync + 'static,
    {
        values.into().to_owned()
    }
}
