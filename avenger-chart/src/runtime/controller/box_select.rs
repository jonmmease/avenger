use std::{collections::HashMap, sync::Arc};

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
use avenger_scales::{
    scales::{linear::LinearScale, ConfiguredScale},
    utils::ScalarValueUtils,
};
use datafusion::{
    prelude::{ident, lit, when, DataFrame, Expr},
    scalar::ScalarValue,
};

use super::{param_stream::ParamStream, Controller};
use crate::runtime::controller::param_stream::ParamStreamContext;
use crate::{
    error::AvengerChartError,
    param::Param,
    runtime::scale::scale_expr,
    types::{
        group::MarkOrGroup,
        mark::Mark,
        scales::{Scale, ScaleRange},
    },
};

const BOX_SELECT_MARK_NAME: &str = "box-select-rect";

fn box_create_init_config() -> EventStreamConfig {
    EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        filter: Some(vec![EventStreamFilter(Arc::new(move |event| {
            let SceneGraphEvent::MouseDown(mouse_down) = event else {
                return false;
            };
            // Skip if the click on an existing box
            if let Some(mark_instance) = event.mark_instance() {
                if mark_instance.name == BOX_SELECT_MARK_NAME {
                    return false;
                }
            }
            mouse_down.button == MouseButton::Left
        }))]),
        ..Default::default()
    }
}

fn box_move_init_config() -> EventStreamConfig {
    EventStreamConfig {
        types: vec![SceneGraphEventType::MouseDown],
        filter: Some(vec![EventStreamFilter(Arc::new(move |event| {
            let SceneGraphEvent::MouseDown(mouse_down) = event else {
                return false;
            };
            // Only keep if the click on an existing box

            if let Some(mark_instance) = event.mark_instance() {
                if mark_instance.name == BOX_SELECT_MARK_NAME
                    && mouse_down.button == MouseButton::Left
                {
                    return true;
                }
            }
            false
        }))]),
        ..Default::default()
    }
}

fn mouse_up_config() -> EventStreamConfig {
    EventStreamConfig {
        types: vec![SceneGraphEventType::MouseUp],
        filter: Some(vec![EventStreamFilter(Arc::new(|event| {
            let SceneGraphEvent::MouseUp(mouse_up) = event else {
                return false;
            };
            mouse_up.button == MouseButton::Left
        }))]),
        ..Default::default()
    }
}

#[derive(Debug, Clone)]
pub struct BoxSelectInitParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl BoxSelectInitParamStream {
    pub fn new(x_scale: Scale, y_scale: Scale) -> Self {
        Self {
            scales: vec![x_scale, y_scale],
            input_params: vec![],
            stream_config: box_create_init_config(),
        }
    }
}

impl ParamStream for BoxSelectInitParamStream {
    fn stream_config(&self) -> &EventStreamConfig {
        &self.stream_config
    }

    fn input_params(&self) -> &[String] {
        &self.input_params
    }

    fn input_scales(&self) -> &[Scale] {
        &self.scales
    }

    fn update(&self, context: ParamStreamContext) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        let event = context.event;
        let scales = context.scales;
        let group_path = context.group_path;
        let rtree = context.rtree;

        // Compute position in group coordinates
        let event_position = event.position().unwrap();
        let plot_origin = rtree.group_origin(group_path).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        // Get scales
        let x_scale = scales[0].clone();
        let y_scale = scales[1].clone();

        // Check if cursor is over the plot area
        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let normalized_x = (plot_x - range_start) / (range_end - range_start);
        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
        let normalized_y = (plot_y - range_start) / (range_end - range_start);
        if normalized_x < 0.0 || normalized_x > 1.0 || normalized_y < 0.0 || normalized_y > 1.0 {
            // Don't update params or rerender
            return (
                HashMap::new(),
                UpdateStatus {
                    rerender: false,
                    rebuild_geometry: true,
                },
            );
        }

