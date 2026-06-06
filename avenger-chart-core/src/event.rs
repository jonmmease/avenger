//! Plot-level chart event bindings.
//!
//! Bindings are serializable chart specs. Runtime app crates lower them to
//! `avenger-eventstream` handlers and compile their DataFusion expressions into
//! physical expression programs.

use std::collections::BTreeSet;

use crate::{
    AvengerChartError, DefaultLogicalExprNodeExt, IntoExpr, LegendSurfaceKind, Param,
    SelectionUpdate, SerializableExpr, StoreUpdate,
    scene_query::{SceneGeometryQueryGeometry, SceneQueryClauseId, SelectionSceneQuery},
    validate_structural_id,
};
use avenger_common::cursor::CursorStyle;
use datafusion::{
    functions_array::expr_fn::{array_element, make_array},
    logical_expr::expr::Placeholder,
    prelude::{Expr, SessionContext, col, lit, when},
};
use datafusion_common::tree_node::Transformed;
use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

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
pub const START_EVENT_ID_FIELD: &str = "__start_event_id";

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
pub const EVENT_AT_START_CLIPPED_COORD_PREFIX: &str = "__event_at_start_clipped_coord_";
pub const PREVIOUS_COORD_PREFIX: &str = "__previous_coord_";
pub const EVENT_DOMAIN_PREFIX: &str = "__event_domain_";
pub const START_DOMAIN_PREFIX: &str = "__start_domain_";
pub const EVENT_DATUM_PREFIX: &str = "__event_datum_";
pub const LEGEND_VALUE_FIELD: &str = "__legend_value";
pub const LEGEND_LABEL_FIELD: &str = "__legend_label";
pub const LEGEND_NAME_FIELD: &str = "__legend_name";
pub const LEGEND_CHANNEL_FIELD: &str = "__legend_channel";
pub const LEGEND_INDEX_FIELD: &str = "__legend_index";
pub const LEGEND_ID_FIELD: &str = "__legend_id";
pub const LEGEND_SURFACE_KEY_FIELD: &str = "__legend_surface_key";
pub const LEGEND_SURFACE_KIND_FIELD: &str = "__legend_surface_kind";
pub const LEGEND_SURFACE_KIND_DISCRETE_ITEM: &str = "discrete-item";
pub const LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR: &str = "continuous-colorbar";
pub const LEGEND_ORIENTATION_FIELD: &str = "__legend_orientation";
pub const LEGEND_VALUE_CHANNEL_FIELD: &str = "__legend_value_channel";
pub const LEGEND_BAND_CHANNEL_FIELD: &str = "__legend_band_channel";
pub const EVENT_PLOT_WIDTH_FIELD: &str = "__event_plot_width";
pub const EVENT_PLOT_HEIGHT_FIELD: &str = "__event_plot_height";
pub const START_PLOT_WIDTH_FIELD: &str = "__start_plot_width";
pub const START_PLOT_HEIGHT_FIELD: &str = "__start_plot_height";
pub const EVENT_SCOPE_ID_FIELD: &str = "__event_scope_id";
pub const START_SCOPE_ID_FIELD: &str = "__start_scope_id";
pub const EVENT_FACET_VALUE_PREFIX: &str = "__event_facet_value_";
pub const START_FACET_VALUE_PREFIX: &str = "__start_facet_value_";
pub const EVENT_PATH_FIELD: &str = "__event_path";
pub const EVENT_PATH_SVG_FIELD: &str = "__event_path_svg";
pub const DEFAULT_EVENT_PATH_MIN_DISTANCE_PX: f32 = 2.0;

