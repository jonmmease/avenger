//! Test how CSS handles color values vs strings

use avenger_chart::theme::Theme;
use avenger_chart::theme::{ThemeContext, ThemeValue};
use indexmap::IndexMap;

#[test]
fn test_color_parsing_behavior() {
    // Test different ways of specifying colors in CSS
    let css = r#"
        /* Bare color keywords become Color values */
        .test1 { fill: red; }

        /* Quoted strings remain strings */
        .test2 { fill: "red"; }

        /* Hex colors become Color values */
        .test3 { fill: #ff0000; }

        /* RGB functions become Color values */
        .test4 { fill: rgb(255, 0, 0); }

        /* Non-color identifiers remain strings */
        .test5 { fill: foobar; }

        /* CSS properties that might accept both */
        .test6 { font-family: Arial; }
        .test7 { font-family: "Arial"; }
    "#;

    let theme = Theme::from_css(css).unwrap();

    // Test bare color keyword
    let ctx1 = ThemeContext::new("element", IndexMap::new()).with_class("test1");
    let fill1 = theme.query(&ctx1, "fill");
    println!("bare 'red': {:?}", fill1);
    assert!(
        matches!(fill1, Some(ThemeValue::Color(_))),
        "Bare color keyword should parse as Color"
    );

    // Test quoted string
    let ctx2 = ThemeContext::new("element", IndexMap::new()).with_class("test2");
    let fill2 = theme.query(&ctx2, "fill");
    println!("quoted \"red\": {:?}", fill2);
    assert!(
        matches!(fill2, Some(ThemeValue::String(s)) if s == "red"),
        "Quoted string should remain String"
    );

    // Test hex color
    let ctx3 = ThemeContext::new("element", IndexMap::new()).with_class("test3");
    let fill3 = theme.query(&ctx3, "fill");
    println!("hex #ff0000: {:?}", fill3);
    assert!(
        matches!(fill3, Some(ThemeValue::Color(_))),
        "Hex color should parse as Color"
    );

    // Test RGB function
    let ctx4 = ThemeContext::new("element", IndexMap::new()).with_class("test4");
    let fill4 = theme.query(&ctx4, "fill");
    println!("rgb(255,0,0): {:?}", fill4);
    assert!(
        matches!(fill4, Some(ThemeValue::Color(_))),
        "RGB function should parse as Color"
    );

    // Test non-color identifier
    let ctx5 = ThemeContext::new("element", IndexMap::new()).with_class("test5");
    let fill5 = theme.query(&ctx5, "fill");
    println!("bare 'foobar': {:?}", fill5);
    assert!(
        matches!(fill5, Some(ThemeValue::String(s)) if s == "foobar"),
        "Non-color identifier should remain String"
    );

    // Test font-family with bare identifier
    let ctx6 = ThemeContext::new("element", IndexMap::new()).with_class("test6");
    let font6 = theme.query(&ctx6, "font-family");
    println!("bare 'Arial': {:?}", font6);
    assert!(
        matches!(font6, Some(ThemeValue::String(s)) if s == "Arial"),
        "Font name should remain String"
    );

    // Test font-family with quoted string
    let ctx7 = ThemeContext::new("element", IndexMap::new()).with_class("test7");
    let font7 = theme.query(&ctx7, "font-family");
    println!("quoted \"Arial\": {:?}", font7);
    assert!(
        matches!(font7, Some(ThemeValue::String(s)) if s == "Arial"),
        "Quoted font name should remain String"
    );
}

#[test]
fn test_css_spec_behavior() {
    // According to CSS spec:
    // - Bare identifiers that are color keywords are treated as colors
    // - Quoted strings are always strings
    // - This is property-agnostic - the parser doesn't know what property expects

    let css = r#"
        /* CSS doesn't care what property expects - parsing is context-free */
        .weird {
            /* These will all parse as colors even though font-family doesn't expect colors */
            font-family: red;

            /* This stays a string because it's quoted */
            color: "red";
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let ctx = ThemeContext::new("element", IndexMap::new()).with_class("weird");

    // Font-family with color keyword - becomes a Color!
    let font = theme.query(&ctx, "font-family");
    println!("font-family: red => {:?}", font);
    assert!(
        matches!(font, Some(ThemeValue::Color(_))),
        "Color keyword always parses as Color, even for font-family"
    );

    // Color with quoted string - stays a String!
    let color = theme.query(&ctx, "color");
    println!("color: \"red\" => {:?}", color);
    assert!(
        matches!(color, Some(ThemeValue::String(s)) if s == "red"),
        "Quoted string always stays String, even for color property"
    );
}

#[test]
fn test_practical_implications() {
    // What this means for users:
    // 1. If you want a literal string "red", use quotes: "red"
    // 2. If you want the color red, use bare: red
    // 3. The property name doesn't affect parsing

    let css = r#"
        mark {
            /* This will be a color */
            fill: steelblue;

            /* This will be a string (though unusual) */
            stroke: "steelblue";

            /* For non-color properties, quotes ensure strings */
            shape: "circle";

            /* Without quotes, still a string if not a color keyword */
            shape2: circle;
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new("mark", IndexMap::new());

    let fill = theme.query(&ctx, "fill");
    assert!(matches!(fill, Some(ThemeValue::Color(_))));

    let stroke = theme.query(&ctx, "stroke");
    assert!(matches!(stroke, Some(ThemeValue::String(_))));

    let shape = theme.query(&ctx, "shape");
    assert!(matches!(shape, Some(ThemeValue::String(s)) if s == "circle"));

    let shape2 = theme.query(&ctx, "shape2");
    assert!(matches!(shape2, Some(ThemeValue::String(s)) if s == "circle"));
}
