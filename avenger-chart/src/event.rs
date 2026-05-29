//! Plot-level chart event bindings.
//!
//! Bindings are serializable chart specs. Runtime app crates lower them to
//! `avenger-eventstream` handlers and compile their DataFusion expressions into
//! physical expression programs.

use std::collections::BTreeSet;

use avenger_chart_core::{AvengerChartError, DefaultLogicalExprNodeExt, IntoExpr, Param};
use avenger_eventstream::scene::SceneGraphEventType;
use datafusion::{
    functions_array::expr_fn::{array_element, make_array},
    logical_expr::expr::Placeholder,
    prelude::{Expr, col, lit},
};
use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};
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

// Reserved derived interaction columns. The channel name is appended to the
// prefix (e.g. `__event_coord_x`). The channel is author-chosen, not a
// hard-coded Cartesian assumption, so future coordinate systems can reuse the
// same helpers with channels like `r` and `theta`.
pub const EVENT_COORD_PREFIX: &str = "__event_coord_";
pub const START_COORD_PREFIX: &str = "__start_coord_";
pub const EVENT_AT_START_COORD_PREFIX: &str = "__event_at_start_coord_";
pub const PREVIOUS_COORD_PREFIX: &str = "__previous_coord_";
pub const EVENT_DOMAIN_PREFIX: &str = "__event_domain_";
pub const START_DOMAIN_PREFIX: &str = "__start_domain_";

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

/// Current event point inverted through the current evaluated coordinate scope.
pub fn event_coord(channel: &str) -> Expr {
    col(event_coord_column_name(channel))
}

/// Start event point inverted through the frozen start coordinate scope.
pub fn start_coord(channel: &str) -> Expr {
    col(start_coord_column_name(channel))
}

/// Current event point inverted through the frozen start coordinate scope.
///
/// This is the primary pan delta primitive: it avoids feedback as raw domains
/// update during a drag because it always uses the start scale.
pub fn event_at_start_coord(channel: &str) -> Expr {
    col(event_at_start_coord_column_name(channel))
}

/// Previous event point inverted through the previous/current coordinate scope.
pub fn previous_coord(channel: &str) -> Expr {
    col(previous_coord_column_name(channel))
}

/// Current configured domain for a coordinate channel as a two-element list.
pub fn event_domain(channel: &str) -> Expr {
    col(event_domain_column_name(channel))
}

/// Frozen start configured domain for a coordinate channel as a two-element list.
pub fn start_domain(channel: &str) -> Expr {
    col(start_domain_column_name(channel))
}

/// Build a two-element interval list `[min, max]` from scalar expressions.
///
/// This is a plain array constructor; pair it with [`interval_start`] and
/// [`interval_end`] to decompose a two-element list such as a scale domain.
pub fn interval(min: impl IntoExpr, max: impl IntoExpr) -> Expr {
    make_array(vec![min.into_expr(), max.into_expr()])
}

/// Extract the first element from a two-element interval list.
pub fn interval_start(interval: impl IntoExpr) -> Expr {
    array_element(interval.into_expr(), lit(1_i64))
}

/// Extract the second element from a two-element interval list.
pub fn interval_end(interval: impl IntoExpr) -> Expr {
    array_element(interval.into_expr(), lit(2_i64))
}

pub fn event_coord_column_name(channel: &str) -> String {
    format!("{EVENT_COORD_PREFIX}{channel}")
}

pub fn start_coord_column_name(channel: &str) -> String {
    format!("{START_COORD_PREFIX}{channel}")
}

pub fn event_at_start_coord_column_name(channel: &str) -> String {
    format!("{EVENT_AT_START_COORD_PREFIX}{channel}")
}

pub fn previous_coord_column_name(channel: &str) -> String {
    format!("{PREVIOUS_COORD_PREFIX}{channel}")
}

pub fn event_domain_column_name(channel: &str) -> String {
    format!("{EVENT_DOMAIN_PREFIX}{channel}")
}

pub fn start_domain_column_name(channel: &str) -> String {
    format!("{START_DOMAIN_PREFIX}{channel}")
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

/// Derived interaction columns requested by a binding's expressions, grouped by
/// the kind of inversion they need. The channel sets drive both the event schema
/// and which coordinate inversions the runtime must perform per event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InteractionColumnRequests {
    pub current_coord: BTreeSet<String>,
    pub start_coord: BTreeSet<String>,
    pub event_at_start_coord: BTreeSet<String>,
    pub previous_coord: BTreeSet<String>,
    pub current_domain: BTreeSet<String>,
    pub start_domain: BTreeSet<String>,
}

