//! Runtime helpers for measuring child plots as chart frames.
//!
//! Container coordinate systems decide which child frames exist and where they
//! should be placed. This module owns the common "measure this child plot as a
//! frame" work that those containers should not duplicate.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext,
};

use avenger_chart_core::{AxisSpec, DomainCoordination};

use crate::{
    error::AvengerChartError,
    facet::evaluated_facet_tree::{EvaluatedFacetTree, FacetWrapLayoutContext},
    layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
    render::EvaluationContext,
    scales::{DomainExtent, PlotScaleSpec, ScaleBuilder},
};

use super::{
    ChildFrameChannelDomainExtent, ChildFrameSharingLevel, CompiledPlot, ComponentsMeasurement,
    child_frame_domain_sharing_levels_for_plot, extract_child_frame_shared_domain_extents,
    scale_provider::DynamicScaleProvider,
    scales::build_scale_builder_from_marks_with_render_context,
    session::{ScaleDomainCacheScope, scale_domain_cache_key_for_parts_with_scope},
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
        eval_ctx: &EvaluationContext,
    ) -> Result<PreparedChildFramePlot<'a>, AvengerChartError> {
        let data_override = match data_selection {
            ChildFrameDataSelection::ExplicitChild => None,
            ChildFrameDataSelection::InheritParent => inherited_data.cloned(),
        };
        let (local_facet_tree, facet_data_root) = if EvaluatedFacetTree::plot_contains_facet_mark(
            plot,
        ) {
            let mut slot_cache = crate::partition::PartitionSlotCache::new();
            let tree = EvaluatedFacetTree::from_compiled_plot_with_params_data_override_wrap_layout_context_and_slot_cache(
                    plot,
                    eval_ctx.session_context.as_ref(),
                    eval_ctx.params(),
                    data_override.as_ref(),
                    FacetWrapLayoutContext::default(),
                    &mut slot_cache,
                )
                .await?;
            let facet_data_root = EvaluatedFacetTree::data_root_for_plot(
                plot,
                eval_ctx.session_context.as_ref(),
                data_override.as_ref(),
            );
            (Some(Arc::new(tree)), facet_data_root)
        } else {
            (None, None)
        };
        let cache_lookup = eval_ctx.scale_domain_cache().map(|cache| {
            let scope = ScaleDomainCacheScope::ChildFrame {
                container_path: eval_ctx
                    .child_frame_container_path()
                    .iter()
                    .map(|segment| format!("{segment:?}"))
                    .collect(),
                data_selection: format!("{data_selection:?}"),
            };
            let key = scale_domain_cache_key_for_parts_with_scope(
                &plot.marks,
                &plot.scale_specs,
                &plot.data,
                data_override.as_ref(),
                eval_ctx.session_context.as_ref(),
                eval_ctx.params(),
                scope,
            );
            (cache.clone(), key)
        });
        if let Some((cache, key)) = &cache_lookup {
            let cached_builder = {
                cache
                    .lock()
                    .expect("scale-domain cache lock poisoned")
                    .get(key)
            };
            if let Some(builder) = cached_builder {
                eval_ctx.record_scale_domain_cache_hit();
                let channel_domain_sharing_levels =
                    child_frame_domain_sharing_levels_for_plot(plot);
                let local_domain_extents = extract_child_frame_shared_domain_extents(
                    &builder,
                    &channel_domain_sharing_levels,
                );
                return Ok(PreparedChildFramePlot {
                    plot,
                    data_override,
                    local_facet_tree,
                    facet_data_root,
                    scale_builder: (*builder).clone(),
                    local_domain_extents,
                    channel_domain_sharing_levels,
                });
            }
            eval_ctx.record_scale_domain_cache_miss();
        }
        eval_ctx.record_scale_builder_build();
        let scale_builder = Box::pin(build_scale_builder_from_marks_with_render_context(
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
        if let Some((cache, key)) = cache_lookup {
            cache
                .lock()
                .expect("scale-domain cache lock poisoned")
                .insert(key, scale_builder.clone());
        }

        Ok(PreparedChildFramePlot {
            plot,
            data_override,
            local_facet_tree,
            facet_data_root,
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
                let seed_channels = plot
                    .scale_to_coord_channel
                    .keys()
                    .chain(plot.scale_specs.keys())
                    .cloned()
                    .collect::<HashSet<_>>();
                for extents in domain_extents {
                    if !extents.is_empty() {
                        builder.extend_with_domain_extents_for_channels(extents, &seed_channels);
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
    local_facet_tree: Option<Arc<EvaluatedFacetTree>>,
    facet_data_root: Option<DataFrame>,
    scale_builder: ScaleBuilder,
    local_domain_extents: HashMap<String, ChildFrameChannelDomainExtent>,
    channel_domain_sharing_levels: HashMap<String, DomainCoordination>,
}

impl<'a> PreparedChildFramePlot<'a> {
    pub(crate) fn local_domain_extents(&self) -> &HashMap<String, ChildFrameChannelDomainExtent> {
        &self.local_domain_extents
    }

    pub(crate) fn channel_domain_sharing_levels(&self) -> &HashMap<String, DomainCoordination> {
        &self.channel_domain_sharing_levels
    }

    pub(crate) fn scale_type_signatures_by_channel(&self) -> HashMap<String, String> {
        let mut by_channel: HashMap<String, Vec<String>> = HashMap::new();
        for (scale_name, coord_channel) in &self.plot.scale_to_coord_channel {
            let scale_type = self
                .plot
                .scale_specs
                .get(scale_name)
                .map(plot_scale_type_name)
                .unwrap_or("auto");
            by_channel
                .entry(coord_channel.clone())
                .or_default()
                .push(scale_type.to_string());
        }
        for fallback_channel in ["x", "y"] {
            if !by_channel.contains_key(fallback_channel)
                && let Some(spec) = self.plot.scale_specs.get(fallback_channel)
            {
                by_channel
                    .entry(fallback_channel.to_string())
                    .or_default()
                    .push(plot_scale_type_name(spec).to_string());
            }
        }

        by_channel
            .into_iter()
            .map(|(channel, mut scale_types)| {
                scale_types.sort();
                (channel, scale_types.join(","))
            })
            .collect()
    }

    pub(crate) fn axis_config_signatures_by_channel(
        &self,
        ctx: &SessionContext,
    ) -> HashMap<String, Vec<String>> {
        self.plot
            .axis_specs
            .iter()
            .map(|(channel, axis_spec)| {
                (
                    channel.clone(),
                    axis_config_signature(axis_spec, ctx)
                        .into_iter()
                        .map(|expr| format!("{expr:?}"))
                        .collect(),
                )
            })
            .collect()
    }

    pub(crate) fn data_override(&self) -> Option<&DataFrame> {
        self.data_override.as_ref()
    }

    pub(crate) fn local_facet_tree(&self) -> Option<Arc<EvaluatedFacetTree>> {
        self.local_facet_tree.clone()
    }

    pub(crate) fn facet_data_root(&self) -> Option<DataFrame> {
        self.facet_data_root.clone()
    }

    pub(crate) async fn measure(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        facet_path: &[ScalarValue],
        domain_extents: &[&HashMap<String, DomainExtent>],
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        let mut child_eval_ctx = eval_ctx.clone();
        let local_facet_path;
        let facet_path = if let Some(facet_tree) = &self.local_facet_tree {
            child_eval_ctx = child_eval_ctx
                .with_facet_tree(facet_tree.clone())
                .with_facet_data_root(self.facet_data_root.clone());
            local_facet_path = Vec::new();
            local_facet_path.as_slice()
        } else {
            facet_path
        };
        Box::pin(ChildFrameRuntime::new().measure_with_builder(
            self.plot,
            &child_eval_ctx,
            layout_spec,
            &self.scale_builder,
            self.data_override.as_ref(),
            facet_path,
            domain_extents,
        ))
        .await
    }
}

fn axis_config_signature(axis_spec: &AxisSpec, ctx: &SessionContext) -> Vec<Expr> {
    match axis_spec {
        AxisSpec::Local(axis) => axis.all_exprs(ctx),
    }
}

fn plot_scale_type_name(scale_spec: &PlotScaleSpec) -> &'static str {
    match scale_spec {
        PlotScaleSpec::Local(config) => config
            .scale_spec
            .as_option()
            .map(|spec| spec.name())
            .unwrap_or("auto"),
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
