//! Simplified CSS parser for theme system

use crate::theme::css::types::{
    Declaration, RGBA, Rule, Specificity, Theme, ThemeError, ThemeValue, Unit,
};
use std::collections::{HashMap, HashSet};

/// Parse a CSS string into a Theme using simplified parsing
pub fn parse_css_simple(css: &str) -> Result<Theme, ThemeError> {
    let mut rules = Vec::new();
    let mut variables = HashMap::new();

    // Simple line-based parsing
    let mut current_selector = String::new();
    let mut current_declarations = Vec::new();
    let mut in_rule = false;

    for line in css.lines() {
        let line = line.trim();

        // Skip comments
        if line.starts_with("/*") || line.starts_with("//") || line.is_empty() {
            continue;
        }

        // Check for rule start
        if !in_rule && line.contains('{') {
            let parts: Vec<&str> = line.split('{').collect();
            current_selector = parts[0].trim().to_string();
            in_rule = true;

            // Handle inline declarations
            if parts.len() > 1 {
                let rest = parts[1];
                if rest.contains('}') {
                    // Complete rule on one line
                    let decl_part = rest.split('}').next().unwrap_or("");
                    parse_declarations(
                        decl_part,
                        &mut current_declarations,
                        &mut variables,
                        &current_selector,
                    );

                    if !current_selector.is_empty() {
                        rules.push(Rule {
                            selector: current_selector.clone(),
                            parsed_selector: None, // Will be set later
                            declarations: current_declarations.clone(),
                            specificity: calculate_simple_specificity(&current_selector),
                            source_line: None,
                        });
                    }

                    current_selector.clear();
                    current_declarations.clear();
                    in_rule = false;
                }
            }
        } else if in_rule && line.contains('}') {
            // End of rule
            let decl_part = line.split('}').next().unwrap_or("");
            if !decl_part.trim().is_empty() {
                parse_declarations(
                    decl_part,
                    &mut current_declarations,
                    &mut variables,
                    &current_selector,
                );
            }

            if !current_selector.is_empty() {
                rules.push(Rule {
                    selector: current_selector.clone(),
                    parsed_selector: None,
                    declarations: current_declarations.clone(),
                    specificity: calculate_simple_specificity(&current_selector),
                    source_line: None,
                });
            }

            current_selector.clear();
            current_declarations.clear();
            in_rule = false;
        } else if in_rule {
            // Declaration line
            parse_declarations(
                line,
                &mut current_declarations,
                &mut variables,
                &current_selector,
            );
        }
    }

    Ok(Theme {
        rules,
        variables,
        inherited_properties: default_inherited_properties(),
        source_map: None,
        base_font_size: 12.0,
        chart_width: None,
        chart_height: None,
    })
}

/// Parse declarations from a string
fn parse_declarations(
    text: &str,
    declarations: &mut Vec<Declaration>,
    variables: &mut HashMap<String, ThemeValue>,
    selector: &str,
) {
    for decl in text.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }

        if let Some((prop, val)) = decl.split_once(':') {
            let property = prop.trim().to_lowercase();
            let value_str = val.trim();

            // Parse the value
            let value = parse_value(value_str);

            // Handle shorthand properties
            if property == "padding" || property == "margin" {
                expand_box_shorthand(&property, value_str, declarations);
            } else if property == "gap" {
                expand_gap_shorthand(value_str, declarations);
            } else {
                declarations.push(Declaration {
                    property: property.clone(),
                    value: value.clone(),
                    important: value_str.ends_with("!important"),
                });

                // Store variables from :root
                if selector == ":root" && property.starts_with("--") {
                    variables.insert(property, value);
                }
            }
        }
    }
}

/// Parse a CSS value
fn parse_value(value_str: &str) -> ThemeValue {
    let value_str = value_str.trim().trim_end_matches("!important").trim();

    // Check for var()
    if value_str.starts_with("var(") && value_str.ends_with(')') {
        let inner = &value_str[4..value_str.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        let var_name = parts[0].trim().to_string();
        let fallback = if parts.len() > 1 {
            Some(Box::new(parse_value(parts[1].trim())))
        } else {
            None
        };
        return ThemeValue::Variable(var_name, fallback);
    }

    // Check for calc()
    if value_str.starts_with("calc(") && value_str.ends_with(')') {
        let inner = &value_str[5..value_str.len() - 1];
        return ThemeValue::Calc(inner.to_string());
    }

    // Check for color
    if let Some(color) = parse_color_value(value_str) {
        return ThemeValue::Color(color);
    }

    // Check for dimension
    if let Some((num, unit)) = parse_dimension(value_str) {
        return ThemeValue::Dimension(num, unit);
    }

    // Check for number
    if let Ok(num) = value_str.parse::<f64>() {
        return ThemeValue::Number(num);
    }

    // Check for none
    if value_str == "none" || value_str == "transparent" {
        return ThemeValue::None;
    }

    // Default to string/keyword
    ThemeValue::Keyword(value_str.to_string())
}

/// Parse a color value
fn parse_color_value(value: &str) -> Option<RGBA> {
    // Hex color
    if value.starts_with('#') {
        return parse_hex_color(value);
    }

    // RGB/RGBA function
    if value.starts_with("rgb(") || value.starts_with("rgba(") {
        let start = value.find('(')? + 1;
        let end = value.rfind(')')?;
        let params = &value[start..end];
        let parts: Vec<&str> = params.split(',').map(|s| s.trim()).collect();

        if parts.len() >= 3 {
            let r = parts[0].parse::<u8>().ok()?;
            let g = parts[1].parse::<u8>().ok()?;
            let b = parts[2].parse::<u8>().ok()?;
            let a = if parts.len() > 3 {
                (parts[3].parse::<f32>().ok()? * 255.0) as u8
            } else {
                255
            };
            return Some(RGBA::new(r, g, b, a));
        }
    }

    // Named colors
    parse_named_color(value)
}

/// Parse hex color
fn parse_hex_color(hex: &str) -> Option<RGBA> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            // Short form #RGB
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some(RGBA::new(r, g, b, 255))
        }
        6 => {
            // Full form #RRGGBB
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(RGBA::new(r, g, b, 255))
        }
        8 => {
            // With alpha #RRGGBBAA
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(RGBA::new(r, g, b, a))
        }
        _ => None,
    }
}

