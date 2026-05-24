pub use avenger_chart_polar::Polar;

use crate::{
    coords::{CoordinateSystem, CoordinateSystemTransform},
    polar::PolarGuide,
};

impl CoordinateSystem for Polar {
    type Guide = PolarGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for Polar {
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}
