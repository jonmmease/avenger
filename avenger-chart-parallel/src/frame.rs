use std::collections::HashMap;

use avenger_chart_core::{ArrayRefHelpers, AvengerChartError, ScalarValueHelpers};
use datafusion::{arrow::array::Array, common::ScalarValue};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::coord::ParallelTransformDimension;

/// Runtime state source for committed parallel-axis order.
///
/// The parameter value should be a nullable list of dimension id strings. A
/// null or missing value falls back to static `Parallel::order(...)`, then to
/// declaration order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelOrderState {
    /// Parameter name containing the committed dimension id order.
    pub param: String,
}

impl ParallelOrderState {
    pub fn param(param: impl Into<String>) -> Self {
        Self {
            param: param.into(),
        }
    }
}

/// Runtime state source for a transient parallel-axis display position.
///
/// This is intended for drag previews: one param stores the active dimension
/// id, and another stores the display x pixel position for that axis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelDisplayState {
    /// Parameter name containing the active dimension id.
    pub dimension_id_param: String,
    /// Parameter name containing the active dimension display x in plot pixels.
    pub display_x_param: String,
}

impl ParallelDisplayState {
    pub fn active_axis(
        dimension_id_param: impl Into<String>,
        display_x_param: impl Into<String>,
    ) -> Self {
        Self {
            dimension_id_param: dimension_id_param.into(),
            display_x_param: display_x_param.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParallelFrameDimension {
    pub id: String,
    pub generated_channel: String,
    pub scale_name: String,
}

impl ParallelFrameDimension {
    pub(crate) fn new(
        id: impl Into<String>,
        generated_channel: impl Into<String>,
        scale_name: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            generated_channel: generated_channel.into(),
            scale_name: scale_name.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParallelAxisSlot {
    pub id: String,
    pub generated_channel: String,
    pub scale_name: String,
    pub equilibrium_index: usize,
    pub equilibrium_x: f32,
    pub display_x: f32,
    pub displacement_px: f32,
    pub displacement_slots: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParallelFrameGeometry {
    pub slots: Vec<ParallelAxisSlot>,
}

impl ParallelFrameGeometry {
    pub fn slot(&self, id: &str) -> Option<&ParallelAxisSlot> {
        self.slots.iter().find(|slot| slot.id == id)
    }
}

pub fn resolve_parallel_frame(
    dimensions: &[ParallelTransformDimension],
    order: Option<&[String]>,
    display_overrides: Option<&HashMap<String, f32>>,
    width: f32,
) -> ParallelFrameGeometry {
    let dimensions = dimensions
        .iter()
        .map(|dimension| {
            ParallelFrameDimension::new(
                dimension.id.clone(),
                dimension.generated_channel.clone(),
                dimension.id.clone(),
            )
        })
        .collect::<Vec<_>>();
    resolve_parallel_frame_dimensions(&dimensions, order, display_overrides, width)
}

pub(crate) fn resolve_parallel_frame_dimensions(
    dimensions: &[ParallelFrameDimension],
    order: Option<&[String]>,
    display_overrides: Option<&HashMap<String, f32>>,
    width: f32,
) -> ParallelFrameGeometry {
    let ordered = ordered_frame_dimensions(dimensions, order);
    let count = ordered.len();
    let step = if count > 1 {
        width / (count.saturating_sub(1) as f32)
    } else {
        0.0
    };
    let slots = ordered
        .into_iter()
        .enumerate()
        .map(|(index, dimension)| {
            let equilibrium_x = if count <= 1 {
                width / 2.0
            } else {
                index as f32 * step
            };
            let display_x = display_overrides
                .and_then(|overrides| overrides.get(&dimension.id).copied())
                .unwrap_or(equilibrium_x);
            let displacement_px = display_x - equilibrium_x;
            let displacement_slots = if step.abs() > f32::EPSILON {
                displacement_px / step
            } else {
                0.0
            };
            ParallelAxisSlot {
                id: dimension.id.clone(),
                generated_channel: dimension.generated_channel.clone(),
                scale_name: dimension.scale_name.clone(),
                equilibrium_index: index,
                equilibrium_x,
                display_x,
                displacement_px,
                displacement_slots,
            }
        })
        .collect();
    ParallelFrameGeometry { slots }
}

pub(crate) fn resolve_order_state(
    state: Option<&ParallelOrderState>,
    params: &IndexMap<String, ScalarValue>,
    dimension_ids: &[String],
) -> Result<Option<Vec<String>>, AvengerChartError> {
    let Some(state) = state else {
        return Ok(None);
    };
    let Some(value) = params.get(&state.param) else {
        return Ok(None);
    };
    let Some(order) = scalar_to_string_list(value).map_err(|err| {
        AvengerChartError::InvalidArgument(format!(
            "Invalid parallel order param '{}': {err}",
            state.param
        ))
    })?
    else {
        return Ok(None);
    };
    validate_order_ids(&order, dimension_ids, "parallel order param")?;
    Ok(Some(order))
}

pub(crate) fn resolve_display_state(
    state: Option<&ParallelDisplayState>,
    params: &IndexMap<String, ScalarValue>,
    dimension_ids: &[String],
) -> Result<Option<HashMap<String, f32>>, AvengerChartError> {
    let Some(state) = state else {
        return Ok(None);
    };
    let dimension_id = params
        .get(&state.dimension_id_param)
        .and_then(nullable_scalar_string);
    let display_x = params
        .get(&state.display_x_param)
        .and_then(nullable_scalar_f32);
    let (Some(dimension_id), Some(display_x)) = (dimension_id, display_x) else {
        return Ok(None);
    };
    if !dimension_ids.iter().any(|id| id == &dimension_id) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Parallel display state param '{}' references unknown dimension id '{}'",
            state.dimension_id_param, dimension_id
        )));
    }
    Ok(Some(HashMap::from([(dimension_id, display_x)])))
}

pub(crate) fn validate_order_ids(
    order: &[String],
    dimension_ids: &[String],
    label: &str,
) -> Result<(), AvengerChartError> {
    if order.len() != dimension_ids.len() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must contain exactly {} id(s), got {}",
            dimension_ids.len(),
            order.len()
        )));
    }
    for id in order {
        if !dimension_ids.iter().any(|dimension_id| dimension_id == id) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} references unknown id '{id}'"
            )));
        }
        if order.iter().filter(|candidate| *candidate == id).count() > 1 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} repeats id '{id}'"
            )));
        }
    }
    Ok(())
}

