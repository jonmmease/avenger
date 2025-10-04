# Phase 9: Relative Color Syntax Implementation Plan

**Date:** 2025-10-04
**Status:** Planning
**Foundation:** Phases 1-8 Complete (channel keyword infrastructure in place)

## Executive Summary

This document provides a complete, architecture-aligned plan to implement CSS Color Level 5 Relative Color Syntax in Avenger's theme system. The plan is based on a thorough review of Mozilla's Servo/Stylo implementation and adapted to fit Avenger's existing color and calc infrastructure.

**Key Insight from Servo:** Relative color syntax is NOT implemented at the color function level, but rather through a **component-based architecture** where each color component can be:
1. A literal value (number, percentage, angle)
2. The "none" keyword
3. A channel keyword (r, g, b, l, c, h, etc.)
4. A calc() expression (which may contain channel keywords)

## Architecture Analysis

### Servo's Approach (Reference Implementation)

**File Structure:**
- `color/component.rs` - Generic `ColorComponent<ValueType>` enum
- `color/color_function.rs` - Color functions store components, not resolved values
- `color/parsing.rs` - Parses "from <origin>" and delegates to component parsing
- `color/mod.rs` - `AbsoluteColor::get_component_by_channel_keyword()`

**Key Design Patterns:**

1. **ColorComponent Enum** (from `component.rs`):
```rust
pub enum ColorComponent<ValueType> {
    None,                           // "none" keyword
    Value(ValueType),               // Absolute value (e.g., 0.5, 180deg)
    ChannelKeyword(ChannelKeyword), // Channel reference (e.g., l, h, r)
    Calc(Box<CalcNode>),           // calc() expression
    AlphaOmitted,                   // For default alpha
}
```

2. **ColorFunction Storage** (from `color_function.rs`):
```rust
pub enum ColorFunction<OriginColor> {
    Oklch(
        Optional<OriginColor>,              // origin color (from <color>)
        ColorComponent<NumberOrPercentage>, // lightness
        ColorComponent<NumberOrPercentage>, // chroma
        ColorComponent<NumberOrAngle>,      // hue
        ColorComponent<NumberOrPercentage>, // alpha
    ),
    // ... similar for Oklab, Lab, Lch, Hsl, Hwb, Rgb
}
```

3. **Resolution Flow** (from `component.rs:111-154`):
```rust
impl ColorComponent {
    pub fn resolve(&self, origin_color: Option<&AbsoluteColor>) -> Result<Option<ValueType>, ()> {
        match self {
            Self::Value(v) => Ok(Some(v.clone())),
            Self::ChannelKeyword(kw) => {
                // Extract component from origin color
                let value = origin_color?.get_component_by_channel_keyword(*kw)?;
                Ok(Some(ValueType::from_value(value.unwrap_or(0.0))))
            }
            Self::Calc(node) => {
                // Resolve calc with channel keyword substitution
                let resolved = node.resolve_map(|leaf| {
                    match leaf {
                        Leaf::ColorComponent(kw) => {
                            let value = origin_color?.get_component_by_channel_keyword(*kw)?;
                            Ok(Leaf::Number(value.unwrap_or(0.0)))
                        }
                        l => Ok(l.clone())
                    }
                })?;
                Ok(Some(ValueType::try_from_leaf(&resolved)?))
            }
            // ...
        }
    }
}
```

4. **Component Extraction** (from `mod.rs:457-524`):
```rust
impl AbsoluteColor {
    pub fn get_component_by_channel_keyword(&self, kw: ChannelKeyword) -> Result<Option<f32>, ()> {
        if kw == ChannelKeyword::Alpha {
            return Ok(self.alpha());
        }

        Ok(match (self.color_space, kw) {
            (ColorSpace::Oklch, ChannelKeyword::L) => self.c0(),
            (ColorSpace::Oklch, ChannelKeyword::C) => self.c1(),
            (ColorSpace::Oklch, ChannelKeyword::H) => self.c2(),
            (ColorSpace::Hsl, ChannelKeyword::H) => self.c0(),
            (ColorSpace::Hsl, ChannelKeyword::S) => self.c1(),
            (ColorSpace::Hsl, ChannelKeyword::L) => self.c2(),
            // ... for all color spaces
            _ => return Err(()),
        })
    }
}
```

