use avenger_image::{
    ImageResourceLoadOptions, ImageResourceResolver, load_image_resource_requests_blocking,
};
use avenger_scenegraph::{image_resources::resolve_ready_image_resources, scene_graph::SceneGraph};

use crate::{error::AvengerChartError, render::EvaluatedPlot};

pub(crate) fn resolve_evaluated_plot_image_resources(
    evaluated: &EvaluatedPlot,
    resolver: &dyn ImageResourceResolver,
    load_options: ImageResourceLoadOptions,
) -> Result<SceneGraph, AvengerChartError> {
    load_image_resource_requests_blocking(resolver, &evaluated.resource_requests, load_options)
        .map_err(|err| {
            AvengerChartError::InternalError(format!("failed to load image resources: {err}"))
        })?;

    resolve_ready_image_resources(&evaluated.scene_graph, resolver).map_err(|err| {
        AvengerChartError::InternalError(format!("failed to resolve image resources: {err}"))
    })
}
