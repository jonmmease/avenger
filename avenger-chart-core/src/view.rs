//! View-scoped mark authoring and compiled state.

use std::time::Duration;

use datafusion::{logical_expr::Expr, scalar::ScalarValue};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, CompiledDataContext, DataContext, DefaultLogicalExprNodeExt, IntoExpr,
    Param, RepeatContext, SerializableExpr, resolve_repeat_placeholders, validate_structural_id,
};

/// How a view-scoped mark should behave while a newer view-local result is pending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewStalePolicy {
    /// Do not render the view-local mark until the current result is ready.
    #[default]
    HideUntilReady,
    /// Retarget the last ready result through the current scales while the next
    /// result is being computed.
    RetargetCached,
}

/// Async and stale-result policy for a view-scoped mark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewAsyncPolicy {
    pub stale_policy: ViewStalePolicy,
    pub throttle: Option<Duration>,
    pub debounce: Option<Duration>,
}

impl Default for ViewAsyncPolicy {
    fn default() -> Self {
        Self {
            stale_policy: ViewStalePolicy::HideUntilReady,
            throttle: None,
            debounce: None,
        }
    }
}

/// Entry point for public view builders.
#[derive(Clone, Debug, Default)]
pub struct View;

impl View {
    pub fn cartesian() -> CartesianView {
        CartesianView::default()
    }

    /// A scale-free view whose x/y domains and ranges are the rendered frame
    /// in logical pixels. This is the view scope for [`crate::PixelFrame`]
    /// marks such as composed widget parts.
    pub fn pixel_frame() -> PixelFrameView {
        PixelFrameView::default()
    }
}

/// Cartesian view authoring builder.
#[derive(Clone, Debug, Default)]
pub struct CartesianView {
    id: Option<String>,
    x_domain: Option<Expr>,
    y_domain: Option<Expr>,
    policy: ViewAsyncPolicy,
}

impl CartesianView {
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn x_domain(mut self, expr: impl IntoExpr) -> Self {
        self.x_domain = Some(expr.into_expr());
        self
    }

    pub fn y_domain(mut self, expr: impl IntoExpr) -> Self {
        self.y_domain = Some(expr.into_expr());
        self
    }

    pub fn preview_cached(mut self, preview_cached: bool) -> Self {
        self.policy.stale_policy = if preview_cached {
            ViewStalePolicy::RetargetCached
        } else {
            ViewStalePolicy::HideUntilReady
        };
        self
    }

    pub fn stale_policy(mut self, stale_policy: ViewStalePolicy) -> Self {
        self.policy.stale_policy = stale_policy;
        self
    }

    pub fn throttle(mut self, throttle: Duration) -> Self {
        self.policy.throttle = Some(throttle);
        self
    }

    pub fn debounce(mut self, debounce: Duration) -> Self {
        self.policy.debounce = Some(debounce);
        self
    }
}

/// Authoring-side view builder contract.
pub trait ViewSpec: Clone + Send + Sync + 'static {
    fn into_compiled_and_ref(self) -> Result<(CompiledViewSpec, ViewRef), AvengerChartError>;
}

impl ViewSpec for CartesianView {
    fn into_compiled_and_ref(self) -> Result<(CompiledViewSpec, ViewRef), AvengerChartError> {
        let id = self.id.ok_or_else(|| {
            AvengerChartError::InvalidArgument("Cartesian view requires an id".to_string())
        })?;
        validate_structural_id("view", &id)?;

        let x_domain = self.x_domain.ok_or_else(|| {
            AvengerChartError::InvalidArgument("Cartesian view requires x_domain(...)".to_string())
        })?;
        let y_domain = self.y_domain.ok_or_else(|| {
            AvengerChartError::InvalidArgument("Cartesian view requires y_domain(...)".to_string())
        })?;

        let spec = CompiledViewSpec::Cartesian(CompiledCartesianViewSpec {
            id: id.clone(),
            x_domain: LogicalExprNode::from_default_expr(x_domain)?,
            y_domain: LogicalExprNode::from_default_expr(y_domain)?,
            policy: self.policy,
        });
        Ok((spec, ViewRef { id }))
    }
}

/// Scale-free view authoring builder for logical-pixel frames.
#[derive(Clone, Debug, Default)]
pub struct PixelFrameView {
    id: Option<String>,
    policy: ViewAsyncPolicy,
}

