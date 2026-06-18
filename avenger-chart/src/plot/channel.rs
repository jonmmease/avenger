//! Channel resolution and gathering methods for Plot

use std::collections::{HashMap, hash_map::Entry};

use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use avenger_chart_core::{
    Auto, AvengerChartError, Axis, AxisSpec, ChannelValue, CoordinateScaleSource,
    CoordinateSystemTransformCore, Legend, MarkState, Scale, resolve_all_channel_refs,
    strip_trailing_numbers,
};
use avenger_chart_scales::PlotScaleSpec as ScaleSpec;

fn coord_channel_for_scale_channel(channel_name: &str) -> String {
    strip_trailing_numbers(channel_name).to_string()
}

fn merge_axis_config(
    axis_specs: &mut HashMap<String, AxisSpec>,
    channel_name: &str,
    axis_config: &dyn Axis,
) {
    match axis_specs.entry(channel_name.to_string()) {
        Entry::Occupied(mut occupied) => {
            let AxisSpec::Local(existing) = occupied.get();
            let mut updated = existing.box_clone();
            updated.update(axis_config);
            occupied.insert(AxisSpec::Local(updated));
        }
        Entry::Vacant(vacant) => {
            vacant.insert(AxisSpec::Local(axis_config.box_clone()));
        }
    }
}

fn extract_channel_configs_from_channels(
    encodings: &IndexMap<String, ChannelValue>,
    explicit_axis_configs: &HashMap<String, std::sync::Arc<dyn Axis>>,
    ctx: &SessionContext,
    coord_transform: &dyn CoordinateSystemTransformCore,
    axis_specs: &mut HashMap<String, AxisSpec>,
    legends: &mut IndexMap<String, Legend>,
    scale_specs: &mut HashMap<String, ScaleSpec>,
    scale_to_coord_channel: &mut HashMap<String, String>,
) -> Result<(), AvengerChartError> {
    // Resolve channel references with the proper SessionContext
    let resolved_encodings = match resolve_all_channel_refs(encodings, ctx) {
        Ok(resolved) => resolved,
        Err(_) => {
            // Resolution failed - use original encodings
            encodings.clone()
        }
    };

    for (channel_name, channel_value) in resolved_encodings {
        if let Some(axis_config) = channel_value.get_axis_config() {
            merge_axis_config(axis_specs, &channel_name, axis_config);
        }

        if !coord_transform.channel_uses_scale(&channel_name) {
            if channel_value.has_scale_config() || channel_value.has_legend_config() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Coordinate channel '{channel_name}' is a layout/partition input and does \
                     not support scale or legend configuration. Use coordinate-specific ordering, \
                     sharing, and guide options instead."
                )));
            }
            continue;
        }

        // Extract scale and legend configs
        let (scale_config, legend_config) = match channel_value.clone() {
            ChannelValue::Scaled {
                scale_config,
                legend_config,
                ..
            }
            | ChannelValue::Conditional {
                scale_config,
                legend_config,
                ..
            } => (scale_config, legend_config),
            ChannelValue::Value { .. } => {
                // No scale or legend for identity mappings
                continue;
            }
        };

        // Determine scale name
        let scale_key = match channel_value {
            ChannelValue::Scaled { scale_name, .. } => {
                scale_name.as_deref().unwrap_or(&channel_name).to_string()
            }
            ChannelValue::Conditional { .. } => {
                // Conditional always uses channel name
                channel_name.to_string()
            }
            _ => unreachable!(),
        };
        scale_to_coord_channel
            .entry(scale_key.clone())
            .or_insert_with(|| coord_channel_for_scale_channel(&channel_name));

        // Extract scale config if present
        if let Some(config) = scale_config {
            let config = *config;
            match scale_specs.entry(scale_key.clone()) {
                Entry::Occupied(mut occupied) => {
                    let existing_spec = occupied.get().clone();
                    match existing_spec {
                        ScaleSpec::Local(existing_scale) => {
                            // Compose the two scale configurations using update()
                            // Apply existing config first, then the new config
                            let updated_scale = Scale::<Auto>::from_config(existing_scale.clone())
                                .update(Scale::from_config(config));
                            occupied.insert(ScaleSpec::Local(updated_scale.into_config()));
                        }
                    }
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(ScaleSpec::Local(config));
                }
            }
        }

        // Extract legend config if present
        if let Some(config) = legend_config {
            let config = *config;
            // Compose legend configurations - apply all configs in order
            let existing_legend = legends.shift_remove(channel_name.as_str());
            let configured = if let Some(existing) = existing_legend {
                // Apply new config on top of existing configured legend
                existing.update(config.clone())
            } else {
                // Use the config as-is
                config.clone()
            };
            legends.insert(channel_name.clone(), configured);
        }
    }

    // Extract explicit position-channel axis configurations from the mark after
    // ChannelValue defaults, so `.x_with(..., |c| c.axis(...))` overrides or
    // augments defaults carried by the value itself.
    for (channel_name, axis_config) in explicit_axis_configs.iter() {
        merge_axis_config(axis_specs, channel_name, axis_config.as_ref());
    }

    Ok(())
}

