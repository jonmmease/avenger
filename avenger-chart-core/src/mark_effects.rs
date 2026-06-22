use std::{marker::PhantomData, sync::Arc};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanArray, Float32Array, Float64Array, StringArray},
        datatypes::{Field, Schema},
        record_batch::RecordBatch,
    },
    common::{Column, ScalarValue},
    logical_expr::Expr,
    prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, ChannelValue, CompiledScalarExpressionProgram, DefaultLogicalExprNodeExt,
    IntoExpr, PhysicalScalarExpressionSpec, PhysicalScalarProgramOptions, SerializableExpr,
};
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use avenger_text::measurement::{TextBounds, TextMeasurementConfig};

pub const ITEM_CHANNEL_COLUMN_PREFIX: &str = "__avenger_item_channel_";
pub const ITEM_DATA_COLUMN_PREFIX: &str = "__avenger_item_data_";
pub const ITEM_BBOX_COLUMN_PREFIX: &str = "__avenger_item_bbox_";
pub const ITEM_ADJUSTMENT_COLUMN_PREFIX: &str = "__avenger_item_adjustment_";

pub fn item_channel_column_name(channel: &str) -> String {
    format!("{ITEM_CHANNEL_COLUMN_PREFIX}{channel}")
}

pub fn item_channel_name_from_column(column: &str) -> Option<&str> {
    column
        .strip_prefix(ITEM_CHANNEL_COLUMN_PREFIX)
        .filter(|channel| !channel.is_empty())
}

pub fn item_data_column_name(field: &str) -> String {
    format!("{ITEM_DATA_COLUMN_PREFIX}{field}")
}

pub fn item_data_name_from_column(column: &str) -> Option<&str> {
    column
        .strip_prefix(ITEM_DATA_COLUMN_PREFIX)
        .filter(|field| !field.is_empty())
}

pub fn item_bbox_column_name(field: &str) -> String {
    format!("{ITEM_BBOX_COLUMN_PREFIX}{field}")
}

pub fn item_adjustment_column_name(stage: usize, field: &str) -> String {
    format!("{ITEM_ADJUSTMENT_COLUMN_PREFIX}{stage}_{field}")
}

pub fn is_item_frame_column_name(name: &str) -> bool {
    name.starts_with(ITEM_CHANNEL_COLUMN_PREFIX)
        || name.starts_with(ITEM_DATA_COLUMN_PREFIX)
        || name.starts_with(ITEM_BBOX_COLUMN_PREFIX)
        || name.starts_with(ITEM_ADJUSTMENT_COLUMN_PREFIX)
}

pub fn item_frame_column_refs(expr: &Expr) -> Vec<String> {
    let mut refs = expr
        .column_refs()
        .into_iter()
        .filter_map(|column| is_item_frame_column_name(&column.name).then(|| column.name.clone()))
        .collect::<Vec<_>>();
    refs.sort();
    refs.dedup();
    refs
}

fn item_column_expr(name: String) -> Expr {
    Expr::Column(Column {
        relation: None,
        name,
        spans: Default::default(),
    })
}

#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PrimitiveMarkEffects {
    #[serde(default)]
    pub adjustments: Vec<MarkAdjustmentSpec>,
    #[serde(default)]
    pub derived: Vec<DerivedPrimitiveMarkSpec>,
}

impl PrimitiveMarkEffects {
    pub fn is_empty(&self) -> bool {
        self.adjustments.is_empty() && self.derived.is_empty()
    }

    pub fn push_adjustment(&mut self, adjustment: MarkAdjustmentSpec) {
        self.adjustments.push(adjustment);
    }

    pub fn push_derived(&mut self, derived: DerivedPrimitiveMarkSpec) {
        self.derived.push(derived);
    }

    pub fn has_derived(&self) -> bool {
        !self.derived.is_empty()
    }

    pub fn requires_data_batch(&self) -> bool {
        self.adjustments.iter().any(|adjustment| match adjustment {
            MarkAdjustmentSpec::Expr(spec) => spec.requires_data_batch(),
            MarkAdjustmentSpec::Transform(spec) => spec.requires_data_batch(),
        }) || self
            .derived
            .iter()
            .any(DerivedPrimitiveMarkSpec::requires_data_batch)
    }

