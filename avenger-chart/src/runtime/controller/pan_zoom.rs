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
use avenger_scales::scales::{linear::LinearScale, ConfiguredScale};
use datafusion::{
    prelude::{lit, DataFrame},
    scalar::ScalarValue,
};

use super::{param_stream::ParamStream, Controller};
use crate::runtime::controller::param_stream::ParamStreamContext;
use crate::{
    param::Param,
    types::scales::{Scale, ScaleRange},
};

#[derive(Debug, Clone)]
pub struct PanMouseDownParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl PanMouseDownParamStream {
    pub fn new(x_scale: Scale, y_scale: Scale) -> Self {
        Self {
            scales: vec![x_scale, y_scale],
            input_params: vec![],
            stream_config: EventStreamConfig {
                types: vec![SceneGraphEventType::MouseDown],
                filter: Some(vec![EventStreamFilter(Arc::new(|event| {
                    let SceneGraphEvent::MouseDown(mouse_down) = event else {
                        return false;
                    };
                    mouse_down.button == MouseButton::Left
                }))]),
                ..Default::default()
            },
        }
    }
}

impl ParamStream for PanMouseDownParamStream {
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
                    rebuild_geometry: false,
                },
            );
        }

        // Save state as controller params
        let x_domain_scalar = x_scale.get_domain_scalar();
        let y_domain_scalar = y_scale.get_domain_scalar();
        let range_position =
            ScalarValue::List(Arc::new(
                ListArray::from_iter_primitive::<Float32Type, _, _>(vec![Some(vec![
                    Some(plot_x),
                    Some(plot_y),
                ])]),
            ));

        // return new param values
        let new_params = vec![
            ("anchor_range_position".to_string(), range_position.clone()),
            ("anchor_x_domain".to_string(), x_domain_scalar),
            ("anchor_y_domain".to_string(), y_domain_scalar),
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
pub struct PanMouseMoveParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl PanMouseMoveParamStream {
    pub fn new(x_scale: Scale, y_scale: Scale) -> Self {
        // Mouse down config
        let left_mouse_down_config = EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            filter: Some(vec![EventStreamFilter(Arc::new(|event| {
                let SceneGraphEvent::MouseDown(mouse_down) = event else {
                    return false;
                };
                mouse_down.button == MouseButton::Left
            }))]),
            ..Default::default()
        };

        // Mouse up config
        let left_mouse_up_config = EventStreamConfig {
            types: vec![SceneGraphEventType::MouseUp],
            filter: Some(vec![EventStreamFilter(Arc::new(|event| {
                let SceneGraphEvent::MouseUp(mouse_up) = event else {
                    return false;
                };
                mouse_up.button == MouseButton::Left
            }))]),
            ..Default::default()
        };

        // Drag config
        let drag_config = EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            between: Some((
                Box::new(left_mouse_down_config.clone()),
                Box::new(left_mouse_up_config.clone()),
            )),
            ..Default::default()
        };

        Self {
            scales: vec![x_scale, y_scale],
            input_params: vec![
                "anchor_range_position".to_string(),
                "anchor_x_domain".to_string(),
                "anchor_y_domain".to_string(),
            ],
            stream_config: drag_config,
        }
    }
}

impl ParamStream for PanMouseMoveParamStream {
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
        let Some(ScalarValue::List(range_position)) = params.get("anchor_range_position") else {
            // Don't update params or rerender
            return (
                HashMap::new(),
                UpdateStatus {
                    rerender: false,
                    rebuild_geometry: false,
                },
            );
        };
        let range_position = range_position.value(0);
        let range_position = range_position.as_primitive::<Float32Type>();
        let anchor_x = range_position.value(0);
        let anchor_y = range_position.value(1);

        // Extract stored domains
        let Some(ScalarValue::List(x_domain)) = params.get("anchor_x_domain") else {
            // Don't update params or rerender
            return (
                HashMap::new(),
                UpdateStatus {
                    rerender: false,
                    rebuild_geometry: false,
                },
            );
        };
        let x_domain = x_domain.value(0);

        let Some(ScalarValue::List(y_domain)) = params.get("anchor_y_domain") else {
            // Don't update params or rerender
            return (
                HashMap::new(),
                UpdateStatus {
                    rerender: false,
                    rebuild_geometry: false,
                },
            );
        };
        let y_domain = y_domain.value(0);

        // Get the cursor position in range space
        let event_position = event.position().unwrap();
        let plot_origin = rtree.group_origin(group_path).unwrap();
        let plot_x = event_position[0] - plot_origin[0];
        let plot_y = event_position[1] - plot_origin[1];

        // Build scales reset to the anchor domains
        let x_scale = scales[0].clone().with_domain(x_domain);
        let y_scale = scales[1].clone().with_domain(y_domain);

        // Compute pan deltas
        let (range_start, range_end) = x_scale.numeric_interval_range().unwrap();
        let x_delta = (plot_x - anchor_x) / (range_end - range_start);

        let (range_start, range_end) = y_scale.numeric_interval_range().unwrap();
        let y_delta = (plot_y - anchor_y) / (range_end - range_start);

