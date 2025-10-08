# Expression vs Theme CSS Configuration Analysis

## Executive Summary

This document catalogs all configuration properties in the avenger-chart crate, comparing which properties can be configured via **Rust expressions** vs. which can be configured via the **CSS theme system**.

**Key Finding**: Most properties support both expression-based configuration (via Rust API) and CSS theme-based configuration. However, there are some gaps where properties are configurable via expressions but not yet wired to CSS theme selectors.

---

## 1. Title & Subtitle Configuration

### Properties Available as Expressions

| Property | Type | PlotTitle | PlotSubtitle | Notes |
|----------|------|-----------|--------------|-------|
| `text` | `LogicalExprNode` | ✅ | ✅ | Required field |
| `font_size` | `Maybe<Option<LogicalExprNode>>` | ✅ | ✅ | |
| `font_family` | `Maybe<Option<LogicalExprNode>>` | ✅ | ✅ | |
| `span` | `Maybe<Option<LogicalExprNode>>` | ✅ | ✅ | Canvas vs PlotArea width |
| `align` | `Maybe<Option<LogicalExprNode>>` | ✅ | ✅ | Left/Center/Right alignment |

### CSS Theme Integration Status

| Property | Theme Method | CSS Selector | Status |
|----------|-------------|--------------|--------|
| `font_size` | `theme.font_size(&title_ctx)` | `chart-title { font-size: 1.5rem; }` | ✅ **Wired** |
| `font_family` | `theme.font_family(&title_ctx)` | `:root { font-family: ... }` (inherited) | ✅ **Wired** |
| `font_weight` | `theme.font_weight(&title_ctx)` | `chart-title { font-weight: 500; }` | ✅ **Wired** |
| `color` | `theme.text_color(&title_ctx)` | `chart-title { color: var(--text-color); }` | ✅ **Wired** |
| `span` | `theme.query(&title_ctx, "width")` | `chart-title { width: canvas; }` | ✅ **Wired** |
| `align` | `theme.text_align(&title_ctx)` | `chart-title { text-align: center; }` | ✅ **Wired** |

**Status**: All title/subtitle properties now have full CSS theme integration.

---

## 2. Legend Configuration

### Properties Available as Expressions

| Property | Type | CSS-Themeable? |
|----------|------|----------------|
| `visible` | `Maybe<Option<LogicalExprNode>>` | ❌ |
| `title` | `Maybe<Option<LogicalExprNode>>` | ❌ (dynamic text) |
| `position` | `Maybe<Option<LogicalExprNode>>` | ❌ (layout enum) |
| `orientation` | `Maybe<Option<LogicalExprNode>>` | ❌ (layout enum) |
| `symbol_size` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `gradient_length` | `Maybe<Option<LogicalExprNode>>` | ❌ |
| `gradient_thickness` | `Maybe<Option<LogicalExprNode>>` | ❌ |
| `columns` | `Maybe<Option<LogicalExprNode>>` | ❌ (layout) |
| `label_limit` | `Maybe<Option<LogicalExprNode>>` | ❌ |
| `format_number` | `Maybe<Option<LogicalExprNode>>` | ❌ (formatting) |
| `background_fill` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `background_stroke` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `background_corner_radius` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `background_padding` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `order` | `Maybe<Option<LogicalExprNode>>` | ❌ |
| `title_color` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `label_color` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `title_font_family` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `title_font_size` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `title_font_weight` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `label_font_family` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `label_font_size` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `label_font_weight` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `tick_font_family` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `tick_font_size` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `tick_font_weight` | `Maybe<Option<LogicalExprNode>>` | ✅ |
| `tick_color` | `Maybe<Option<LogicalExprNode>>` | ✅ |

### CSS Theme Integration Status

Legend properties are queried from theme using these contexts:
- `legend` - Base legend element
- `legend[type="symbol"]` - Symbol/point legends
- `legend[type="line"]` - Line legends
- `legend[type="colorbar"]` - Continuous color legends
- `legend[type="rect"]` - Rectangle/bar legends

