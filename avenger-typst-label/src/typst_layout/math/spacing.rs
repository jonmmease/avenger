fn resolved_left_class(
    previous: Option<SimpleMathClass>,
    class: SimpleMathClass,
) -> SimpleMathClass {
    if class == SimpleMathClass::Vary
        && previous.is_some_and(|prev| {
            matches!(
                prev,
                SimpleMathClass::Normal
                    | SimpleMathClass::Alphabetic
                    | SimpleMathClass::Closing
                    | SimpleMathClass::Fence
            )
        })
    {
        SimpleMathClass::Binary
    } else {
        class
    }
}

#[cfg(test)]
fn math_spacing(left: SimpleMathClass, right: SimpleMathClass, font_size: f32) -> f32 {
    math_spacing_for_level(left, right, font_size, 0)
}

#[cfg(test)]
fn math_spacing_for_level(
    left: SimpleMathClass,
    right: SimpleMathClass,
    font_size: f32,
    script_level: u8,
) -> f32 {
    math_spacing_for_items(
        left,
        right,
        (font_size, script_level),
        (font_size, script_level),
    )
}

#[cfg(test)]
fn math_spacing_for_items(
    left: SimpleMathClass,
    right: SimpleMathClass,
    left_item: (f32, u8),
    right_item: (f32, u8),
) -> f32 {
    math_spacing_rule(left, right, left_item, right_item).unwrap_or(0.0)
}

/// The class rule for the gap between two items, or `None` when no rule applies. Only then may
/// an explicit space next to a spaced item survive.
///
/// upstream: crates/typst-library/src/math/ir/process.rs::spacing @ c98e910
fn math_spacing_rule(
    left: SimpleMathClass,
    right: SimpleMathClass,
    (left_size, left_level): (f32, u8),
    (right_size, right_level): (f32, u8),
) -> Option<f32> {
    use SimpleMathClass::*;
    match (left, right) {
        (_, Punctuation) => Some(0.0),
        (Punctuation, _) if left_level == 0 => Some(THIN_EM * left_size),
        (Opening, _) | (_, Closing) => Some(0.0),
        (Relation, Relation) => Some(0.0),
        (Relation, _) if left_level == 0 => Some(THICK_EM * left_size),
        (_, Relation) if right_level == 0 => Some(THICK_EM * right_size),
        (Binary, _) if left_level == 0 => Some(MEDIUM_EM * left_size),
        (_, Binary) if right_level == 0 => Some(MEDIUM_EM * right_size),
        (Large, Opening | Fence) => Some(0.0),
        (Large, _) => Some(THIN_EM * left_size),
        (_, Large) => Some(THIN_EM * right_size),
        _ => None,
    }
}

const THIN_EM: f32 = 1.0 / 6.0;
const MEDIUM_EM: f32 = 2.0 / 9.0;
const THICK_EM: f32 = 5.0 / 18.0;