/// Parse named colors
fn parse_named_color(name: &str) -> Option<RGBA> {
    match name.to_lowercase().as_str() {
        "black" => Some(RGBA::new(0, 0, 0, 255)),
        "white" => Some(RGBA::new(255, 255, 255, 255)),
        "red" => Some(RGBA::new(255, 0, 0, 255)),
        "green" => Some(RGBA::new(0, 128, 0, 255)),
        "blue" => Some(RGBA::new(0, 0, 255, 255)),
        "yellow" => Some(RGBA::new(255, 255, 0, 255)),
        "cyan" => Some(RGBA::new(0, 255, 255, 255)),
        "magenta" => Some(RGBA::new(255, 0, 255, 255)),
        "gray" | "grey" => Some(RGBA::new(128, 128, 128, 255)),
        "transparent" | "none" => Some(RGBA::new(0, 0, 0, 0)),
        _ => None,
    }
}

/// Parse dimension value
fn parse_dimension(value: &str) -> Option<(f64, Unit)> {
    if value.ends_with("px") {
        let num = value[..value.len() - 2].trim().parse().ok()?;
        Some((num, Unit::Px))
    } else if value.ends_with("rem") {
        let num = value[..value.len() - 3].trim().parse().ok()?;
        Some((num, Unit::Rem))
    } else if value.ends_with("em") {
        let num = value[..value.len() - 2].trim().parse().ok()?;
        Some((num, Unit::Em))
    } else if value.ends_with('%') {
        let num = value[..value.len() - 1].trim().parse().ok()?;
        Some((num, Unit::Percent))
    } else if value.ends_with("vw") {
        let num = value[..value.len() - 2].trim().parse().ok()?;
        Some((num, Unit::Vw))
    } else if value.ends_with("vh") {
        let num = value[..value.len() - 2].trim().parse().ok()?;
        Some((num, Unit::Vh))
    } else {
        None
    }
}

/// Expand box model shorthand (padding/margin)
fn expand_box_shorthand(property: &str, value: &str, declarations: &mut Vec<Declaration>) {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let values = match parts.len() {
        1 => vec![parts[0], parts[0], parts[0], parts[0]],
        2 => vec![parts[0], parts[1], parts[0], parts[1]],
        3 => vec![parts[0], parts[1], parts[2], parts[1]],
        4 => vec![parts[0], parts[1], parts[2], parts[3]],
        _ => return,
    };

    let props = ["-top", "-right", "-bottom", "-left"];
    for (i, prop_suffix) in props.iter().enumerate() {
        declarations.push(Declaration {
            property: format!("{}{}", property, prop_suffix),
            value: parse_value(values[i]),
            important: false,
        });
    }
}

/// Expand gap shorthand
fn expand_gap_shorthand(value: &str, declarations: &mut Vec<Declaration>) {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let row_gap = parse_value(parts[0]);
    let column_gap = if parts.len() > 1 {
        parse_value(parts[1])
    } else {
        row_gap.clone()
    };

    declarations.push(Declaration {
        property: "row-gap".to_string(),
        value: row_gap,
        important: false,
    });
    declarations.push(Declaration {
        property: "column-gap".to_string(),
        value: column_gap,
        important: false,
    });
}

/// Calculate simple specificity based on selector
fn calculate_simple_specificity(selector: &str) -> Specificity {
    let mut element_count = 0;

    // Count IDs
    let id_count = selector.matches('#').count() as u32;

    // Count classes and pseudo-classes
    let mut class_count = selector.matches('.').count() as u32;
    class_count += selector.matches(':').count() as u32;

    // Count element selectors (simplified)
    let parts: Vec<&str> = selector.split_whitespace().collect();
    for part in parts {
        if !part.starts_with('#')
            && !part.starts_with('.')
            && !part.starts_with(':')
            && !part.is_empty()
        {
            element_count += 1;
        }
    }

    Specificity(0, id_count, class_count, element_count)
}

/// Get default inherited properties
fn default_inherited_properties() -> HashSet<String> {
    let mut props = HashSet::new();

    // Typography
    props.insert("font-family".to_string());
    props.insert("font-size".to_string());
    props.insert("font-weight".to_string());
    props.insert("font-style".to_string());
    props.insert("line-height".to_string());
    props.insert("letter-spacing".to_string());
    props.insert("text-align".to_string());
    props.insert("text-transform".to_string());

    // Colors
    props.insert("color".to_string());

    // Visibility
    props.insert("visibility".to_string());

    props
}