### Avenger's Current Architecture

**Strengths:**
- ✅ Complete color space conversion (`color/convert.rs`)
- ✅ `AbsoluteColor` type with component storage
- ✅ CalcNode infrastructure with variable substitution
- ✅ ChannelKeyword enum and parsing

**Current Limitations:**
- ❌ Color functions return `CssRgba` immediately (no component preservation)
- ❌ No "from <origin>" parsing
- ❌ No component-based color representation
- ❌ CalcNode cannot be threaded through color resolution

**Example of Current Architecture:**
```rust
// Current: parse_oklch_function() in lab_color.rs
pub fn parse_oklch_function(args: &[ThemeValue]) -> Option<CssRgba> {
    let lightness = extract_number_or_percentage(&args[0], 1.0)?;
    let chroma = extract_number(&args[1])?;
    let hue = extract_number_or_angle(&args[2])?;
    let alpha = if args.len() > 3 { extract_alpha(&args[3])? } else { 1.0 };

    let color = AbsoluteColor::new(ColorSpace::Oklch, lightness, chroma, hue, alpha);
    Some(color.to_css_rgba()) // ← Immediately converts to RGBA
}
```

## Implementation Strategy

### Core Principle: Minimal Disruption

We will **NOT** rewrite Avenger's entire color system to match Servo's. Instead, we'll create a **parallel path** for relative colors that preserves the existing absolute color logic.

### Decision: Two-Phase Color Resolution

**Phase A: Parse Time** (for absolute colors - existing behavior)
- Parse color function → resolve to `CssRgba` → store in `ThemeValue::Color`

**Phase B: Theme Resolution Time** (for relative colors - new behavior)
- Parse color function → detect "from <origin>" → store components in new `ThemeValue::RelativeColor`
- During theme resolution: resolve origin → extract components → evaluate calc → build AbsoluteColor → convert to CssRgba

### Key Design Decision: Where to Store Components

**Option 1: New ThemeValue Variant** (RECOMMENDED)
```rust
pub enum ThemeValue {
    // Existing variants...
    Color(CssRgba),

    // New variant for relative colors
    RelativeColor {
        space: ColorSpace,
        origin: Box<ThemeValue>,           // The "from <color>" value
        components: Vec<ColorComponent>,   // L, C, H, alpha (may contain calc/keywords)
    },
}

pub enum ColorComponent {
    Literal(f64),
    None,
    ChannelKeyword(ChannelKeyword),
    Calc(CalcNode),
}
```

**Option 2: Extend Calc System** (Alternative)
```rust
// Store relative colors as calc expressions with color context
ThemeValue::Calc(Box<CalcNode>)

// CalcNode gains awareness of color space
pub enum CalcNode {
    // ...existing variants...
    ColorDerivation {
        space: ColorSpace,
        origin: Box<ThemeValue>,
        components: [Box<CalcNode>; 4], // L/C/H/alpha as calc nodes
    },
}
```

**Recommendation: Option 1** - cleaner separation, easier to understand, follows Servo's proven design.

## Implementation Plan

### Step 1: Add ColorComponent Type (NEW FILE)

**File:** `avenger-chart/src/theme/color_component.rs` (~200 lines)

