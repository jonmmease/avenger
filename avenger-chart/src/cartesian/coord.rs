pub use avenger_chart_cartesian::Cartesian;

use crate::{
    cartesian::CartesianGuide,
    coords::{CoordMeasureRequest, CoordMeasurement, CoordinateSystem, CoordinateSystemTransform},
    error::AvengerChartError,
};

impl CoordinateSystem for Cartesian {
    type Guide = CartesianGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for Cartesian {
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    async fn measure(
        &self,
        request: CoordMeasureRequest<'_>,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        crate::cartesian::positioned_subplot::measure_cartesian_positioned_subplots(
            request.scales(),
            request.plot_width(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await
    }
}
