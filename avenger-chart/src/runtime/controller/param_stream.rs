use std::collections::HashMap;

use crate::types::scales::Scale;
use avenger_eventstream::{
    scene::SceneGraphEvent,
    stream::{EventStreamConfig, UpdateStatus},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scales::scales::ConfiguredScale;
use datafusion::scalar::ScalarValue;
use std::fmt::Debug;

pub trait ParamStream: Debug + Send + Sync + 'static {
    fn stream_config(&self) -> &EventStreamConfig;

    fn input_params(&self) -> &[String];

    fn input_scales(&self) -> &[Scale];

    fn update(
        &self,
        event: &SceneGraphEvent,
        params: &HashMap<String, ScalarValue>,
        scales: &[ConfiguredScale],
        group_path: &[usize],
        rtree: &SceneGraphRTree,
    ) -> (HashMap<String, ScalarValue>, UpdateStatus);
}