    pub fn transform_requirements(&self) -> AdjustmentTransformRequirements {
        let mut requirements = AdjustmentTransformRequirements::default();
        for adjustment in &self.adjustments {
            requirements.merge(adjustment.transform_requirements());
        }
        for derived in &self.derived {
            requirements.merge(derived.transform_requirements());
        }
        requirements
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MarkAdjustmentSpec {
    Expr(ExpressionMarkAdjustmentSpec),
    Transform(TransformMarkAdjustmentSpec),
}

impl MarkAdjustmentSpec {
    pub fn transform_requirements(&self) -> AdjustmentTransformRequirements {
        match self {
            Self::Expr(_) => AdjustmentTransformRequirements::default(),
            Self::Transform(spec) => spec.transform.requirements(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DerivedPrimitiveMarkSpec {
    Symbol(DerivedSymbolMarkSpec),
    Rule(DerivedRuleMarkSpec),
    Rect(DerivedRectMarkSpec),
    Text(DerivedTextMarkSpec),
}

impl DerivedPrimitiveMarkSpec {
    pub fn requires_data_batch(&self) -> bool {
        match self {
            Self::Symbol(spec) => spec.requires_data_batch(),
            Self::Rule(spec) => spec.requires_data_batch(),
            Self::Rect(spec) => spec.requires_data_batch(),
            Self::Text(spec) => spec.requires_data_batch(),
        }
    }

    pub fn transform_requirements(&self) -> AdjustmentTransformRequirements {
        match self {
            Self::Text(spec) => spec.effects.transform_requirements(),
            _ => AdjustmentTransformRequirements::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedSymbolMarkSpec {
    pub assignments: Vec<ItemChannelAssignment>,
    pub zindex: Option<i32>,
}

impl DerivedSymbolMarkSpec {
    pub fn new(assignments: Vec<ItemChannelAssignment>, zindex: Option<i32>) -> Self {
        Self {
            assignments,
            zindex,
        }
    }

    pub fn requires_data_batch(&self) -> bool {
        self.assignments
            .iter()
            .any(|assignment| !assignment.data_fields.is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedRuleMarkSpec {
    pub assignments: Vec<ItemChannelAssignment>,
    pub zindex: Option<i32>,
}

impl DerivedRuleMarkSpec {
    pub fn new(assignments: Vec<ItemChannelAssignment>, zindex: Option<i32>) -> Self {
        Self {
            assignments,
            zindex,
        }
    }

    pub fn requires_data_batch(&self) -> bool {
        self.assignments
            .iter()
            .any(|assignment| !assignment.data_fields.is_empty())
    }
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpressionMarkAdjustmentSpec {
    pub assignments: Vec<ItemChannelAssignment>,
}

#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct TransformMarkAdjustmentSpec {
    pub transform: Box<dyn CompiledMarkAdjustmentTransform>,
    pub assignments: Vec<ItemChannelAssignment>,
}

impl std::fmt::Debug for TransformMarkAdjustmentSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransformMarkAdjustmentSpec")
            .field("assignments", &self.assignments)
            .finish_non_exhaustive()
    }
}

impl TransformMarkAdjustmentSpec {
    pub fn new(
        transform: Box<dyn CompiledMarkAdjustmentTransform>,
        assignments: Vec<ItemChannelAssignment>,
    ) -> Self {
        Self {
            transform,
            assignments,
        }
    }

    pub fn requires_data_batch(&self) -> bool {
        self.transform.requirements().requires_data_batch()
            || self
                .assignments
                .iter()
                .any(|assignment| !assignment.data_fields.is_empty())
    }
}

impl ExpressionMarkAdjustmentSpec {
    pub fn new(assignments: Vec<ItemChannelAssignment>) -> Self {
        Self { assignments }
    }

    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    pub fn requires_data_batch(&self) -> bool {
        self.assignments
            .iter()
            .any(|assignment| !assignment.data_fields.is_empty())
    }
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemChannelAssignment {
    pub channel: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    #[serde(default)]
    pub data_fields: Vec<String>,
}

impl ItemChannelAssignment {
    pub fn new(channel: impl Into<String>, expr: impl IntoExpr) -> Result<Self, AvengerChartError> {
        let expr = expr.into_expr();
        let data_fields = collect_item_data_fields(&expr);
        Ok(Self {
            channel: channel.into(),
            expr: LogicalExprNode::from_default_expr(expr)?,
            data_fields,
        })
    }

    pub fn expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        self.expr.to_default_expr(ctx)
    }
}

fn collect_item_data_fields(expr: &Expr) -> Vec<String> {
    let mut fields = expr
        .column_refs()
        .into_iter()
        .filter_map(|column| {
            column
                .name
                .strip_prefix(ITEM_DATA_COLUMN_PREFIX)
                .filter(|field| !field.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    fields.sort();
    fields.dedup();
    fields
}

pub fn extract_adjustment_assignments(
    mut routed_channels: IndexMap<String, ChannelValue>,
    original_channels: &IndexMap<String, ChannelValue>,
) -> (IndexMap<String, ChannelValue>, Vec<ItemChannelAssignment>) {
    let ctx = SessionContext::new();
    let mut assignments = Vec::new();
    let routed_names = routed_channels.keys().cloned().collect::<Vec<_>>();

    for channel in routed_names {
        let Some(value) = routed_channels.get(&channel) else {
            continue;
        };
        let refs = value
            .all_exprs(&ctx)
            .into_iter()
            .flat_map(|expr| item_frame_column_refs(&expr))
            .collect::<Vec<_>>();
        if refs.is_empty() {
            continue;
        }

        let expr = value.expr(&ctx).unwrap_or_else(|| {
            panic!(
                "Adjustment transform routing for channel '{channel}' must be a single item-frame expression"
            )
        });
        assignments.push(
            ItemChannelAssignment::new(channel.clone(), expr)
                .expect("Failed to serialize adjustment transform routing expression"),
        );
        if let Some(original) = original_channels.get(&channel) {
            routed_channels.insert(channel, original.clone());
        } else {
            routed_channels.shift_remove(&channel);
        }
    }

    (routed_channels, assignments)
}

pub fn evaluate_item_assignments<'a>(
    assignments: impl Iterator<Item = &'a ItemChannelAssignment>,
    item_batch: &RecordBatch,
    ctx: &SessionContext,
) -> Result<RecordBatch, AvengerChartError> {
    let assignments = assignments.collect::<Vec<_>>();
    if assignments.is_empty() {
        return Ok(RecordBatch::new_empty(Arc::new(Schema::empty())));
    }
    let allowed_columns = item_batch
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<std::collections::HashSet<_>>();
    let expression_specs = assignments
        .iter()
        .map(|assignment| {
            Ok(PhysicalScalarExpressionSpec::new(
                assignment.channel.clone(),
                assignment.expr(ctx)?,
            ))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    let program = CompiledScalarExpressionProgram::compile(
        ctx,
        item_batch.schema(),
        expression_specs,
        PhysicalScalarProgramOptions::default().with_allowed_columns(allowed_columns),
    )?;
    Ok(program.evaluate_batch(item_batch)?)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedRectMarkSpec {
    pub assignments: Vec<ItemChannelAssignment>,
    pub zindex: Option<i32>,
}

impl DerivedRectMarkSpec {
    pub fn new(assignments: Vec<ItemChannelAssignment>, zindex: Option<i32>) -> Self {
        Self {
            assignments,
            zindex,
        }
    }

    pub fn requires_data_batch(&self) -> bool {
        self.assignments
            .iter()
            .any(|assignment| !assignment.data_fields.is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedTextMarkSpec {
    pub assignments: Vec<ItemChannelAssignment>,
    #[serde(default)]
    pub effects: PrimitiveMarkEffects,
    pub zindex: Option<i32>,
}

impl DerivedTextMarkSpec {
    pub fn new(
        assignments: Vec<ItemChannelAssignment>,
        effects: PrimitiveMarkEffects,
        zindex: Option<i32>,
    ) -> Self {
        Self {
            assignments,
            effects,
            zindex,
        }
    }

    pub fn requires_data_batch(&self) -> bool {
        self.assignments
            .iter()
            .any(|assignment| !assignment.data_fields.is_empty())
            || self.effects.requires_data_batch()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PointGeometryItem;

#[derive(Clone, Copy, Debug, Default)]
pub struct RectGeometryItem;

#[derive(Clone, Copy, Debug, Default)]
pub struct RuleGeometryItem;

#[derive(Clone, Copy, Debug, Default)]
pub struct ImageGeometryItem;

#[derive(Clone, Copy, Debug, Default)]
pub struct AreaGeometryItem;

#[doc(hidden)]
pub trait SizeAdjustmentChannel {}

#[doc(hidden)]
pub trait AngleAdjustmentChannel {}

#[doc(hidden)]
pub trait TextAdjustmentChannels {}

#[doc(hidden)]
pub trait StrokeWidthAdjustmentChannel {}

#[doc(hidden)]
pub trait PathAdjustmentChannels {}

#[doc(hidden)]
pub trait FillAdjustmentChannel {}

#[doc(hidden)]
pub trait OpacityAdjustmentChannel {}

#[doc(hidden)]
pub trait ShapeAdjustmentChannel {}

#[doc(hidden)]
pub trait StrokeAdjustmentChannel {}

#[doc(hidden)]
pub trait DefinedAdjustmentChannel {}

#[doc(hidden)]
pub trait StrokeDashAdjustmentChannel {}

#[doc(hidden)]
pub trait StrokeCapAdjustmentChannel {}

#[doc(hidden)]
pub trait StrokeJoinAdjustmentChannel {}

#[derive(Clone, Debug, Default)]
pub struct BboxExpr {
    _private: (),
}

impl BboxExpr {
    pub fn left(&self) -> Expr {
        item_column_expr(item_bbox_column_name("left"))
    }

    pub fn right(&self) -> Expr {
        item_column_expr(item_bbox_column_name("right"))
    }

    pub fn top(&self) -> Expr {
        item_column_expr(item_bbox_column_name("top"))
    }

    pub fn bottom(&self) -> Expr {
        item_column_expr(item_bbox_column_name("bottom"))
    }
}

#[derive(Debug)]
pub struct AdjustItem<M, G> {
    assignments: Vec<ItemChannelAssignment>,
    _mark: PhantomData<M>,
    _geometry: PhantomData<G>,
}

impl<M, G> Clone for AdjustItem<M, G> {
    fn clone(&self) -> Self {
        Self {
            assignments: self.assignments.clone(),
            _mark: PhantomData,
            _geometry: PhantomData,
        }
    }
}

impl<M, G> Default for AdjustItem<M, G> {
    fn default() -> Self {
        Self {
            assignments: Vec::new(),
            _mark: PhantomData,
            _geometry: PhantomData,
        }
    }
}

impl<M, G> AdjustItem<M, G> {
    pub fn channel(&self, channel: &str) -> Expr {
        item_column_expr(item_channel_column_name(channel))
    }

    pub fn data(&self, field: &str) -> Expr {
        item_column_expr(item_data_column_name(field))
    }

    pub fn bbox(&self) -> BboxExpr {
        BboxExpr::default()
    }

    pub fn set_channel(
        &self,
        channel: impl Into<String>,
        expr: impl IntoExpr,
    ) -> Result<Self, AvengerChartError> {
        let mut next = self.clone();
        next.assignments
            .push(ItemChannelAssignment::new(channel, expr)?);
        Ok(next)
    }

    pub fn into_adjustment_spec(self) -> ExpressionMarkAdjustmentSpec {
        ExpressionMarkAdjustmentSpec::new(self.assignments)
    }
}

impl<M> AdjustItem<M, PointGeometryItem> {
    pub fn x(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x", expr)
            .expect("Failed to serialize x adjustment expression")
    }

    pub fn y(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y", expr)
            .expect("Failed to serialize y adjustment expression")
    }
}

impl<M: SizeAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn size(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("size", expr)
            .expect("Failed to serialize size adjustment expression")
    }
}

impl<M: StrokeWidthAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn stroke_width(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_width", expr)
            .expect("Failed to serialize stroke_width adjustment expression")
    }
}

impl<M: FillAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn fill(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("fill", expr)
            .expect("Failed to serialize fill adjustment expression")
    }
}

impl<M: OpacityAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn opacity(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("opacity", expr)
            .expect("Failed to serialize opacity adjustment expression")
    }
}

impl<M: StrokeAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn stroke(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke", expr)
            .expect("Failed to serialize stroke adjustment expression")
    }
}

impl<M: DefinedAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn defined(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("defined", expr)
            .expect("Failed to serialize defined adjustment expression")
    }
}

impl<M: StrokeDashAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn stroke_dash(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_dash", expr)
            .expect("Failed to serialize stroke_dash adjustment expression")
    }
}

impl<M: StrokeCapAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn stroke_cap(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_cap", expr)
            .expect("Failed to serialize stroke_cap adjustment expression")
    }
}

impl<M: StrokeJoinAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn stroke_join(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_join", expr)
            .expect("Failed to serialize stroke_join adjustment expression")
    }
}

impl<M: ShapeAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn shape(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("shape", expr)
            .expect("Failed to serialize shape adjustment expression")
    }
}

impl<M: PathAdjustmentChannels> AdjustItem<M, PointGeometryItem> {
    pub fn path(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("path", expr)
            .expect("Failed to serialize path adjustment expression")
    }

    pub fn path_transform(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("path_transform", expr)
            .expect("Failed to serialize path_transform adjustment expression")
    }
}

impl<M: AngleAdjustmentChannel> AdjustItem<M, PointGeometryItem> {
    pub fn angle(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("angle", expr)
            .expect("Failed to serialize angle adjustment expression")
    }
}

impl<M: TextAdjustmentChannels> AdjustItem<M, PointGeometryItem> {
    pub fn color(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("color", expr)
            .expect("Failed to serialize color adjustment expression")
    }

    pub fn text(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("text", expr)
            .expect("Failed to serialize text adjustment expression")
    }

    pub fn align(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("align", expr)
            .expect("Failed to serialize align adjustment expression")
    }

    pub fn baseline(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("baseline", expr)
            .expect("Failed to serialize baseline adjustment expression")
    }

    pub fn font(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("font", expr)
            .expect("Failed to serialize font adjustment expression")
    }

    pub fn font_size(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("font_size", expr)
            .expect("Failed to serialize font_size adjustment expression")
    }

    pub fn font_weight(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("font_weight", expr)
            .expect("Failed to serialize font_weight adjustment expression")
    }

    pub fn font_style(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("font_style", expr)
            .expect("Failed to serialize font_style adjustment expression")
    }

    pub fn limit(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("limit", expr)
            .expect("Failed to serialize limit adjustment expression")
    }

    pub fn leader(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader", expr)
            .expect("Failed to serialize leader adjustment expression")
    }

    pub fn leader_offset_x(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_offset_x", expr)
            .expect("Failed to serialize leader_offset_x adjustment expression")
    }

    pub fn leader_offset_y(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_offset_y", expr)
            .expect("Failed to serialize leader_offset_y adjustment expression")
    }

    pub fn leader_stroke(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_stroke", expr)
            .expect("Failed to serialize leader_stroke adjustment expression")
    }

    pub fn leader_stroke_width(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_stroke_width", expr)
            .expect("Failed to serialize leader_stroke_width adjustment expression")
    }

    pub fn leader_stroke_dash(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_stroke_dash", expr)
            .expect("Failed to serialize leader_stroke_dash adjustment expression")
    }

    pub fn leader_stroke_cap(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_stroke_cap", expr)
            .expect("Failed to serialize leader_stroke_cap adjustment expression")
    }

    pub fn leader_stroke_join(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_stroke_join", expr)
            .expect("Failed to serialize leader_stroke_join adjustment expression")
    }

    pub fn leader_label_padding(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_label_padding", expr)
            .expect("Failed to serialize leader_label_padding adjustment expression")
    }

    pub fn leader_target_radius(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_target_radius", expr)
            .expect("Failed to serialize leader_target_radius adjustment expression")
    }

    pub fn leader_min_length(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_min_length", expr)
            .expect("Failed to serialize leader_min_length adjustment expression")
    }

    pub fn leader_shape(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_shape", expr)
            .expect("Failed to serialize leader_shape adjustment expression")
    }

    pub fn leader_arrow(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_arrow", expr)
            .expect("Failed to serialize leader_arrow adjustment expression")
    }

    pub fn leader_arrow_length(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_arrow_length", expr)
            .expect("Failed to serialize leader_arrow_length adjustment expression")
    }

    pub fn leader_arrow_width(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("leader_arrow_width", expr)
            .expect("Failed to serialize leader_arrow_width adjustment expression")
    }
}

impl<M> AdjustItem<M, RectGeometryItem> {
    pub fn x(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x", expr)
            .expect("Failed to serialize x adjustment expression")
    }

    pub fn y(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y", expr)
            .expect("Failed to serialize y adjustment expression")
    }

    pub fn x2(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x2", expr)
            .expect("Failed to serialize x2 adjustment expression")
    }

    pub fn y2(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y2", expr)
            .expect("Failed to serialize y2 adjustment expression")
    }

    pub fn corner_radius(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("corner_radius", expr)
            .expect("Failed to serialize corner_radius adjustment expression")
    }

    pub fn fill(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("fill", expr)
            .expect("Failed to serialize fill adjustment expression")
    }

    pub fn stroke(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke", expr)
            .expect("Failed to serialize stroke adjustment expression")
    }

    pub fn stroke_width(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_width", expr)
            .expect("Failed to serialize stroke_width adjustment expression")
    }

    pub fn opacity(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("opacity", expr)
            .expect("Failed to serialize opacity adjustment expression")
    }
}

impl<M> AdjustItem<M, RuleGeometryItem> {
    pub fn x(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x", expr)
            .expect("Failed to serialize x adjustment expression")
    }

    pub fn y(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y", expr)
            .expect("Failed to serialize y adjustment expression")
    }

    pub fn x2(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x2", expr)
            .expect("Failed to serialize x2 adjustment expression")
    }

    pub fn y2(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y2", expr)
            .expect("Failed to serialize y2 adjustment expression")
    }

    pub fn stroke_width(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_width", expr)
            .expect("Failed to serialize stroke_width adjustment expression")
    }

    pub fn stroke(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke", expr)
            .expect("Failed to serialize stroke adjustment expression")
    }

    pub fn stroke_dash(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_dash", expr)
            .expect("Failed to serialize stroke_dash adjustment expression")
    }

    pub fn stroke_cap(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_cap", expr)
            .expect("Failed to serialize stroke_cap adjustment expression")
    }

    pub fn opacity(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("opacity", expr)
            .expect("Failed to serialize opacity adjustment expression")
    }
}

impl<M> AdjustItem<M, ImageGeometryItem> {
    pub fn image(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("image", expr)
            .expect("Failed to serialize image adjustment expression")
    }

    pub fn x(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x", expr)
            .expect("Failed to serialize x adjustment expression")
    }

    pub fn y(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y", expr)
            .expect("Failed to serialize y adjustment expression")
    }

    pub fn width(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("width", expr)
            .expect("Failed to serialize width adjustment expression")
    }

    pub fn height(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("height", expr)
            .expect("Failed to serialize height adjustment expression")
    }

    pub fn align(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("align", expr)
            .expect("Failed to serialize align adjustment expression")
    }

    pub fn baseline(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("baseline", expr)
            .expect("Failed to serialize baseline adjustment expression")
    }

    pub fn aspect(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("aspect", expr)
            .expect("Failed to serialize aspect adjustment expression")
    }

    pub fn smooth(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("smooth", expr)
            .expect("Failed to serialize smooth adjustment expression")
    }
}

impl<M> AdjustItem<M, AreaGeometryItem> {
    pub fn orientation(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("orientation", expr)
            .expect("Failed to serialize orientation adjustment expression")
    }

    pub fn x(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x", expr)
            .expect("Failed to serialize x adjustment expression")
    }

    pub fn y(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y", expr)
            .expect("Failed to serialize y adjustment expression")
    }

    pub fn x2(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("x2", expr)
            .expect("Failed to serialize x2 adjustment expression")
    }

    pub fn y2(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("y2", expr)
            .expect("Failed to serialize y2 adjustment expression")
    }

    pub fn stroke_width(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_width", expr)
            .expect("Failed to serialize stroke_width adjustment expression")
    }

    pub fn fill(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("fill", expr)
            .expect("Failed to serialize fill adjustment expression")
    }

    pub fn stroke(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke", expr)
            .expect("Failed to serialize stroke adjustment expression")
    }

    pub fn stroke_dash(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_dash", expr)
            .expect("Failed to serialize stroke_dash adjustment expression")
    }

    pub fn stroke_cap(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_cap", expr)
            .expect("Failed to serialize stroke_cap adjustment expression")
    }

    pub fn stroke_join(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("stroke_join", expr)
            .expect("Failed to serialize stroke_join adjustment expression")
    }

    pub fn opacity(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("opacity", expr)
            .expect("Failed to serialize opacity adjustment expression")
    }

    pub fn defined(&self, expr: impl IntoExpr) -> Self {
        self.set_channel("defined", expr)
            .expect("Failed to serialize defined adjustment expression")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjustmentTransformRequirements {
    #[serde(default)]
    pub data_fields: Vec<String>,
    #[serde(default)]
    pub source_frame: bool,
    #[serde(default)]
    pub base_scene: bool,
    #[serde(default)]
    pub text_measurement: bool,
}

impl AdjustmentTransformRequirements {
    pub fn with_data_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.data_fields = fields.into_iter().map(Into::into).collect();
        self.data_fields.sort();
        self.data_fields.dedup();
        self
    }

    pub fn merge(&mut self, other: AdjustmentTransformRequirements) {
        self.data_fields.extend(other.data_fields);
        self.data_fields.sort();
        self.data_fields.dedup();
        self.source_frame |= other.source_frame;
        self.base_scene |= other.base_scene;
        self.text_measurement |= other.text_measurement;
    }

    pub fn with_source_frame(mut self) -> Self {
        self.source_frame = true;
        self
    }

    pub fn with_base_scene(mut self) -> Self {
        self.base_scene = true;
        self
    }

    pub fn with_text_measurement(mut self) -> Self {
        self.text_measurement = true;
        self
    }

    pub fn requires_data_batch(&self) -> bool {
        !self.data_fields.is_empty()
    }

    pub fn requires_base_scene(&self) -> bool {
        self.base_scene
    }
}

pub struct AdjustmentTransformContext<'a> {
    pub source: Option<&'a MarkEvaluationFrame>,
    pub plot_area: Option<PlotAreaInfo<'a>>,
    pub base_scene: Option<&'a BasePlotAreaScene>,
    pub text_measurement: Option<&'a dyn TextMeasurementService>,
}

impl<'a> AdjustmentTransformContext<'a> {
    pub fn new() -> Self {
        Self {
            source: None,
            plot_area: None,
            base_scene: None,
            text_measurement: None,
        }
    }

    pub fn with_source(mut self, source: &'a MarkEvaluationFrame) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_plot_area(mut self, plot_area: PlotAreaInfo<'a>) -> Self {
        self.plot_area = Some(plot_area);
        self
    }

    pub fn with_base_scene(mut self, base_scene: &'a BasePlotAreaScene) -> Self {
        self.base_scene = Some(base_scene);
        self
    }

    pub fn with_text_measurement(
        mut self,
        text_measurement: &'a dyn TextMeasurementService,
    ) -> Self {
        self.text_measurement = Some(text_measurement);
        self
    }
}

impl Default for AdjustmentTransformContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlotAreaInfo<'a> {
    pub facet_path: &'a [ScalarValue],
    pub width: f32,
    pub height: f32,
    pub origin: [f32; 2],
    pub clip: Option<&'a Clip>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeometryBounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl GeometryBounds {
    pub fn overlaps(&self, other: &Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BasePlotAreaScene {
    marks: Vec<SceneMark>,
}

impl BasePlotAreaScene {
    pub fn from_scene_marks(marks: &[SceneMark]) -> Self {
        Self {
            marks: marks.to_vec(),
        }
    }

    pub fn marks(&self) -> &[SceneMark] {
        &self.marks
    }
}

pub trait TextMeasurementService: Send + Sync {
    fn measure_text_bounds(&self, config: &TextMeasurementConfig<'_>) -> TextBounds;
}

#[derive(Clone, Debug)]
pub struct MarkAdjustmentCompileContext {
    stage_index: usize,
}

impl MarkAdjustmentCompileContext {
    pub fn new(stage_index: usize) -> Self {
        Self { stage_index }
    }

    pub fn output_column_name(&self, field: &str) -> String {
        item_adjustment_column_name(self.stage_index, field)
    }

    pub fn output_expr(&self, field: &str) -> Expr {
        item_column_expr(self.output_column_name(field))
    }
}

pub trait MarkAdjustmentTransform: Clone + Send + Sync + 'static {
    type Output;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError>;
}

#[typetag::serde(tag = "type")]
pub trait CompiledMarkAdjustmentTransform: Send + Sync {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform>;

    fn requirements(&self) -> AdjustmentTransformRequirements {
        AdjustmentTransformRequirements::default()
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError>;
}

impl Clone for Box<dyn CompiledMarkAdjustmentTransform> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

pub struct MarkEvaluationFrame {
    len: usize,
    columns: IndexMap<String, (Field, ArrayRef)>,
}

impl MarkEvaluationFrame {
    pub fn new(len: usize, columns: impl IntoIterator<Item = (Field, ArrayRef)>) -> Self {
        let columns = columns
            .into_iter()
            .map(|(field, array)| (field.name().clone(), (field, array)))
            .collect();
        Self { len, columns }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn column(&self, name: &str) -> Option<&ArrayRef> {
        self.columns.get(name).map(|(_, array)| array)
    }

    pub fn set_column(
        &mut self,
        name: impl Into<String>,
        array: ArrayRef,
    ) -> Result<(), AvengerChartError> {
        let name = name.into();
        if array.len() != self.len {
            return Err(AvengerChartError::InternalError(format!(
                "Adjustment frame column '{name}' has {} rows, expected {}",
                array.len(),
                self.len
            )));
        }
        let field = Field::new(name.clone(), array.data_type().clone(), true);
        self.columns.insert(name, (field, array));
        Ok(())
    }

    pub fn record_batch(&self) -> Result<RecordBatch, AvengerChartError> {
        let fields = self
            .columns
            .values()
            .map(|(field, _)| field.clone())
            .collect::<Vec<_>>();
        let arrays = self
            .columns
            .values()
            .map(|(_, array)| array.clone())
            .collect::<Vec<_>>();
        Ok(RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)?)
    }

    pub fn f32_values(&self, name: &str) -> Result<Vec<f32>, AvengerChartError> {
        let array = self.column(name).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Adjustment transform expected item-frame column '{name}'"
            ))
        })?;
        let mut result = Vec::with_capacity(array.len());
        if let Some(values) = array.as_any().downcast_ref::<Float32Array>() {
            for index in 0..values.len() {
                if values.is_null(index) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Adjustment transform item-frame column '{name}' is null at row {index}"
                    )));
                }
                result.push(values.value(index));
            }
        } else if let Some(values) = array.as_any().downcast_ref::<Float64Array>() {
            for index in 0..values.len() {
                if values.is_null(index) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Adjustment transform item-frame column '{name}' is null at row {index}"
                    )));
                }
                result.push(values.value(index) as f32);
            }
        } else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Adjustment transform expected item-frame column '{name}' to be Float32 or Float64, got {:?}",
                array.data_type()
            )));
        }
        Ok(result)
    }

    pub fn scalar_values(&self, name: &str) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let array = self.column(name).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Adjustment transform expected item-frame column '{name}'"
            ))
        })?;
        let mut result = Vec::with_capacity(array.len());
        for index in 0..array.len() {
            result.push(ScalarValue::try_from_array(array, index)?);
        }
        Ok(result)
    }

    pub fn string_values(&self, name: &str) -> Result<Vec<String>, AvengerChartError> {
        let array = self.column(name).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Adjustment transform expected item-frame column '{name}'"
            ))
        })?;
        let values = array
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Adjustment transform expected item-frame column '{name}' to be Utf8, got {:?}",
                    array.data_type()
                ))
            })?;
        let mut result = Vec::with_capacity(values.len());
        for index in 0..values.len() {
            if values.is_null(index) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Adjustment transform item-frame column '{name}' is null at row {index}"
                )));
            }
            result.push(values.value(index).to_string());
        }
        Ok(result)
    }

    pub fn bool_values(&self, name: &str) -> Result<Vec<bool>, AvengerChartError> {
        let array = self.column(name).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Adjustment transform expected item-frame column '{name}'"
            ))
        })?;
        let values = array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Adjustment transform expected item-frame column '{name}' to be Boolean, got {:?}",
                    array.data_type()
                ))
            })?;
        let mut result = Vec::with_capacity(values.len());
        for index in 0..values.len() {
            if values.is_null(index) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Adjustment transform item-frame column '{name}' is null at row {index}"
                )));
            }
            result.push(values.value(index));
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Nudge {
    dx: f32,
    dy: f32,
}

