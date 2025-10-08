# apply_theme_to_* Pattern Assessment

## Overview
The `apply_theme_to_legend` helper function successfully reduced code duplication in legend construction. This document assesses where else this pattern can be applied.

## Pattern Analysis

### Current Implementation (legends.rs)

**Helper Function:**
```rust
fn apply_theme_to_legend<T, U, F, G>(
    legend: &mut Legend,
    field_check: F,
    theme_query: G,
    setter: impl FnOnce(Legend, T) -> Legend,
) where
    F: FnOnce(&Legend) -> &crate::maybe::Maybe<Option<U>>,
    G: FnOnce() -> Option<T>,
{
    if matches!(field_check(legend), crate::maybe::Maybe::Unset) {
        if let Some(value) = theme_query() {
            *legend = setter(legend.clone(), value);
        }
    }
}
```

**Usage Pattern:**
```rust
Self::apply_theme_to_legend(legend, |l| &l.title_color,
    || theme.text_color(&legend_ctx.child("title")).map(color_array_to_hex),
    |l, v| l.title_color(v));
```

**Benefits:**
- Eliminates ~6-7 lines per property application
- Single source of truth for Maybe::Unset checking
- Consistent pattern across all properties
- Type-safe with generics

---

## Potential Applications

### 1. Title/Subtitle Rendering (src/plot/compiled/titles.rs)

**File Size:** 226 lines
**Repetition Count:** 6 match patterns (3 in title, 3 in subtitle)

**Current Pattern (repeated 6 times):**
```rust
let font_size = match title.font_size.as_ref() {
    crate::maybe::Maybe::Set(Some(node)) => {
        let expr = node.to_expr(ctx)?;
        super::expr_eval::evaluate_f32_expr(&expr, ctx, params).await?
    }
    _ => theme.font_size(&title_ctx).unwrap_or(16.0),
};
```

**Opportunity:** MEDIUM
- 6 similar blocks (~7-10 lines each = ~42-60 lines)
- Pattern is slightly different (async expression evaluation vs direct theme query)
- Would need async support in helper function
- Less duplication than legends.rs but still worthwhile

**Recommendation:** Create `apply_theme_to_text` helper

---

### 2. Other Potential Locations

Searched the codebase for similar patterns:

**Found:**
- `rendering.rs` - Margin evaluation (4 instances, but already compact)
- Legend renderers - Already use direct theme queries (minimal duplication)

**Not Found:**
- Most other code uses direct theme queries without builder patterns
- Axes use different configuration patterns (not builder-based)

---

## Generalization Proposal

### Option A: Generic Helper (Parameterized by Type)

Create a generic `apply_theme_to_*` that works with any type:

```rust
/// Generic helper for applying theme values to builder-pattern structs
fn apply_theme_to<TStruct, TValue, TField, F, G, S>(
    instance: &mut TStruct,
    field_check: F,
    theme_query: G,
    setter: S,
) where
    F: FnOnce(&TStruct) -> &crate::maybe::Maybe<Option<TField>>,
    G: FnOnce() -> Option<TValue>,
    S: FnOnce(TStruct, TValue) -> TStruct,
    TStruct: Clone,
{
    if matches!(field_check(instance), crate::maybe::Maybe::Unset) {
        if let Some(value) = theme_query() {
            *instance = setter(instance.clone(), value);
        }
    }
}
```

**Pros:**
- Maximum reusability across Legend, Title, Subtitle, etc.
- Single implementation
- Consistent API

**Cons:**
- Complex generic signature
- May be harder to understand for readers
- Doesn't handle async expression evaluation

---

### Option B: Async-Aware Generic Helper

For titles.rs which needs async expression evaluation:

```rust
/// Async helper for applying theme values with expression evaluation
async fn apply_theme_to_with_expr<TStruct, TValue, TField, F, G, E, S>(
    instance: &mut TStruct,
    field_check: F,
    expr_eval: E,
    theme_fallback: G,
    setter: S,
) -> Result<(), AvengerChartError>
where
    F: FnOnce(&TStruct) -> &crate::maybe::Maybe<Option<LogicalExprNode>>,
    E: FnOnce(&LogicalExprNode) -> Future<Output = Result<TValue, AvengerChartError>>,
    G: FnOnce() -> TValue,
    S: FnOnce(TStruct, TValue) -> TStruct,
    TStruct: Clone,
{
    match field_check(instance) {
        crate::maybe::Maybe::Set(Some(node)) => {
            let value = expr_eval(node).await?;
            *instance = setter(instance.clone(), value);
        }
        _ => {
            let value = theme_fallback();
            *instance = setter(instance.clone(), value);
        }
    }
    Ok(())
}
```

**Pros:**
- Handles async expression evaluation
- Works for titles, subtitles, and other async patterns
- Still reduces duplication

**Cons:**
- More complex than sync version
- Futures make it harder to use
- Different API than legends.rs version

---

### Option C: Trait-Based Approach

Create a trait for theme-aware builders:

```rust
trait ThemeApplicable: Clone {
    fn apply_theme<T, F, G>(
        &mut self,
        field_check: F,
        theme_query: G,
        setter: impl FnOnce(Self, T) -> Self,
    ) where
        F: for<'a> FnOnce(&'a Self) -> &'a crate::maybe::Maybe<Option<T>>,
        G: FnOnce() -> Option<T>,
    {
        if matches!(field_check(self), crate::maybe::Maybe::Unset) {
            if let Some(value) = theme_query() {
                *self = setter(self.clone(), value);
            }
        }
    }
}

impl ThemeApplicable for Legend {}
impl ThemeApplicable for Title {}
impl ThemeApplicable for Subtitle {}
```