impl InteractionColumnRequests {
    pub fn is_empty(&self) -> bool {
        self.current_coord.is_empty()
            && self.start_coord.is_empty()
            && self.event_at_start_coord.is_empty()
            && self.previous_coord.is_empty()
            && self.current_domain.is_empty()
            && self.start_domain.is_empty()
    }

    /// Union of every coordinate channel referenced by any request kind.
    pub fn all_channels(&self) -> BTreeSet<String> {
        let mut channels = BTreeSet::new();
        channels.extend(self.current_coord.iter().cloned());
        channels.extend(self.start_coord.iter().cloned());
        channels.extend(self.event_at_start_coord.iter().cloned());
        channels.extend(self.previous_coord.iter().cloned());
        channels.extend(self.current_domain.iter().cloned());
        channels.extend(self.start_domain.iter().cloned());
        channels
    }

    fn record_column(&mut self, name: &str) {
        // Most-specific prefixes first; the prefixes are mutually exclusive but
        // ordering keeps the intent explicit.
        if let Some(channel) = name.strip_prefix(EVENT_AT_START_COORD_PREFIX) {
            self.event_at_start_coord.insert(channel.to_string());
        } else if let Some(channel) = name.strip_prefix(EVENT_COORD_PREFIX) {
            self.current_coord.insert(channel.to_string());
        } else if let Some(channel) = name.strip_prefix(START_COORD_PREFIX) {
            self.start_coord.insert(channel.to_string());
        } else if let Some(channel) = name.strip_prefix(PREVIOUS_COORD_PREFIX) {
            self.previous_coord.insert(channel.to_string());
        } else if let Some(channel) = name.strip_prefix(EVENT_DOMAIN_PREFIX) {
            self.current_domain.insert(channel.to_string());
        } else if let Some(channel) = name.strip_prefix(START_DOMAIN_PREFIX) {
            self.start_domain.insert(channel.to_string());
        }
    }
}

/// Scan binding filter/assignment expressions for reserved interaction columns.
pub fn scan_interaction_columns(exprs: &[Expr]) -> InteractionColumnRequests {
    let mut requests = InteractionColumnRequests::default();
    for expr in exprs {
        let _ = expr.apply(|node| {
            if let Expr::Column(column) = node {
                requests.record_column(&column.name);
            }
            Ok(TreeNodeRecursion::Continue)
        });
    }
    requests
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
    fn interaction_helpers_produce_expected_column_names() {
        assert_eq!(event_coord_column_name("x"), "__event_coord_x".to_string());
        assert_eq!(
            event_at_start_coord_column_name("y"),
            "__event_at_start_coord_y".to_string()
        );
        assert_eq!(
            start_domain_column_name("x"),
            "__start_domain_x".to_string()
        );
        // The public helper expressions reference those reserved columns.
        assert_eq!(event_coord("x"), col("__event_coord_x"));
        assert_eq!(start_domain("y"), col("__start_domain_y"));
    }

    #[test]
    fn interval_helpers_build_valid_expressions() {
        // interval builds a two-element list; interval_start/end index it.
        let interval_expr = interval(lit(2.0), lit(8.0));
        let start = interval_start(start_domain("x"));
        let end = interval_end(start_domain("x"));
        // These must serialize as ordinary DataFusion expressions.
        for expr in [interval_expr, start, end] {
            LogicalExprNode::from_expr(expr).expect("interaction interval expr serializes");
        }
    }

    #[test]
    fn scan_detects_requested_interaction_columns() {
        let requests = scan_interaction_columns(&[
            event_at_start_coord("x") - start_coord("x"),
            interval_start(start_domain("x")),
        ]);
        assert!(requests.event_at_start_coord.contains("x"));
        assert!(requests.start_coord.contains("x"));
        assert!(requests.start_domain.contains("x"));
        assert!(requests.current_coord.is_empty());
        assert!(!requests.is_empty());
        assert!(requests.all_channels().contains("x"));
    }

    #[test]
    fn scan_without_coordinate_helpers_is_empty() {
        let requests = scan_interaction_columns(&[x() - start_x(), button().eq(lit("left"))]);
        assert!(requests.is_empty());
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