impl Nudge {
    pub fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }
}

#[derive(Clone, Debug)]
pub struct NudgeOutput {
    x: Expr,
    y: Expr,
}

impl NudgeOutput {
    pub fn x(&self) -> Expr {
        self.x.clone()
    }

    pub fn y(&self) -> Expr {
        self.y.clone()
    }
}

impl MarkAdjustmentTransform for Nudge {
    type Output = NudgeOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        let x_column = ctx.output_column_name("x");
        let y_column = ctx.output_column_name("y");
        let output = NudgeOutput {
            x: item_column_expr(x_column.clone()),
            y: item_column_expr(y_column.clone()),
        };
        Ok((
            Box::new(CompiledNudge {
                dx: self.dx,
                dy: self.dy,
                x_column,
                y_column,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledNudge {
    dx: f32,
    dy: f32,
    x_column: String,
    y_column: String,
}

#[typetag::serde(name = "nudge")]
impl CompiledMarkAdjustmentTransform for CompiledNudge {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        _context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        let x = frame.f32_values(&item_channel_column_name("x"))?;
        let y = frame.f32_values(&item_channel_column_name("y"))?;
        frame.set_column(
            self.x_column.clone(),
            Arc::new(Float32Array::from(
                x.into_iter()
                    .map(|value| value + self.dx)
                    .collect::<Vec<_>>(),
            )),
        )?;
        frame.set_column(
            self.y_column.clone(),
            Arc::new(Float32Array::from(
                y.into_iter()
                    .map(|value| value + self.dy)
                    .collect::<Vec<_>>(),
            )),
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum PointAdjustmentAxis {
    X,
    Y,
}

impl PointAdjustmentAxis {
    fn channel_name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Jitter {
    axis: PointAdjustmentAxis,
    width_px: f32,
    seed: u64,
}

impl Jitter {
    pub fn x() -> Self {
        Self {
            axis: PointAdjustmentAxis::X,
            width_px: 1.0,
            seed: 0,
        }
    }

    pub fn y() -> Self {
        Self {
            axis: PointAdjustmentAxis::Y,
            width_px: 1.0,
            seed: 0,
        }
    }

    pub fn width_px(mut self, width_px: f32) -> Self {
        self.width_px = width_px;
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
}

#[derive(Clone, Debug)]
pub struct JitterOutput {
    x: Expr,
    y: Expr,
}

impl JitterOutput {
    pub fn x(&self) -> Expr {
        self.x.clone()
    }

    pub fn y(&self) -> Expr {
        self.y.clone()
    }
}

impl MarkAdjustmentTransform for Jitter {
    type Output = JitterOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        if !self.width_px.is_finite() || self.width_px < 0.0 {
            return Err(AvengerChartError::InvalidArgument(
                "Jitter::width_px(...) must be finite and non-negative".to_string(),
            ));
        }
        let x_column = ctx.output_column_name("x");
        let y_column = ctx.output_column_name("y");
        let output = JitterOutput {
            x: item_column_expr(x_column.clone()),
            y: item_column_expr(y_column.clone()),
        };
        Ok((
            Box::new(CompiledJitter {
                axis: self.axis,
                width_px: self.width_px,
                seed: self.seed,
                x_column,
                y_column,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledJitter {
    axis: PointAdjustmentAxis,
    width_px: f32,
    seed: u64,
    x_column: String,
    y_column: String,
}

#[typetag::serde(name = "jitter")]
impl CompiledMarkAdjustmentTransform for CompiledJitter {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        _context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        let mut x = frame.f32_values(&item_channel_column_name("x"))?;
        let mut y = frame.f32_values(&item_channel_column_name("y"))?;
        let values = match self.axis {
            PointAdjustmentAxis::X => &mut x,
            PointAdjustmentAxis::Y => &mut y,
        };
        for (index, value) in values.iter_mut().enumerate() {
            let unit = deterministic_unit(self.seed, index as u64, self.axis.channel_name());
            *value += (unit - 0.5) * self.width_px;
        }
        write_point_output_columns(frame, &self.x_column, &self.y_column, x, y)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dodge {
    axis: PointAdjustmentAxis,
    by: Option<String>,
    step_px: f32,
}

impl Dodge {
    pub fn x() -> Self {
        Self {
            axis: PointAdjustmentAxis::X,
            by: None,
            step_px: 1.0,
        }
    }

    pub fn y() -> Self {
        Self {
            axis: PointAdjustmentAxis::Y,
            by: None,
            step_px: 1.0,
        }
    }

    pub fn by(mut self, field: impl Into<String>) -> Self {
        self.by = Some(field.into());
        self
    }

    pub fn step_px(mut self, step_px: f32) -> Self {
        self.step_px = step_px;
        self
    }
}

#[derive(Clone, Debug)]
pub struct DodgeOutput {
    x: Expr,
    y: Expr,
}

impl DodgeOutput {
    pub fn x(&self) -> Expr {
        self.x.clone()
    }

    pub fn y(&self) -> Expr {
        self.y.clone()
    }
}

impl MarkAdjustmentTransform for Dodge {
    type Output = DodgeOutput;

    fn compile(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustmentTransform>, Self::Output), AvengerChartError> {
        if !self.step_px.is_finite() || self.step_px < 0.0 {
            return Err(AvengerChartError::InvalidArgument(
                "Dodge::step_px(...) must be finite and non-negative".to_string(),
            ));
        }
        let by = self.by.ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "Dodge adjustment transforms require Dodge::by(...)".to_string(),
            )
        })?;
        let x_column = ctx.output_column_name("x");
        let y_column = ctx.output_column_name("y");
        let output = DodgeOutput {
            x: item_column_expr(x_column.clone()),
            y: item_column_expr(y_column.clone()),
        };
        Ok((
            Box::new(CompiledDodge {
                axis: self.axis,
                by,
                step_px: self.step_px,
                x_column,
                y_column,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledDodge {
    axis: PointAdjustmentAxis,
    by: String,
    step_px: f32,
    x_column: String,
    y_column: String,
}

#[typetag::serde(name = "dodge")]
impl CompiledMarkAdjustmentTransform for CompiledDodge {
    fn clone_box(&self) -> Box<dyn CompiledMarkAdjustmentTransform> {
        Box::new(self.clone())
    }

    fn requirements(&self) -> AdjustmentTransformRequirements {
        AdjustmentTransformRequirements::default().with_data_fields([self.by.clone()])
    }

    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        _context: &AdjustmentTransformContext<'_>,
    ) -> Result<(), AvengerChartError> {
        let mut x = frame.f32_values(&item_channel_column_name("x"))?;
        let mut y = frame.f32_values(&item_channel_column_name("y"))?;
        let groups = frame.scalar_values(&item_data_column_name(&self.by))?;
        let anchors = match self.axis {
            PointAdjustmentAxis::X => &x,
            PointAdjustmentAxis::Y => &y,
        };
        let anchor_bits = anchors
            .iter()
            .map(|anchor| anchor.to_bits())
            .collect::<Vec<_>>();
        let group_order = dodge_group_order(&anchor_bits, &groups);
        let values = match self.axis {
            PointAdjustmentAxis::X => &mut x,
            PointAdjustmentAxis::Y => &mut y,
        };
        for (index, value) in values.iter_mut().enumerate() {
            let anchor = anchor_bits[index];
            let groups_for_anchor = group_order.get(&anchor).ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Dodge adjustment lost anchor grouping".to_string(),
                )
            })?;
            let group_index = groups_for_anchor.get(&groups[index]).ok_or_else(|| {
                AvengerChartError::InternalError("Dodge adjustment lost group index".to_string())
            })?;
            let group_count = groups_for_anchor.len();
            let centered_index = *group_index as f32 - (group_count as f32 - 1.0) / 2.0;
            *value += centered_index * self.step_px;
        }
        write_point_output_columns(frame, &self.x_column, &self.y_column, x, y)
    }
}

fn write_point_output_columns(
    frame: &mut MarkEvaluationFrame,
    x_column: &str,
    y_column: &str,
    x: Vec<f32>,
    y: Vec<f32>,
) -> Result<(), AvengerChartError> {
    frame.set_column(x_column.to_string(), Arc::new(Float32Array::from(x)))?;
    frame.set_column(y_column.to_string(), Arc::new(Float32Array::from(y)))?;
    Ok(())
}

fn dodge_group_order(
    anchor_bits: &[u32],
    groups: &[ScalarValue],
) -> IndexMap<u32, IndexMap<ScalarValue, usize>> {
    let mut group_order = IndexMap::<u32, IndexMap<ScalarValue, usize>>::new();
    for (anchor, group) in anchor_bits.iter().zip(groups) {
        let groups_for_anchor = group_order.entry(*anchor).or_default();
        if !groups_for_anchor.contains_key(group) {
            let index = groups_for_anchor.len();
            groups_for_anchor.insert(group.clone(), index);
        }
    }
    group_order
}

fn deterministic_unit(seed: u64, row: u64, salt: &str) -> f32 {
    let mut state = seed ^ row.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for byte in salt.bytes() {
        state ^= u64::from(byte);
        state = splitmix64(state);
    }
    let value = splitmix64(state);
    ((value >> 40) as f32) / ((1u64 << 24) as f32)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