```rust
use super::calc::{CalcNode, ChannelKeyword};
use crate::color::types::AbsoluteColor;
use indexmap::IndexMap;
use datafusion_common::ScalarValue;

/// A single color component that may contain channel keywords or calc expressions
#[derive(Debug, Clone, PartialEq)]
pub enum ColorComponent {
    /// A literal numeric value (0.5, 180.0, etc.)
    Literal(f64),

    /// The "none" keyword (not initially supported)
    None,

    /// A channel keyword reference (l, c, h, r, g, b, etc.)
    ChannelKeyword(ChannelKeyword),

    /// A calc() expression (may contain channel keywords)
    Calc(CalcNode),
}

impl ColorComponent {
    /// Resolve component to a concrete value with optional origin color
    pub fn resolve(
        &self,
        origin_color: Option<&AbsoluteColor>,
        params: &IndexMap<String, f64>,
        base_font_size: f32,
    ) -> Result<f64, String> {
        match self {
            ColorComponent::Literal(value) => Ok(*value),

            ColorComponent::None => {
                // "none" becomes 0.0 or origin value if available
                Ok(0.0)
            }

            ColorComponent::ChannelKeyword(keyword) => {
                let origin = origin_color.ok_or("Channel keyword requires origin color")?;
                let value = origin.get_component_by_channel_keyword(*keyword)?;
                Ok(value)
            }

            ColorComponent::Calc(node) => {
                // Substitute channel keywords in calc
                let with_keywords = node.substitute_channel_keywords(origin_color)?;

                // Resolve calc with params
                let resolved = with_keywords.resolve_with_params(params, base_font_size)?;

                // Extract numeric value
                resolved.as_number()
                    .ok_or_else(|| "Color component calc must resolve to number".to_string())
            }
        }
    }

    /// Parse from ThemeValue
    pub fn from_theme_value(value: &ThemeValue) -> Result<Self, String> {
        match value {
            ThemeValue::Number(n) => Ok(ColorComponent::Literal(*n)),
            ThemeValue::Percentage(p) => Ok(ColorComponent::Literal(p / 100.0)),
            ThemeValue::Angle(_, unit) => {
                let deg = unit.to_degrees(*value);
                Ok(ColorComponent::Literal(deg))
            }
            ThemeValue::Calc(node) => Ok(ColorComponent::Calc(*node.clone())),
            _ => Err(format!("Cannot convert {:?} to ColorComponent", value)),
        }
    }
}
```

### Step 2: Add Channel Keyword Substitution to CalcNode

**File:** `avenger-chart/src/theme/calc.rs` (~150 lines added)

```rust
impl CalcNode {
    /// Substitute channel keywords with values from origin color
    ///
    /// This is used during relative color syntax resolution to replace
    /// channel keywords (l, c, h, r, g, b, etc.) with actual component values.
    pub fn substitute_channel_keywords(
        &self,
        origin_color: Option<&AbsoluteColor>,
    ) -> Result<CalcNode, String> {
        match self {
            CalcNode::Leaf(CalcLeaf::ChannelKeyword(keyword)) => {
                let origin = origin_color
                    .ok_or("Channel keyword requires origin color context")?;

                let value = origin.get_component_by_channel_keyword(*keyword)?;

                // Convert to appropriate leaf based on keyword type
                // Hue keywords become angles, others become numbers
                let leaf = match keyword {
                    ChannelKeyword::H => {
                        CalcLeaf::Angle(value as f64, AngleUnit::Deg)
                    }
                    _ => CalcLeaf::Number(value as f64),
                };

                Ok(CalcNode::Leaf(leaf))
            }

            // Recursively process tree
            CalcNode::Negate(node) => {
                Ok(CalcNode::Negate(Box::new(
                    node.substitute_channel_keywords(origin_color)?
                )))
            }

            CalcNode::Sum(nodes) => {
                Ok(CalcNode::Sum(
                    nodes.iter()
                        .map(|n| n.substitute_channel_keywords(origin_color))
                        .collect::<Result<Vec<_>, _>>()?
                ))
            }

            CalcNode::Product(nodes) => {
                Ok(CalcNode::Product(
                    nodes.iter()
                        .map(|n| n.substitute_channel_keywords(origin_color))
                        .collect::<Result<Vec<_>, _>>()?
                ))
            }

            // Similar for all other node types...
            CalcNode::Min(nodes) => {
                Ok(CalcNode::Min(
                    nodes.iter()
                        .map(|n| n.substitute_channel_keywords(origin_color))
                        .collect::<Result<Vec<_>, _>>()?
                ))
            }

            CalcNode::Max(nodes) => {
                Ok(CalcNode::Max(
                    nodes.iter()
                        .map(|n| n.substitute_channel_keywords(origin_color))
                        .collect::<Result<Vec<_>, _>>()?
                ))
            }

            CalcNode::Clamp { min, center, max } => {
                Ok(CalcNode::Clamp {
                    min: Box::new(min.substitute_channel_keywords(origin_color)?),
                    center: Box::new(center.substitute_channel_keywords(origin_color)?),
                    max: Box::new(max.substitute_channel_keywords(origin_color)?),
                })
            }

            // Other nodes: abs, sign, trig, exponential, etc.
            // All follow the same pattern: recursively substitute in children

            _ => Ok(self.clone()),
        }
    }
}
```

