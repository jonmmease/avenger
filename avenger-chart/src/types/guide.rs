use crate::error::AvengerChartError;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use std::fmt::Debug;

use super::scales::Scale;

pub struct GuideCompilationContext<'a> {
    /// The size of the plot area
    pub size: [f32; 2],

    /// The origin of the guide's plot area
    pub origin: [f32; 2],

    /// The scene group that the guide will be added to
    pub group: &'a SceneGroup,

    /// The scales that the guide will use
    pub scales: &'a [ConfiguredScale],
}

pub trait Guide: Debug + Send + Sync + 'static {
    fn scales(&self) -> &[Scale] {
        &[]
    }

    fn compile(
        &self,
        context: &GuideCompilationContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}