impl PixelFrameView {
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn preview_cached(mut self, preview_cached: bool) -> Self {
        self.policy.stale_policy = if preview_cached {
            ViewStalePolicy::RetargetCached
        } else {
            ViewStalePolicy::HideUntilReady
        };
        self
    }

    pub fn stale_policy(mut self, stale_policy: ViewStalePolicy) -> Self {
        self.policy.stale_policy = stale_policy;
        self
    }

    pub fn throttle(mut self, throttle: Duration) -> Self {
        self.policy.throttle = Some(throttle);
        self
    }

    pub fn debounce(mut self, debounce: Duration) -> Self {
        self.policy.debounce = Some(debounce);
        self
    }
}

impl ViewSpec for PixelFrameView {
    fn into_compiled_and_ref(self) -> Result<(CompiledViewSpec, ViewRef), AvengerChartError> {
        let id = self.id.ok_or_else(|| {
            AvengerChartError::InvalidArgument("Pixel-frame view requires an id".to_string())
        })?;
        validate_structural_id("view", &id)?;
        let spec = CompiledViewSpec::PixelFrame(CompiledPixelFrameViewSpec {
            id: id.clone(),
            policy: self.policy,
        });
        Ok((spec, ViewRef { id }))
    }
}

/// Serialized/compiled view specification.
#[derive(Clone, Debug, PartialEq)]
// Boxing variants would change the public construction API and compiled serde contract.
#[allow(clippy::large_enum_variant)]
pub enum CompiledViewSpec {
    Cartesian(CompiledCartesianViewSpec),
    PixelFrame(CompiledPixelFrameViewSpec),
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HumanReadableCompiledViewSpecRef<'a> {
    Cartesian(&'a CompiledCartesianViewSpec),
    PixelFrame(&'a CompiledPixelFrameViewSpec),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
enum HumanReadableCompiledViewSpec {
    Cartesian(CompiledCartesianViewSpec),
    PixelFrame(CompiledPixelFrameViewSpec),
}

#[derive(Serialize)]
enum BinaryCompiledViewSpecRef<'a> {
    Cartesian(&'a CompiledCartesianViewSpec),
    PixelFrame(&'a CompiledPixelFrameViewSpec),
}

#[derive(Deserialize)]
#[allow(clippy::large_enum_variant)]
enum BinaryCompiledViewSpec {
    Cartesian(CompiledCartesianViewSpec),
    PixelFrame(CompiledPixelFrameViewSpec),
}

impl Serialize for CompiledViewSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            match self {
                Self::Cartesian(spec) => {
                    HumanReadableCompiledViewSpecRef::Cartesian(spec).serialize(serializer)
                }
                Self::PixelFrame(spec) => {
                    HumanReadableCompiledViewSpecRef::PixelFrame(spec).serialize(serializer)
                }
            }
        } else {
            match self {
                Self::Cartesian(spec) => {
                    BinaryCompiledViewSpecRef::Cartesian(spec).serialize(serializer)
                }
                Self::PixelFrame(spec) => {
                    BinaryCompiledViewSpecRef::PixelFrame(spec).serialize(serializer)
                }
            }
        }
    }
}

impl<'de> Deserialize<'de> for CompiledViewSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            Ok(
                match HumanReadableCompiledViewSpec::deserialize(deserializer)? {
                    HumanReadableCompiledViewSpec::Cartesian(spec) => Self::Cartesian(spec),
                    HumanReadableCompiledViewSpec::PixelFrame(spec) => Self::PixelFrame(spec),
                },
            )
        } else {
            Ok(match BinaryCompiledViewSpec::deserialize(deserializer)? {
                BinaryCompiledViewSpec::Cartesian(spec) => Self::Cartesian(spec),
                BinaryCompiledViewSpec::PixelFrame(spec) => Self::PixelFrame(spec),
            })
        }
    }
}

impl CompiledViewSpec {
    pub fn id(&self) -> &str {
        match self {
            Self::Cartesian(spec) => &spec.id,
            Self::PixelFrame(spec) => &spec.id,
        }
    }

    pub fn policy(&self) -> &ViewAsyncPolicy {
        match self {
            Self::Cartesian(spec) => &spec.policy,
            Self::PixelFrame(spec) => &spec.policy,
        }
    }