### Step 3: Add Component Extraction to AbsoluteColor

**File:** `avenger-chart/src/color/types.rs` (~100 lines added)

```rust
impl AbsoluteColor {
    /// Extract a component value by channel keyword
    ///
    /// Returns the component value in the color's current color space.
    /// For rgb() with legacy syntax, values are in 0-255 range.
    /// For modern color spaces, values are normalized (0-1 for lightness, etc.)
    pub fn get_component_by_channel_keyword(
        &self,
        keyword: ChannelKeyword,
    ) -> Result<f32, String> {
        // Alpha is universal
        if matches!(keyword, ChannelKeyword::Alpha | ChannelKeyword::A) {
            return Ok(self.alpha);
        }

        match (self.color_space, keyword) {
            // Oklch: L (lightness), C (chroma), H (hue)
            (ColorSpace::Oklch, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Oklch, ChannelKeyword::C) => Ok(self.components[1]),
            (ColorSpace::Oklch, ChannelKeyword::H) => Ok(self.components[2]),

            // Oklab: L (lightness), A (green-red), B (blue-yellow)
            (ColorSpace::Oklab, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Oklab, ChannelKeyword::A) => Ok(self.components[1]),
            (ColorSpace::Oklab, ChannelKeyword::LabB) => Ok(self.components[2]),

            // Lch: L (lightness), C (chroma), H (hue)
            (ColorSpace::Lch, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Lch, ChannelKeyword::C) => Ok(self.components[1]),
            (ColorSpace::Lch, ChannelKeyword::H) => Ok(self.components[2]),

            // Lab: L (lightness), A (green-red), B (blue-yellow)
            (ColorSpace::Lab, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Lab, ChannelKeyword::A) => Ok(self.components[1]),
            (ColorSpace::Lab, ChannelKeyword::LabB) => Ok(self.components[2]),

            // HSL: H (hue), S (saturation), L (lightness)
            (ColorSpace::Hsl, ChannelKeyword::H) => Ok(self.components[0]),
            (ColorSpace::Hsl, ChannelKeyword::S) => Ok(self.components[1]),
            (ColorSpace::Hsl, ChannelKeyword::L) => Ok(self.components[2]),

            // HWB: H (hue), W (whiteness), B (blackness)
            (ColorSpace::Hwb, ChannelKeyword::H) => Ok(self.components[0]),
            (ColorSpace::Hwb, ChannelKeyword::W) => Ok(self.components[1]),
            (ColorSpace::Hwb, ChannelKeyword::BlacknessB) => Ok(self.components[2]),

            // sRGB: R, G, B (0-1 range in modern syntax)
            (ColorSpace::Srgb, ChannelKeyword::R) => Ok(self.components[0]),
            (ColorSpace::Srgb, ChannelKeyword::G) => Ok(self.components[1]),
            (ColorSpace::Srgb, ChannelKeyword::B) => Ok(self.components[2]),

            _ => Err(format!(
                "Invalid channel keyword {:?} for color space {:?}",
                keyword, self.color_space
            )),
        }
    }
}
```

