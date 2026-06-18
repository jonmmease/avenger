use std::{any::Any, collections::HashMap};

use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use datafusion::{arrow::datatypes::DataType, common::ScalarValue};
use indexmap::IndexMap;

use crate::{
    AvengerChartError, ChannelValue, PlotGeometry, ScaleRangeBinding, ScaleTypePreference,
};

/// Display/equilibrium geometry for a generated position channel.
///
/// Coordinate systems with internally generated positional channels can return
/// these slots so custom coordinate marks, guides, and event datum rows all
/// share one frame calculation.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedPositionSlot {
    pub channel: String,
    pub id: String,
    pub scale_name: String,
    pub order_index: usize,
    pub equilibrium_x: f32,
    pub display_x: f32,
    pub displacement_px: f32,
    pub displacement_slots: f32,
}

/// Request to invert a local plot-area point back to data-space channel values.
///
/// `local_point` is in the coordinate scope's local plot-area coordinates (the
/// same space the configured scale ranges are bound to). The transform inverts
/// each requested channel through its configured scale.
pub struct InteractionPointInversionRequest<'a> {
    pub local_point: [f32; 2],
    pub plot_area_width: f32,
    pub plot_area_height: f32,
    pub channels: &'a [&'a str],
    pub scales: &'a HashMap<String, ConfiguredScale>,
}

/// Core-safe coordinate transform behavior.
///
/// This trait contains the coordinate operations that do not need the
/// top-level chart layout/runtime engine. The top-level chart crate layers its
/// measurement hook on top while facet/concat layout remains core-owned there.
pub trait CoordinateSystemTransformCore: Send + Sync {
    /// Channels required by this coordinate transform.
    ///
    /// Required channels are coordinate inputs. Most are backed by scales, but
    /// container-like transforms may use some required channels as partition or
    /// layout keys instead.
    fn required_channels(&self) -> &'static [&'static str];

    /// Whether a coordinate/input channel should participate in scale building.
    fn channel_uses_scale(&self, _channel: &str) -> bool {
        true
    }

    /// Whether a scale/channel name should be treated as positional.
    ///
    /// The default preserves the legacy static-channel contract. Coordinates
    /// with runtime scale families can override this without pretending those
    /// channels are required mark inputs.
    fn is_position_scale_channel(&self, channel: &str) -> bool {
        self.required_channels().contains(&channel)
    }

    /// Transform position channels to coordinate system geometry.
    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError>;

    /// Get the default range binding for a coordinate channel.
    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

    /// Get the default range for a coordinate channel.
    fn default_range(
        &self,
        channel: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        self.default_range_binding(channel)
            .and_then(|binding| binding.resolve(plot_area_width, plot_area_height))
    }

    /// Get default scale options for a coordinate channel.
    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue>;

    /// Coordinate-system semantic scale preference for a position scale.
    fn preferred_scale_type(
        &self,
        _channel: &str,
        _data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        None
    }

    /// Ordered generated position channels owned by the coordinate system.
    fn generated_position_channels(&self) -> IndexMap<String, ChannelValue> {
        IndexMap::new()
    }

    /// Resolved generated position slots for the current plot width and params.
    fn generated_position_slots(
        &self,
        plot_width: f32,
        _params: &IndexMap<String, ScalarValue>,
    ) -> Result<Vec<GeneratedPositionSlot>, AvengerChartError> {
        let generated = self.generated_position_channels();
        let count = generated.len();
        let step = if count > 1 {
            plot_width / (count.saturating_sub(1) as f32)
        } else {
            0.0
        };
        Ok(generated
            .into_iter()
            .enumerate()
            .map(|(index, (channel, value))| {
                let equilibrium_x = if count <= 1 {
                    plot_width / 2.0
                } else {
                    index as f32 * step
                };
                GeneratedPositionSlot {
                    id: channel.clone(),
                    scale_name: value
                        .get_scale_name(&channel)
                        .unwrap_or_else(|| channel.clone()),
                    channel,
                    order_index: index,
                    equilibrium_x,
                    display_x: equilibrium_x,
                    displacement_px: 0.0,
                    displacement_slots: 0.0,
                }
            })
            .collect())
    }

    /// Coordinate channels this transform can invert from a local plot-area point.
    ///
    /// Only transforms with a non-empty list export interaction coordinate
    /// scopes. This keeps facet/concat container transforms from becoming bogus
    /// coordinate targets while their Cartesian leaf subplots still export
    /// scopes. The default is empty (no interaction inversion).
    fn interaction_invertible_channels(&self) -> Vec<String> {
        Vec::new()
    }

    /// Invert a local plot-area point to data-space values for each channel.
    ///
    /// The default implementation reports that the coordinate system does not
    /// support event coordinate inversion.
    fn invert_interaction_point(
        &self,
        _request: InteractionPointInversionRequest<'_>,
    ) -> Result<IndexMap<String, ScalarValue>, AvengerChartError> {
        Err(AvengerChartError::InvalidArgument(
            "coordinate system does not support event coordinate inversion".to_string(),
        ))
    }
}

/// Serializable, object-safe coordinate transform contract.
///
/// This core trait owns the transform behavior external coordinate crates need
/// to implement. The top-level chart crate layers facet/concat measurement
/// dispatch around this trait without exposing layout runtime state through the
/// extension boundary.
#[typetag::serde(tag = "type")]
pub trait CoordinateSystemTransform: CoordinateSystemTransformCore {
    /// Downcast support for top-level built-in measurement dispatch.
    fn as_any(&self) -> &dyn Any;

    /// Clone this transform into a boxed trait object.
    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform>;
}

impl Clone for Box<dyn CoordinateSystemTransform> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
