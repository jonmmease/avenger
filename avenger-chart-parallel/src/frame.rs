use std::collections::HashMap;

use crate::coord::ParallelTransformDimension;

#[derive(Clone, Debug, PartialEq)]
pub struct ParallelAxisSlot {
    pub id: String,
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
    let ordered = ordered_dimensions(dimensions, order);
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
                scale_name: dimension.id.clone(),
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

fn ordered_dimensions<'a>(
    dimensions: &'a [ParallelTransformDimension],
    order: Option<&[String]>,
) -> Vec<&'a ParallelTransformDimension> {
    let Some(order) = order else {
        return dimensions.iter().collect();
    };
    order
        .iter()
        .filter_map(|id| dimensions.iter().find(|dimension| dimension.id == *id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(ids: &[&str]) -> Vec<ParallelTransformDimension> {
        ids.iter()
            .map(|id| ParallelTransformDimension {
                id: (*id).to_string(),
                generated_channel: format!("generated_{id}"),
            })
            .collect()
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
}