### Step 4: Extend ThemeValue for Relative Colors

**File:** `avenger-chart/src/theme/value.rs` (~150 lines added)

```rust
use super::color_component::ColorComponent;

pub enum ThemeValue {
    // ... existing variants ...
    Color(CssRgba),

    /// Relative color derived from origin using channel keywords
    /// Example: oklch(from blue calc(l - 0.2) c h)
    RelativeColor {
        space: ColorSpace,
        origin: Box<ThemeValue>,
        lightness: ColorComponent,   // or c0
        component1: ColorComponent,  // chroma/a/saturation/whiteness/g
        component2: ColorComponent,  // hue/b/r
        alpha: ColorComponent,
    },
}

impl ThemeValue {
    /// Resolve to color with runtime parameters and origin color context
    pub fn as_color_with_params(
        &self,
        params: &IndexMap<String, ScalarValue>,
        base_font_size: f32,
    ) -> Option<CssRgba> {
        match self {
            ThemeValue::Color(rgba) => Some(*rgba),

            ThemeValue::RelativeColor {
                space,
                origin,
                lightness,
                component1,
                component2,
                alpha,
            } => {
                // 1. Resolve origin color
                let origin_rgba = origin.as_color_with_params(params, base_font_size)?;
                let mut origin_abs = AbsoluteColor::from_css_rgba(&origin_rgba);

                // 2. Convert origin to target color space
                origin_abs = origin_abs.to_color_space(*space);

                // 3. Convert params to f64
                let params_f64 = scalar_value_params_to_f64(params);

                // 4. Resolve each component
                let c0 = lightness.resolve(Some(&origin_abs), &params_f64, base_font_size).ok()?;
                let c1 = component1.resolve(Some(&origin_abs), &params_f64, base_font_size).ok()?;
                let c2 = component2.resolve(Some(&origin_abs), &params_f64, base_font_size).ok()?;
                let a = alpha.resolve(Some(&origin_abs), &params_f64, base_font_size).ok()?;

                // 5. Build derived color
                let derived = AbsoluteColor::new(*space, c0 as f32, c1 as f32, c2 as f32, a as f32);

                // 6. Convert to CssRgba
                Some(derived.to_css_rgba())
            }

            _ => None,
        }
    }
}
```

### Step 5: Update Color Function Parsers

**File:** `avenger-chart/src/theme/parser.rs` (~300 lines added/modified)

