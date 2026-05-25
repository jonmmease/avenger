use std::{any::Any, collections::HashMap, future::Future, sync::Arc};

use avenger_chart::plot::{CompiledPlot, Plot};
use avenger_chart_cartesian::{
    Cartesian, CartesianSubplotPositionChannels, CARTESIAN_SUBPLOT_X_CHANNEL,
    CARTESIAN_SUBPLOT_Y_CHANNEL,
};
use avenger_chart_core::{
    compile_positioned_subplot_mark, AvengerChartError, CompiledGuide, CompiledMark,
    CompiledMarkCore, CompiledMarkState, CompiledPositionedSubplot, CoordMeasurement,
    CoordinateGuide, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, GuideSharingContext, GuideUpdate, LayoutBounds, Mark,
    OverflowSpaceRequirement, PlotAreaRangeEndpoint, PlotGeometry, PositionedSubplotChannel,
    PositionedSubplotSpec, ScaleRangeBinding, SubplotContainerCoordinateSystem, SubplotGeometry,
    SubplotMarkCore, SubplotRect, Theme, ZeroDCoord,
};
use avenger_chart_external_test::external_subplot_coord::{
    ExternalSubplotCoord, ExternalSubplotPositionChannels,
};
use avenger_chart_marks::Subplot;
use avenger_chart_polar::{Polar, PolarSubplotPositionChannels};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{
    arrow::{array::Float32Array, record_batch::RecordBatch},
    common::ScalarValue,
    dataframe::DataFrame,
    prelude::{col, lit, SessionContext},
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

fn run_with_large_stack<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name("external-subplot-coord-large-stack".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime for external subplot coord test");
            rt.block_on(f());
        })
        .expect("spawn large-stack external subplot coord test thread")
        .join()
        .expect("large-stack external subplot coord test panicked");
}

fn external_position_data(ctx: &SessionContext) -> DataFrame {
    ctx.read_batch(
        RecordBatch::try_from_iter(vec![
            (
                "subplot_u",
                Arc::new(Float32Array::from(vec![10.0]))
                    as Arc<dyn datafusion::arrow::array::Array>,
            ),
            (
                "subplot_v",
                Arc::new(Float32Array::from(vec![20.0]))
                    as Arc<dyn datafusion::arrow::array::Array>,
            ),
        ])
        .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn external_coordinate_can_compile_subplot_mark() {
    let ctx = SessionContext::new();
    let child = Plot::<ZeroDCoord>::new();
    let subplot = Subplot::<ExternalSubplotCoord>::new(child)
        .subplot_u(lit(10.0))
        .subplot_v(lit(20.0))
        .label("child label")
        .key("child-key");

    let compiled = subplot.compile_untransformed(&ctx).await.unwrap();

    assert_eq!(compiled.mark_type(), "subplot");
    assert!(compiled.as_positioned_subplot().is_some());
    let compiled = compiled
        .as_any()
        .downcast_ref::<CompiledPositionedSubplot>()
        .unwrap();
    assert_eq!(compiled.payload().label(), Some("child label"));
    assert_eq!(compiled.payload().key(), Some("child-key"));
    assert!(compiled.payload().inherits_parent_data());
    let child = compiled
        .payload()
        .compiled_child_plot()
        .as_any()
        .downcast_ref::<CompiledPlot>()
        .unwrap();
    assert_eq!(child.marks().len(), 0);
}

#[test]
fn external_coordinate_subplot_can_be_added_to_plot() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let plot = Plot::<ExternalSubplotCoord>::new()
            .data(external_position_data(&ctx))
            .mark(
                Subplot::new(Plot::<ZeroDCoord>::new())
                    .subplot_u(col("subplot_u"))
                    .subplot_v(col("subplot_v"))
                    .plot_size(40.0, 30.0),
            );

        let compiled = plot.compile(&ctx).await.unwrap();

        assert_eq!(compiled.marks().len(), 1);
        assert_eq!(compiled.marks()[0].mark_type(), "subplot");
        assert!(compiled.marks()[0].as_positioned_subplot().is_some());
        compiled.evaluate(&ctx, None).await.unwrap();
    });
}