**Pros:**
- Clean API: `legend.apply_theme(...)`
- Type-specific implementations possible
- Extensible to other types

**Cons:**
- Adds trait complexity
- May be overkill for this use case
- Still doesn't solve async problem

---

## Recommendations

### Immediate Actions (High Value)

1. **Keep current `apply_theme_to_legend` as-is**
   - Specific to Legend type
   - Works well for its purpose
   - Clear and understandable

2. **Create `apply_theme_to_title` helper for titles.rs**
   - Don't try to generalize yet
   - Handle the specific async pattern in titles.rs
   - Reduces ~42-60 lines of duplication

### Future Considerations (Lower Priority)

3. **Extract to `maybe` module if pattern emerges elsewhere**
   - Wait to see if more similar patterns appear
   - Could create `maybe::helpers` module with:
     - `apply_if_unset()` - sync version
     - `apply_if_unset_async()` - async version
   - Only worthwhile if used in 3+ places

4. **Don't over-generalize**
   - Current duplication is manageable
   - Premature abstraction adds complexity
   - Wait for real need before generalizing

---

## Titles.rs Refactoring Proposal

### Current Code (repeated 3 times in title, 3 times in subtitle):

```rust
// Pattern 1: Simple theme query with fallback
let font_size = match title.font_size.as_ref() {
    crate::maybe::Maybe::Set(Some(node)) => {
        let expr = node.to_expr(ctx)?;
        super::expr_eval::evaluate_f32_expr(&expr, ctx, params).await?
    }
    _ => theme.font_size(&title_ctx).unwrap_or(16.0),
};

// Pattern 2: Theme query with string fallback
let font_family = match title.font_family.as_ref() {
    crate::maybe::Maybe::Set(Some(node)) => {
        let expr = node.to_expr(ctx)?;
        super::expr_eval::evaluate_string_expr(&expr, ctx, params).await?
    }
    _ => theme
        .font_family(&title_ctx)
        .unwrap_or_else(|| "sans-serif".to_string()),
};

// Pattern 3: Theme query with enum mapping
let text_align = match title.align.as_ref() {
    crate::maybe::Maybe::Set(Some(node)) => {
        let expr = node.to_expr(ctx)?;
        let align_str = super::expr_eval::evaluate_string_expr(&expr, ctx, params).await?;
        match align_str.to_lowercase().as_str() {
            "left" => TextAlign::Left,
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            _ => TextAlign::Center,
        }
    }
    _ => {
        theme
            .text_align(&title_ctx)
            .and_then(|s| match s.to_lowercase().as_str() {
                "left" => Some(TextAlign::Left),
                "center" => Some(TextAlign::Center),
                "right" => Some(TextAlign::Right),
                _ => None,
            })
            .unwrap_or(TextAlign::Center)
    }
};
```

### Proposed Helper (in CompiledPlot impl):

```rust
/// Helper to evaluate expression or fall back to theme value
async fn eval_or_theme<T, E, F>(
    field: &crate::maybe::Maybe<Option<LogicalExprNode>>,
    ctx: &datafusion::prelude::SessionContext,
    params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    expr_eval: E,
    theme_fallback: F,
) -> Result<T, AvengerChartError>
where
    E: FnOnce(&datafusion::logical_expr::Expr, &datafusion::prelude::SessionContext, &indexmap::IndexMap<String, datafusion::common::ScalarValue>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, AvengerChartError>>>>,
    F: FnOnce() -> T,
{
    match field.as_ref() {
        crate::maybe::Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            expr_eval(&expr, ctx, params).await
        }
        _ => Ok(theme_fallback()),
    }
}
```

### Refactored Usage:

```rust
let font_size = Self::eval_or_theme(
    &title.font_size,
    ctx,
    params,
    |e, c, p| Box::pin(super::expr_eval::evaluate_f32_expr(e, c, p)),
    || theme.font_size(&title_ctx).unwrap_or(16.0),
).await?;

let font_family = Self::eval_or_theme(
    &title.font_family,
    ctx,
    params,
    |e, c, p| Box::pin(super::expr_eval::evaluate_string_expr(e, c, p)),
    || theme.font_family(&title_ctx).unwrap_or_else(|| "sans-serif".to_string()),
).await?;
```

**Impact:**
- Reduces 6 blocks × ~8 lines = ~48 lines to 6 × 5 lines = ~30 lines
- Net saving: ~18 lines
- Improved consistency and maintainability

---

## Summary

| Location | File | Current Lines | Repetitive Blocks | Potential Savings | Priority | Recommendation |
|----------|------|---------------|-------------------|-------------------|----------|----------------|
| **Legend construction** | legends.rs | 698 | 12 (already done) | ✅ Completed (-11 lines) | ✅ Done | Keep as-is |
| **Title/Subtitle** | titles.rs | 226 | 6 | ~18 lines | MEDIUM | Create async helper |
| **Other files** | Various | N/A | <3 each | Minimal | LOW | Leave as-is |

**Overall Assessment:**
- **Current refactoring (legends.rs):** ✅ Successful, should be kept
- **Next opportunity (titles.rs):** Medium value, worth doing
- **Generalization:** Not recommended yet - wait for more use cases
- **Total potential savings:** ~18 additional lines across codebase

**Next Steps:**
1. ✅ Keep `apply_theme_to_legend` in legends.rs (completed)
2. Consider creating `eval_or_theme` helper for titles.rs (optional)
3. Monitor for additional patterns before generalizing further
