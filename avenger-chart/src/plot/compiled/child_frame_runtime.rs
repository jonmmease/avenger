//! Runtime helpers for measuring child plots as chart frames.
//!
//! Container coordinate systems decide which child frames exist and where they
//! should be placed. This module owns the common "measure this child plot as a
//! frame" work that those containers should not duplicate.

use std::collections::HashMap;

use datafusion::{common::ScalarValue, dataframe::DataFrame};

use avenger_chart_core::{EvaluationContext as CoreEvaluationContext, SharingLevel};

use crate::{
    error::AvengerChartError,
    layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
    render::EvaluationContext,
    scales::{DomainExtent, ScaleBuilder},
};

use super::{
    ChildFrameChannelDomainExtent, ChildFrameSharingLevel, CompiledPlot, ComponentsMeasurement,
    child_frame_domain_sharing_levels_for_plot, extract_child_frame_shared_domain_extents,
    scale_provider::DynamicScaleProvider, scales::build_scale_builder_from_marks,
};

/// How a child frame should source data when the container measures it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildFrameDataSelection {
    /// The child plot has its own plot-level data.
    ExplicitChild,
    /// The child plot should inherit the container's data.
    InheritParent,
}

/// Service for measuring child plots as chart frames inside container
/// coordinate systems.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ChildFrameRuntime;

impl ChildFrameRuntime {
    pub(crate) fn new() -> Self {
        Self
    }

    /// Fixed plot-area layout for a child plot measured inside a container.
    pub(crate) fn fixed_plot_area_layout_spec(
        &self,
        width: f32,
        height: f32,
    ) -> EvaluatedLayoutSpec {
        EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Auto,
            plot_area: EvaluatedSizeMode::Fixed {
                width: width.max(1.0),
                height: height.max(1.0),
            },
            margins: EvaluatedMargins {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        }
    }

    /// Build the child evaluation context for a container child frame.
    pub(crate) fn eval_context(
        &self,
        eval_ctx: &EvaluationContext,
        sharing_level: ChildFrameSharingLevel,
    ) -> EvaluationContext {
        eval_ctx.with_child_frame_sharing_level_appended(sharing_level)
    }

    /// Build the scale and domain-sharing state needed to measure a child plot.
    pub(crate) async fn prepare_plot<'a>(
        &self,
        plot: &'a CompiledPlot,
        data_selection: ChildFrameDataSelection,
        inherited_data: Option<&DataFrame>,
        eval_ctx: &CoreEvaluationContext,
    ) -> Result<PreparedChildFramePlot<'a>, AvengerChartError> {
        let data_override = match data_selection {
            ChildFrameDataSelection::ExplicitChild => None,
            ChildFrameDataSelection::InheritParent => inherited_data.cloned(),
        };
        let scale_builder = Box::pin(build_scale_builder_from_marks(
            &plot.marks,
            &plot.scale_specs,
            &plot.coord_transform,
            &plot.data,
            data_override.clone(),
            eval_ctx,
            plot.get_theme().as_ref(),
        ))
        .await?;

        let channel_domain_sharing_levels = child_frame_domain_sharing_levels_for_plot(plot);
        let local_domain_extents = extract_child_frame_shared_domain_extents(
            &scale_builder,
            &channel_domain_sharing_levels,
        );

        Ok(PreparedChildFramePlot {
            plot,
            data_override,
            scale_builder,
            local_domain_extents,
            channel_domain_sharing_levels,
        })
    }

    /// Measure a child plot with a caller-provided scale builder.
    pub(crate) async fn measure_with_builder(
        &self,
        plot: &CompiledPlot,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_builder: &ScaleBuilder,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        domain_extents: &[&HashMap<String, DomainExtent>],
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        let extended_builder;
        let scale_builder = if domain_extents.iter().all(|extents| extents.is_empty()) {
            scale_builder
        } else {
            extended_builder = {
                let mut builder = scale_builder.clone();
                for extents in domain_extents {
                    if !extents.is_empty() {
                        builder.extend_with_domain_extents(extents);
                    }
                }
                builder
            };
            &extended_builder
        };
        let scale_provider = DynamicScaleProvider {
            builder: scale_builder,
            plot,
        };
        Box::pin(plot.measure_plot_components(
            eval_ctx,
            layout_spec,
            &scale_provider,
            data_override,
            facet_path,
        ))
        .await
    }
}

/// Prepared measurement state for one child plot.
pub(crate) struct PreparedChildFramePlot<'a> {
    plot: &'a CompiledPlot,
    data_override: Option<DataFrame>,
    scale_builder: ScaleBuilder,
    local_domain_extents: HashMap<String, ChildFrameChannelDomainExtent>,
    channel_domain_sharing_levels: HashMap<String, SharingLevel>,
}

impl<'a> PreparedChildFramePlot<'a> {
    pub(crate) fn local_domain_extents(&self) -> &HashMap<String, ChildFrameChannelDomainExtent> {
        &self.local_domain_extents
    }

    pub(crate) fn channel_domain_sharing_levels(&self) -> &HashMap<String, SharingLevel> {
        &self.channel_domain_sharing_levels
    }

    pub(crate) fn data_override(&self) -> Option<&DataFrame> {
        self.data_override.as_ref()
    }

    pub(crate) async fn measure(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        facet_path: &[ScalarValue],
        domain_extents: &[&HashMap<String, DomainExtent>],
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        Box::pin(ChildFrameRuntime::new().measure_with_builder(
            self.plot,
            eval_ctx,
            layout_spec,
            &self.scale_builder,
            self.data_override.as_ref(),
            facet_path,
            domain_extents,
        ))
        .await
    }
}

/// Fixed plot-area layout for a child plot measured inside a container.
pub(crate) fn fixed_child_plot_area_layout_spec(width: f32, height: f32) -> EvaluatedLayoutSpec {
    ChildFrameRuntime::new().fixed_plot_area_layout_spec(width, height)
}

/// Measure a child plot with a caller-provided scale builder.
pub(crate) async fn measure_child_frame_plot_with_builder(
    plot: &CompiledPlot,
    eval_ctx: &EvaluationContext,
    layout_spec: &EvaluatedLayoutSpec,
    scale_builder: &ScaleBuilder,
    data_override: Option<&DataFrame>,
    facet_path: &[ScalarValue],
    domain_extents: &[&HashMap<String, DomainExtent>],
) -> Result<ComponentsMeasurement, AvengerChartError> {
    Box::pin(ChildFrameRuntime::new().measure_with_builder(
        plot,
        eval_ctx,
        layout_spec,
        scale_builder,
        data_override,
        facet_path,
        domain_extents,
    ))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_child_plot_area_layout_spec_clamps_to_positive_plot_area() {
        let spec = fixed_child_plot_area_layout_spec(0.0, -5.0);
        match spec.plot_area {
            EvaluatedSizeMode::Fixed { width, height } => {
                assert_eq!(width, 1.0);
                assert_eq!(height, 1.0);
            }
            _ => panic!("expected fixed plot-area size"),
        }
    }
}
