use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    GuideSharingContext, LayoutBounds, OverflowSpaceRequirement, Theme,
    serialization::DefaultLogicalExprNodeExt,
};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
    point::make_point_axis_marks,
};
use avenger_scales::scales::{ConfiguredScale, DomainKind, band::BandScale};
use avenger_scenegraph::marks::{
    group::Clip, mark::SceneMark, rect::SceneRectMark, text::SceneTextMark,
};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ParallelAxis;
use crate::frame::{
    ParallelFrameDimension, resolve_display_state, resolve_order_state,
    resolve_parallel_frame_dimensions,
};

/// Parallel-coordinate guide configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParallelGuide {
    pub axes: HashMap<String, ParallelAxis>,
}

impl CoordinateGuide for ParallelGuide {
    type Axis = ParallelAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn set_compiled_marks<M>(
        &mut self,
        _compiled_marks: &[Arc<M>],
        _session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, other: Self) {
        for (channel, axis) in other.axes {
            match self.axes.get_mut(&channel) {
                Some(existing) => *existing = existing.clone().update(axis),
                None => {
                    self.axes.insert(channel, axis);
                }
            }
        }
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(CompiledParallelGuide { axes: self.axes })
    }
}

/// Compiled guide for parallel-coordinate axes and dimension headers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompiledParallelGuide {
    pub axes: HashMap<String, ParallelAxis>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParallelAxisGuideDatum {
    pub dimension_id: String,
    pub scale_name: String,
    pub title: String,
    pub order_index: usize,
    pub equilibrium_x: f32,
    pub display_x: f32,
    pub displacement_px: f32,
    pub displacement_slots: f32,
}

