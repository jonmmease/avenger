//! Channel resolution and gathering methods for Plot

use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use avenger_chart_core::{
    Auto, AvengerChartError, Axis, AxisSpec, ChannelValue, CompiledMark, CoordinateSystemCore,
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
        extract_channel_config_from_value(
            &channel_name,
            &channel_value,
            coord_transform,
            axis_specs,
            legends,
            scale_specs,
            scale_to_coord_channel,
        )?;
    }

    // Extract explicit position-channel axis configurations from the mark after
    // ChannelValue defaults, so `.x_with(..., |c| c.axis(...))` overrides or
    // augments defaults carried by the value itself.
    for (channel_name, axis_config) in explicit_axis_configs.iter() {
        merge_axis_config(axis_specs, channel_name, axis_config.as_ref());
    }

    Ok(())
}

fn extract_channel_config_from_value(
    channel_name: &str,
    channel_value: &ChannelValue,
    coord_transform: &dyn CoordinateSystemTransformCore,
    axis_specs: &mut HashMap<String, AxisSpec>,
    legends: &mut IndexMap<String, Legend>,
    scale_specs: &mut HashMap<String, ScaleSpec>,
    scale_to_coord_channel: &mut HashMap<String, String>,
) -> Result<(), AvengerChartError> {
    if let Some(axis_config) = channel_value.get_axis_config() {
        merge_axis_config(axis_specs, channel_name, axis_config);
    }

    if !coord_transform.channel_uses_scale(channel_name) {
        if channel_value.has_scale_config() || channel_value.has_legend_config() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Coordinate channel '{channel_name}' is a layout/partition input and does \
                 not support scale or legend configuration. Use coordinate-specific ordering, \
                 sharing, and guide options instead."
            )));
        }
        return Ok(());
    }

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
            return Ok(());
        }
    };

    let scale_key = match channel_value {
        ChannelValue::Scaled { scale_name, .. } => {
            scale_name.as_deref().unwrap_or(channel_name).to_string()
        }
        ChannelValue::Conditional { .. } => channel_name.to_string(),
        ChannelValue::Value { .. } => unreachable!(),
    };
    scale_to_coord_channel
        .entry(scale_key.clone())
        .or_insert_with(|| coord_channel_for_scale_channel(channel_name));

    if let Some(config) = scale_config {
        let config = *config;
        match scale_specs.entry(scale_key.clone()) {
            Entry::Occupied(mut occupied) => {
                let existing_spec = occupied.get().clone();
                match existing_spec {
                    ScaleSpec::Local(existing_scale) => {
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

    if let Some(config) = legend_config {
        let config = *config;
        let existing_legend = legends.shift_remove(channel_name);
        let configured = if let Some(existing) = existing_legend {
            existing.update(config.clone())
        } else {
            config.clone()
        };
        legends.insert(channel_name.to_string(), configured);
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
    )?;
    if let Some(view) = mark_state.view.as_ref() {
        extract_channel_configs_from_channels(
            view.data.channels(),
            &mark_state.axis_configs,
            ctx,
            coord_transform,
            axis_specs,
            legends,
            scale_specs,
            scale_to_coord_channel,
        )?;
    }
    Ok(())
}

pub(crate) fn extract_channel_configs_from_compiled_domain_channels(
    compiled_marks: &[Arc<dyn CompiledMark>],
    coord_transform: &dyn CoordinateSystemTransformCore,
    axis_specs: &mut HashMap<String, AxisSpec>,
    legends: &mut IndexMap<String, Legend>,
    scale_specs: &mut HashMap<String, ScaleSpec>,
    scale_to_coord_channel: &mut HashMap<String, String>,
) -> Result<(), AvengerChartError> {
    for mark in compiled_marks {
        for source in mark.scale_domain_channels()? {
            extract_channel_config_from_value(
                &source.channel,
                &source.channel_value,
                coord_transform,
                axis_specs,
                legends,
                scale_specs,
                scale_to_coord_channel,
            )?;
        }
    }
    Ok(())
}

/// Extract axis configurations owned by the resolved coordinate frame.
pub(crate) fn extract_axis_configs_from_coordinate<C: CoordinateSystemCore>(
    coord_system: &C,
    axis_specs: &mut HashMap<String, AxisSpec>,
) {
    for (channel_name, axis_config) in coord_system.coordinate_axis_configs() {
        merge_axis_config(axis_specs, &channel_name, axis_config.as_ref());
    }
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
