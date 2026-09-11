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

fn math_spacing_for_items(
    left: SimpleMathClass,
    right: SimpleMathClass,
    (left_size, left_level): (f32, u8),
    (right_size, right_level): (f32, u8),
) -> f32 {
    use SimpleMathClass::*;
    match (left, right) {
        (_, Punctuation) => 0.0,
        (Punctuation, _) if left_level == 0 => THIN_EM * left_size,
        (Opening, _) | (_, Closing) => 0.0,
        (Relation, Relation) => 0.0,
        (Relation, _) if left_level == 0 => THICK_EM * left_size,
        (_, Relation) if right_level == 0 => THICK_EM * right_size,
        (Binary, _) if left_level == 0 => MEDIUM_EM * left_size,
        (_, Binary) if right_level == 0 => MEDIUM_EM * right_size,
        (Large, Opening | Fence) => 0.0,
        (Large, _) => THIN_EM * left_size,
        (_, Large) => THIN_EM * right_size,
        _ => 0.0,
    }
}

const THIN_EM: f32 = 1.0 / 6.0;
const MEDIUM_EM: f32 = 2.0 / 9.0;
const THICK_EM: f32 = 5.0 / 18.0;