**Wired to Theme**:
```rust
// From src/plot/compiled/legends.rs
theme.fill_color(&bg_ctx)               -> legend background { fill: ... }
theme.stroke_color(&bg_ctx)             -> legend background { stroke: ... }
theme.query(&bg_ctx, "padding")         -> legend background { padding: ... }
theme.query(&bg_ctx, "corner-radius")   -> legend background { corner-radius: ... }
theme.text_color(&legend_ctx.child("title"))  -> legend title { color: ... }
theme.text_color(&legend_ctx.child("label"))  -> legend label { color: ... }
theme.font_family(&legend_ctx.child("title")) -> legend title { font-family: ... }
theme.font_size(&legend_ctx.child("title"))   -> legend title { font-size: ... }
theme.font_weight(&legend_ctx.child("title")) -> legend title { font-weight: ... }
theme.font_family(&legend_ctx.child("label")) -> legend label { font-family: ... }
theme.font_size(&legend_ctx.child("label"))   -> legend label { font-size: ... }
theme.font_weight(&legend_ctx.child("label")) -> legend label { font-weight: ... }
theme.font_family(&legend_ctx.child("tick"))  -> legend tick { font-family: ... }
theme.font_size(&legend_ctx.child("tick"))    -> legend tick { font-size: ... }
theme.font_weight(&legend_ctx.child("tick"))  -> legend tick { font-weight: ... }
theme.text_color(&legend_ctx.child("tick"))   -> legend tick { color: ... }
```

**Wired to Theme (Additional)**:
```rust
// From src/legend/renderer/symbol.rs, rect.rs, line.rs
theme.query(&legend_ctx, "symbol-size")         -> legend { symbol-size: ... }

// From src/legend/renderer/colorbar.rs
theme.query(&legend_ctx, "gradient-length")     -> legend[type="colorbar"] { gradient-length: ... }
theme.query(&legend_ctx, "gradient-thickness")  -> legend[type="colorbar"] { gradient-thickness: ... }
```

**Gaps**: The following expression-configurable properties have **NO CSS theme integration**:
- `label_limit` - Max label length (requires avenger-guides changes)
- `columns` - Number of columns in discrete legends (requires avenger-guides changes)
- `visible` - Show/hide legend (better as expression)
- `position` - Top/Bottom/Left/Right (better as expression)
- `orientation` - Horizontal/Vertical (better as expression)

---

## 3. Layout & Sizing Configuration

### Properties Available as Expressions

From `LayoutSpec`:
```rust
pub struct LayoutSpec {
    pub canvas: SizeMode,           // Fixed/Width/Height/Auto
    pub plot_area: SizeMode,        // Fixed/Width/Height/Auto
    pub margins: Margins,           // top, right, bottom, left
}

pub struct Margins {
    pub top: Maybe<Option<LogicalExprNode>>,
    pub right: Maybe<Option<LogicalExprNode>>,
    pub bottom: Maybe<Option<LogicalExprNode>>,
    pub left: Maybe<Option<LogicalExprNode>>,
}

pub enum SizeMode {
    Fixed { width: LogicalExprNode, height: LogicalExprNode },
    Width(LogicalExprNode),
    Height(LogicalExprNode),
    Auto,
}
```

### CSS Theme Integration Status

| Property | Theme Method | Status |
|----------|-------------|--------|
| `margins.top` | `theme.query(&canvas_ctx, "margin-top")` | ✅ **Wired** |
| `margins.right` | `theme.query(&canvas_ctx, "margin-right")` | ✅ **Wired** |
| `margins.bottom` | `theme.query(&canvas_ctx, "margin-bottom")` | ✅ **Wired** |
| `margins.left` | `theme.query(&canvas_ctx, "margin-left")` | ✅ **Wired** |
| Canvas/plot sizing | ❌ None | ❌ **NOT wired to theme** |

**Status**: Canvas margins now have CSS theme integration with default fallback of 10px.

**CSS Example**:
```css
canvas {
    margin-top: 10px;
    margin-right: 15px;
    margin-bottom: 10px;
    margin-left: 15px;
}
```

**Rationale**: Canvas/plot sizing is better as expression-based since it's typically data-driven or responsive.

---

## 4. Axis Configuration

Axes have extensive theme integration. From `src/cartesian/axis.rs`:

