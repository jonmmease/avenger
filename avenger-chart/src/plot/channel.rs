//! Channel resolution and gathering methods for Plot

use std::collections::{HashMap, hash_map::Entry};

use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use avenger_chart_core::{
    Auto, AxisSpec, ChannelValue, CoordinateSystem, Legend, Mark, resolve_all_channel_refs,
    strip_trailing_numbers,
};
use avenger_chart_scales::{PlotScaleSpec as ScaleSpec, Scale};

/// Extract scale, legend, and axis configurations from a mark's channels
pub(crate) fn extract_channel_configs<C: CoordinateSystem>(
    mark: &dyn Mark<C>,
    ctx: &SessionContext,
    axis_specs: &mut HashMap<String, AxisSpec>,
    legends: &mut IndexMap<String, Legend>,
    scale_specs: &mut HashMap<String, ScaleSpec>,
    scale_to_coord_channel: &mut HashMap<String, String>,
) {
    // Extract axis configurations from the mark
    for (channel_name, axis_config) in mark.state().axis_configs.iter() {
        match axis_specs.entry(channel_name.clone()) {
            Entry::Occupied(mut occupied) => {
                // Update existing axis with new configuration
                let AxisSpec::Local(existing) = occupied.get();
                let mut updated = existing.box_clone();
                updated.update(axis_config.as_ref());
                occupied.insert(AxisSpec::Local(updated.box_clone()));
            }
            Entry::Vacant(vacant) => {
                vacant.insert(AxisSpec::Local(axis_config.box_clone()));
            }
        }
    }

    // Get all channel encodings from the mark
    let encodings = mark.data_context().channels();

    // Resolve channel references with the proper SessionContext
    let resolved_encodings = match resolve_all_channel_refs(encodings, ctx) {
        Ok(resolved) => resolved,
        Err(_) => {
            // Resolution failed - use original encodings
            encodings.clone()
        }
    };

    for (channel_name, channel_value) in resolved_encodings {
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
            .or_insert_with(|| strip_trailing_numbers(&channel_name).to_string());

        // Extract scale config if present
        if let Some(config) = scale_config {
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
}
