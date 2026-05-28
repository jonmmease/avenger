//! Plot-level chart event bindings.
//!
//! Bindings are serializable chart specs. Runtime app crates lower them to
//! `avenger-eventstream` handlers and compile their DataFusion expressions into
//! physical expression programs.

use avenger_chart_core::{AvengerChartError, DefaultLogicalExprNodeExt, IntoExpr, Param};
use avenger_eventstream::scene::SceneGraphEventType;
use datafusion::{
    logical_expr::expr::Placeholder,
    prelude::{Expr, col},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::serialization::SerializableExpr;

pub const EVENT_TYPE_FIELD: &str = "__event_type";
pub const EVENT_X_FIELD: &str = "__event_x";
pub const EVENT_Y_FIELD: &str = "__event_y";
pub const EVENT_CANVAS_WIDTH_FIELD: &str = "__event_canvas_width";
pub const EVENT_CANVAS_HEIGHT_FIELD: &str = "__event_canvas_height";
pub const EVENT_WINDOW_WIDTH_FIELD: &str = "__event_window_width";
pub const EVENT_WINDOW_HEIGHT_FIELD: &str = "__event_window_height";
pub const EVENT_WHEEL_DELTA_X_FIELD: &str = "__event_wheel_delta_x";
pub const EVENT_WHEEL_DELTA_Y_FIELD: &str = "__event_wheel_delta_y";
pub const EVENT_BUTTON_FIELD: &str = "__event_button";
pub const EVENT_KEY_FIELD: &str = "__event_key";
pub const EVENT_SHIFT_FIELD: &str = "__event_shift";
pub const EVENT_CONTROL_FIELD: &str = "__event_control";
pub const EVENT_ALT_FIELD: &str = "__event_alt";
pub const EVENT_META_FIELD: &str = "__event_meta";

pub const START_X_FIELD: &str = "__start_x";
pub const START_Y_FIELD: &str = "__start_y";
pub const START_CANVAS_WIDTH_FIELD: &str = "__start_canvas_width";
pub const START_CANVAS_HEIGHT_FIELD: &str = "__start_canvas_height";
pub const START_WINDOW_WIDTH_FIELD: &str = "__start_window_width";
pub const START_WINDOW_HEIGHT_FIELD: &str = "__start_window_height";
pub const START_TIME_MS_FIELD: &str = "__start_time_ms";

pub const PREVIOUS_X_FIELD: &str = "__previous_x";
pub const PREVIOUS_Y_FIELD: &str = "__previous_y";
pub const PREVIOUS_TIME_MS_FIELD: &str = "__previous_time_ms";

pub const ELAPSED_MS_FIELD: &str = "__elapsed_ms";
pub const PREVIOUS_ELAPSED_MS_FIELD: &str = "__previous_elapsed_ms";

pub const PARAM_PREFIX: &str = "__param_";
pub const START_PARAM_PREFIX: &str = "__start_param_";
pub const PREVIOUS_PARAM_PREFIX: &str = "__previous_param_";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartEventType {
    MouseDown,
    MouseUp,
    Click,
    DoubleClick,
    MouseWheel,
    KeyPress,
    KeyRelease,
    CursorMoved,
    MarkMouseEnter,
    MarkMouseLeave,
    WindowResize,
    WindowResizeSettled,
    CanvasResize,
    CanvasResizeSettled,
    WindowMoved,
    WindowFocused,
    WindowCloseRequested,
}

impl From<ChartEventType> for SceneGraphEventType {
    fn from(value: ChartEventType) -> Self {
        match value {
            ChartEventType::MouseDown => SceneGraphEventType::MouseDown,
            ChartEventType::MouseUp => SceneGraphEventType::MouseUp,
            ChartEventType::Click => SceneGraphEventType::Click,
            ChartEventType::DoubleClick => SceneGraphEventType::DoubleClick,
            ChartEventType::MouseWheel => SceneGraphEventType::MouseWheel,
            ChartEventType::KeyPress => SceneGraphEventType::KeyPress,
            ChartEventType::KeyRelease => SceneGraphEventType::KeyRelease,
            ChartEventType::CursorMoved => SceneGraphEventType::CursorMoved,
            ChartEventType::MarkMouseEnter => SceneGraphEventType::MarkMouseEnter,
            ChartEventType::MarkMouseLeave => SceneGraphEventType::MarkMouseLeave,
            ChartEventType::WindowResize => SceneGraphEventType::WindowResize,
            ChartEventType::WindowResizeSettled => SceneGraphEventType::WindowResizeSettled,
            ChartEventType::CanvasResize => SceneGraphEventType::CanvasResize,
            ChartEventType::CanvasResizeSettled => SceneGraphEventType::CanvasResizeSettled,
            ChartEventType::WindowMoved => SceneGraphEventType::WindowMoved,
            ChartEventType::WindowFocused => SceneGraphEventType::WindowFocused,
            ChartEventType::WindowCloseRequested => SceneGraphEventType::WindowCloseRequested,
        }
    }
}

impl TryFrom<SceneGraphEventType> for ChartEventType {
    type Error = AvengerChartError;

    fn try_from(value: SceneGraphEventType) -> Result<Self, Self::Error> {
        match value {
            SceneGraphEventType::MouseDown => Ok(Self::MouseDown),
            SceneGraphEventType::MouseUp => Ok(Self::MouseUp),
            SceneGraphEventType::Click => Ok(Self::Click),
            SceneGraphEventType::DoubleClick => Ok(Self::DoubleClick),
            SceneGraphEventType::MouseWheel => Ok(Self::MouseWheel),
            SceneGraphEventType::KeyPress => Ok(Self::KeyPress),
            SceneGraphEventType::KeyRelease => Ok(Self::KeyRelease),
            SceneGraphEventType::CursorMoved => Ok(Self::CursorMoved),
            SceneGraphEventType::MarkMouseEnter => Ok(Self::MarkMouseEnter),
            SceneGraphEventType::MarkMouseLeave => Ok(Self::MarkMouseLeave),
            SceneGraphEventType::WindowResize => Ok(Self::WindowResize),
            SceneGraphEventType::WindowResizeSettled => Ok(Self::WindowResizeSettled),
            SceneGraphEventType::CanvasResize => Ok(Self::CanvasResize),
            SceneGraphEventType::CanvasResizeSettled => Ok(Self::CanvasResizeSettled),
            SceneGraphEventType::WindowMoved => Ok(Self::WindowMoved),
            SceneGraphEventType::WindowFocused => Ok(Self::WindowFocused),
            SceneGraphEventType::WindowCloseRequested => Ok(Self::WindowCloseRequested),
            SceneGraphEventType::FileChanged(_) => Err(AvengerChartError::InvalidArgument(
                "FileChanged events are not supported by chart event bindings".to_string(),
            )),
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartEventParamAssignment {
    pub param_name: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChartEventStream {
    pub event_type: Option<ChartEventType>,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub filters: Vec<LogicalExprNode>,
    pub source_group: Option<Vec<usize>>,
    pub mark_paths: Option<Vec<Vec<usize>>>,
}

impl ChartEventStream {
    pub fn on(event_type: impl IntoChartEventType) -> Self {
        Self {
            event_type: Some(event_type.into_chart_event_type()),
            ..Default::default()
        }
    }

    pub fn filter(mut self, expr: impl IntoExpr) -> Self {
        self.filters
            .push(expr_node(expr.into_expr(), "event stream filter"));
        self
    }

    pub fn source_group(mut self, group: Vec<usize>) -> Self {
        self.source_group = Some(group);
        self
    }

    pub fn mark_paths(mut self, paths: Vec<Vec<usize>>) -> Self {
        self.mark_paths = Some(paths);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartEventBetween {
    pub start: ChartEventStream,
    pub end: ChartEventStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartEventEvaluationMode {
    Preview,
    Exact,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartEventBinding {
    pub event_type: ChartEventType,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub filters: Vec<LogicalExprNode>,
    pub between: Option<ChartEventBetween>,
    pub throttle_ms: Option<u64>,
    pub consume: bool,
    pub assignments: Vec<ChartEventParamAssignment>,
    pub evaluation_mode: ChartEventEvaluationMode,
    pub settle_exact: bool,
}

impl ChartEventBinding {
    pub fn on(event_type: impl IntoChartEventType) -> Self {
        Self {
            event_type: event_type.into_chart_event_type(),
            filters: Vec::new(),
            between: None,
            throttle_ms: None,
            consume: false,
            assignments: Vec::new(),
            evaluation_mode: ChartEventEvaluationMode::Preview,
            settle_exact: false,
        }
    }

    pub fn filter(mut self, expr: impl IntoExpr) -> Self {
        self.filters
            .push(expr_node(expr.into_expr(), "event binding filter"));
        self
    }

    pub fn between(mut self, start: ChartEventStream, end: ChartEventStream) -> Self {
        self.between = Some(ChartEventBetween { start, end });
        self
    }

    pub fn throttle_ms(mut self, ms: u64) -> Self {
        self.throttle_ms = Some(ms);
        self
    }

    pub fn consume(mut self, consume: bool) -> Self {
        self.consume = consume;
        self
    }

    pub fn set_param(mut self, param: impl IntoParamName, expr: impl IntoExpr) -> Self {
        self.assignments.push(ChartEventParamAssignment {
            param_name: param.into_param_name(),
            expr: expr_node(expr.into_expr(), "event param assignment"),
        });
        self
    }

    pub fn preview(mut self) -> Self {
        self.evaluation_mode = ChartEventEvaluationMode::Preview;
        self
    }

    pub fn exact(mut self) -> Self {
        self.evaluation_mode = ChartEventEvaluationMode::Exact;
        self
    }

    pub fn settle_exact(mut self) -> Self {
        self.settle_exact = true;
        self
    }

    pub fn validate(&self) -> Result<(), AvengerChartError> {
        let mut targets = std::collections::HashSet::new();
        for assignment in &self.assignments {
            if !targets.insert(assignment.param_name.as_str()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Chart event binding assigns param '{}' more than once",
                    assignment.param_name
                )));
            }
        }
        Ok(())
    }
}

pub trait IntoChartEventType {
    fn into_chart_event_type(self) -> ChartEventType;
}

impl IntoChartEventType for ChartEventType {
    fn into_chart_event_type(self) -> ChartEventType {
        self
    }
}

impl IntoChartEventType for SceneGraphEventType {
    fn into_chart_event_type(self) -> ChartEventType {
        ChartEventType::try_from(self).expect("unsupported chart event type")
    }
}

pub trait IntoParamName {
    fn into_param_name(self) -> String;
}

impl IntoParamName for &Param {
    fn into_param_name(self) -> String {
        self.name.clone()
    }
}

impl IntoParamName for Param {
    fn into_param_name(self) -> String {
        self.name
    }
}

impl IntoParamName for &str {
    fn into_param_name(self) -> String {
        self.to_string()
    }
}

impl IntoParamName for String {
    fn into_param_name(self) -> String {
        self
    }
}

pub fn x() -> Expr {
    col(EVENT_X_FIELD)
}

pub fn y() -> Expr {
    col(EVENT_Y_FIELD)
}

pub fn canvas_width() -> Expr {
    col(EVENT_CANVAS_WIDTH_FIELD)
}

pub fn canvas_height() -> Expr {
    col(EVENT_CANVAS_HEIGHT_FIELD)
}

pub fn window_width() -> Expr {
    col(EVENT_WINDOW_WIDTH_FIELD)
}

pub fn window_height() -> Expr {
    col(EVENT_WINDOW_HEIGHT_FIELD)
}

pub fn wheel_delta_x() -> Expr {
    col(EVENT_WHEEL_DELTA_X_FIELD)
}

pub fn wheel_delta_y() -> Expr {
    col(EVENT_WHEEL_DELTA_Y_FIELD)
}

pub fn button() -> Expr {
    col(EVENT_BUTTON_FIELD)
}

pub fn key() -> Expr {
    col(EVENT_KEY_FIELD)
}

pub fn shift() -> Expr {
    col(EVENT_SHIFT_FIELD)
}

pub fn control() -> Expr {
    col(EVENT_CONTROL_FIELD)
}

pub fn alt() -> Expr {
    col(EVENT_ALT_FIELD)
}

pub fn meta() -> Expr {
    col(EVENT_META_FIELD)
}

pub fn start_x() -> Expr {
    col(START_X_FIELD)
}

pub fn start_y() -> Expr {
    col(START_Y_FIELD)
}

pub fn start_canvas_width() -> Expr {
    col(START_CANVAS_WIDTH_FIELD)
}

pub fn start_canvas_height() -> Expr {
    col(START_CANVAS_HEIGHT_FIELD)
}

pub fn elapsed_ms() -> Expr {
    col(ELAPSED_MS_FIELD)
}

pub fn previous_x() -> Expr {
    col(PREVIOUS_X_FIELD)
}

pub fn previous_y() -> Expr {
    col(PREVIOUS_Y_FIELD)
}

pub fn previous_elapsed_ms() -> Expr {
    col(PREVIOUS_ELAPSED_MS_FIELD)
}

pub fn dx() -> Expr {
    x() - start_x()
}

pub fn dy() -> Expr {
    y() - start_y()
}

pub fn param(param: impl IntoParamName) -> Expr {
    let name = param.into_param_name();
    Expr::Placeholder(Placeholder {
        id: format!("${name}"),
        data_type: None,
    })
}

pub fn start_param(param: impl IntoParamName) -> Expr {
    col(format!("{}{}", START_PARAM_PREFIX, param.into_param_name()))
}

pub fn previous_param(param: impl IntoParamName) -> Expr {
    col(format!(
        "{}{}",
        PREVIOUS_PARAM_PREFIX,
        param.into_param_name()
    ))
}

pub fn param_column_name(param_name: &str) -> String {
    format!("{PARAM_PREFIX}{param_name}")
}

pub fn start_param_column_name(param_name: &str) -> String {
    format!("{START_PARAM_PREFIX}{param_name}")
}

pub fn previous_param_column_name(param_name: &str) -> String {
    format!("{PREVIOUS_PARAM_PREFIX}{param_name}")
}

fn expr_node(expr: Expr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_expr(expr)
        .unwrap_or_else(|err| panic!("Failed to serialize {label}: {err}"))
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::lit;

    use super::*;

    #[test]
    fn chart_event_binding_rejects_duplicate_assignments() {
        let binding = ChartEventBinding::on(ChartEventType::CanvasResize)
            .set_param("width", canvas_width())
            .set_param("width", canvas_height());

        let err = binding.validate().expect_err("duplicate assignment");
        assert!(err.to_string().contains("more than once"));
    }

    #[test]
    fn chart_event_binding_serializes() {
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(
                ChartEventStream::on(ChartEventType::MouseDown).filter(button().eq(lit("left"))),
                ChartEventStream::on(ChartEventType::MouseUp).filter(button().eq(lit("left"))),
            )
            .filter(shift().eq(lit(false)))
            .set_param("x0", start_param("x0") + dx())
            .preview()
            .settle_exact();

        let json = serde_json::to_string(&binding).expect("serialize binding");
        let restored: ChartEventBinding = serde_json::from_str(&json).expect("deserialize binding");
        assert_eq!(restored.event_type, ChartEventType::CursorMoved);
        assert_eq!(restored.assignments.len(), 1);
        assert!(restored.settle_exact);
    }
}