pub fn propose_axis_order(
    current_order: &[String],
    dragged_id: &str,
    display_x: f32,
    slots: &[ParallelAxisSlot],
) -> Vec<String> {
    let mut remaining = current_order
        .iter()
        .filter(|id| id.as_str() != dragged_id)
        .cloned()
        .collect::<Vec<_>>();
    let mut insert_at = remaining.len();
    for index in 0..remaining.len() {
        let Some(slot) = slots.iter().find(|slot| slot.id == remaining[index]) else {
            continue;
        };
        if display_x < slot.equilibrium_x {
            insert_at = index;
            break;
        }
    }
    remaining.insert(insert_at, dragged_id.to_string());
    remaining
}

fn ordered_frame_dimensions<'a>(
    dimensions: &'a [ParallelFrameDimension],
    order: Option<&[String]>,
) -> Vec<&'a ParallelFrameDimension> {
    let Some(order) = order else {
        return dimensions.iter().collect();
    };
    order
        .iter()
        .filter_map(|id| dimensions.iter().find(|dimension| dimension.id == *id))
        .collect()
}

fn scalar_to_string_list(value: &ScalarValue) -> Result<Option<Vec<String>>, String> {
    match value {
        ScalarValue::Null
        | ScalarValue::Utf8(None)
        | ScalarValue::LargeUtf8(None)
        | ScalarValue::Utf8View(None) => Ok(None),
        ScalarValue::List(array) => {
            if array.is_empty() || array.is_null(0) {
                return Ok(None);
            }
            array
                .value(0)
                .to_scalar_vec()
                .map_err(|err| err.to_string())?
                .into_iter()
                .map(|value| value.as_scalar_string().map_err(|err| err.to_string()))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
        ScalarValue::LargeList(array) => {
            if array.is_empty() || array.is_null(0) {
                return Ok(None);
            }
            array
                .value(0)
                .to_scalar_vec()
                .map_err(|err| err.to_string())?
                .into_iter()
                .map(|value| value.as_scalar_string().map_err(|err| err.to_string()))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
        _ => Err(format!(
            "expected a list of dimension id strings, got {value}"
        )),
    }
}

fn nullable_scalar_string(value: &ScalarValue) -> Option<String> {
    value.as_scalar_string().ok()
}

fn nullable_scalar_f32(value: &ScalarValue) -> Option<f32> {
    value.as_f32().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::ChannelValue;
    use datafusion::arrow::datatypes::DataType;
    use datafusion::prelude::col;

    fn dims(ids: &[&str]) -> Vec<ParallelTransformDimension> {
        ids.iter()
            .map(|id| ParallelTransformDimension {
                id: (*id).to_string(),
                generated_channel: format!("generated_{id}"),
                channel_value: ChannelValue::from(col(*id)).with_scale_name(*id),
            })
            .collect()
    }

    fn string_list(values: &[&str]) -> ScalarValue {
        ScalarValue::List(ScalarValue::new_list(
            &values
                .iter()
                .map(|value| ScalarValue::Utf8(Some((*value).to_string())))
                .collect::<Vec<_>>(),
            &DataType::Utf8,
            true,
        ))
    }

    #[test]
    fn declaration_order_produces_evenly_spaced_slots() {
        let frame = resolve_parallel_frame(&dims(&["a", "b", "c"]), None, None, 200.0);
        assert_eq!(
            frame
                .slots
                .iter()
                .map(|slot| (slot.id.as_str(), slot.equilibrium_x))
                .collect::<Vec<_>>(),
            vec![("a", 0.0), ("b", 100.0), ("c", 200.0)]
        );
    }

    #[test]
    fn explicit_order_changes_slot_order() {
        let order = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        let frame = resolve_parallel_frame(&dims(&["a", "b", "c"]), Some(&order), None, 200.0);
        assert_eq!(
            frame
                .slots
                .iter()
                .map(|slot| slot.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn display_override_changes_display_position_only() {
        let overrides = HashMap::from([("b".to_string(), 130.0)]);
        let frame = resolve_parallel_frame(&dims(&["a", "b", "c"]), None, Some(&overrides), 200.0);
        let slot = frame.slot("b").expect("slot b");
        assert_eq!(slot.equilibrium_x, 100.0);
        assert_eq!(slot.display_x, 130.0);
        assert_eq!(slot.displacement_px, 30.0);
        assert_eq!(slot.displacement_slots, 0.3);
    }

    #[test]
    fn propose_order_moves_dragged_dimension_by_display_position() {
        let current = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let frame = resolve_parallel_frame(&dims(&["a", "b", "c"]), Some(&current), None, 200.0);
        assert_eq!(
            propose_axis_order(&current, "c", 50.0, &frame.slots),
            vec!["a".to_string(), "c".to_string(), "b".to_string()]
        );
        assert_eq!(
            propose_axis_order(&current, "a", 250.0, &frame.slots),
            vec!["b".to_string(), "c".to_string(), "a".to_string()]
        );
    }

    #[test]
    fn order_state_reads_valid_string_list_param() {
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let params = IndexMap::from([("order".to_string(), string_list(&["c", "a", "b"]))]);
        let order = resolve_order_state(Some(&ParallelOrderState::param("order")), &params, &ids)
            .expect("resolve order")
            .expect("order");
        assert_eq!(order, vec!["c", "a", "b"]);
    }

    #[test]
    fn order_state_rejects_missing_or_repeated_ids() {
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let params = IndexMap::from([("order".to_string(), string_list(&["a", "a", "b"]))]);
        let err = resolve_order_state(Some(&ParallelOrderState::param("order")), &params, &ids)
            .unwrap_err()
            .to_string();
        assert!(err.contains("repeats id 'a'"), "{err}");
    }

    #[test]
    fn display_state_reads_active_axis_override() {
        let ids = vec!["a".to_string(), "b".to_string()];
        let params = IndexMap::from([
            (
                "active_dimension".to_string(),
                ScalarValue::Utf8(Some("b".to_string())),
            ),
            ("display_x".to_string(), ScalarValue::Float64(Some(75.0))),
        ]);
        let overrides = resolve_display_state(
            Some(&ParallelDisplayState::active_axis(
                "active_dimension",
                "display_x",
            )),
            &params,
            &ids,
        )
        .expect("display state")
        .expect("override");
        assert_eq!(overrides.get("b"), Some(&75.0));
    }

    #[test]
    fn display_state_ignores_null_or_missing_values() {
        let ids = vec!["a".to_string(), "b".to_string()];
        let params = IndexMap::from([(
            "active_dimension".to_string(),
            ScalarValue::Utf8(Some("b".to_string())),
        )]);
        assert!(
            resolve_display_state(
                Some(&ParallelDisplayState::active_axis(
                    "active_dimension",
                    "display_x",
                )),
                &params,
                &ids,
            )
            .expect("display state")
            .is_none()
        );
    }
}