```rust
/// Parse oklch() with optional "from <origin>" syntax
fn parse_oklch_function<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &[&'static str],
) -> Result<ThemeValue, ParseError<'i, ()>> {
    parser.parse_nested_block(|parser| {
        // Try to parse "from <color>"
        let origin_color = try_parse_origin_color(parser, unsupported_units)?;

        if origin_color.is_some() {
            // Relative color syntax
            parse_oklch_relative(parser, origin_color.unwrap(), unsupported_units)
        } else {
            // Absolute color syntax (existing logic)
            parse_oklch_absolute(parser, unsupported_units)
        }
    })
}

/// Try to parse "from <color>" prefix
fn try_parse_origin_color<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &[&'static str],
) -> Result<Option<ThemeValue>, ParseError<'i, ()>> {
    // Try to match "from" keyword
    if parser.try_parse(|p| p.expect_ident_matching("from")).is_err() {
        return Ok(None);
    }

    // Parse the origin color
    let color = parse_color_value(parser, unsupported_units)?;
    Ok(Some(color))
}

/// Parse oklch() in relative syntax: oklch(from <origin> <L> <C> <H> [/ <alpha>])
fn parse_oklch_relative<'i, 't>(
    parser: &mut Parser<'i, 't>,
    origin: ThemeValue,
    unsupported_units: &[&'static str],
) -> Result<ThemeValue, ParseError<'i, ()>> {
    // Parse L component (can be literal, calc, or channel keyword)
    let lightness = parse_color_component(parser, unsupported_units)?;

    // Parse C component
    let chroma = parse_color_component(parser, unsupported_units)?;

    // Parse H component (may be angle or number)
    let hue = parse_color_component(parser, unsupported_units)?;

    // Parse optional alpha
    let alpha = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
        parse_color_component(parser, unsupported_units)?
    } else {
        // Default alpha: inherit from origin (handled during resolution)
        ColorComponent::ChannelKeyword(ChannelKeyword::Alpha)
    };

    Ok(ThemeValue::RelativeColor {
        space: ColorSpace::Oklch,
        origin: Box::new(origin),
        lightness,
        component1: chroma,
        component2: hue,
        alpha,
    })
}

/// Parse a color component (number, percentage, angle, calc, or channel keyword)
fn parse_color_component<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &[&'static str],
) -> Result<ColorComponent, ParseError<'i, ()>> {
    // Try calc expression first
    if let Ok(calc) = parser.try_parse(|p| parse_calc_value(p, unsupported_units)) {
        return Ok(ColorComponent::Calc(calc));
    }

    // Try channel keyword
    if let Ok(Token::Ident(ref ident)) = parser.try_parse(|p| p.next().cloned()) {
        if ident.eq_ignore_ascii_case("none") {
            return Ok(ColorComponent::None);
        }
        if let Some(keyword) = ChannelKeyword::from_ident(ident.as_ref()) {
            return Ok(ColorComponent::ChannelKeyword(keyword));
        }
    }

    // Try number, percentage, or angle
    let value = parse_number_or_percentage_or_angle(parser)?;
    Ok(ColorComponent::Literal(value))
}

/// Existing absolute oklch parser (no changes)
fn parse_oklch_absolute<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &[&'static str],
) -> Result<ThemeValue, ParseError<'i, ()>> {
    // Current implementation remains unchanged
    // ...
}
```

**Repeat for all color functions:**
- `parse_oklab_function()` - L, A, B
- `parse_lch_function()` - L, C, H
- `parse_lab_function()` - L, A, B
- `parse_hsl_function()` - H, S, L
- `parse_hwb_function()` - H, W, B
- `parse_rgb_function()` - R, G, B (with legacy syntax detection)

### Step 6: Update Theme Property Resolution

**File:** `avenger-chart/src/theme/theme.rs` (~50 lines modified)

```rust
impl Theme {
    /// Get background color for a context
    pub fn background_color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        self.query(context, "background-color")
            .and_then(|v| v.as_color_with_params(&context.params, self.get_base_font_size(&context.params)))
            .map(|c| [c.red as f32 / 255.0, c.green as f32 / 255.0, c.blue as f32 / 255.0, c.alpha as f32 / 255.0])
    }

    /// Get text color for a context
    pub fn text_color(&self, context: &ThemeContext) -> Option<[f32; 4]> {
        self.query(context, "color")
            .and_then(|v| v.as_color_with_params(&context.params, self.get_base_font_size(&context.params)))
            .map(|c| [c.red as f32 / 255.0, c.green as f32 / 255.0, c.blue as f32 / 255.0, c.alpha as f32 / 255.0])
    }

    // Similar for fill_color(), stroke_color(), etc.
}
```

## Testing Strategy

### Unit Tests (~200 lines)

**File:** `avenger-chart/tests/test_relative_colors.rs`