    pub fn view_ref(&self) -> ViewRef {
        ViewRef {
            id: self.id().to_string(),
        }
    }

    pub fn resolve_repeat(&self, ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        match self {
            Self::Cartesian(spec) => Ok(Self::Cartesian(spec.resolve_repeat(ctx)?)),
            Self::PixelFrame(spec) => Ok(Self::PixelFrame(spec.clone())),
        }
    }
}

/// Serialized scale-free view for a logical-pixel frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledPixelFrameViewSpec {
    pub id: String,
    pub policy: ViewAsyncPolicy,
}

/// Serialized/compiled Cartesian view specification.
#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledCartesianViewSpec {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub x_domain: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub y_domain: LogicalExprNode,
    pub policy: ViewAsyncPolicy,
}

impl CompiledCartesianViewSpec {
    pub fn resolve_repeat(&self, ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        let session_context = datafusion::prelude::SessionContext::new();
        Ok(Self {
            id: self.id.clone(),
            x_domain: LogicalExprNode::from_default_expr(resolve_repeat_placeholders(
                self.x_domain.to_default_expr(&session_context)?,
                ctx,
            )?)?,
            y_domain: LogicalExprNode::from_default_expr(resolve_repeat_placeholders(
                self.y_domain.to_default_expr(&session_context)?,
                ctx,
            )?)?,
            policy: self.policy.clone(),
        })
    }
}

/// Authoring-side view scope state stored on a mark.
#[derive(Clone)]
pub struct ViewScopeState {
    pub spec: CompiledViewSpec,
    pub data: DataContext,
}

impl ViewScopeState {
    pub fn new(spec: CompiledViewSpec, data: DataContext) -> Self {
        Self { spec, data }
    }

    pub fn resolve_repeat(&self, ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        Ok(Self {
            spec: self.spec.resolve_repeat(ctx)?,
            data: self.data.resolve_repeat(ctx)?,
        })
    }
}

/// Compiled view scope state stored on a compiled mark.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledViewScope {
    pub spec: CompiledViewSpec,
    pub data: CompiledDataContext,
}

impl CompiledViewScope {
    pub fn from_view_scope_state(scope: &ViewScopeState) -> Self {
        let data = if let Some(store_data) = scope.data.store_data_ref() {
            CompiledDataContext::new_store_data_with_pattern_channels(
                store_data.clone(),
                scope.data.transforms().to_vec(),
                scope.data.channels().clone(),
                scope.data.pattern_channels().clone(),
            )
        } else {
            CompiledDataContext::new_with_pattern_channels(
                scope.data.dataframe().cloned(),
                scope.data.transforms().to_vec(),
                scope.data.channels().clone(),
                scope.data.pattern_channels().clone(),
            )
        };

        Self {
            spec: scope.spec.clone(),
            data,
        }
    }
}

/// Runtime expression handle passed into `mark.view(...)` closures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewRef {
    id: String,
}

impl ViewRef {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn x(&self) -> ViewAxisRef {
        ViewAxisRef::new(self.id.clone(), ViewAxis::X)
    }

    pub fn y(&self) -> ViewAxisRef {
        ViewAxisRef::new(self.id.clone(), ViewAxis::Y)
    }
}

/// Runtime expression handle for one view axis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewAxisRef {
    id: String,
    axis: ViewAxis,
}

impl ViewAxisRef {
    fn new(id: String, axis: ViewAxis) -> Self {
        Self { id, axis }
    }

    pub fn domain_start(&self) -> Expr {
        self.float_param("domain_start")
    }

    pub fn domain_end(&self) -> Expr {
        self.float_param("domain_end")
    }

    pub fn range_start(&self) -> Expr {
        self.float_param("range_start")
    }

    pub fn range_end(&self) -> Expr {
        self.float_param("range_end")
    }

    pub fn pixels(&self) -> Expr {
        Param::new(self.param_name("pixels"), ScalarValue::UInt32(Some(1))).expr()
    }

    fn float_param(&self, field: &str) -> Expr {
        Param::new(self.param_name(field), ScalarValue::Float64(Some(0.0))).expr()
    }