### CSS Theme Integration (FULLY WIRED)

```rust
// Colors
theme.text_color(&label_ctx)        -> axis label { color: ... }
theme.text_color(&title_ctx)        -> axis title { color: ... }
theme.stroke_color(&domain_ctx)     -> axis domain { stroke: ... }
theme.stroke_color(&tick_ctx)       -> axis tick { stroke: ... }
theme.stroke_color(&grid_ctx)       -> axis grid { stroke: ... }

// Typography
theme.font_family(&label_ctx)       -> axis label { font-family: ... }
theme.font_family(&title_ctx)       -> axis title { font-family: ... }
theme.font_size(&label_ctx)         -> axis label { font-size: ... }
theme.font_size(&title_ctx)         -> axis title { font-size: ... }
theme.font_weight(&label_ctx)       -> axis label { font-weight: ... }
theme.font_weight(&title_ctx)       -> axis title { font-weight: ... }

// Sizes
theme.stroke_width(&domain_ctx)     -> axis domain { stroke-width: ... }
theme.stroke_width(&tick_ctx)       -> axis tick { stroke-width: ... }
theme.stroke_width(&grid_ctx)       -> axis grid { stroke-width: ... }
theme.axis_tick_length(&axis_ctx)   -> axis tick { size: ... }
theme.opacity(&grid_ctx)            -> axis grid { opacity: ... }

// Spacing
theme.query(&label_ctx, "padding")  -> axis label { padding: ... }
```

✅ **Axes are fully integrated with CSS themes.**

---

## 5. Mark Default Properties

Marks (symbol, line, rect, text) have CSS theme integration for default values:

### CSS Theme Integration (FULLY WIRED)

```rust
theme.get_range_for_channel(mark_type, channel, range_kind)

// Queries CSS like:
mark[type="symbol"] {
    fill: var(--categorical-color-0);
    stroke: var(--bg-color);
    stroke-width: 0.5;
    size: 72;
    shape: circle;
    opacity: 1.0;
}

mark[type="line"] {
    stroke: var(--categorical-color-0);
    stroke-width: 2.0;
    stroke-dash: solid;
    stroke-cap: round;
    stroke-join: round;
    opacity: 1.0;
}

mark {
    fill-discrete: var(--categorical-colors);
    fill-continuous: var(--viridis-colors);
    stroke-discrete: var(--categorical-colors);
    stroke-continuous: var(--viridis-colors);
    shape-discrete: circle, cross, diamond, square, star, ...;
    size-discrete: 30, 80, 140, 200, 260;
    size-continuous: 30, 200;
    stroke-dash-discrete: solid, dashed, dotted, ...;
}
```

✅ **Mark defaults are fully integrated with CSS themes.**

---

## 6. Background Colors

### Canvas & Plot Background

From `src/cartesian/guide.rs` and `src/polar/guide.rs`:

```rust
// CSS property: plot_background_color
pub struct CartesianOptions {
    pub plot_background_color: Maybe<Option<LogicalExprNode>>,
}
```

**Integration**:
- ✅ Expression-configurable
- ✅ Theme-queryable via `theme.fill_color(&plot_ctx)` or `theme.query(&plot_ctx, "background-color")`
- ✅ CSS: `plot { background-color: var(--bg-color); }`

---

## Summary Table: Expression vs Theme Coverage

| Category | Expression Support | CSS Theme Support | Gap? |
|----------|-------------------|-------------------|------|
| **Title/Subtitle** | ✅ Full (5 props) | ✅ Full (5/5) | ✅ No gaps |
| **Legend - Typography** | ✅ Full (12 props) | ✅ Full | ✅ No gaps |
| **Legend - Background** | ✅ Full (4 props) | ✅ Full | ✅ No gaps |
| **Legend - Sizing** | ✅ Full (3 props) | ✅ Full | ✅ No gaps |
| **Legend - Layout** | ✅ Full (4 props) | ⚠️ Partial | ⚠️ `columns`, `label_limit` need guide library changes |
| **Layout & Margins** | ✅ Full (4 margin props) | ✅ Full | ✅ No gaps |
| **Axis** | ✅ Full | ✅ Full | ✅ No gaps |
| **Marks** | ✅ Full | ✅ Full | ✅ No gaps |
| **Backgrounds** | ✅ Full | ✅ Full | ✅ No gaps |

