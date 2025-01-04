use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;
use std::fmt::Debug;

use crate::error::AvengerChartError;
use crate::types::scales::Scale;

pub mod axis;
pub mod legend;

pub struct GuideCompilationContext<'a> {
    /// The size of the plot area
    pub size: [f32; 2],

    /// The origin of the guide's plot area
    pub origin: [f32; 2],

    /// The scene group that the guide will be added to
    pub group: &'a SceneGroup,

    /// The scales that the guide will use
    pub scales: &'a HashMap<String, ConfiguredScale>,
}

pub trait Guide: Debug + Send + Sync + 'static {
    fn scales(&self) -> HashMap<String, Scale> {
        Default::default()
    }

    fn compile(
        &self,
        context: &GuideCompilationContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}
