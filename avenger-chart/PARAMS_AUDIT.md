# Runtime Parameters Audit - Theme System

## Issue Summary

The theme system should consistently pass runtime parameters through the entire call chain. Currently, there are two main issues:

1. **Empty IndexMap creation**: Many places create `IndexMap::new()` instead of passing params
2. **Methods without params parameter**: Some methods exist in both `_with_params` and non-params versions

## Critical Issues

### 1. value.rs - Dual Method Versions

**Problem:** Two versions of methods exist - one without params, one with params

```rust
// Line 143: Version WITHOUT params (should be removed)
pub fn as_font_size(&self, base_font_size: f32) -> Option<f32>

// Line 169: Version WITH params (should be the only version)
pub fn as_font_size_with_params(
    &self,
    params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    base_font_size: f32,
) -> Option<f32>
```

**Impact:** Callers use the wrong version and lose param support

**Recommendation:**
- Remove `as_font_size()`
- Rename `as_font_size_with_params()` → `as_font_size()`
- Update all call sites

---

### 2. calc.rs - resolve() without params

**Location:** Line 913

```rust
pub fn resolve(&self, base_font_size: f32) -> Result<CalcResult, String> {
    self.resolve_with_params(&IndexMap::new(), base_font_size)
}
```

**Problem:** Creates empty params instead of requiring caller to pass them

**Recommendation:**
- Remove `resolve()` method
- Rename `resolve_with_params()` → `resolve()`
- Update all callers to pass params

---

### 3. context.rs - Default empty params

**Location:** Line 48

```rust
impl Default for ThemeContext {
    fn default() -> Self {
        Self::new("root")
    }
}

impl ThemeContext {
    pub fn new(element: &str) -> Self {
        Self {
            element: element.to_string(),
            element_type: None,
            parent: None,
            params: IndexMap::new(),  // ← Empty params by default
        }
    }
}
```

**Problem:** Creating context without params means params are lost

**Recommendation:**
- Make `new()` require params parameter
- Or document that callers MUST call `.with_params()` immediately after construction

---

## Test Code (Acceptable)

The following are **test code** and can stay as-is (empty params are fine for unit tests):

### contrast_color.rs
- Lines 332, 354, 373, 388, 397, 478, 512: All in `#[cfg(test)]` mod

### color_component.rs
- Lines 170, 177, 185, 192, 209, 229, 248: All in `#[cfg(test)]` mod

### media_query.rs
- Lines 220, 318, 322, 457, 462, 467, 472, 477, 482: All in `#[cfg(test)]` mod

### theme.rs
- Lines 837, 1173-1222, 1300, 1358, 1381, 1399, 1550-1600, 1672-2481: All in `#[cfg(test)]` mod

### parser.rs
- Lines 2079-2259: All in `#[cfg(test)]` mod

---

## Production Code Issues

### theme.rs - Production usage

**Line 292** (in `resolve_variable` - production code):
```rust
let mut variables = IndexMap::new();
```

**Context:** This is building a map of variable definitions, NOT runtime params.
**Verdict:** This is **CORRECT** - it's not params, it's a different IndexMap for variable resolution.

---

**Line 313** (in `get_base_font_size`):
```rust
// Uses DEFAULT_BASE_FONT_SIZE for bootstrap to avoid circular dependency
if let Some(size) = resolved.as_font_size(DEFAULT_BASE_FONT_SIZE) {
    return size;
}
```

**Problem:** Calls `as_font_size()` without params
**Impact:** If base font size is defined with calc() using params, it won't resolve correctly

**Recommendation:** This is during theme construction, so params might not be available. Consider:
- Pass empty params explicitly with comment explaining why
- Or ensure base font size cannot use calc() with variables

---

### parser.rs - Production code

**Line 190** (in parser initialization):
```rust
declarations: IndexMap::new(),
```

**Context:** This initializes an empty declarations map for a CSS rule.
**Verdict:** **CORRECT** - this is not params, it's CSS declarations storage.

---

## Recommendations

### Immediate Actions (Breaking Changes Required)

1. **value.rs**:
   - ❌ Remove `as_font_size(base_font_size)`
   - ✅ Rename `as_font_size_with_params()` → `as_font_size()`
   - Update signature: `pub fn as_font_size(&self, params: &IndexMap<String, ScalarValue>, base_font_size: f32)`

2. **calc.rs**:
   - ❌ Remove `resolve(base_font_size)`
   - ✅ Rename `resolve_with_params()` → `resolve()`

3. **context.rs**:
   - Document that `ThemeContext::new()` creates context with empty params
   - Callers MUST call `.with_params()` if params are needed
   - Or consider making `new()` private and requiring `new_with_params()`

### Update All Callers

After the above changes, update all production callers to pass params:

**In theme.rs:**
```rust
// Before
.and_then(|v| v.as_font_size(self.get_base_font_size(&context.params)))

// After
.and_then(|v| v.as_font_size(&context.params, self.get_base_font_size(&context.params)))
```

**In grid.rs, chart_layout.rs, titles.rs:**
- All async text measurement functions already pass params ✅
- No changes needed

---

## Summary

**Test Code:** 60+ empty IndexMap creations - all acceptable ✅

**Production Code Issues:**
1. `value.rs`: Dual methods - remove non-params version ❌
2. `calc.rs`: Dual methods - remove non-params version ❌
3. `context.rs`: Document empty params behavior or change API ⚠️
4. `theme.rs`: One call site needs updating after value.rs fix 🔧

**Total breaking changes needed:** 2 method removals + documentation updates
**Estimated impact:** Medium - need to update ~10 call sites in production code