```rust
#[test]
fn test_oklch_relative_darken() {
    let css = r#"
        .dark {
            background: oklch(from blue calc(l - 0.2) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new(".dark");
    let bg = theme.background_color(&ctx).unwrap();

    // blue in oklch: l≈0.45, c≈0.31, h≈264
    // darkened: l≈0.25
    let derived = AbsoluteColor::from_srgb(bg[0], bg[1], bg[2], bg[3])
        .to_color_space(ColorSpace::Oklch);

    assert!((derived.components[0] - 0.25).abs() < 0.01, "Lightness should be ~0.25");
}

#[test]
fn test_oklch_relative_with_params() {
    let css = r#"
        .button {
            background: oklch(from var(--primary) calc(l - var(--darken)) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();

    let mut params = IndexMap::new();
    params.insert("primary".to_string(), /* blue color as scalar */);
    params.insert("darken".to_string(), ScalarValue::Float64(Some(0.2)));

    let ctx = ThemeContext::new(".button").with_params(params);
    let bg = theme.background_color(&ctx).unwrap();

    // Verify darkened by 0.2
}

#[test]
fn test_hsl_relative_rotate_hue() {
    let css = r#"
        .complementary {
            background: hsl(from var(--base) calc(h + 180deg) s l);
        }
    "#;

    // Test hue rotation for complementary color
}

#[test]
fn test_rgb_relative_adjust_red() {
    let css = r#"
        .adjusted {
            background: rgb(from blue calc(r + 50) g b);
        }
    "#;

    // Test RGB component adjustment
}
```

### Integration Tests (~150 lines)

**File:** `avenger-chart/tests/test_relative_colors_integration.rs`

```rust
#[test]
fn test_css_article_example() {
    // Test example from https://urre.me/writing/darken-or-lighten-with-css
    let css = r#"
        :root {
            --second-color: oklch(0.6 0.2 180deg);
        }

        .darkened {
            background: oklch(from var(--second-color) calc(l - 0.2) c h);
        }
    "#;

    let theme = Theme::from_css(css).unwrap();
    let ctx = ThemeContext::new(".darkened");
    let bg = theme.background_color(&ctx).unwrap();

    // Verify: original L=0.6, darkened L=0.4
}

#[test]
fn test_nested_relative_colors() {
    let css = r#"
        :root {
            --primary: oklch(0.6 0.2 200deg);
        }

        .light {
            background: oklch(from var(--primary) calc(l + 0.1) c h);
        }

        .extra-light {
            background: oklch(from var(--light) calc(l + 0.1) c h);
        }
    "#;

    // Test nested color derivation
}
```

### Visual Regression Tests (~100 lines)

**File:** `avenger-chart/tests/visual_tests/test_relative_colors.rs`

```rust
#[test]
fn test_color_palette_derivation() {
    // Generate color palette from single base color
    let css = r#"
        :root {
            --base: oklch(0.6 0.2 200deg);
        }

        .bg-primary { background: var(--base); }
        .bg-light   { background: oklch(from var(--base) calc(l + 0.2) c h); }
        .bg-dark    { background: oklch(from var(--base) calc(l - 0.2) c h); }
        .bg-muted   { background: oklch(from var(--base) l calc(c * 0.5) h); }
        .bg-complement { background: oklch(from var(--base) l c calc(h + 180deg)); }
    "#;

    // Render color swatches and compare to baseline
}
```

## Implementation Checklist

### Core Infrastructure (~500 lines)
- [ ] Create `color_component.rs` with `ColorComponent` enum and resolution
- [ ] Add `substitute_channel_keywords()` to `CalcNode` (all node types)
- [ ] Add `get_component_by_channel_keyword()` to `AbsoluteColor`
- [ ] Add `ThemeValue::RelativeColor` variant
- [ ] Add `as_color_with_params()` method to `ThemeValue`

### Parser Updates (~400 lines)
- [ ] Add `try_parse_origin_color()` helper
- [ ] Add `parse_color_component()` helper
- [ ] Update `parse_oklch_function()` with "from" detection
- [ ] Update `parse_oklab_function()` with "from" detection
- [ ] Update `parse_lch_function()` with "from" detection
- [ ] Update `parse_lab_function()` with "from" detection
- [ ] Update `parse_hsl_function()` with "from" detection
- [ ] Update `parse_hwb_function()` with "from" detection
- [ ] Update `parse_rgb_function()` with "from" detection (handle legacy)

