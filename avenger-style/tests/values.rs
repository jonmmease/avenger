//! The grammars of the properties: the forms that each kind of value accepts and rejects.

use avenger_style::{
    values::{Calc, Category, Unit, UnitSizes},
    Color, Property, Value,
};

#[track_caller]
fn accepts(property: Property, texts: &[&str]) {
    for text in texts {
        assert!(
            property.parse_value(text).is_some(),
            "`{}` takes `{text}`",
            property.name()
        );
    }
}

#[track_caller]
fn rejects(property: Property, texts: &[&str]) {
    for text in texts {
        assert!(
            property.parse_value(text).is_none(),
            "`{}` rejects `{text}`",
            property.name()
        );
    }
}

fn numeric(property: Property, text: &str) -> Calc {
    match property.parse_value(text) {
        Some(Value::Numeric(calc)) => calc,
        other => panic!("`{text}` is numeric, got {other:?}"),
    }
}

#[test]
fn every_property_parses_its_initial_value() {
    for property in Property::ALL {
        assert!(
            property.parse_value(property.initial()).is_some(),
            "`{}` parses its initial value `{}`",
            property.name(),
            property.initial()
        );
        assert!(
            !property.elements().is_empty(),
            "`{}` applies to some element",
            property.name()
        );
    }
}

#[test]
fn lengths_take_pixels_rem_em_and_calc() {
    accepts(
        Property::FontSize,
        &[
            "12px",
            "12",
            "1.1rem",
            "2em",
            "calc(1rem + 2px)",
            "calc((10px - 2px) / 2)",
        ],
    );
    rejects(
        Property::FontSize,
        &[
            "12pt",
            "3cqw",
            "10vw",
            "red",
            "-1px",
            "calc(2px * 3px)",
            "calc(10px + 5deg)",
        ],
    );
    let sizes = UnitSizes {
        rem: 10.0,
        em: 12.0,
    };
    assert_eq!(
        numeric(Property::FontSize, "calc(1.1rem + 2px)").resolve(&sizes),
        13.0
    );
    assert_eq!(numeric(Property::FontSize, "2em").resolve(&sizes), 24.0);
    assert_eq!(
        numeric(Property::FontSize, "12").category(),
        Some(Category::Number),
        "a number without a unit is a length in pixels"
    );
}

#[test]
fn numbers_angles_integers_and_percentages() {
    accepts(Property::Opacity, &["0", "0.5", "1", "calc(1 / 2)"]);
    rejects(Property::Opacity, &["1.5", "-0.1", "50%", "2px"]);
    // A value out of range is valid in `calc()`, which the cascade clamps, as in CSS.
    accepts(Property::Opacity, &["calc(1.5)", "calc(-1 + 0)"]);
    accepts(
        Property::LabelAngle,
        &[
            "45",
            "45deg",
            "-90",
            "calc(360deg / 8)",
            "1rad",
            "0.25turn",
            "100grad",
        ],
    );
    rejects(Property::LabelAngle, &["45px", "1rem"]);
    let sizes = UnitSizes::default();
    for (text, degrees) in [
        ("0.25turn", 90.0),
        ("100grad", 90.0),
        ("3.14159265rad", 180.0),
    ] {
        let resolved = numeric(Property::LabelAngle, text).resolve(&sizes);
        assert!(
            (resolved - degrees).abs() < 1e-3,
            "{text} is {resolved} degrees"
        );
    }
    accepts(Property::Columns, &["3", "auto", "none"]);
    rejects(Property::Columns, &["0", "1.5", "-2"]);
    accepts(Property::Size, &["30", "70%"]);
    assert_eq!(
        numeric(Property::Size, "70%"),
        Calc::Leaf(70.0, Unit::Percent)
    );
}

#[test]
fn colors_take_css_color_4_mixes_contrast_and_system_colors() {
    accepts(
        Property::Color,
        &[
            "#4c78a8",
            "#4c78a880",
            "steelblue",
            "transparent",
            "currentColor",
            "rgb(76 120 168 / 50%)",
            "rgba(76, 120, 168, 0.5)",
            "hsl(120 50% 50%)",
            "hwb(120 10% 10%)",
            "lab(50 20 -30)",
            "oklab(0.6 0.1 -0.1)",
            "oklch(70% 0.1 calc(100 * 2))",
            "color(srgb 0.3 0.5 0.7)",
            "color-mix(in srgb, white 30%, black)",
            "color-mix(in oklch longer hue, red, blue 40%)",
            "contrast-color(#1e1e2e)",
            "Canvas",
            "canvastext",
        ],
    );
    rejects(
        Property::Color,
        &[
            "colour",
            "color-mix(in xyz, red, blue)",
            "color-mix(in srgb longer hue, red, blue)",
            "color-mix(in srgb, red 120%, blue)",
            "color(display-p3 1 0 0)",
            "12px",
        ],
    );
    let Some(Value::Color(Color::Absolute { color: green, .. })) =
        Property::Color.parse_value("hsl(120 100% 50%)")
    else {
        panic!("an absolute color")
    };
    let [r, g, b, a] = green.to_rgba();
    assert!(r.abs() < 1e-3 && (g - 1.0).abs() < 1e-3 && b.abs() < 1e-3 && (a - 1.0).abs() < 1e-3);
    let Some(Value::Color(mix)) =
        Property::Color.parse_value("color-mix(in srgb, currentColor 30%, white)")
    else {
        panic!("a mix")
    };
    assert!(mix.reads_current_color());
}

