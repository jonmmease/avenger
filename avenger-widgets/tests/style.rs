//! Disabled paint and widget names.

use avenger_widgets::{Checkbox, Paint, Widgets};

/// A blend moves a color's red, green and blue toward the background and keeps its alpha, so an
/// opaque color stays opaque and a transparent one stays transparent.
#[test]
fn a_blend_keeps_alpha() {
    let paint = Paint {
        fill: [0.0, 0.0, 0.0, 1.0],
        border: [0.25, 0.5, 0.75, 0.0],
        foreground: [1.0, 1.0, 1.0, 1.0],
    };
    let blended = paint.blend([1.0, 1.0, 1.0, 1.0], 0.5);
    assert_eq!(blended.fill, [0.5, 0.5, 0.5, 1.0]);
    assert_eq!(blended.border, [0.625, 0.75, 0.875, 0.0]);
    assert_eq!(blended.foreground, [1.0, 1.0, 1.0, 1.0]);
}

#[test]
#[should_panic(expected = "two widgets are named \"grid\"")]
fn two_widgets_cannot_share_a_name() {
    let _ = Widgets::<()>::new("controls", 10)
        .with(Checkbox::new("grid", "Grid"))
        .with(Checkbox::new("grid", "Grid lines"));
}
