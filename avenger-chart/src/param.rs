use crate::runtime::controller::param_stream::{ParamStream, ParamStreamContext};
use crate::types::scales::Scale;
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scales::scales::ConfiguredScale;
use datafusion::physical_expr::aggregate::utils::Hashable;
use datafusion::{logical_expr::expr::Placeholder, prelude::Expr, scalar::ScalarValue};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: ScalarValue,
    pub stream: Option<Arc<dyn ParamStream>>,
}

impl Param {
    pub fn new<S: Into<String>, T: Into<ScalarValue>>(name: S, default: T) -> Self {
        Self {
            name: name.into(),
            default: default.into(),
            stream: None,
        }
    }

    pub fn expr(&self) -> Expr {
        Expr::Placeholder(Placeholder {
            id: format!("${}", self.name),
            data_type: Some(self.default.data_type()),
        })
    }

    pub fn with_stream(
        mut self,
        config: EventStreamConfig,
        input_params: &[String],
        input_scales: &[Scale],
        stream: Arc<
            dyn Fn(
                    &SceneGraphEvent,
                    &HashMap<String, ScalarValue>,
                    &[ConfiguredScale],
                    [f32; 2],
                ) -> Option<ScalarValue>
                + Send
                + Sync
                + 'static,
        >,
    ) -> Self {
        Self {
            stream: Some(Arc::new(InlineParamStream {
                config,
                input_params: Vec::from(input_params),
                input_scales: Vec::from(input_scales),
                param_name: self.name.clone(),
                callback: InlineParamStreamCallback(stream),
            })),
            ..self
        }
    }
}

impl From<(String, ScalarValue)> for Param {
    fn from(params: (String, ScalarValue)) -> Self {
        Param::new(params.0, params.1)
    }
}

impl From<Param> for Expr {
    fn from(param: Param) -> Self {
        param.expr()
    }
}

impl From<&Param> for Expr {
    fn from(param: &Param) -> Self {
        param.expr()
    }
}

#[derive(Clone)]
pub struct InlineParamStreamCallback(
    Arc<
        dyn Fn(
                &SceneGraphEvent,
                &HashMap<String, ScalarValue>,
                &[ConfiguredScale],
                [f32; 2],
            ) -> Option<ScalarValue>
            + Send
            + Sync
            + 'static,
    >,
);

impl Debug for InlineParamStreamCallback {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "InlineParamStreamCallback")
    }
}

#[derive(Clone, Debug)]
pub struct InlineParamStream {
    config: EventStreamConfig,
    input_params: Vec<String>,
    input_scales: Vec<Scale>,
    param_name: String,
    callback: InlineParamStreamCallback,
}

impl ParamStream for InlineParamStream {
    fn stream_config(&self) -> &EventStreamConfig {
        &self.config
    }

    fn input_params(&self) -> &[String] {
        &self.input_params
    }

    fn input_scales(&self) -> &[Scale] {
        &self.input_scales
    }

    fn update(
        &self,
        context: ParamStreamContext
    ) -> (HashMap<String, ScalarValue>, UpdateStatus) {
        let event = context.event;
        let params = context.params;
        let scales= context.scales;
        let rtree = context.rtree;

        let group_origin = rtree.group_origin(&[0]).unwrap();
        let Some(new_value) = self.callback.0(event, params, scales, group_origin) else {
            return (Default::default(), Default::default())
        };
        let new_params = vec![(self.param_name.clone(), new_value)]
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