impl CompiledParallelGuide {
    pub fn axis_guide_datums(
        &self,
        plot_width: f32,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<ParallelAxisGuideDatum>, AvengerChartError> {
        let axes = ordered_axes(&self.axes);
        if axes.is_empty() {
            return Ok(Vec::new());
        }

        let base_order = axes
            .iter()
            .map(|(channel, axis)| {
                axis.dimension_id
                    .clone()
                    .unwrap_or_else(|| (*channel).to_string())
            })
            .collect::<Vec<_>>();
        let dimensions = axes
            .iter()
            .map(|(channel, axis)| {
                let dimension_id = axis
                    .dimension_id
                    .clone()
                    .unwrap_or_else(|| (*channel).to_string());
                ParallelFrameDimension::new(dimension_id.clone(), (*channel).clone(), dimension_id)
            })
            .collect::<Vec<_>>();
        let order_state = axes.iter().find_map(|(_, axis)| axis.order_state.as_ref());
        let display_state = axes
            .iter()
            .find_map(|(_, axis)| axis.display_state.as_ref());
        let param_order = resolve_order_state(order_state, params, &base_order)?;
        let display_overrides = resolve_display_state(display_state, params, &base_order)?;
        let frame = resolve_parallel_frame_dimensions(
            &dimensions,
            param_order.as_deref().or(Some(base_order.as_slice())),
            display_overrides.as_ref(),
            plot_width,
        );
        let axis_by_id = axes
            .into_iter()
            .map(|(channel, axis)| {
                let dimension_id = axis
                    .dimension_id
                    .clone()
                    .unwrap_or_else(|| channel.to_string());
                (dimension_id, axis)
            })
            .collect::<HashMap<_, _>>();
        frame
            .slots
            .into_iter()
            .map(|slot| {
                let axis = axis_by_id.get(&slot.id).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing parallel axis metadata for dimension '{}'",
                        slot.id
                    ))
                })?;
                Ok(ParallelAxisGuideDatum {
                    scale_name: slot.scale_name,
                    title: axis_title(axis, ctx),
                    dimension_id: slot.id,
                    order_index: slot.equilibrium_index,
                    equilibrium_x: slot.equilibrium_x,
                    display_x: slot.display_x,
                    displacement_px: slot.displacement_px,
                    displacement_slots: slot.displacement_slots,
                })
            })
            .collect()
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for CompiledParallelGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement {
            top: if self.axes.is_empty() { 0.0 } else { 34.0 },
            ..OverflowSpaceRequirement::default()
        })
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let datums = self.axis_guide_datums(plot_width, params, ctx)?;
        if datums.is_empty() {
            return Ok(Vec::new());
        }
        let count = datums.len();
        let xs = datums
            .iter()
            .map(|datum| datum.display_x)
            .collect::<Vec<_>>();
        let titles = datums
            .iter()
            .map(|datum| datum.title.clone())
            .collect::<Vec<_>>();

        let mut marks = Vec::with_capacity(count + 2);
        for datum in &datums {
            let scale = scales.get(&datum.scale_name).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing configured scale for parallel axis '{}'",
                    datum.scale_name
                ))
            })?;
            marks.push(make_axis_mark(
                scale,
                plot_bounds.x + datum.display_x,
                plot_bounds.y,
                plot_width,
                plot_height,
                theme,
                params,
            )?);
        }

        let hit_half_width = 54.0_f32;
        let title_hit_rect = SceneRectMark {
            name: "parallel_axis_title_hit".to_string(),
            interactive: true,
            clip: false,
            len: count as u32,
            gradients: Vec::new(),
            x: ScalarOrArray::from(
                xs.iter()
                    .map(|x| plot_bounds.x + *x - hit_half_width)
                    .collect::<Vec<_>>(),
            ),
            y: ScalarOrArray::new_scalar(plot_bounds.y - 34.0),
            width: None,
            height: None,
            x2: Some(ScalarOrArray::from(
                xs.iter()
                    .map(|x| plot_bounds.x + *x + hit_half_width)
                    .collect::<Vec<_>>(),
            )),
            y2: Some(ScalarOrArray::new_scalar(plot_bounds.y + 2.0)),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: Some(3),
        };

        let title_mark = SceneTextMark {
            name: "parallel_axis_title".to_string(),
            interactive: true,
            clip: false,
            len: count as u32,
            text: ScalarOrArray::from(titles),
            x: ScalarOrArray::from(xs.iter().map(|x| plot_bounds.x + *x).collect::<Vec<_>>()),
            y: ScalarOrArray::new_scalar(plot_bounds.y - 12.0),
            align: ScalarOrArray::new_scalar(TextAlign::Center),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Bottom),
            angle: ScalarOrArray::new_scalar(0.0),
            color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.12, 0.12, 0.12, 1.0])),
            font: ScalarOrArray::new_scalar("sans-serif".to_string()),
            font_size: ScalarOrArray::new_scalar(12.0),
            font_weight: ScalarOrArray::new_scalar(FontWeight::Name(FontWeightNameSpec::Normal)),
            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
            limit: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: Some(4),
        };

        marks.push(SceneMark::Rect(title_hit_rect));
        marks.push(SceneMark::from(title_mark));
        Ok(marks)
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn make_axis_mark(
    scale: &ConfiguredScale,
    display_x: f32,
    display_y: f32,
    plot_width: f32,
    plot_height: f32,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> Result<SceneMark, AvengerChartError> {
    let axis_config = parallel_axis_config(plot_width, plot_height, theme, params);
    let mut group = match scale.scale_impl.domain_kind() {
        DomainKind::Categorical => match scale.scale_impl.scale_type() {
            "band" => make_band_axis_marks(scale, "", [display_x, display_y], &axis_config)?,
            "point" => {
                make_point_axis_marks(scale.clone(), "", [display_x, display_y], &axis_config)?
            }
            "ordinal" => {
                let band_scale = BandScale::from_point_scale(scale);
                make_band_axis_marks(&band_scale, "", [display_x, display_y], &axis_config)?
            }
            scale_type => {
                return Err(AvengerChartError::InternalError(format!(
                    "Unsupported parallel categorical axis scale type '{scale_type}'"
                )));
            }
        },
        DomainKind::NestedCategorical => {
            return Err(AvengerChartError::InternalError(
                "Nested categorical scales are not supported on parallel axes".to_string(),
            ));
        }
        DomainKind::Numeric | DomainKind::Temporal => {
            make_numeric_axis_marks(scale, "", [display_x, display_y], &axis_config)?
        }
    };
    group.name = "parallel_axis".to_string();
    Ok(SceneMark::Group(group))
}

fn parallel_axis_config(
    plot_width: f32,
    plot_height: f32,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> AxisConfig {
    let axis_ctx =
        theme.axis_context_with_params(Some("parallel"), Some("dimension"), params.clone());
    let label_ctx = axis_ctx.child("label");

    AxisConfig {
        orientation: AxisOrientation::Left,
        dimensions: [plot_width, plot_height],
        grid: false,
        domain_color: theme.stroke_color(&axis_ctx.child("domain")),
        tick_color: theme.stroke_color(&axis_ctx.child("tick")),
        label_color: theme.text_color(&label_ctx),
        tick_length: theme.axis_tick_length(&axis_ctx),
        label_font_size: theme.font_size(&label_ctx),
        label_font_weight: theme.font_weight(&label_ctx),
        label_font_family: theme.font_family(&label_ctx),
        title_visible: Some(false),
        ..AxisConfig::default()
    }
}

fn ordered_axes(axes: &HashMap<String, ParallelAxis>) -> Vec<(&String, &ParallelAxis)> {
    let mut axes = axes.iter().collect::<Vec<_>>();
    axes.sort_by_key(|(channel, axis)| {
        (
            axis.order_index.unwrap_or(usize::MAX),
            axis.dimension_id
                .as_deref()
                .unwrap_or(channel.as_str())
                .to_string(),
        )
    });
    axes
}

fn axis_title(axis: &ParallelAxis, ctx: &SessionContext) -> String {
    if let Some(title) = axis.title.as_option().and_then(|title| title.as_ref())
        && let Ok(expr) = title.to_default_expr(ctx)
    {
        return match expr {
            Expr::Literal(ScalarValue::Utf8(Some(value)), _)
            | Expr::Literal(ScalarValue::LargeUtf8(Some(value)), _) => value,
            other => other.to_string(),
        };
    }
    axis.dimension_id
        .clone()
        .unwrap_or_else(|| "dimension".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use avenger_chart_core::{
        AxisGuideVisibilityConfig, AxisPosition, AxisVisibility, ChildFrameGuideSharingView,
        CoordinationAxis, EmptyCoordMeasurement, FacetGuideSharingView, SharingLevel,
        guide_sharing::AxisOwnershipMode,
    };
    use avenger_common::value::ScalarOrArrayValue;
    use avenger_scales::scales::{linear::LinearScale, point::PointScale};
    use datafusion::arrow::array::StringArray;

    #[test]
    fn ordered_axes_use_parallel_dimension_order() {
        let mut axes = HashMap::new();
        axes.insert(
            "generated_b".to_string(),
            ParallelAxis::new().with_dimension_metadata("b", 1),
        );
        axes.insert(
            "generated_a".to_string(),
            ParallelAxis::new().with_dimension_metadata("a", 0),
        );

        let ordered_ids = ordered_axes(&axes)
            .into_iter()
            .map(|(_, axis)| axis.dimension_id.as_deref().unwrap().to_string())
            .collect::<Vec<_>>();

        assert_eq!(ordered_ids, vec!["a", "b"]);
    }

    #[test]
    fn axis_title_uses_configured_title_then_dimension_id() {
        let ctx = SessionContext::new();
        let titled = ParallelAxis::new()
            .title("Miles Per Gallon")
            .with_dimension_metadata("mpg", 0);
        assert_eq!(axis_title(&titled, &ctx), "Miles Per Gallon");

        let defaulted = ParallelAxis::new().with_dimension_metadata("mpg", 0);
        assert_eq!(axis_title(&defaulted, &ctx), "mpg");
    }

    #[test]
    fn axis_guide_datums_include_title_and_positions() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1),
        );
        let guide = CompiledParallelGuide { axes };
        let datums = guide
            .axis_guide_datums(300.0, &IndexMap::new(), &ctx)
            .expect("guide datums");

        assert_eq!(datums.len(), 2);
        assert_eq!(datums[0].dimension_id, "speed");
        assert_eq!(datums[0].scale_name, "speed");
        assert_eq!(datums[0].title, "Speed");
        assert_eq!(datums[0].equilibrium_x, 0.0);
        assert_eq!(datums[0].display_x, 0.0);
        assert_eq!(datums[1].dimension_id, "cost");
        assert_eq!(datums[1].equilibrium_x, 300.0);
    }

    #[test]
    fn axis_guide_datums_use_order_and_display_state_params() {
        let ctx = SessionContext::new();
        let order_state = crate::ParallelOrderState::param("axis_order");
        let display_state =
            crate::ParallelDisplayState::active_axis("drag_dimension", "drag_display_x");
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0)
                .with_frame_state(Some(order_state.clone()), Some(display_state.clone())),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1)
                .with_frame_state(Some(order_state), Some(display_state)),
        );
        let guide = CompiledParallelGuide { axes };
        let params = IndexMap::from([
            (
                "axis_order".to_string(),
                ScalarValue::List(ScalarValue::new_list(
                    &[
                        ScalarValue::Utf8(Some("cost".to_string())),
                        ScalarValue::Utf8(Some("speed".to_string())),
                    ],
                    &datafusion::arrow::datatypes::DataType::Utf8,
                    true,
                )),
            ),
            (
                "drag_dimension".to_string(),
                ScalarValue::Utf8(Some("speed".to_string())),
            ),
            (
                "drag_display_x".to_string(),
                ScalarValue::Float64(Some(210.0)),
            ),
        ]);
        let datums = guide
            .axis_guide_datums(300.0, &params, &ctx)
            .expect("guide datums");

        assert_eq!(
            datums
                .iter()
                .map(|datum| datum.dimension_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cost", "speed"]
        );
        assert_eq!(datums[0].equilibrium_x, 0.0);
        assert_eq!(datums[0].display_x, 0.0);
        assert_eq!(datums[1].equilibrium_x, 300.0);
        assert_eq!(datums[1].display_x, 210.0);
        assert_eq!(datums[1].displacement_px, -90.0);
        assert_eq!(datums[1].displacement_slots, -0.3);
    }

    #[test]
    fn guide_renders_axis_title_hit_rects() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let scales = HashMap::from([(
            "speed".to_string(),
            LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
        )]);
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
        ))
        .expect("evaluate guide");

        assert!(matches!(&marks[0], SceneMark::Group(group) if group.name == "parallel_axis"));
        assert!(
            matches!(&marks[1], SceneMark::Rect(rect) if rect.name == "parallel_axis_title_hit" && rect.interactive)
        );
        assert!(
            matches!(&marks[2], SceneMark::Text(text) if text.name == "parallel_axis_title" && text.interactive)
        );
    }

    #[test]
    fn guide_marks_are_offset_by_plot_bounds() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let scales = HashMap::from([
            (
                "speed".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
            (
                "cost".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
        ]);
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 25.0,
                y: 40.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
        ))
        .expect("evaluate guide");

        match &marks[0] {
            SceneMark::Group(group) => assert_eq!(group.origin, [25.0, 40.0]),
            _ => panic!("expected first parallel axis group"),
        }
        let rect = match &marks[2] {
            SceneMark::Rect(rect) => rect,
            _ => panic!("expected title hit rect"),
        };
        match rect.x.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[-29.0, 271.0]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected title hit rect x array"),
        }
        match rect.y.value() {
            ScalarOrArrayValue::Scalar(value) => assert_eq!(*value, 6.0),
            ScalarOrArrayValue::Array(_) => panic!("expected title hit rect scalar y"),
        }
    }

    #[test]
    fn guide_dispatches_numeric_and_point_axes() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_origin".to_string(),
            ParallelAxis::new()
                .title("Origin")
                .with_dimension_metadata("origin", 1),
        );
        let guide = CompiledParallelGuide { axes };
        let scales = HashMap::from([
            (
                "speed".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
            (
                "origin".to_string(),
                PointScale::configured(
                    Arc::new(StringArray::from(vec!["EU", "JP", "US"])),
                    (200.0, 0.0),
                ),
            ),
        ]);
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
        ))
        .expect("evaluate guide");

        assert_eq!(marks.len(), 4);
        assert!(matches!(&marks[0], SceneMark::Group(group) if group.name == "parallel_axis"));
        assert!(matches!(&marks[1], SceneMark::Group(group) if group.name == "parallel_axis"));
        assert!(
            matches!(&marks[2], SceneMark::Rect(rect) if rect.name == "parallel_axis_title_hit")
        );
        assert!(matches!(&marks[3], SceneMark::Text(text) if text.name == "parallel_axis_title"));
    }

    struct TestFacetGuideSharingView;

    impl FacetGuideSharingView for TestFacetGuideSharingView {
        fn channel_axis_visibility_for_path_checked(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
        ) -> Option<AxisVisibility> {
            None
        }

        fn channel_axis_visibility_for_path_checked_with_mode(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
            _ownership_mode: AxisOwnershipMode,
        ) -> Option<AxisVisibility> {
            None
        }

        fn is_jagged_for_axis(&self, _axis_position: AxisPosition) -> bool {
            false
        }

        fn channel_domain_sharing_level(&self, _channel: &str) -> SharingLevel {
            SharingLevel::FREE
        }

        fn effective_edge_indices_for_values_at_path(
            &self,
            _facet_path: &[ScalarValue],
            _values: &[ScalarValue],
        ) -> Option<(usize, usize)> {
            None
        }
    }

    struct TestChildFrameGuideSharingView;

    impl ChildFrameGuideSharingView for TestChildFrameGuideSharingView {
        fn position_indices(&self) -> Vec<usize> {
            Vec::new()
        }

        fn level_counts(&self) -> Vec<usize> {
            Vec::new()
        }

        fn level_axes(&self) -> Vec<CoordinationAxis> {
            Vec::new()
        }

        fn axis_guide_visibility_config_for_axis(
            &self,
            _axis: CoordinationAxis,
        ) -> AxisGuideVisibilityConfig {
            AxisGuideVisibilityConfig::auto()
        }
    }
}
