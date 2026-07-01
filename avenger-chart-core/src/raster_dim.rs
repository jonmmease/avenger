use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RasterDim {
    name: String,
}

impl RasterDim {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

pub fn dim(name: impl Into<String>) -> RasterDim {
    RasterDim::new(name)
}