/// Extract scale, legend, and axis configurations from a mark state's channels.
pub(crate) fn extract_channel_configs_from_state(
    mark_state: &MarkState,
    ctx: &SessionContext,
    coord_transform: &dyn CoordinateSystemTransformCore,
    axis_specs: &mut HashMap<String, AxisSpec>,
    legends: &mut IndexMap<String, Legend>,
    scale_specs: &mut HashMap<String, ScaleSpec>,
    scale_to_coord_channel: &mut HashMap<String, String>,
) -> Result<(), AvengerChartError> {
    extract_channel_configs_from_channels(
        mark_state.data.channels(),
        &mark_state.axis_configs,
        ctx,
        coord_transform,
        axis_specs,
        legends,
        scale_specs,
        scale_to_coord_channel,
    )
}

/// Extract scale, legend, and axis configurations from a coordinate-owned
/// scale source.
pub(crate) fn extract_channel_configs_from_coordinate_source(
    source: &CoordinateScaleSource,
    ctx: &SessionContext,
    coord_transform: &dyn CoordinateSystemTransformCore,
    axis_specs: &mut HashMap<String, AxisSpec>,
    legends: &mut IndexMap<String, Legend>,
    scale_specs: &mut HashMap<String, ScaleSpec>,
    scale_to_coord_channel: &mut HashMap<String, String>,
) -> Result<(), AvengerChartError> {
    extract_channel_configs_from_channels(
        source.data.channels(),
        &source.axis_configs,
        ctx,
        coord_transform,
        axis_specs,
        legends,
        scale_specs,
        scale_to_coord_channel,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use avenger_chart_cartesian::{
        Cartesian, CartesianAxis, marks::CartesianSymbolPositionChannels,
    };
    use avenger_chart_core::{CoordinateSystem, serialization::DefaultLogicalExprNodeExt};
    use avenger_chart_marks::Symbol;
    use datafusion::{
        common::ScalarValue,
        logical_expr::Expr,
        prelude::{SessionContext, col},
    };

    fn axis_for_channel(axis_specs: &HashMap<String, AxisSpec>, channel: &str) -> CartesianAxis {
        let AxisSpec::Local(axis) = axis_specs.get(channel).expect("axis config");
        axis.as_any()
            .downcast_ref::<CartesianAxis>()
            .expect("cartesian axis")
            .clone()
    }

    fn axis_title(axis: &CartesianAxis, ctx: &SessionContext) -> String {
        let title_node = axis
            .title
            .as_option()
            .and_then(|value| value.as_ref())
            .expect("axis title");
        match title_node.to_default_expr(ctx).expect("axis title expr") {
            Expr::Literal(ScalarValue::Utf8(Some(title)), _) => title,
            Expr::Literal(ScalarValue::LargeUtf8(Some(title)), _) => title,
            other => other.to_string(),
        }
    }

    #[test]
    fn channel_value_axis_config_is_extracted_and_explicit_axis_updates_it() {
        let ctx = SessionContext::new();
        let value = ChannelValue::from(col("x"))
            .with_axis_config(CartesianAxis::new().title("Default title").grid(true));
        let mark =
            Symbol::<Cartesian>::new().x_with(value, |c| c.axis(|a| a.title("Explicit title")));

        let mut axis_specs = HashMap::new();
        let mut legends = IndexMap::new();
        let mut scale_specs = HashMap::new();
        let mut scale_to_coord_channel = HashMap::new();
        let coord_transform = Cartesian::default().create_transform();
        extract_channel_configs_from_state(
            mark.state(),
            &ctx,
            coord_transform.as_ref(),
            &mut axis_specs,
            &mut legends,
            &mut scale_specs,
            &mut scale_to_coord_channel,
        )
        .expect("extract channel configs");

        let axis = axis_for_channel(&axis_specs, "x");
        assert_eq!(axis_title(&axis, &ctx), "Explicit title");
        assert!(
            axis.grid.is_set(),
            "ChannelValue axis defaults should preserve fields not overwritten explicitly"
        );
    }
}
