//! Relative luminance for sRGB colors.
//!
//! https://www.w3.org/TR/WCAG21/#dfn-relative-luminance

#[inline]
fn srgb_to_linear(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

/// Calculate relative luminance from normalized sRGB components.
pub fn relative_luminance_srgb(r: f32, g: f32, b: f32) -> f32 {
    let r_linear = srgb_to_linear(r);
    let g_linear = srgb_to_linear(g);
    let b_linear = srgb_to_linear(b);

    // Apply weighted sum based on human eye sensitivity
    0.2126 * r_linear + 0.7152 * g_linear + 0.0722 * b_linear
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_luminance_matches_reference_values() {
        for (rgb, expected) in [
            ([0.0, 0.0, 0.0], 0.0),
            ([1.0, 1.0, 1.0], 1.0),
            ([1.0, 0.0, 0.0], 0.2126),
            ([0.0, 1.0, 0.0], 0.7152),
            ([0.0, 0.0, 1.0], 0.0722),
            ([0.03, 0.03, 0.03], 0.0023219814),
            ([0.5, 0.5, 0.5], 0.21404114),
        ] {
            let actual = relative_luminance_srgb(rgb[0], rgb[1], rgb[2]);
            assert!(
                (actual - expected).abs() < 1e-6,
                "{rgb:?}: {actual} != {expected}"
            );
        }
    }
}
