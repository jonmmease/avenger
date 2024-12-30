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
use arrow::array::RecordBatch;

pub struct ParamStreamContext<'a> {
    pub event: &'a SceneGraphEvent,
    pub params: &'a HashMap<String, ScalarValue>,
    pub scales: &'a [ConfiguredScale],
    pub group_path: &'a [usize],
    pub rtree: &'a SceneGraphRTree,
    pub details: &'a HashMap<Vec<usize>, RecordBatch>
}

pub trait ParamStream: Debug + Send + Sync + 'static {
    fn stream_config(&self) -> &EventStreamConfig;

    fn input_params(&self) -> &[String];

    fn input_scales(&self) -> &[Scale];

    fn update(
        &self,
        context: ParamStreamContext
    ) -> (HashMap<String, ScalarValue>, UpdateStatus);
}
