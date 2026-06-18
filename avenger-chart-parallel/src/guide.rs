use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    GuideSharingContext, LayoutBounds, OverflowSpaceRequirement, Theme,
    serialization::DefaultLogicalExprNodeExt,
};
use avenger_color::ColorOrGradient;
use avenger_common::{types::StrokeCap, value::ScalarOrArray};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    group::Clip, mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark,
};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ParallelAxis;

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
        _scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let axes = ordered_axes(&self.axes);
        if axes.is_empty() {
            return Ok(Vec::new());
        }
        let count = axes.len();
        let step = if count > 1 {
            plot_width / (count.saturating_sub(1) as f32)
        } else {
            0.0
        };
        let xs = (0..count)
            .map(|index| {
                if count <= 1 {
                    plot_width / 2.0
                } else {
                    index as f32 * step
                }
            })
            .collect::<Vec<_>>();
        let titles = axes
            .iter()
            .map(|(_, axis)| axis_title(axis, ctx))
            .collect::<Vec<_>>();

        let axis_rules = SceneRuleMark {
            name: "parallel_axis_rule".to_string(),
            interactive: false,
            clip: true,
            len: count as u32,
            gradients: Vec::new(),
            stroke_dash: None,
            x: ScalarOrArray::from(xs.clone()),
            y: ScalarOrArray::new_scalar(0.0),
            x2: ScalarOrArray::from(xs.clone()),
            y2: ScalarOrArray::new_scalar(plot_height),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.18, 0.18, 0.18, 1.0])),
            stroke_width: ScalarOrArray::new_scalar(1.0),
            stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Butt),
            indices: None,
            zindex: Some(1),
        };

        let title_mark = SceneTextMark {
            name: "parallel_axis_title".to_string(),
            interactive: true,
            clip: false,
            len: count as u32,
            text: ScalarOrArray::from(titles),
            x: ScalarOrArray::from(xs),
            y: ScalarOrArray::new_scalar(-12.0),
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
            zindex: Some(2),
        };

        Ok(vec![
            SceneMark::Rule(axis_rules),
            SceneMark::from(title_mark),
        ])
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
}