### Theme Integration (~100 lines)
- [ ] Update `Theme.background_color()` to use `as_color_with_params()`
- [ ] Update `Theme.text_color()` to use `as_color_with_params()`
- [ ] Update `Theme.fill_color()` to use `as_color_with_params()`
- [ ] Update `Theme.stroke_color()` to use `as_color_with_params()`

### Testing (~450 lines)
- [ ] Unit tests for channel keyword extraction
- [ ] Unit tests for calc with channel keywords
- [ ] Integration tests for all color functions with relative syntax
- [ ] Integration tests with runtime parameters
- [ ] Visual regression tests for color derivation
- [ ] Error handling tests (invalid keywords, missing origin, etc.)

### Documentation (~100 lines)
- [ ] Update module-level docs in `calc.rs`
- [ ] Add examples to `color_component.rs`
- [ ] Update CLAUDE.md with relative color syntax examples
- [ ] Add inline documentation for all new public APIs

**Total Estimated LOC:** ~1,550 lines
**Total Estimated Effort:** 4-5 days for one developer

## Files to Create/Modify

### New Files (3)
1. `avenger-chart/src/theme/color_component.rs` - ColorComponent type (~200 lines)
2. `avenger-chart/tests/test_relative_colors.rs` - Unit tests (~200 lines)
3. `avenger-chart/tests/visual_tests/test_relative_colors.rs` - Visual tests (~100 lines)

### Modified Files (5)
1. `avenger-chart/src/theme/calc.rs` - Add channel keyword substitution (~150 lines added)
2. `avenger-chart/src/color/types.rs` - Add component extraction (~100 lines added)
3. `avenger-chart/src/theme/value.rs` - Add RelativeColor variant (~150 lines added)
4. `avenger-chart/src/theme/parser.rs` - Update color parsers (~400 lines added/modified)
5. `avenger-chart/src/theme/theme.rs` - Update property resolution (~50 lines modified)

## Success Criteria

1. ✅ All unit tests pass
2. ✅ All integration tests pass
3. ✅ Visual regression tests pass
4. ✅ Can derive colors from origin colors: `oklch(from blue calc(l - 0.2) c h)`
5. ✅ Runtime parameters work: `oklch(from var(--primary) calc(l - var(--adjust)) c h)`
6. ✅ All color functions support relative syntax (oklch, oklab, lch, lab, hsl, hwb, rgb)
7. ✅ Channel keywords work in calc: `calc(l - 0.2)`, `calc(h + 180deg)`
8. ✅ Nested derivation works: derive from a derived color
9. ✅ Performance is acceptable (no significant slowdown)
10. ✅ Documentation is complete

## Migration Path for Existing Code

**No Breaking Changes:**
- Existing absolute color functions continue to work unchanged
- `ThemeValue::Color` variant is preserved
- Parser detects "from" keyword to enable relative mode
- All existing tests continue to pass

**New Capabilities:**
- New `ThemeValue::RelativeColor` variant for relative colors
- `as_color_with_params()` supersedes `as_color()` for color properties
- Channel keywords in calc expressions

## References

### Specifications
- [CSS Color Level 5 - Relative Colors](https://drafts.csswg.org/css-color-5/#relative-colors)
- [CSS Values Level 4 - calc()](https://drafts.csswg.org/css-values-4/#calc-notation)

### Reference Implementation
- Servo: `/Users/jonmmease/Downloads/style/color/component.rs`
- Servo: `/Users/jonmmease/Downloads/style/color/color_function.rs`
- Servo: `/Users/jonmmease/Downloads/style/color/parsing.rs`
- Servo: `/Users/jonmmease/Downloads/style/color/mod.rs:457-524`

### Avenger Codebase
- Calc system: `avenger-chart/src/theme/calc.rs`
- Color conversion: `avenger-chart/src/color/convert.rs`
- Color types: `avenger-chart/src/color/types.rs`
- Theme values: `avenger-chart/src/theme/value.rs`
- Parser: `avenger-chart/src/theme/parser.rs`