#[test]
fn keywords_lists_and_the_values_of_scales() {
    accepts(Property::TextAlign, &["start", "center", "END", "auto"]);
    rejects(Property::TextAlign, &["left", "justify"]);
    accepts(
        Property::LabelPosition,
        &["inside", "top right bottom left"],
    );
    rejects(Property::LabelPosition, &["center", ""]);
    accepts(
        Property::Scheme,
        &[
            "auto",
            "tableau10",
            "blues 5",
            "blueorange reverse",
            "blues 7 reverse",
        ],
    );
    rejects(Property::Scheme, &["5", "blues reverse 5"]);
    accepts(
        Property::Range,
        &[
            "auto",
            "#e69f00, #56b4e9",
            "circle, square, triangle-up",
            "1 0, 4 2, 2 1",
            "0.3, 0.8",
        ],
    );
    accepts(Property::Unknown, &["auto", "gray", "#888"]);
    accepts(Property::StrokeDash, &["none", "4 2", "1 2 4 2"]);
    rejects(Property::StrokeDash, &["4, 2", "-1 2"]);
    accepts(Property::FontWeight, &["normal", "bold", "300", "700"]);
    rejects(Property::FontWeight, &["heavy", "0", "1001"]);
    accepts(Property::AspectRatio, &["auto", "1.5", "16 / 9"]);
    accepts(Property::Shape, &["circle", "\"M0,0 L1,1 Z\""]);
    accepts(Property::Pattern, &["none", "\"stripes 45 8 2\""]);
    rejects(Property::Pattern, &["stripes", "\"zigzag\""]);
    accepts(Property::LabelFlush, &["true", "false", "4"]);
    accepts(Property::Display, &["none", "block", "flex"]);
    accepts(Property::Separator, &["\" · \""]);

    let Some(Value::List(families)) =
        Property::FontFamily.parse_value("Lato, \"Helvetica Neue\", Open Sans, sans-serif")
    else {
        panic!("a list of families")
    };
    let families: Vec<&Value> = families.iter().collect();
    assert_eq!(
        families,
        [
            &Value::String("Lato".to_string()),
            &Value::String("Helvetica Neue".to_string()),
            &Value::String("Open Sans".to_string()),
            &Value::String("sans-serif".to_string())
        ]
    );
}

/// The components, the alpha, and the missing components of an absolute color.
#[track_caller]
fn absolute(text: &str) -> ([f32; 4], [bool; 4]) {
    match Property::Color.parse_value(text) {
        Some(Value::Color(Color::Absolute { color, missing })) => {
            let [c0, c1, c2] = color.components;
            ([c0, c1, c2, color.alpha], missing)
        }
        other => panic!("`{text}` is an absolute color, got {other:?}"),
    }
}

#[track_caller]
fn close(actual: [f32; 4], expected: [f32; 4]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-4, "{actual:?} is {expected:?}");
    }
}

#[test]
fn colors_keep_their_missing_components_and_normalize_as_css_does() {
    // `none` draws as zero and stays missing, for `color-mix()`.
    let (components, missing) = absolute("oklch(0.5 0.2 none)");
    close(components, [0.5, 0.2, 0.0, 1.0]);
    assert_eq!(missing, [false, false, true, false]);
    assert_eq!(
        absolute("color(srgb none 0 0)").1,
        [true, false, false, false]
    );
    assert_eq!(
        absolute("hsl(none 100% 50% / none)").1,
        [true, false, false, true]
    );

    // Chroma and saturation are at least 0, lightness is within its range, and alpha is from 0
    // to 1.
    close(absolute("oklch(0.5 -0.2 30)").0, [0.5, 0.0, 30.0, 1.0]);
    close(absolute("lch(150 -5 30)").0, [100.0, 0.0, 30.0, 1.0]);
    close(absolute("oklab(-0.5 0.1 0.1)").0, [0.0, 0.1, 0.1, 1.0]);
    close(absolute("hsl(0 -50% 50%)").0, [0.0, 0.0, 50.0, 1.0]);
    close(absolute("rgb(0 0 0 / 2)").0, [0.0, 0.0, 0.0, 1.0]);

    // `rgb()` keeps fractional channels and `none`, and clamps channels to their range.
    close(absolute("rgb(127.5 0 300)").0, [0.5, 0.0, 1.0, 1.0]);
    let (components, missing) = absolute("rgb(none 51 100% / 50%)");
    close(components, [0.0, 0.2, 1.0, 0.5]);
    assert_eq!(missing, [true, false, false, false]);
    close(absolute("rgba(255, 0, 0, 0.5)").0, [1.0, 0.0, 0.0, 0.5]);
    close(absolute("rgb(100%, 0%, 0%)").0, [1.0, 0.0, 0.0, 1.0]);
    rejects(
        Property::Color,
        &[
            "rgb(none, 0, 0)",
            "rgb(1 2)",
            "rgb(1, 2 3)",
            "rgb(100%, 0, 0)",
            "rgb(255, 0%, 0%)",
        ],
    );
    // The modern syntax mixes numbers and percentages.
    close(absolute("rgb(100% 0 0%)").0, [1.0, 0.0, 0.0, 1.0]);

    // Hues take every angle unit.
    close(
        absolute("hsl(0.5turn 100% 50%)").0,
        [180.0, 100.0, 50.0, 1.0],
    );
    close(
        absolute("hsl(200grad 100% 50%)").0,
        [180.0, 100.0, 50.0, 1.0],
    );
}