#[tokio::test]
async fn generic_helper_validates_partitioned_subplot_constraints() {
    let ctx = SessionContext::new();
    let child_df = ctx
        .read_batch(
            RecordBatch::try_from_iter(vec![(
                "x",
                Arc::new(datafusion::arrow::array::Float32Array::from(vec![1.0]))
                    as Arc<dyn datafusion::arrow::array::Array>,
            )])
            .unwrap(),
        )
        .unwrap();
    let subplot = Subplot::<ExternalSubplotCoord>::new(Plot::<ZeroDCoord>::new().data(child_df))
        .subplot_u(lit(10.0))
        .subplot_v(lit(20.0))
        .partition_by(col("group"));

    let err = match subplot.compile_untransformed(&ctx).await {
        Ok(_) => panic!("partitioned subplot with child plot data should fail"),
        Err(err) => err,
    };

    assert!(
        err.to_string()
            .contains("Partitioned ExternalSubplotCoord subplots inherit parent data"),
        "{err}"
    );
}

#[tokio::test]
async fn cartesian_subplot_mapping_uses_distinct_subplot_channels() {
    let ctx = SessionContext::new();
    let subplot = Subplot::<Cartesian>::new(Plot::<ZeroDCoord>::new())
        .subplot_x(lit(10.0))
        .subplot_y(lit(20.0));

    let compiled = subplot.compile_untransformed(&ctx).await.unwrap();
    let positioned = compiled.as_positioned_subplot().unwrap();

    let mappings = positioned
        .spec()
        .placement_channels
        .iter()
        .map(|channel| (channel.channel.as_str(), channel.transform_channel.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        mappings,
        vec![
            (CARTESIAN_SUBPLOT_X_CHANNEL, "x"),
            (CARTESIAN_SUBPLOT_Y_CHANNEL, "y"),
        ]
    );
}

#[tokio::test]
async fn polar_subplot_mapping_uses_polar_channels() {
    let ctx = SessionContext::new();
    let subplot = Subplot::<Polar>::new(Plot::<ZeroDCoord>::new())
        .r(lit(10.0))
        .theta(lit(20.0));

    let compiled = subplot.compile_untransformed(&ctx).await.unwrap();
    let positioned = compiled.as_positioned_subplot().unwrap();

    let mappings = positioned
        .spec()
        .placement_channels
        .iter()
        .map(|channel| (channel.channel.as_str(), channel.transform_channel.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(mappings, vec![("r", "r"), ("theta", "theta")]);
}

#[tokio::test]
async fn non_point_positioned_subplot_transform_returns_clear_error() {
    let ctx = SessionContext::new();
    let plot = Plot::<NonPointSubplotCoord>::new()
        .data(external_position_data(&ctx))
        .mark(
            Subplot::new(Plot::<ZeroDCoord>::new())
                .with_channel_value("subplot_u", col("subplot_u").into())
                .with_channel_value("subplot_v", col("subplot_v").into()),
        );
    let compiled = plot.compile(&ctx).await.unwrap();

    let err = match compiled.evaluate(&ctx, None).await {
        Ok(_) => panic!("non-point positioned subplot transform should fail"),
        Err(err) => err,
    };

    assert!(
        err.to_string()
            .contains("NonPointSubplotCoord subplots require the parent coordinate transform to produce PointGeometry"),
        "{err}"
    );
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NonPointSubplotCoord;

impl CoordinateSystemCore for NonPointSubplotCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for NonPointSubplotCoord {
    type Guide = NonPointSubplotCoordGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(NonPointSubplotCoordTransform)
    }
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for NonPointSubplotCoord {
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        compile_positioned_subplot_mark(
            subplot,
            compiled_state,
            session_context,
            PositionedSubplotSpec::new(
                "NonPointSubplotCoord",
                "non_point_subplot",
                vec![
                    PositionedSubplotChannel::new("subplot_u", "subplot_u"),
                    PositionedSubplotChannel::new("subplot_v", "subplot_v"),
                ],
            ),
        )
        .await
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NonPointSubplotCoordTransform;

impl CoordinateSystemTransformCore for NonPointSubplotCoordTransform {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        Ok(Box::new(SubplotGeometry::new(vec![SubplotRect::default()])))
    }

    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        match _channel {
            "subplot_u" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::WIDTH,
            )),
            "subplot_v" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::HEIGHT,
                PlotAreaRangeEndpoint::ZERO,
            )),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn avenger_scales::scales::ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for NonPointSubplotCoordTransform {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct NonPointSubplotCoordGuide;

impl GuideUpdate for NonPointSubplotCoordGuide {
    fn update(self, _other: Self) -> Self {
        self
    }
}

impl CoordinateGuide for NonPointSubplotCoordGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks<M>(
        &mut self,
        _compiled_marks: &[Arc<M>],
        _session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for NonPointSubplotCoordGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(Default::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Clip {
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