        // Update domains
        let x_domain_scalar = x_scale.pan(x_delta).unwrap().get_domain_scalar();
        let y_domain_scalar = y_scale.pan(y_delta).unwrap().get_domain_scalar();

        // Return new param values
        let new_params = vec![
            ("x_domain_raw".to_string(), x_domain_scalar),
            ("y_domain_raw".to_string(), y_domain_scalar),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

        (
            new_params,
            UpdateStatus {
                rerender: true,
                rebuild_geometry: false,
            },
        )
    }
}

#[derive(Debug, Clone)]
pub struct PanMouseUpParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl PanMouseUpParamStream {
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

impl ParamStream for PanMouseUpParamStream {
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
            ("anchor_range_position".to_string(), ScalarValue::Null),
            ("anchor_x_domain".to_string(), ScalarValue::Null),
            ("anchor_y_domain".to_string(), ScalarValue::Null),
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
pub struct PanDoubleClickParamStream {
    scales: Vec<Scale>,
    input_params: Vec<String>,
    stream_config: EventStreamConfig,
}

impl PanDoubleClickParamStream {
    pub fn new() -> Self {
        Self {
            scales: vec![],
            input_params: vec![],
            stream_config: EventStreamConfig {
                types: vec![SceneGraphEventType::DoubleClick],
                filter: Some(vec![EventStreamFilter(Arc::new(|event| {
                    let SceneGraphEvent::DoubleClick(_) = event else {
                        return false;
                    };
                    true
                }))]),
                ..Default::default()
            },
        }
    }
}

impl ParamStream for PanDoubleClickParamStream {
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
        // Null raw domains so we fall back to default
        let new_params = vec![
            ("x_domain_raw".to_string(), ScalarValue::Null),
            ("y_domain_raw".to_string(), ScalarValue::Null),
            ("anchor_range_position".to_string(), ScalarValue::Null),
            ("anchor_x_domain".to_string(), ScalarValue::Null),
            ("anchor_y_domain".to_string(), ScalarValue::Null),
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
pub struct PanZoomController {
    x_scale: Scale,
    y_scale: Scale,
    x_domain_raw: Param,
    y_domain_raw: Param,
    width: Param,
    height: Param,
    param_streams: Vec<Arc<dyn ParamStream>>,
}

impl PanZoomController {
    pub fn with_auto_range(
        data_frame: DataFrame,
        x: &str,
        y: &str,
        initial_width: f32,
        initial_height: f32,
    ) -> Self {
        // Initialize params
        let x_domain_raw = Param::new("x_domain_raw", ScalarValue::Null);
        let y_domain_raw = Param::new("y_domain_raw", ScalarValue::Null);
        let width = Param::new("width", ScalarValue::from(initial_width));
        let height = Param::new("height", ScalarValue::from(initial_height));

        // Initialize scales
        let x_scale = Scale::new(LinearScale)
            .domain_data_field(Arc::new(data_frame.clone()), x)
            .raw_domain(&x_domain_raw)
            .range(ScaleRange::new_interval(lit(0.0), width.clone()));

        let y_scale = Scale::new(LinearScale)
            .domain_data_field(Arc::new(data_frame.clone()), y)
            .raw_domain(&y_domain_raw)
            .range(ScaleRange::new_interval(height.clone(), lit(0.0)));

        // Initialize param streams
        let pan_mouse_down_stream = PanMouseDownParamStream::new(x_scale.clone(), y_scale.clone());
        let pan_mouse_move_stream = PanMouseMoveParamStream::new(x_scale.clone(), y_scale.clone());
        let pan_mouse_up_stream = PanMouseUpParamStream::new();
        let pan_double_click_stream = PanDoubleClickParamStream::new();

        Self {
            x_scale,
            y_scale,
            x_domain_raw,
            y_domain_raw,
            width,
            height,
            param_streams: vec![
                Arc::new(pan_mouse_down_stream),
                Arc::new(pan_mouse_move_stream),
                Arc::new(pan_mouse_up_stream),
                Arc::new(pan_double_click_stream),
            ],
        }
    }

    pub fn x_scale(&self) -> &Scale {
        &self.x_scale
    }

    pub fn y_scale(&self) -> &Scale {
        &self.y_scale
    }

    pub fn width(&self) -> &Param {
        &self.width
    }

    pub fn height(&self) -> &Param {
        &self.height
    }

    pub fn x_domain_raw(&self) -> &Param {
        &self.x_domain_raw
    }

    pub fn y_domain_raw(&self) -> &Param {
        &self.y_domain_raw
    }
}

impl Controller for PanZoomController {
    fn name(&self) -> &str {
        "pan-zoom"
    }

    fn param_streams(&self) -> Vec<Arc<dyn ParamStream>> {
        self.param_streams.clone()
    }

    fn params(&self) -> Vec<Param> {
        vec![
            self.x_domain_raw.clone(),
            self.y_domain_raw.clone(),
            self.width.clone(),
            self.height.clone(),
        ]
    }
}
