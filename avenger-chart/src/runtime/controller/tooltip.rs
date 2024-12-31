use std::{collections::HashMap, sync::Arc};

use super::{param_stream::ParamStream, Controller};
use crate::runtime::controller::param_stream::ParamStreamContext;
use crate::runtime::scale::scale_expr;
use crate::types::group::MarkOrGroup;
use crate::types::mark::Mark;
use crate::{
    param::Param,
    types::scales::{Scale, ScaleRange},
};
use arrow::array::{ArrayRef, StructArray};
use arrow::{
    array::{AsArray, ListArray},
    datatypes::Float32Type,
};
use avenger_eventstream::{
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, EventStreamFilter, UpdateStatus},
    window::MouseButton,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scales::scales::{linear::LinearScale, ConfiguredScale};
use datafusion::{
    prelude::{lit, DataFrame},
    scalar::ScalarValue,
};
use std::fmt::Write;

const TOOLTIP_MARK_NAME: &str = "tooltip-mark";

#[derive(Debug, Clone)]
pub struct TooltipParamStream {
    stream_config: EventStreamConfig,
}

impl TooltipParamStream {
    pub fn new() -> Self {
        Self {
            stream_config: EventStreamConfig {
                types: vec![SceneGraphEventType::CursorMoved],
                ..Default::default()
            },
        }
    }
}

impl ParamStream for TooltipParamStream {
    fn stream_config(&self) -> &EventStreamConfig {
        &self.stream_config
    }

    fn input_params(&self) -> &[String] {
        &[]
    }

    fn input_scales(&self) -> &[Scale] {
        &[]
    }

    fn update(&self, context: ParamStreamContext) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        let event = context.event;
        let scales = context.scales;
        let group_path = context.group_path;
        let rtree = context.rtree;

        // Compute position in group coordinates
        let event_position = event.position().unwrap();
        let plot_origin = rtree.group_origin(group_path).unwrap();

        let mut tooltip_text = "".to_string();
        if let Some(mark) = context.event.mark_instance() {
            if let (Some(batch), Some(instance)) =
                (context.details.get(&mark.mark_path), mark.instance_index)
            {
                let struct_array = StructArray::from(batch.clone());
                if let Ok(element) =
                    ScalarValue::try_from_array(&(Arc::new(struct_array) as ArrayRef), instance)
                {
                    tooltip_text = element.to_string();
                }
            }
        }

        let x = (event_position[0] - plot_origin[0]) + 20.0;
        let y = event_position[1] - plot_origin[1];

        let new_params = vec![
            ("tooltip_x".to_string(), x.into()),
            ("tooltip_y".to_string(), y.into()),
            ("tooltip_text".to_string(), tooltip_text.into()),
        ]
        .into_iter()
        .collect::<HashMap<_, ScalarValue>>();

        (
            new_params,
            UpdateStatus {
                rerender: true,
                rebuild_geometry: true,
            },
        )
    }
}

#[derive(Debug, Clone)]
pub struct TooltipController {
    tooltip_x: Param,
    tooltip_y: Param,
    tooltip_text: Param,
    param_streams: Vec<Arc<dyn ParamStream>>,
}

impl TooltipController {
    pub fn new() -> Self {
        // Initialize param streams
        let tooltip_param_stream = TooltipParamStream::new();
        let tooltip_x = Param::new("tooltip_x", ScalarValue::from(0.0f32));
        let tooltip_y = Param::new("tooltip_y", ScalarValue::from(0.0f32));
        let tooltip_text = Param::new("tooltip_text", ScalarValue::from(""));

        Self {
            tooltip_x,
            tooltip_y,
            tooltip_text,
            param_streams: vec![Arc::new(tooltip_param_stream)],
        }
    }

    pub fn tooltip_x(&self) -> &Param {
        &self.tooltip_x
    }

    pub fn y(&self) -> &Param {
        &self.tooltip_y
    }
}

impl Controller for TooltipController {
    fn name(&self) -> &str {
        "tooltip"
    }

    fn param_streams(&self) -> Vec<Arc<dyn ParamStream>> {
        self.param_streams.clone()
    }

    fn params(&self) -> Vec<Param> {
        vec![
            self.tooltip_x.clone(),
            self.tooltip_y.clone(),
            self.tooltip_text.clone(),
        ]
    }

    fn marks(&self) -> Vec<MarkOrGroup> {
        vec![MarkOrGroup::Mark(
            Mark::text()
                .name(TOOLTIP_MARK_NAME)
                .encode("x", &self.tooltip_x)
                .encode("y", &self.tooltip_y)
                .encode("text", &self.tooltip_text),
        )]
    }
}
