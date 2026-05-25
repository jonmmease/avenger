//! Zero-dimensional coordinate system
//!
//! The ZeroDCoord type represents a zero-dimensional coordinate system - essentially
//! a single point with no spatial extent.

pub use avenger_chart_core::ZeroDCoord;

use crate::{
    coords::{CoordinateSystem, CoordinateSystemTransform},
    guide::NoGuide,
};

impl CoordinateSystem for ZeroDCoord {
    type Guide = NoGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for ZeroDCoord {
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}
