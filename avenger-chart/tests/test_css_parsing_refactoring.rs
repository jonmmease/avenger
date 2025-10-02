//! Test the CSS parsing refactoring

use avenger_chart::theme::css::CssTheme;
use avenger_chart::theme::{ThemeContext, ThemeValue};

#[test]
fn test_always_parse_as_list() {
    // Test that single values work
    let css = r#".test { fill: red; }"#;
    let theme = CssTheme::from_css(css).unwrap();
    let ctx = ThemeContext::new("element").with_class("test");
    let fill = theme.query(&ctx, "fill");

    println!("Single value fill: {:?}", fill);
    assert!(matches!(fill, ThemeValue::Color(_)));

    // Test that multiple values create a list
    let css = r#".test { fill-discrete: red, blue, green; }"#;
    let theme = CssTheme::from_css(css).unwrap();
    let ctx = ThemeContext::new("element").with_class("test");
    let fill = theme.query(&ctx, "fill-discrete");

    println!("List fill-discrete: {:?}", fill);
    assert!(matches!(fill, ThemeValue::List(_)));
}

#[test]
fn test_color_parsing() {
    let css = r#"
        .test {
            color1: red;
            color2: "red";
            color3: foobar;
        }
    "#;

    let theme = CssTheme::from_css(css).unwrap();
    let ctx = ThemeContext::new("element").with_class("test");

    // Bare identifier that is a color becomes Color
    let color1 = theme.query(&ctx, "color1");
    println!("color1 (red): {:?}", color1);
    assert!(matches!(color1, ThemeValue::Color(c) if c.red == 255 && c.green == 0 && c.blue == 0));

    // Quoted strings stay as strings (no longer parse as colors)
    let color2 = theme.query(&ctx, "color2");
    println!("color2 (\"red\"): {:?}", color2);
    assert!(matches!(color2, ThemeValue::String(s) if s == "red"));

    // Non-color identifier stays as string
    let color3 = theme.query(&ctx, "color3");
    println!("color3 (foobar): {:?}", color3);
    assert!(matches!(color3, ThemeValue::String(s) if s == "foobar"));
}
