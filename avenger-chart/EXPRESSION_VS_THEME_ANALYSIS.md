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
| `span` | ❌ None | N/A | ❌ **NOT wired to theme** |
| `align` | ❌ None | N/A | ❌ **NOT wired to theme** |

**Gap**: `span` and `align` properties are expression-configurable but have no CSS theme integration.

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

**Gaps**: The following expression-configurable properties have **NO CSS theme integration**:
- `gradient_length` - Colorbar gradient size
- `gradient_thickness` - Colorbar gradient thickness
- `label_limit` - Max label length
- `columns` - Number of columns in discrete legends
- `visible` - Show/hide legend
- `position` - Top/Bottom/Left/Right
- `orientation` - Horizontal/Vertical

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
| `margins.top` | ❌ None | ❌ **NOT wired to theme** |
| `margins.right` | ❌ None | ❌ **NOT wired to theme** |
| `margins.bottom` | ❌ None | ❌ **NOT wired to theme** |
| `margins.left` | ❌ None | ❌ **NOT wired to theme** |
| Canvas/plot sizing | ❌ None | ❌ **NOT wired to theme** |

**Gap**: Layout and margins are entirely expression-based with no CSS theme integration.

**Rationale**: These are typically data-driven or responsive layout decisions, not styling concerns. However, it could be useful to set default margins via CSS:
```css
canvas {
    margin: 10px;  /* Could set default margins */
}
```

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
| **Title/Subtitle** | ✅ Full (5 props) | ⚠️ Partial (3/5) | ✅ Missing: `span`, `align` |
| **Legend - Typography** | ✅ Full (12 props) | ✅ Full | ✅ No gaps |
| **Legend - Background** | ✅ Full (4 props) | ✅ Full | ✅ No gaps |
| **Legend - Layout** | ✅ Full (7 props) | ❌ None | ✅ Missing: `gradient_*`, `columns`, `label_limit`, `visible`, `position`, `orientation` |
| **Layout & Margins** | ✅ Full (7 props) | ❌ None | ✅ Missing: all margin properties |
| **Axis** | ✅ Full | ✅ Full | ✅ No gaps |
| **Marks** | ✅ Full | ✅ Full | ✅ No gaps |
| **Backgrounds** | ✅ Full | ✅ Full | ✅ No gaps |

---

## Recommendations

### High Priority - Add CSS Theme Support For:

1. **Title/Subtitle Alignment**
   ```css
   chart-title {
       text-align: center;  /* left, center, right */
   }
   chart-subtitle {
       text-align: center;
   }
   ```

2. **Title/Subtitle Span**
   ```css
   chart-title {
       width: canvas;  /* or: plot-area */
   }
   ```

3. **Legend Symbol Size**
   ```css
   legend {
       symbol-size: 100;  /* Already exists in default theme! */
   }
   ```

   **Note**: This is already in the CSS but may not be fully wired through all legend renderers.

### Medium Priority - Consider Adding:

4. **Legend Gradient Dimensions** (for colorbar legends)
   ```css
   legend[type="colorbar"] {
       gradient-length: 150px;
       gradient-thickness: 15px;
   }
   ```

5. **Legend Layout Properties**
   ```css
   legend {
       columns: 3;
       label-limit: 20;
   }
   ```

6. **Canvas Margins**
   ```css
   canvas {
       margin: 10px;  /* uniform */
       /* or */
       margin-top: 10px;
       margin-right: 15px;
       margin-bottom: 10px;
       margin-left: 15px;
   }
   ```

### Low Priority (Likely Better as Expressions):

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

The avenger-chart crate has excellent CSS theme integration for typography, colors, and visual styling. The main gaps are:

1. **Title/subtitle alignment properties** - Should be added to CSS
2. **Legend layout properties** - Some should be themeable (symbol-size, gradient dimensions)
3. **Margin defaults** - Could benefit from CSS defaults

Most expression-configurable properties ARE theme-configurable, with the exceptions noted above.