    pub fn param_name(&self, field: &str) -> String {
        format!(
            "__avenger_view_{}_{}_{}",
            self.id,
            self.axis.as_str(),
            field
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewAxis {
    X,
    Y,
}

impl ViewAxis {
    fn as_str(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(dead_code, unused_mut)]

    use super::*;
    use std::marker::PhantomData;

    use async_trait::async_trait;
    use datafusion::{dataframe::DataFrame, logical_expr::Expr, prelude::col};
    use serde::{Deserialize, Serialize};

    use crate::{
        ChannelValue, CompiledDataTransform, DataTransform, DataTransformCompileContext,
        DataTransformExecutionContext, DataTransformResult, MarkState, RepeatTypeHint,
        ResolvedRepeatVariable, impl_mark_base, row,
    };

    struct TestMark<C> {
        state: MarkState,
        _phantom: PhantomData<C>,
    }

    impl_mark_base!(TestMark);

    #[derive(Clone)]
    struct TestTransform {
        label: String,
    }

    impl TestTransform {
        fn new(label: impl Into<String>) -> Self {
            Self {
                label: label.into(),
            }
        }
    }

    impl DataTransform for TestTransform {
        type Output = ();

        fn into_compiled_and_output(
            self,
            _ctx: DataTransformCompileContext,
        ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
            Ok((Box::new(TestCompiledTransform { label: self.label }), ()))
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestCompiledTransform {
        label: String,
    }

    #[typetag::serde(name = "view_test_transform")]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl CompiledDataTransform for TestCompiledTransform {
        fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
            Box::new(self.clone())
        }

        async fn apply(
            &self,
            dataframe: DataFrame,
            _ctx: &DataTransformExecutionContext<'_>,
        ) -> Result<DataTransformResult, AvengerChartError> {
            Ok(DataTransformResult::dataframe(dataframe))
        }
    }

    fn placeholder_id(expr: Expr) -> String {
        match expr {
            Expr::Placeholder(placeholder) => placeholder.id,
            other => panic!("expected placeholder, got {other:?}"),
        }
    }

    fn compiled_policy(view: CartesianView) -> ViewAsyncPolicy {
        view.into_compiled_and_ref()
            .expect("compile view")
            .0
            .policy()
            .clone()
    }

    #[test]
    fn view_ref_uses_stable_reserved_placeholders() {
        let (_, view_ref) = View::cartesian()
            .id("pickup_density")
            .x_domain(col("pickup_x"))
            .y_domain(col("pickup_y"))
            .into_compiled_and_ref()
            .expect("compile view");

        assert_eq!(
            placeholder_id(view_ref.x().domain_start()),
            "$__avenger_view_pickup_density_x_domain_start"
        );
        assert_eq!(
            placeholder_id(view_ref.y().pixels()),
            "$__avenger_view_pickup_density_y_pixels"
        );
    }

    #[test]
    fn pixel_frame_view_is_scale_free_and_binary_stable() {
        let (spec, view_ref) = View::pixel_frame()
            .id("widget_part")
            .preview_cached(true)
            .into_compiled_and_ref()
            .expect("compile pixel-frame view");
        assert_eq!(view_ref.id(), "widget_part");
        assert!(matches!(spec, CompiledViewSpec::PixelFrame(_)));
        assert_eq!(spec.policy().stale_policy, ViewStalePolicy::RetargetCached);

        let decoded: CompiledViewSpec =
            bincode::deserialize(&bincode::serialize(&spec).unwrap()).unwrap();
        assert_eq!(decoded, spec);
    }

    #[test]
    fn preview_cached_sets_stale_policy_sugar() {
        let base = View::cartesian()
            .id("density")
            .x_domain(col("x"))
            .y_domain(col("y"));

        let default_policy = compiled_policy(base.clone());
        assert_eq!(default_policy.stale_policy, ViewStalePolicy::HideUntilReady);

        let preview_policy = compiled_policy(base.clone().preview_cached(true));
        assert_eq!(preview_policy.stale_policy, ViewStalePolicy::RetargetCached);

        let hidden_policy = compiled_policy(base.clone().preview_cached(false));
        assert_eq!(hidden_policy.stale_policy, ViewStalePolicy::HideUntilReady);

        let explicit_policy = compiled_policy(
            base.stale_policy(ViewStalePolicy::RetargetCached)
                .preview_cached(false),
        );
        assert_eq!(
            explicit_policy.stale_policy,
            ViewStalePolicy::HideUntilReady
        );
    }

    #[test]
    fn view_policy_serializes_only_stale_policy() {
        let (spec, _view_ref) = View::cartesian()
            .id("density")
            .x_domain(col("x"))
            .y_domain(col("y"))
            .stale_policy(ViewStalePolicy::RetargetCached)
            .into_compiled_and_ref()
            .expect("compile view");

        let json = serde_json::to_string(&spec).expect("serialize view spec");
        assert!(json.contains("\"kind\":\"cartesian\""));
        assert!(json.contains("stale_policy"));
        assert!(!json.contains("preview_cached"));

        let roundtrip: CompiledViewSpec =
            serde_json::from_str(&json).expect("deserialize view spec");
        assert_eq!(
            roundtrip.policy().stale_policy,
            ViewStalePolicy::RetargetCached
        );

        let binary = bincode::serialize(&spec).expect("serialize binary view spec");
        let binary_roundtrip: CompiledViewSpec =
            bincode::deserialize(&binary).expect("deserialize binary view spec");
        assert_eq!(binary_roundtrip, spec);
    }

    #[test]
    fn view_closure_transforms_land_in_view_data_context() {
        let mark = TestMark::<()>::new()
            .transform(TestTransform::new("base_before"), |mark, ()| mark)
            .view(
                View::cartesian()
                    .id("density")
                    .x_domain(col("x"))
                    .y_domain(col("y")),
                |mark, view| {
                    mark.transform(TestTransform::new("view_local"), |mark, ()| {
                        mark.with_channel_value("x", ChannelValue::from(view.x().domain_start()))
                    })
                },
            )
            .transform(TestTransform::new("base_after"), |mark, ()| {
                mark.with_channel_value("fill", ChannelValue::from(col("category")))
            });

        assert_eq!(mark.state.data.transforms().len(), 2);
        assert!(mark.state.data.channel("x").is_none());
        assert!(mark.state.data.channel("fill").is_some());

        let view = mark.state.view.as_ref().expect("view scope");
        assert_eq!(view.data.transforms().len(), 1);
        assert!(view.data.channel("x").is_some());
    }

    #[test]
    #[should_panic(expected = "Nested mark.view(...) scopes are not supported")]
    fn nested_view_scopes_are_rejected() {
        let _mark = TestMark::<()>::new().view(
            View::cartesian()
                .id("outer")
                .x_domain(col("x"))
                .y_domain(col("y")),
            |mark, _| {
                mark.view(
                    View::cartesian()
                        .id("inner")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |mark, _| mark,
                )
            },
        );
    }

    #[test]
    fn repeat_resolution_visits_view_spec_and_view_local_channels() {
        let mark = TestMark::<()>::new().view(
            View::cartesian()
                .id("repeated")
                .x_domain(crate::column())
                .y_domain(col("fixed_y")),
            |mark, _| mark.with_channel_value("x", ChannelValue::from(row())),
        );

        let repeat_context = RepeatContext::new()
            .with_row(
                ResolvedRepeatVariable {
                    id: "row_metric".to_string(),
                    expr: col("row_value"),
                    title: "Row metric".to_string(),
                    type_hint: Some(RepeatTypeHint::Quantitative),
                },
                0,
                1,
            )
            .with_column(
                ResolvedRepeatVariable {
                    id: "column_metric".to_string(),
                    expr: col("column_value"),
                    title: "Column metric".to_string(),
                    type_hint: Some(RepeatTypeHint::Quantitative),
                },
                0,
                1,
            );

        let resolved = mark
            .state
            .resolve_repeat(&repeat_context)
            .expect("resolve repeat");
        let view = resolved.view.expect("resolved view");
        let CompiledViewSpec::Cartesian(spec) = view.spec else {
            panic!("expected Cartesian view")
        };
        let session_context = datafusion::prelude::SessionContext::new();

        match spec
            .x_domain
            .to_default_expr(&session_context)
            .expect("x domain expr")
        {
            Expr::Column(column) => assert_eq!(column.name, "column_value"),
            other => panic!("expected column x domain, got {other:?}"),
        }

        let x_channel = view.data.channel("x").expect("view-local x channel");
        match x_channel.expr(&session_context).expect("x channel expr") {
            Expr::Column(column) => assert_eq!(column.name, "row_value"),
            other => panic!("expected column x channel, got {other:?}"),
        }
    }
}