        // Save state as controller params
        let box_x = ScalarValue::from(x_scale.invert_scalar(plot_x).unwrap());
        let box_y = ScalarValue::from(y_scale.invert_scalar(plot_y).unwrap());

        // return new param values
        let new_params = vec![
            ("box_x".to_string(), box_x.clone()),
            ("box_y".to_string(), box_y.clone()),
            ("box_x2".to_string(), box_x),
            ("box_y2".to_string(), box_y),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

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
pub struct BoxSelectInitMoveParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl BoxSelectInitMoveParamStream {
    pub fn new(x_scale: Scale, y_scale: Scale) -> Self {
        Self {
            scales: vec![x_scale, y_scale],
            input_params: vec![
                "box_x".to_string(),
                "box_y".to_string(),
                "box_x2".to_string(),
                "box_y2".to_string(),
            ],
            stream_config: box_move_init_config(),
        }
    }
}

impl ParamStream for BoxSelectInitMoveParamStream {
    fn stream_config(&self) -> &EventStreamConfig {
        &self.stream_config
    }

    fn input_params(&self) -> &[String] {
        &self.input_params
    }

    fn input_scales(&self) -> &[Scale] {
        &self.scales
    }

    fn update(&self, context: ParamStreamContext) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        let event = context.event;
        let params = context.params;
        let scales = context.scales;
        let group_path = context.group_path;
        let rtree = context.rtree;

        // Compute position in group coordinates
        let event_position = event.position().unwrap();
        let plot_origin = rtree.group_origin(group_path).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        // Get scales
        let x_scale = scales[0].clone();
        let y_scale = scales[1].clone();

        // extract initial box coordinates
        let (Some(box_anchor_x), Some(box_anchor_y), Some(box_anchor_x2), Some(box_anchor_y2)) = (
            params.get("box_x"),
            params.get("box_y"),
            params.get("box_x2"),
            params.get("box_y2"),
        ) else {
            return (HashMap::new(), UpdateStatus::default());
        };

        // Save state as controller params
        let anchor_start_x = ScalarValue::from(x_scale.invert_scalar(plot_x).unwrap());
        let anchor_start_y = ScalarValue::from(y_scale.invert_scalar(plot_y).unwrap());

        // Save them as anchor params.
        let new_params = vec![
            ("box_anchor_x".to_string(), box_anchor_x.clone()),
            ("box_anchor_y".to_string(), box_anchor_y.clone()),
            ("box_anchor_x2".to_string(), box_anchor_x2.clone()),
            ("box_anchor_y2".to_string(), box_anchor_y2.clone()),
            ("anchor_start_x".to_string(), anchor_start_x.clone()),
            ("anchor_start_y".to_string(), anchor_start_y.clone()),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

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
pub struct BoxSelectDragExpandParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl BoxSelectDragExpandParamStream {
    pub fn new(x_scale: Scale, y_scale: Scale) -> Self {
        // Drag config
        let drag_config = EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            between: Some((
                Box::new(box_create_init_config()),
                Box::new(mouse_up_config()),
            )),
            ..Default::default()
        };

        Self {
            scales: vec![x_scale, y_scale],
            input_params: vec!["box_x".to_string(), "box_y".to_string()],
            stream_config: drag_config,
        }
    }
}

impl ParamStream for BoxSelectDragExpandParamStream {
    fn stream_config(&self) -> &EventStreamConfig {
        &self.stream_config
    }

    fn input_params(&self) -> &[String] {
        &self.input_params
    }

    fn input_scales(&self) -> &[Scale] {
        &self.scales
    }

    fn update(&self, context: ParamStreamContext) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        let event = context.event;
        let params = context.params;
        let scales = context.scales;
        let group_path = context.group_path;
        let rtree = context.rtree;

        // Extract stored anchor position
        let (Some(box_x), Some(box_y)) = (params.get("box_x"), params.get("box_y")) else {
            // Don't update params or rerender
            return (
                HashMap::new(),
                UpdateStatus {
                    rerender: false,
                    rebuild_geometry: true,
                },
            );
        };

        // Get the cursor position in range space
        let event_position = event.position().unwrap();
        let plot_origin = rtree.group_origin(group_path).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        // Build scales reset to the anchor domains
        let x_scale = scales[0].clone();
        let y_scale = scales[1].clone();

        // Save state as controller params
        let box_x2 = ScalarValue::from(x_scale.invert_scalar(plot_x).unwrap());
        let box_y2 = ScalarValue::from(y_scale.invert_scalar(plot_y).unwrap());

        // Return new param values
        let new_params = vec![
            ("box_x2".to_string(), box_x2),
            ("box_y2".to_string(), box_y2),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

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
pub struct BoxSelectDragMoveParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl BoxSelectDragMoveParamStream {
    pub fn new(x_scale: Scale, y_scale: Scale) -> Self {
        // Drag config
        let drag_config = EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            between: Some((
                Box::new(box_move_init_config()),
                Box::new(mouse_up_config()),
            )),
            ..Default::default()
        };

        Self {
            scales: vec![x_scale, y_scale],
            input_params: vec![
                "box_anchor_x".to_string(),
                "box_anchor_y".to_string(),
                "box_anchor_x2".to_string(),
                "box_anchor_y2".to_string(),
                "anchor_start_x".to_string(),
                "anchor_start_y".to_string(),
            ],
            stream_config: drag_config,
        }
    }
}

impl ParamStream for BoxSelectDragMoveParamStream {
    fn stream_config(&self) -> &EventStreamConfig {
        &self.stream_config
    }

    fn input_params(&self) -> &[String] {
        &self.input_params
    }

    fn input_scales(&self) -> &[Scale] {
        &self.scales
    }

    fn update(&self, context: ParamStreamContext) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        let event = context.event;
        let params = context.params;
        let scales = context.scales;
        let group_path = context.group_path;
        let rtree = context.rtree;

        // Extract stored anchor position
        let (
            Some(box_anchor_x),
            Some(box_anchor_y),
            Some(box_anchor_x2),
            Some(box_anchor_y2),
            Some(anchor_start_x),
            Some(anchor_start_y),
        ) = (
            params.get("box_anchor_x").map(|v| v.as_f32().unwrap()),
            params.get("box_anchor_y").map(|v| v.as_f32().unwrap()),
            params.get("box_anchor_x2").map(|v| v.as_f32().unwrap()),
            params.get("box_anchor_y2").map(|v| v.as_f32().unwrap()),
            params.get("anchor_start_x").map(|v| v.as_f32().unwrap()),
            params.get("anchor_start_y").map(|v| v.as_f32().unwrap()),
        )
        else {
            // Don't update params or rerender
            return (
                HashMap::new(),
                UpdateStatus {
                    rerender: false,
                    rebuild_geometry: true,
                },
            );
        };

        // Get the cursor position in range space
        let event_position = event.position().unwrap();
        let plot_origin = rtree.group_origin(group_path).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        // Build scales reset to the anchor domains
        let x_scale = scales[0].clone();
        let y_scale = scales[1].clone();

        // Save state as controller params
        let anchor_end_x = ScalarValue::from(x_scale.invert_scalar(plot_x).unwrap())
            .as_f32()
            .unwrap();
        let anchor_end_y = ScalarValue::from(y_scale.invert_scalar(plot_y).unwrap())
            .as_f32()
            .unwrap();

        let delta_x = anchor_end_x - anchor_start_x;
        let delta_y = anchor_end_y - anchor_start_y;

        let box_x = box_anchor_x + delta_x;
        let box_y = box_anchor_y + delta_y;
        let box_x2 = box_anchor_x2 + delta_x;
        let box_y2 = box_anchor_y2 + delta_y;

        // Return new param values
        let new_params = vec![
            ("box_x".to_string(), ScalarValue::from(box_x)),
            ("box_y".to_string(), ScalarValue::from(box_y)),
            ("box_x2".to_string(), ScalarValue::from(box_x2)),
            ("box_y2".to_string(), ScalarValue::from(box_y2)),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

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
pub struct MouseUpParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl MouseUpParamStream {
    pub fn new() -> Self {
        Self {
            scales: vec![],
            input_params: vec![],
            stream_config: EventStreamConfig {
                types: vec![SceneGraphEventType::MouseUp],
                filter: Some(vec![EventStreamFilter(Arc::new(|event| {
                    let SceneGraphEvent::MouseUp(mouse_up) = event else {
                        return false;
                    };
                    mouse_up.button == MouseButton::Left
                }))]),
                ..Default::default()
            },
        }
    }
}

impl ParamStream for MouseUpParamStream {
    fn stream_config(&self) -> &EventStreamConfig {
        &self.stream_config
    }

    fn input_params(&self) -> &[String] {
        &self.input_params
    }

    fn input_scales(&self) -> &[Scale] {
        &self.scales
    }

    fn update(&self, _context: ParamStreamContext) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        // Clear anchor params
        let new_params = vec![
            ("box_x".to_string(), ScalarValue::from(0.0f32)),
            ("box_y".to_string(), ScalarValue::from(0.0f32)),
            ("box_x2".to_string(), ScalarValue::from(0.0f32)),
            ("box_y2".to_string(), ScalarValue::from(0.0f32)),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

        (
            new_params,
            UpdateStatus {
                rerender: true,
                rebuild_geometry: true,
            },
        )
    }
}

// #[derive(Debug, Clone)]
// pub struct PanDoubleClickParamStream {
//     scales: Vec<Scale>,
//     input_params: Vec<String>,
//     stream_config: EventStreamConfig,
// }
//
// impl PanDoubleClickParamStream {
//     pub fn new() -> Self {
//         Self {
//             scales: vec![],
//             input_params: vec![],
//             stream_config: EventStreamConfig {
//                 types: vec![SceneGraphEventType::DoubleClick],
//                 filter: Some(vec![EventStreamFilter(Arc::new(|event| {
//                     let SceneGraphEvent::DoubleClick(_) = event else {
//                         return false;
//                     };
//                     true
//                 }))]),
//                 ..Default::default()
//             },
//         }
//     }
// }
//
// impl ParamStream for PanDoubleClickParamStream {
//     fn stream_config(&self) -> &EventStreamConfig {
//         &self.stream_config
//     }
//
//     fn input_params(&self) -> &[String] {
//         &self.input_params
//     }
//
//     fn input_scales(&self) -> &[Scale] {
//         &self.scales
//     }
//
//     fn update(
//         &self,
//         _event: &SceneGraphEvent,
//         _params: &HashMap<String, ScalarValue>,
//         _scales: &[ConfiguredScale],
//         _group_path: &[usize],
//         _rtree: &SceneGraphRTree,
//     ) -> (HashMap<String, ScalarValue>, UpdateStatus) {
//         // Clear anchor params
//         // Null raw domains so we fall back to default
//         let new_params = vec![
//             ("x_domain_raw".to_string(), ScalarValue::Null),
//             ("y_domain_raw".to_string(), ScalarValue::Null),
//             ("anchor_range_position".to_string(), ScalarValue::Null),
//             ("anchor_x_domain".to_string(), ScalarValue::Null),
//             ("anchor_y_domain".to_string(), ScalarValue::Null),
//         ]
//         .into_iter()
//         .collect::<HashMap<_, _>>();
//
//         (
//             new_params,
//             UpdateStatus {
//                 rerender: true,
//                 rebuild_geometry: true,
//             },
//         )
//     }
// }

#[derive(Debug, Clone)]
pub struct BoxSelectController {
    x_scale: Scale,
    y_scale: Scale,
    x: String,
    y: String,
    box_x: Param,
    box_y: Param,
    box_x2: Param,
    box_y2: Param,
    param_streams: Vec<Arc<dyn ParamStream>>,
}

impl BoxSelectController {
    pub fn new(x: &str, x_scale: Scale, y: &str, y_scale: Scale) -> Self {
        // Initialize param streams
        let mouse_down_create_stream =
            BoxSelectInitParamStream::new(x_scale.clone(), y_scale.clone());
        let mouse_move_create_stream =
            BoxSelectDragExpandParamStream::new(x_scale.clone(), y_scale.clone());

        let mouse_down_move_stream =
            BoxSelectInitMoveParamStream::new(x_scale.clone(), y_scale.clone());
        let mouse_move_move_stream =
            BoxSelectDragMoveParamStream::new(x_scale.clone(), y_scale.clone());

        // let mouse_up_stream = MouseUpParamStream::new();
        // let pan_double_click_stream = PanDoubleClickParamStream::new();

        // Initialize params
        let box_x = Param::new("box_x", ScalarValue::from(0.0f32));
        let box_y = Param::new("box_y", ScalarValue::from(0.0f32));
        let box_x2 = Param::new("box_x2", ScalarValue::from(0.0f32));
        let box_y2 = Param::new("box_y2", ScalarValue::from(0.0f32));

        Self {
            x_scale,
            y_scale,
            x: x.to_string(),
            y: y.to_string(),
            box_x,
            box_y,
            box_x2,
            box_y2,
            param_streams: vec![
                Arc::new(mouse_down_create_stream),
                Arc::new(mouse_move_create_stream),
                Arc::new(mouse_down_move_stream),
                Arc::new(mouse_move_move_stream),
                // Arc::new(mouse_up_stream),
                // Arc::new(pan_double_click_stream),
            ],
        }
    }

    pub fn x_scale(&self) -> &Scale {
        &self.x_scale
    }

    pub fn y_scale(&self) -> &Scale {
        &self.y_scale
    }

    pub fn selection(&self) -> Result<Expr, AvengerChartError> {
        let x = ident(&self.x);
        let y = ident(&self.y);
        let box_x = self.box_x.expr();
        let box_y = self.box_y.expr();
        let box_x2 = self.box_x2.expr();
        let box_y2 = self.box_y2.expr();

        let x_lower =
            when(box_x2.clone().lt(box_x.clone()), box_x2.clone()).otherwise(box_x.clone())?;
        let x_upper =
            when(box_x2.clone().lt(box_x.clone()), box_x.clone()).otherwise(box_x2.clone())?;
        let y_lower =
            when(box_y2.clone().lt(box_y.clone()), box_y2.clone()).otherwise(box_y.clone())?;
        let y_upper =
            when(box_y2.clone().lt(box_y.clone()), box_y.clone()).otherwise(box_y2.clone())?;

        Ok(x_lower
            .clone()
            .lt_eq(x.clone())
            .and(x.clone().lt_eq(x_upper.clone()))
            .and(y_lower.clone().lt_eq(y.clone()))
            .and(y.clone().lt_eq(y_upper.clone())))
    }
}

impl Controller for BoxSelectController {
    fn name(&self) -> &str {
        "box-select"
    }

    fn param_streams(&self) -> Vec<Arc<dyn ParamStream>> {
        self.param_streams.clone()
    }

    fn params(&self) -> Vec<Param> {
        vec![
            self.box_x.clone(),
            self.box_y.clone(),
            self.box_x2.clone(),
            self.box_y2.clone(),
        ]
    }

    fn marks(&self) -> Vec<MarkOrGroup> {
        vec![MarkOrGroup::Mark(
            Mark::rect()
                .name(BOX_SELECT_MARK_NAME)
                .encode("x", scale_expr(&self.x_scale, &self.box_x).unwrap())
                .encode("y", scale_expr(&self.y_scale, &self.box_y).unwrap())
                .encode("x2", scale_expr(&self.x_scale, &self.box_x2).unwrap())
                .encode("y2", scale_expr(&self.y_scale, &self.box_y2).unwrap())
                .encode("fill", lit("rgba(0, 0, 255, 0.1)"))
                .encode("stroke", lit("rgba(0, 0, 255, 0.6)")),
        )]
    }
}