---

## Recommendations

### ✅ Completed - CSS Theme Support Added For:

1. **Title/Subtitle Alignment** ✅
   ```css
   chart-title {
       text-align: center;  /* left, center, right */
   }
   chart-subtitle {
       text-align: center;
   }
   ```
   - Wired in `src/plot/compiled/titles.rs`
   - Added `Theme::text_align()` method

2. **Title/Subtitle Span** ✅
   ```css
   chart-title {
       width: canvas;  /* or: plot-area */
   }
   ```
   - Wired in `src/layout/chart_layout.rs`
   - Queries `theme.query(&title_ctx, "width")`

3. **Legend Symbol Size** ✅
   ```css
   legend {
       symbol-size: 100;
   }
   ```
   - Wired through all legend renderers (symbol, rect, line)
   - Maps to line length for line legends

4. **Legend Gradient Dimensions** ✅
   ```css
   legend[type="colorbar"] {
       gradient-length: 150px;
       gradient-thickness: 15px;
   }
   ```
   - Wired in `src/legend/renderer/colorbar.rs`

5. **Canvas Margins** ✅
   ```css
   canvas {
       margin-top: 10px;
       margin-right: 15px;
       margin-bottom: 10px;
       margin-left: 15px;
   }
   ```
   - Wired in `src/plot/compiled/rendering.rs`

### Remaining Gaps (Require Guide Library Changes):

6. **Legend Layout Properties** (Partially Complete)
   ```css
   legend {
       columns: 3;       /* CSS defined, not yet wired (needs avenger-guides) */
       label-limit: 20;  /* CSS defined, not yet wired (needs avenger-guides) */
   }
   ```
   - These properties are in the CSS but require changes to `avenger-guides` crate
   - Need to add `columns` and `label_limit` fields to legend config structs

### Not Recommended for CSS (Better as Expressions):

- `legend.visible` - Dynamic show/hide is better as expression
- `legend.position` - Layout positioning is better as API call
- `legend.orientation` - Layout decision is better as API call
- `legend.order` - Ordering logic is better as expression

---

## Implementation Notes

### Current Theme Query Pattern

Properties are wired to themes using this pattern:

1. **Define CSS property** in default theme:
   ```css
   legend title {
       font-size: 1.0rem;
   }
   ```

2. **Query in Rust code**:
   ```rust
   let legend_ctx = theme.legend_context(Some("symbol"));
   if let Some(size) = theme.font_size(&legend_ctx.child("title")) {
       config.title_font_size = Some(size);
   }
   ```

3. **Expression overrides theme**:
   ```rust
   if let Some(node) = legend_config.title_font_size.as_option() {
       let expr = node.to_expr(ctx)?;
       config.title_font_size = Some(evaluate_f32_expr(&expr, ctx, params).await?);
   }
   ```

### To Add New Theme Property:

1. Add CSS property to `src/theme/theme.rs` default theme
2. Add query method to `Theme` if needed (or use existing like `query()`, `font_size()`, etc.)
3. Wire in rendering code (e.g., `src/plot/compiled/titles.rs`, `src/plot/compiled/legends.rs`)
4. Ensure expression-based config takes precedence over theme

---

## Conclusion

The avenger-chart crate now has **comprehensive CSS theme integration** for typography, colors, visual styling, and layout properties.

### ✅ Completed Implementation

All high and medium priority items have been implemented:
- Title/subtitle alignment and span properties
- Legend symbol size wiring across all renderers
- Legend gradient dimensions for colorbars
- Canvas margin defaults

### 📋 Remaining Work

Only two properties remain unwired:
- `columns` - Requires `avenger-guides` crate changes
- `label_limit` - Requires `avenger-guides` crate changes

These are defined in CSS but need underlying library support to be fully functional.

### 🎯 Coverage Summary

**Expression-configurable properties with CSS theme support**: ~95%
- All typography, colors, and sizing properties: ✅
- All margin properties: ✅
- All legend visual properties: ✅
- Layout properties requiring guide library changes: ⚠️ (2 properties)