pub const LEGEND_ITEM_ONLY_DATUM_FIELDS: &[&str] =
    &[LEGEND_VALUE_FIELD, LEGEND_LABEL_FIELD, LEGEND_INDEX_FIELD];

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartEventAssignmentScope {
    #[default]
    Current,
    Start,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartEventParamAssignment {
    pub param_name: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    #[serde(default)]
    pub scope: ChartEventAssignmentScope,
    #[serde(default)]
    pub replace_scoped_values: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartEventStoreAssignment {
    pub store_name: String,
    pub update: StoreUpdate,
    #[serde(default)]
    pub scope: ChartEventAssignmentScope,
    #[serde(default)]
    pub replace_scoped_values: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartEventSelectionAssignment {
    pub selection_id: String,
    pub update: SelectionUpdate,
    #[serde(default)]
    pub scope: ChartEventAssignmentScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartEventScopeTarget {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subplot_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_coord_node_path_prefix: Option<Vec<usize>>,
}

impl ChartEventScopeTarget {
    pub fn within_subplot(id: impl Into<String>) -> Self {
        Self {
            subplot_ids: vec![id.into()],
            resolved_coord_node_path_prefix: None,
        }
    }

    pub fn within_subplots<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            subplot_ids: ids.into_iter().map(Into::into).collect(),
            resolved_coord_node_path_prefix: None,
        }
    }

    pub fn subplot_ids(&self) -> &[String] {
        &self.subplot_ids
    }

    #[doc(hidden)]
    pub fn resolved_coord_node_path_prefix(&self) -> Option<&[usize]> {
        self.resolved_coord_node_path_prefix.as_deref()
    }

    #[doc(hidden)]
    pub fn with_resolved_coord_node_path_prefix(prefix: Vec<usize>) -> Self {
        Self {
            subplot_ids: Vec::new(),
            resolved_coord_node_path_prefix: (!prefix.is_empty()).then_some(prefix),
        }
    }

    pub fn validate(&self) -> Result<(), AvengerChartError> {
        for id in &self.subplot_ids {
            validate_structural_id("subplot target", id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartEventSurfaceTarget {
    All,
    PlotSurface,
    LegendSurface {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        surface_keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        kinds: Vec<LegendSurfaceKind>,
    },
}

impl ChartEventSurfaceTarget {
    pub fn validate(&self) -> Result<(), AvengerChartError> {
        if let ChartEventSurfaceTarget::LegendSurface { surface_keys, .. } = self {
            for key in surface_keys {
                if key.is_empty() {
                    return Err(AvengerChartError::InvalidArgument(
                        "Legend event surface key must not be empty".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn legend_surface_keys(&self) -> Option<&[String]> {
        match self {
            ChartEventSurfaceTarget::LegendSurface { surface_keys, .. } => Some(surface_keys),
            _ => None,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChartEventStream {
    pub event_type: Option<ChartEventType>,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub filters: Vec<LogicalExprNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mark_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_source_group: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_mark_paths: Option<Vec<Vec<usize>>>,
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

    pub fn mark(mut self, id: impl Into<String>) -> Self {
        self.mark_ids = vec![id.into()];
        self
    }

    pub fn marks<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.mark_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn mark_ids(&self) -> &[String] {
        &self.mark_ids
    }

    #[doc(hidden)]
    pub fn resolved_source_group(&self) -> Option<&[usize]> {
        self.resolved_source_group.as_deref()
    }

    #[doc(hidden)]
    pub fn resolved_mark_paths(&self) -> Option<&[Vec<usize>]> {
        self.resolved_mark_paths.as_deref()
    }

    #[doc(hidden)]
    pub fn with_resolved_source_group(mut self, group: Vec<usize>) -> Self {
        self.resolved_source_group = Some(group);
        self
    }

    #[doc(hidden)]
    pub fn with_resolved_mark_paths(mut self, paths: Vec<Vec<usize>>) -> Self {
        self.resolved_mark_paths = Some(paths);
        self
    }

    pub fn validate(&self) -> Result<(), AvengerChartError> {
        for id in &self.mark_ids {
            validate_structural_id("mark target", id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChartEventBetween {
    pub start: ChartEventStream,
    pub end: ChartEventStream,
    #[serde(default)]
    pub emit_end_event: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_path_min_distance_px: Option<f32>,
    pub throttle_ms: Option<u64>,
    pub consume: bool,
    pub assignments: Vec<ChartEventParamAssignment>,
    #[serde(default)]
    pub store_assignments: Vec<ChartEventStoreAssignment>,
    #[serde(default)]
    pub selection_assignments: Vec<ChartEventSelectionAssignment>,
    pub evaluation_mode: ChartEventEvaluationMode,
    pub settle_exact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_target: Option<ChartEventScopeTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_target: Option<ChartEventSurfaceTarget>,
}

impl ChartEventBinding {
    pub fn on(event_type: impl IntoChartEventType) -> Self {
        Self {
            event_type: event_type.into_chart_event_type(),
            filters: Vec::new(),
            between: None,
            event_path_min_distance_px: None,
            throttle_ms: None,
            consume: false,
            assignments: Vec::new(),
            store_assignments: Vec::new(),
            selection_assignments: Vec::new(),
            evaluation_mode: ChartEventEvaluationMode::Preview,
            settle_exact: false,
            scope_target: None,
            surface_target: None,
        }
    }

    pub fn on_between_end(start: ChartEventStream, end: ChartEventStream) -> Self {
        let event_type = end
            .event_type
            .expect("ChartEventBinding::on_between_end requires an end stream with an event type");
        Self {
            event_type,
            filters: Vec::new(),
            between: Some(ChartEventBetween {
                start,
                end,
                emit_end_event: true,
            }),
            event_path_min_distance_px: None,
            throttle_ms: None,
            consume: false,
            assignments: Vec::new(),
            store_assignments: Vec::new(),
            selection_assignments: Vec::new(),
            evaluation_mode: ChartEventEvaluationMode::Preview,
            settle_exact: false,
            scope_target: None,
            surface_target: None,
        }
    }

    pub fn filter(mut self, expr: impl IntoExpr) -> Self {
        self.filters
            .push(expr_node(expr.into_expr(), "event binding filter"));
        self
    }

    pub fn within_subplot(mut self, id: impl Into<String>) -> Self {
        self.scope_target = Some(ChartEventScopeTarget::within_subplot(id));
        self
    }

    pub fn within_subplots<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scope_target = Some(ChartEventScopeTarget::within_subplots(ids));
        self
    }

    /// Allow this binding to receive events from any chart interaction surface.
    pub fn all_surfaces(mut self) -> Self {
        self.surface_target = Some(ChartEventSurfaceTarget::All);
        self
    }

    #[doc(hidden)]
    pub fn with_plot_surface_target(mut self) -> Self {
        if !matches!(self.surface_target, Some(ChartEventSurfaceTarget::All)) {
            self.surface_target = Some(ChartEventSurfaceTarget::PlotSurface);
        }
        self
    }

    #[doc(hidden)]
    pub fn with_legend_surface_target(
        mut self,
        surface_keys: Vec<String>,
        kinds: Vec<LegendSurfaceKind>,
    ) -> Self {
        self.surface_target = Some(ChartEventSurfaceTarget::LegendSurface {
            surface_keys,
            kinds,
        });
        self
    }

    #[doc(hidden)]
    pub fn with_resolved_coord_node_path_target(
        mut self,
        coord_node_path_prefix: Vec<usize>,
    ) -> Self {
        self.scope_target = (!coord_node_path_prefix.is_empty()).then_some(
            ChartEventScopeTarget::with_resolved_coord_node_path_prefix(coord_node_path_prefix),
        );
        self
    }

    pub fn between(mut self, start: ChartEventStream, end: ChartEventStream) -> Self {
        self.between = Some(ChartEventBetween {
            start,
            end,
            emit_end_event: false,
        });
        self
    }

    pub fn event_path_min_distance_px(mut self, distance: f32) -> Self {
        self.event_path_min_distance_px = Some(distance);
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
            scope: ChartEventAssignmentScope::Current,
            replace_scoped_values: false,
        });
        self
    }

    pub fn set_param_replacing_scopes(
        mut self,
        param: impl IntoParamName,
        expr: impl IntoExpr,
    ) -> Self {
        self.assignments.push(ChartEventParamAssignment {
            param_name: param.into_param_name(),
            expr: expr_node(expr.into_expr(), "event param assignment"),
            scope: ChartEventAssignmentScope::Current,
            replace_scoped_values: true,
        });
        self
    }

    pub fn set_param_at_start_scope(
        mut self,
        param: impl IntoParamName,
        expr: impl IntoExpr,
    ) -> Self {
        self.assignments.push(ChartEventParamAssignment {
            param_name: param.into_param_name(),
            expr: expr_node(expr.into_expr(), "event param assignment"),
            scope: ChartEventAssignmentScope::Start,
            replace_scoped_values: false,
        });
        self
    }

    pub fn set_param_at_start_scope_replacing_scopes(
        mut self,
        param: impl IntoParamName,
        expr: impl IntoExpr,
    ) -> Self {
        self.assignments.push(ChartEventParamAssignment {
            param_name: param.into_param_name(),
            expr: expr_node(expr.into_expr(), "event param assignment"),
            scope: ChartEventAssignmentScope::Start,
            replace_scoped_values: true,
        });
        self
    }

    pub fn set_store(mut self, store: impl Into<String>, update: StoreUpdate) -> Self {
        self.store_assignments.push(ChartEventStoreAssignment {
            store_name: store.into(),
            update,
            scope: ChartEventAssignmentScope::Current,
            replace_scoped_values: false,
        });
        self
    }

    pub fn set_store_replacing_scopes(
        mut self,
        store: impl Into<String>,
        update: StoreUpdate,
    ) -> Self {
        self.store_assignments.push(ChartEventStoreAssignment {
            store_name: store.into(),
            update,
            scope: ChartEventAssignmentScope::Current,
            replace_scoped_values: true,
        });
        self
    }

    pub fn set_store_at_start_scope(
        mut self,
        store: impl Into<String>,
        update: StoreUpdate,
    ) -> Self {
        self.store_assignments.push(ChartEventStoreAssignment {
            store_name: store.into(),
            update,
            scope: ChartEventAssignmentScope::Start,
            replace_scoped_values: false,
        });
        self
    }

    pub fn set_store_at_start_scope_replacing_scopes(
        mut self,
        store: impl Into<String>,
        update: StoreUpdate,
    ) -> Self {
        self.store_assignments.push(ChartEventStoreAssignment {
            store_name: store.into(),
            update,
            scope: ChartEventAssignmentScope::Start,
            replace_scoped_values: true,
        });
        self
    }

    pub fn set_selection(
        mut self,
        selection: impl Into<String>,
        update: impl Into<SelectionUpdate>,
    ) -> Self {
        self.selection_assignments
            .push(ChartEventSelectionAssignment {
                selection_id: selection.into(),
                update: update.into(),
                scope: ChartEventAssignmentScope::Current,
            });
        self
    }

    pub fn set_selection_at_start_scope(
        mut self,
        selection: impl Into<String>,
        update: impl Into<SelectionUpdate>,
    ) -> Self {
        self.selection_assignments
            .push(ChartEventSelectionAssignment {
                selection_id: selection.into(),
                update: update.into(),
                scope: ChartEventAssignmentScope::Start,
            });
        self
    }

    pub fn clear_selection(self, selection: impl Into<String>) -> Self {
        self.set_selection(selection, SelectionUpdate::clear())
    }

    pub fn clear_selection_at_start_scope(self, selection: impl Into<String>) -> Self {
        self.set_selection_at_start_scope(selection, SelectionUpdate::clear())
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
        if let Some(distance) = self.event_path_min_distance_px
            && (!distance.is_finite() || distance < 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(
                "Chart event binding event path minimum distance must be finite and non-negative"
                    .to_string(),
            ));
        }
        if let Some(target) = &self.scope_target {
            target.validate()?;
        }
        if let Some(target) = &self.surface_target {
            target.validate()?;
        }
        if let Some(between) = &self.between {
            between.start.validate()?;
            between.end.validate()?;
        }
        let mut targets = std::collections::HashSet::new();
        for assignment in &self.assignments {
            if !targets.insert(assignment.param_name.as_str()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Chart event binding assigns param '{}' more than once",
                    assignment.param_name
                )));
            }
        }
        let mut selection_targets = std::collections::HashSet::new();
        for assignment in &self.selection_assignments {
            if !selection_targets.insert(assignment.selection_id.as_str()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Chart event binding assigns selection '{}' more than once",
                    assignment.selection_id
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

pub fn cursor(style: CursorStyle) -> Expr {
    lit(style.as_str().to_string())
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

pub fn start_event_id() -> Expr {
    col(START_EVENT_ID_FIELD)
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

/// Current event point clamped to the frozen start scope and inverted through it.
pub fn event_at_start_clipped_coord(channel: &str) -> Expr {
    col(event_at_start_clipped_coord_column_name(channel))
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

/// Datum value for `field` from the rendered mark instance hit by the current event.
///
/// The value is drawn from the evaluated logical mark row before visual channels
/// are scaled to pixels. Events that do not hit a mark instance, or hit a mark
/// without the requested datum field, produce null.
pub fn datum(field: &str) -> Expr {
    col(event_datum_column_name(field))
}

/// Domain value associated with a clicked/hovered discrete legend item.
pub fn legend_value() -> Expr {
    datum(LEGEND_VALUE_FIELD)
}

/// Display label associated with a clicked/hovered discrete legend item.
pub fn legend_label() -> Expr {
    datum(LEGEND_LABEL_FIELD)
}

/// Stable item name associated with a clicked/hovered discrete legend item.
pub fn legend_name() -> Expr {
    datum(LEGEND_NAME_FIELD)
}

/// Visual channel associated with a clicked/hovered discrete legend item.
pub fn legend_channel() -> Expr {
    datum(LEGEND_CHANNEL_FIELD)
}

/// Zero-based item index associated with a clicked/hovered discrete legend item.
pub fn legend_index() -> Expr {
    datum(LEGEND_INDEX_FIELD)
}

/// Public legend id associated with a clicked/hovered discrete legend item.
pub fn legend_id() -> Expr {
    datum(LEGEND_ID_FIELD)
}

#[doc(hidden)]
pub fn legend_surface_key() -> Expr {
    datum(LEGEND_SURFACE_KEY_FIELD)
}

#[doc(hidden)]
pub fn legend_surface_kind() -> Expr {
    datum(LEGEND_SURFACE_KIND_FIELD)
}

/// True for events whose hit mark is a discrete legend item hit rectangle.
pub fn is_legend_item() -> Expr {
    legend_surface_kind().eq(lit(LEGEND_SURFACE_KIND_DISCRETE_ITEM))
}

/// Current routed plot-area width in scene pixels.
pub fn event_plot_width() -> Expr {
    col(EVENT_PLOT_WIDTH_FIELD)
}

/// Current routed plot-area height in scene pixels.
pub fn event_plot_height() -> Expr {
    col(EVENT_PLOT_HEIGHT_FIELD)
}

/// Frozen start routed plot-area width in scene pixels.
pub fn start_plot_width() -> Expr {
    col(START_PLOT_WIDTH_FIELD)
}

/// Frozen start routed plot-area height in scene pixels.
pub fn start_plot_height() -> Expr {
    col(START_PLOT_HEIGHT_FIELD)
}

/// Stable string id for the current routed interaction scope.
pub fn event_scope_id() -> Expr {
    col(EVENT_SCOPE_ID_FIELD)
}

/// Stable string id for the frozen start interaction scope.
pub fn start_scope_id() -> Expr {
    col(START_SCOPE_ID_FIELD)
}

/// Logical facet value at `index` for the current routed interaction scope.
///
/// Values are represented as UTF-8 strings so event bindings can capture them
/// without needing to know the concrete partition value type.
pub fn event_facet_value(index: usize) -> Expr {
    col(event_facet_value_column_name(index))
}

/// Logical facet value at `index` for the frozen start interaction scope.
///
/// Values are represented as UTF-8 strings so event bindings can capture them
/// without needing to know the concrete partition value type.
pub fn start_facet_value(index: usize) -> Expr {
    col(start_facet_value_column_name(index))
}

/// Scene-space path accumulated during a `between(...)` gesture.
///
/// The path is encoded as a flat `List(Float64)`: `[x0, y0, x1, y1, ...]`.
pub fn event_path() -> Expr {
    col(EVENT_PATH_FIELD)
}

/// SVG path string derived from the current `between(...)` gesture path.
///
/// The path is relative to the first sampled scene-space point, so it can be
/// drawn by a `PathMark` anchored at the gesture-start data coordinate.
pub fn event_path_svg() -> Expr {
    col(EVENT_PATH_SVG_FIELD)
}

/// Build a two-element interval list `[min, max]` from scalar expressions.
///
/// This is a plain array constructor; pair it with [`interval_start`] and
/// [`interval_end`] to decompose a two-element list such as a scale domain.
pub fn interval(min: impl IntoExpr, max: impl IntoExpr) -> Expr {
    make_array(vec![min.into_expr(), max.into_expr()])
}

/// Build a two-element interval list from unordered scalar endpoints.
pub fn interval_ordered(a: impl IntoExpr, b: impl IntoExpr) -> Expr {
    let a = a.into_expr();
    let b = b.into_expr();
    let is_ordered = a.clone().lt_eq(b.clone());
    let min = when(is_ordered.clone(), a.clone())
        .otherwise(b.clone())
        .expect("valid ordered interval minimum expression");
    let max = when(is_ordered, b)
        .otherwise(a)
        .expect("valid ordered interval maximum expression");
    interval(min, max)
}

/// Extract the first element from a two-element interval list.
pub fn interval_start(interval: impl IntoExpr) -> Expr {
    array_element(interval.into_expr(), lit(1_i64))
}

/// Extract the second element from a two-element interval list.
pub fn interval_end(interval: impl IntoExpr) -> Expr {
    array_element(interval.into_expr(), lit(2_i64))
}

pub fn rewrite_legend_event_binding_local_datums(
    mut binding: ChartEventBinding,
    ctx: &SessionContext,
) -> Result<ChartEventBinding, AvengerChartError> {
    for filter in &mut binding.filters {
        rewrite_expr_node_legend_datums(filter, ctx)?;
    }
    for assignment in &mut binding.assignments {
        rewrite_expr_node_legend_datums(&mut assignment.expr, ctx)?;
    }
    for assignment in &mut binding.store_assignments {
        rewrite_store_update_legend_datums(&mut assignment.update, ctx)?;
    }
    for assignment in &mut binding.selection_assignments {
        rewrite_selection_update_legend_datums(&mut assignment.update, ctx)?;
    }
    if let Some(between) = &mut binding.between {
        rewrite_event_stream_legend_datums(&mut between.start, ctx)?;
        rewrite_event_stream_legend_datums(&mut between.end, ctx)?;
    }
    Ok(binding)
}

pub fn is_legend_item_only_datum_field(field: &str) -> bool {
    LEGEND_ITEM_ONLY_DATUM_FIELDS.contains(&field)
}

fn rewrite_event_stream_legend_datums(
    stream: &mut ChartEventStream,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    for filter in &mut stream.filters {
        rewrite_expr_node_legend_datums(filter, ctx)?;
    }
    Ok(())
}

fn rewrite_store_update_legend_datums(
    update: &mut StoreUpdate,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    match update {
        StoreUpdate::Clear => {}
        StoreUpdate::ReplaceRows { rows }
        | StoreUpdate::InsertRows { rows }
        | StoreUpdate::UpsertRows { rows }
        | StoreUpdate::ToggleRows { rows } => {
            for row in rows {
                for value in row.fields.values_mut() {
                    rewrite_expr_node_legend_datums(&mut value.expr, ctx)?;
                }
            }
        }
        StoreUpdate::UpdateByKey { key, fields } => {
            for value in key.fields.values_mut() {
                rewrite_expr_node_legend_datums(&mut value.expr, ctx)?;
            }
            for value in fields.fields.values_mut() {
                rewrite_expr_node_legend_datums(&mut value.expr, ctx)?;
            }
        }
        StoreUpdate::DeleteByKey { key } => {
            for value in key.fields.values_mut() {
                rewrite_expr_node_legend_datums(&mut value.expr, ctx)?;
            }
        }
    }
    Ok(())
}

fn rewrite_selection_update_legend_datums(
    update: &mut SelectionUpdate,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    match update {
        SelectionUpdate::Clear | SelectionUpdate::ClearInScope { .. } => {}
        SelectionUpdate::ReplaceAllClauses { clauses }
        | SelectionUpdate::ReplaceClausesInScope { clauses, .. }
        | SelectionUpdate::UpsertClauses { clauses }
        | SelectionUpdate::ToggleClauses { clauses } => {
            for clause in clauses {
                rewrite_expr_node_legend_datums(&mut clause.id.expr, ctx)?;
                match &mut clause.predicate {
                    crate::SelectionPredicateUpdate::Interval { dimensions } => {
                        for dimension in dimensions {
                            rewrite_expr_node_legend_datums(&mut dimension.min.expr, ctx)?;
                            rewrite_expr_node_legend_datums(&mut dimension.max.expr, ctx)?;
                        }
                    }
                    crate::SelectionPredicateUpdate::Equality { dimensions } => {
                        for dimension in dimensions {
                            rewrite_expr_node_legend_datums(&mut dimension.value.expr, ctx)?;
                        }
                    }
                    crate::SelectionPredicateUpdate::Predicate { values, .. } => {
                        for value in values {
                            rewrite_expr_node_legend_datums(&mut value.value.expr, ctx)?;
                        }
                    }
                }
            }
        }
        SelectionUpdate::DeleteClauses { ids }
        | SelectionUpdate::DeleteClausesInScope { ids, .. } => {
            for id in ids {
                rewrite_expr_node_legend_datums(&mut id.expr, ctx)?;
            }
        }
        SelectionUpdate::ReplaceAllFromSceneQuery { query }
        | SelectionUpdate::ReplaceFromSceneQueryInScope { query }
        | SelectionUpdate::UpsertFromSceneQuery { query }
        | SelectionUpdate::ToggleFromSceneQuery { query } => {
            rewrite_scene_query_legend_datums(query, ctx)?;
        }
    }
    Ok(())
}

fn rewrite_scene_query_legend_datums(
    query: &mut SelectionSceneQuery,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    match &mut query.query.geometry {
        SceneGeometryQueryGeometry::Rect { x0, y0, x1, y1 } => {
            rewrite_expr_node_legend_datums(x0, ctx)?;
            rewrite_expr_node_legend_datums(y0, ctx)?;
            rewrite_expr_node_legend_datums(x1, ctx)?;
            rewrite_expr_node_legend_datums(y1, ctx)?;
        }
        SceneGeometryQueryGeometry::Circle { cx, cy, radius } => {
            rewrite_expr_node_legend_datums(cx, ctx)?;
            rewrite_expr_node_legend_datums(cy, ctx)?;
            rewrite_expr_node_legend_datums(radius, ctx)?;
        }
        SceneGeometryQueryGeometry::Polygon { points } => {
            rewrite_expr_node_legend_datums(points, ctx)?;
        }
    }
    if let SceneQueryClauseId::Expr(expr) = &mut query.clause_id {
        rewrite_expr_node_legend_datums(expr, ctx)?;
    }
    Ok(())
}

fn rewrite_expr_node_legend_datums(
    node: &mut LogicalExprNode,
    ctx: &SessionContext,
) -> Result<(), AvengerChartError> {
    let expr = node.to_expr(ctx)?;
    let rewritten = rewrite_expr_legend_datums(expr)?;
    *node = expr_node(rewritten, "legend event local datum expression");
    Ok(())
}

fn rewrite_expr_legend_datums(expr: Expr) -> Result<Expr, AvengerChartError> {
    expr.transform(|candidate| {
        if let Expr::Column(column) = &candidate
            && let Some(field) = legend_local_datum_reserved_field(&column.name)
        {
            return Ok(Transformed::yes(col(event_datum_column_name(field))));
        }
        Ok(Transformed::no(candidate))
    })
    .map(|transformed| transformed.data)
    .map_err(|err| AvengerChartError::InvalidArgument(err.to_string()))
}

fn legend_local_datum_reserved_field(column_name: &str) -> Option<&'static str> {
    let local = column_name.strip_prefix(EVENT_DATUM_PREFIX)?;
    match local {
        "value" => Some(LEGEND_VALUE_FIELD),
        "label" => Some(LEGEND_LABEL_FIELD),
        "name" => Some(LEGEND_NAME_FIELD),
        "channel" => Some(LEGEND_CHANNEL_FIELD),
        "index" => Some(LEGEND_INDEX_FIELD),
        "legend_id" => Some(LEGEND_ID_FIELD),
        "surface_key" => Some(LEGEND_SURFACE_KEY_FIELD),
        "surface_kind" => Some(LEGEND_SURFACE_KIND_FIELD),
        "orientation" => Some(LEGEND_ORIENTATION_FIELD),
        "value_channel" => Some(LEGEND_VALUE_CHANNEL_FIELD),
        "band_channel" => Some(LEGEND_BAND_CHANNEL_FIELD),
        _ => None,
    }
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

pub fn event_at_start_clipped_coord_column_name(channel: &str) -> String {
    format!("{EVENT_AT_START_CLIPPED_COORD_PREFIX}{channel}")
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

pub fn event_datum_column_name(field: &str) -> String {
    format!("{EVENT_DATUM_PREFIX}{field}")
}

pub fn event_facet_value_column_name(index: usize) -> String {
    format!("{EVENT_FACET_VALUE_PREFIX}{index}")
}

pub fn start_facet_value_column_name(index: usize) -> String {
    format!("{START_FACET_VALUE_PREFIX}{index}")
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
    pub event_at_start_clipped_coord: BTreeSet<String>,
    pub previous_coord: BTreeSet<String>,
    pub current_domain: BTreeSet<String>,
    pub start_domain: BTreeSet<String>,
    pub current_datum: BTreeSet<String>,
    pub current_plot_size: bool,
    pub start_plot_size: bool,
    pub current_scope_id: bool,
    pub start_scope_id: bool,
    pub start_event_id: bool,
    pub current_facet_values: BTreeSet<usize>,
    pub start_facet_values: BTreeSet<usize>,
    pub event_path: bool,
    pub event_path_svg: bool,
}

impl InteractionColumnRequests {
    pub fn is_empty(&self) -> bool {
        self.current_coord.is_empty()
            && self.start_coord.is_empty()
            && self.event_at_start_coord.is_empty()
            && self.event_at_start_clipped_coord.is_empty()
            && self.previous_coord.is_empty()
            && self.current_domain.is_empty()
            && self.start_domain.is_empty()
            && self.current_datum.is_empty()
            && !self.current_plot_size
            && !self.start_plot_size
            && !self.current_scope_id
            && !self.start_scope_id
            && !self.start_event_id
            && self.current_facet_values.is_empty()
            && self.start_facet_values.is_empty()
            && !self.event_path
            && !self.event_path_svg
    }

    /// Union of every coordinate channel referenced by any request kind.
    pub fn all_channels(&self) -> BTreeSet<String> {
        let mut channels = BTreeSet::new();
        channels.extend(self.current_coord.iter().cloned());
        channels.extend(self.start_coord.iter().cloned());
        channels.extend(self.event_at_start_coord.iter().cloned());
        channels.extend(self.event_at_start_clipped_coord.iter().cloned());
        channels.extend(self.previous_coord.iter().cloned());
        channels.extend(self.current_domain.iter().cloned());
        channels.extend(self.start_domain.iter().cloned());
        channels
    }

    fn record_column(&mut self, name: &str) {
        // Most-specific prefixes first; the prefixes are mutually exclusive but
        // ordering keeps the intent explicit.
        if let Some(channel) = name.strip_prefix(EVENT_AT_START_CLIPPED_COORD_PREFIX) {
            self.event_at_start_clipped_coord
                .insert(channel.to_string());
        } else if let Some(channel) = name.strip_prefix(EVENT_AT_START_COORD_PREFIX) {
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
        } else if let Some(field) = name.strip_prefix(EVENT_DATUM_PREFIX) {
            self.current_datum.insert(field.to_string());
        } else if name == EVENT_PLOT_WIDTH_FIELD || name == EVENT_PLOT_HEIGHT_FIELD {
            self.current_plot_size = true;
        } else if name == START_PLOT_WIDTH_FIELD || name == START_PLOT_HEIGHT_FIELD {
            self.start_plot_size = true;
        } else if name == EVENT_SCOPE_ID_FIELD {
            self.current_scope_id = true;
        } else if name == START_SCOPE_ID_FIELD {
            self.start_scope_id = true;
        } else if name == START_EVENT_ID_FIELD {
            self.start_event_id = true;
        } else if name == EVENT_PATH_FIELD {
            self.event_path = true;
        } else if name == EVENT_PATH_SVG_FIELD {
            self.event_path_svg = true;
        } else if let Some(index) = name
            .strip_prefix(EVENT_FACET_VALUE_PREFIX)
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.current_facet_values.insert(index);
        } else if let Some(index) = name
            .strip_prefix(START_FACET_VALUE_PREFIX)
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.start_facet_values.insert(index);
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

pub fn scan_chart_event_binding_interaction_columns(
    binding: &ChartEventBinding,
    ctx: &SessionContext,
) -> Result<InteractionColumnRequests, AvengerChartError> {
    let mut exprs = Vec::new();
    for filter in &binding.filters {
        exprs.push(filter.to_expr(ctx)?);
    }
    for assignment in &binding.assignments {
        exprs.push(assignment.expr.to_expr(ctx)?);
    }
    for assignment in &binding.store_assignments {
        collect_store_update_exprs(&assignment.update, ctx, &mut exprs)?;
    }
    for assignment in &binding.selection_assignments {
        collect_selection_update_exprs(&assignment.update, ctx, &mut exprs)?;
    }
    let mut requests = scan_interaction_columns(&exprs);
    for assignment in &binding.selection_assignments {
        collect_selection_update_datum_requests(&assignment.update, &mut requests);
    }
    Ok(requests)
}

fn collect_store_update_exprs(
    update: &StoreUpdate,
    ctx: &SessionContext,
    exprs: &mut Vec<Expr>,
) -> Result<(), AvengerChartError> {
    match update {
        StoreUpdate::Clear => {}
        StoreUpdate::ReplaceRows { rows }
        | StoreUpdate::InsertRows { rows }
        | StoreUpdate::UpsertRows { rows }
        | StoreUpdate::ToggleRows { rows } => {
            for row in rows {
                for value in row.fields.values() {
                    exprs.push(value.expr.to_expr(ctx)?);
                }
            }
        }
        StoreUpdate::UpdateByKey { key, fields } => {
            for value in key.fields.values() {
                exprs.push(value.expr.to_expr(ctx)?);
            }
            for value in fields.fields.values() {
                exprs.push(value.expr.to_expr(ctx)?);
            }
        }
        StoreUpdate::DeleteByKey { key } => {
            for value in key.fields.values() {
                exprs.push(value.expr.to_expr(ctx)?);
            }
        }
    }
    Ok(())
}

fn collect_selection_update_exprs(
    update: &SelectionUpdate,
    ctx: &SessionContext,
    exprs: &mut Vec<Expr>,
) -> Result<(), AvengerChartError> {
    match update {
        SelectionUpdate::Clear | SelectionUpdate::ClearInScope { .. } => {}
        SelectionUpdate::ReplaceAllClauses { clauses }
        | SelectionUpdate::ReplaceClausesInScope { clauses, .. }
        | SelectionUpdate::UpsertClauses { clauses }
        | SelectionUpdate::ToggleClauses { clauses } => {
            for clause in clauses {
                exprs.push(clause.id.expr.to_expr(ctx)?);
                match &clause.predicate {
                    crate::SelectionPredicateUpdate::Interval { dimensions } => {
                        for dimension in dimensions {
                            exprs.push(dimension.min.expr.to_expr(ctx)?);
                            exprs.push(dimension.max.expr.to_expr(ctx)?);
                        }
                    }
                    crate::SelectionPredicateUpdate::Equality { dimensions } => {
                        for dimension in dimensions {
                            exprs.push(dimension.value.expr.to_expr(ctx)?);
                        }
                    }
                    crate::SelectionPredicateUpdate::Predicate { values, .. } => {
                        for value in values {
                            exprs.push(value.value.expr.to_expr(ctx)?);
                        }
                    }
                }
            }
        }
        SelectionUpdate::DeleteClauses { ids } => {
            for id in ids {
                exprs.push(id.expr.to_expr(ctx)?);
            }
        }
        SelectionUpdate::DeleteClausesInScope { ids, .. } => {
            for id in ids {
                exprs.push(id.expr.to_expr(ctx)?);
            }
        }
        SelectionUpdate::ReplaceAllFromSceneQuery { query }
        | SelectionUpdate::ReplaceFromSceneQueryInScope { query }
        | SelectionUpdate::UpsertFromSceneQuery { query }
        | SelectionUpdate::ToggleFromSceneQuery { query } => {
            collect_scene_query_update_exprs(query, ctx, exprs)?;
        }
    }
    Ok(())
}

fn collect_scene_query_update_exprs(
    update: &SelectionSceneQuery,
    ctx: &SessionContext,
    exprs: &mut Vec<Expr>,
) -> Result<(), AvengerChartError> {
    update.query.target.validate()?;
    match &update.query.geometry {
        SceneGeometryQueryGeometry::Rect { x0, y0, x1, y1 } => {
            exprs.push(x0.to_expr(ctx)?);
            exprs.push(y0.to_expr(ctx)?);
            exprs.push(x1.to_expr(ctx)?);
            exprs.push(y1.to_expr(ctx)?);
        }
        SceneGeometryQueryGeometry::Circle { cx, cy, radius } => {
            exprs.push(cx.to_expr(ctx)?);
            exprs.push(cy.to_expr(ctx)?);
            exprs.push(radius.to_expr(ctx)?);
        }
        SceneGeometryQueryGeometry::Polygon { points } => {
            exprs.push(points.to_expr(ctx)?);
        }
    }
    if let SceneQueryClauseId::Expr(expr) = &update.clause_id {
        exprs.push(expr.to_expr(ctx)?);
    }
    Ok(())
}

fn collect_selection_update_datum_requests(
    update: &SelectionUpdate,
    requests: &mut InteractionColumnRequests,
) {
    let query = match update {
        SelectionUpdate::ReplaceAllFromSceneQuery { query }
        | SelectionUpdate::ReplaceFromSceneQueryInScope { query }
        | SelectionUpdate::UpsertFromSceneQuery { query }
        | SelectionUpdate::ToggleFromSceneQuery { query } => query,
        _ => return,
    };
    for field in &query.query.datum_fields {
        requests.current_datum.insert(field.datum_field.clone());
    }
}

fn expr_node(expr: Expr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_expr(expr)
        .unwrap_or_else(|err| panic!("Failed to serialize {label}: {err}"))
}

#[cfg(test)]
mod tests {
    use crate::{CoordinationScope, SceneGeometryQuery, SceneQueryDatumField};
    use datafusion::prelude::{SessionContext, lit};

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
            event_at_start_clipped_coord_column_name("x"),
            "__event_at_start_clipped_coord_x".to_string()
        );
        assert_eq!(
            start_domain_column_name("x"),
            "__start_domain_x".to_string()
        );
        assert_eq!(event_plot_width(), col("__event_plot_width"));
        assert_eq!(start_scope_id(), col("__start_scope_id"));
        assert_eq!(start_event_id(), col("__start_event_id"));
        assert_eq!(event_facet_value(2), col("__event_facet_value_2"));
        assert_eq!(datum("category"), col("__event_datum_category"));
        assert_eq!(legend_value(), col("__event_datum___legend_value"));
        assert_eq!(legend_label(), col("__event_datum___legend_label"));
        assert_eq!(legend_name(), col("__event_datum___legend_name"));
        assert_eq!(legend_channel(), col("__event_datum___legend_channel"));
        assert_eq!(legend_index(), col("__event_datum___legend_index"));
        assert_eq!(legend_id(), col("__event_datum___legend_id"));
        // The public helper expressions reference those reserved columns.
        assert_eq!(event_coord("x"), col("__event_coord_x"));
        assert_eq!(start_domain("y"), col("__start_domain_y"));
    }

    #[test]
    fn legend_event_binding_rewrites_local_datum_fields() {
        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::Click)
            .filter(datum("value").eq(lit("A")))
            .set_selection(
                "picked",
                SelectionUpdate::toggle_clause(crate::SelectionClauseUpdate::equality_value(
                    col("category"),
                    datum("value"),
                )),
            );

        let rewritten = rewrite_legend_event_binding_local_datums(binding, &ctx)
            .expect("legend local datum rewrite succeeds");
        let requests = scan_chart_event_binding_interaction_columns(&rewritten, &ctx)
            .expect("scan rewritten binding");
        assert!(requests.current_datum.contains(LEGEND_VALUE_FIELD));
        assert!(!requests.current_datum.contains("value"));
    }

    #[test]
    fn legend_event_binding_scan_detects_colorbar_coordinate_requests() {
        let ctx = SessionContext::new();
        let binding = ChartEventBinding::on(ChartEventType::CursorMoved)
            .filter(datum("surface_kind").eq(lit(LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR)))
            .filter(event_coord("y").is_not_null())
            .between(
                ChartEventStream::on(ChartEventType::MouseDown)
                    .filter(start_coord("y").is_not_null()),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .set_param("domain", interval(event_coord("y"), start_coord("y")))
            .set_param("clipped", event_at_start_clipped_coord("y"))
            .set_param("start_domain", start_domain("y"))
            .set_param("event_domain", event_domain("y"));

        let rewritten = rewrite_legend_event_binding_local_datums(binding, &ctx)
            .expect("legend local datum rewrite succeeds");
        let requests = scan_chart_event_binding_interaction_columns(&rewritten, &ctx)
            .expect("scan rewritten binding");
        assert!(requests.current_datum.contains(LEGEND_SURFACE_KIND_FIELD));
        assert!(requests.current_coord.contains("y"));
        assert!(requests.start_coord.contains("y"));
        assert!(requests.event_at_start_clipped_coord.contains("y"));
        assert!(requests.start_domain.contains("y"));
        assert!(requests.current_domain.contains("y"));
    }

    #[test]
    fn interval_helpers_build_valid_expressions() {
        // interval builds a two-element list; interval_start/end index it.
        let interval_expr = interval(lit(2.0), lit(8.0));
        let ordered_interval_expr = interval_ordered(lit(8.0), lit(2.0));
        let start = interval_start(start_domain("x"));
        let end = interval_end(start_domain("x"));
        // These must serialize as ordinary DataFusion expressions.
        for expr in [interval_expr, ordered_interval_expr, start, end] {
            LogicalExprNode::from_expr(expr).expect("interaction interval expr serializes");
        }
    }

    #[test]
    fn on_between_end_builds_end_event_binding() {
        let binding = ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown),
            ChartEventStream::on(ChartEventType::MouseUp),
        );

        assert_eq!(binding.event_type, ChartEventType::MouseUp);
        assert!(
            binding
                .between
                .as_ref()
                .expect("between config")
                .emit_end_event
        );
    }

    #[test]
    fn scan_detects_requested_interaction_columns() {
        let requests = scan_interaction_columns(&[
            event_at_start_coord("x") - start_coord("x"),
            event_at_start_clipped_coord("y"),
            interval_start(start_domain("x")),
            event_plot_width(),
            start_facet_value(0),
            datum("category"),
            event_path(),
            event_path_svg(),
        ]);
        assert!(requests.event_at_start_coord.contains("x"));
        assert!(requests.event_at_start_clipped_coord.contains("y"));
        assert!(requests.start_coord.contains("x"));
        assert!(requests.start_domain.contains("x"));
        assert!(requests.current_datum.contains("category"));
        assert!(requests.current_plot_size);
        assert!(requests.start_facet_values.contains(&0));
        assert!(requests.event_path);
        assert!(requests.event_path_svg);
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
            .event_path_min_distance_px(6.0)
            .filter(shift().eq(lit(false)))
            .set_param("x0", start_param("x0") + dx())
            .preview()
            .settle_exact();

        let json = serde_json::to_string(&binding).expect("serialize binding");
        let restored: ChartEventBinding = serde_json::from_str(&json).expect("deserialize binding");
        assert_eq!(restored.event_type, ChartEventType::CursorMoved);
        assert_eq!(restored.assignments.len(), 1);
        assert!(restored.settle_exact);
        assert_eq!(restored.event_path_min_distance_px, Some(6.0));
    }

    #[test]
    fn chart_event_binding_rejects_invalid_event_path_distance() {
        let binding =
            ChartEventBinding::on(ChartEventType::CursorMoved).event_path_min_distance_px(f32::NAN);
        assert!(binding.validate().is_err());

        let binding =
            ChartEventBinding::on(ChartEventType::CursorMoved).event_path_min_distance_px(-1.0);
        assert!(binding.validate().is_err());
    }

    #[test]
    fn scene_query_selection_binding_serializes_and_requests_datums() {
        let binding = ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .set_selection_at_start_scope(
            "picked",
            SelectionUpdate::replace_all_from_scene_query(
                SelectionSceneQuery::new(
                    SceneGeometryQuery::polygon(event_path())
                        .datum_field(SceneQueryDatumField::new("point_id"))
                        .unique_by(["point_id"]),
                )
                .sharing(CoordinationScope::Shared),
            ),
        )
        .exact();

        let json = serde_json::to_string(&binding).expect("serialize scene query binding");
        let restored: ChartEventBinding =
            serde_json::from_str(&json).expect("deserialize scene query binding");
        assert_eq!(restored.selection_assignments.len(), 1);
        assert!(matches!(
            restored.selection_assignments[0].update,
            SelectionUpdate::ReplaceAllFromSceneQuery { .. }
        ));

        let ctx = SessionContext::new();
        let requests = scan_chart_event_binding_interaction_columns(&restored, &ctx)
            .expect("scan scene query binding");
        assert!(requests.event_path);
        assert!(requests.current_datum.contains("point_id"));
    }
}
