use crate::guide::{CompiledGuide, CoordinateGuide, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetRowGuide {
    // Collected facet sources from compiled marks (populated via set_compiled_marks)
    #[serde(skip)]
    facet_sources: Vec<FacetSource>,
}

#[derive(Clone)]
struct FacetSource {
    subplot: std::sync::Arc<crate::plot::CompiledPlot>,
    data: crate::marks::CompiledDataContext,
}

impl CoordinateGuide for FacetRowGuide {
    type Axis = crate::cartesian::axis::CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<std::sync::Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        self.facet_sources.clear();
        for m in compiled_marks {
            if let Some(facet) = m.as_any().downcast_ref::<crate::facet::marks::facet::CompiledFacetRow>() {
                self.facet_sources.push(FacetSource {
                    subplot: facet.compiled_subplot.clone(),
                    data: facet.state.data.clone(),
                });
            }
        }
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetRowGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        _theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, crate::error::AvengerChartError> {
        use crate::scales::ConfiguredScaleLegendExt;
        use datafusion::logical_expr::lit;

        // Need 'row' scale
        let row_scale = scales
            .get("row")
            .ok_or_else(|| crate::error::AvengerChartError::InternalError("Missing 'row' scale for FacetRowGuide".into()))?;

        // Extract discrete domain order
        let domain_vals = match row_scale.domain_values()? {
            crate::scales::extensions::DomainValues::Discrete(vals) => vals,
            _ => vec![],
        };

        let mut max_left: f32 = 0.0;
        let mut max_right: f32 = 0.0;
        let mut top: f32 = 0.0;
        let mut bottom: f32 = 0.0;

        // For each facet source (there could be more than one Facet mark)
        for source in &self.facet_sources {
            // Row expression from channels
            let row_expr = source
                .data
                .channels()
                .get("row")
                .and_then(|cv| cv.expr(ctx))
                .ok_or_else(|| crate::error::AvengerChartError::InternalError("Facet 'row' channel not found in guide".into()))?;

            // DataFrame for this facet source
            let df = source
                .data
                .dataframe_with_context(ctx)
                .ok_or_else(|| crate::error::AvengerChartError::InternalError("Facet guide could not access data".into()))?;

            for (i, facet_val) in domain_vals.iter().enumerate() {
                let filter_df = df.clone().filter(row_expr.clone().eq(lit(facet_val.clone())))?;
                // Build inner scales and measure inner guide overflow for this band height
                let inner_scales = source
                    .subplot
                    .build_scales_for_dataframe(&filter_df, plot_width, plot_height / domain_vals.len() as f32, ctx, params)
                    .await?;
                let overflow = source
                    .subplot
                    .measure_guide_overflow_with_scales(&inner_scales, plot_width, plot_height / domain_vals.len() as f32, ctx, params)
                    .await?;

                if i == 0 {
                    top = top.max(overflow.top);
                }
                if i == domain_vals.len() - 1 {
                    bottom = bottom.max(overflow.bottom);
                }
                max_left = max_left.max(overflow.left);
                max_right = max_right.max(overflow.right);
            }
        }

        Ok(OverflowSpaceRequirement {
            top,
            bottom,
            left: max_left,
            right: max_right,
        })
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _theme: &crate::theme::Theme,
        _params: &IndexMap<String, datafusion::common::ScalarValue>,
        _ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, crate::error::AvengerChartError> {
        Ok(vec![])
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        avenger_scenegraph::marks::group::Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }
}
