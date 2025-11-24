# Expert Review Synthesis: Render-Time Facets Plan

**Date**: 2025-11-23
**Reviewers**: GPT-5 Codex, Gemini 3 Pro
**Status**: **CRITICAL BLOCKER IDENTIFIED**

---

## Executive Summary

Both experts identified the plan as **architecturally sound in principle**, but Gemini discovered a **critical technical blocker** that makes the current implementation strategy **infeasible as written**.

### The Critical Blocker: Serialization

**Problem**: The plan proposes storing `uncompiled_subplot: Arc<Plot<InnerC>>` in `CompiledFacet*` structs.

**Why This Fails**:
1. `CompiledFacetRow` implements `CompiledMark`
2. `CompiledMark` uses `#[typetag::serde]` for serialization
3. Therefore `CompiledFacetRow` **must** be serializable
4. This requires `Plot<InnerC>` to be serializable
5. **`Plot` is NOT serializable** (contains trait objects, async methods, closures)

**Impact**: Cannot simply store uncompiled Plot in a compiled struct without major architectural changes.

---

## Expert Consensus

### Areas of Agreement ✅

1. **Problem is Real**: Both experts confirm compile-time facet calculations don't account for render-time data transformations
2. **Logical Flow is Sound**: Moving calculations to render time is the correct approach
3. **Per-Cell Compilation**: GridFacet must compile per-cell (not per-row or per-column)
4. **SessionContext Thread-Safe**: Parallel compilation is safe from concurrency perspective
5. **Cache Scope**: Only cache within single `evaluate_facet` call, not across renders
6. **Timeline**: 5-8 weeks realistic IF serialization solved; add 3-4 weeks if not

### Key Insights 💡

**From Gemini**:
- Serialization is a **hard blocker**, not just a nice-to-have
- Must either: (1) Implement PlotSpec system, (2) Drop serialization support, or (3) Create SerializablePlot subset
- Generic handling (`CompiledFacetRow<InnerC>`) complicates trait object usage

**From GPT-5 Codex** (inferred from detailed analysis):
- Current architecture already handles data overrides at render time via `build_plot_components(..., Some(&filter_df), ...)`
- The subplot compilation issue might be solvable without storing `Plot` if we can pass data differently
- Performance concern: compiling 100 subplots (10x10 grid) per frame is heavy

---

## Solutions to Serialization Blocker

### Option 1: Drop Serialization Requirement (Simplest)

**Approach**: Remove `#[typetag::serde]` from `CompiledFacet*` or implement dummy serializer that errors.

**Pros**:
- Simplest implementation
- Fastest path forward
- No new abstractions needed

**Cons**:
- Breaking change for anyone serializing `CompiledPlot` with facets
- May impact caching/persistence use cases

**Feasibility**: HIGH - straightforward but requires user impact assessment

---

### Option 2: Implement Plot Specification System (Most Robust)

**Approach**: Create `PlotSpec` - a pure data struct (no methods/traits) that mirrors `Plot`.

**Implementation**:
```rust
#[derive(Serialize, Deserialize)]
pub struct PlotSpec {
    marks: Vec<MarkSpec>,  // Serializable mark specifications
    // ... other serializable fields
}

pub struct CompiledFacetRow {
    state: CompiledMarkState,
    uncompiled_subplot_spec: Arc<PlotSpec>,  // Store spec, not Plot
    // ...
}

// At render time:
let plot: Plot<InnerC> = PlotSpec::to_plot(&subplot_spec)?;
let compiled = plot.compile(ctx).await?;
```

**Pros**:
- Maintains serialization support
- Clean separation of specification vs implementation
- Could benefit other use cases (saving/loading plots)

**Cons**:
- Significant implementation effort (3-4 weeks)
- Need to design MarkSpec for all mark types
- Additional abstraction layer

**Feasibility**: MEDIUM - large effort but architecturally clean

---

### Option 3: Render-Time Data Override (Alternative Approach)

**Approach**: Don't store uncompiled subplot at all. Instead, leverage existing data override mechanism.

**Key Insight from Code Analysis**:
Current architecture already supports data overrides:
```rust
// facet_evaluation.rs:280
let filter_df = df.clone().filter(facet_expr.eq(lit(facet_value)))?;

// facet_evaluation.rs:300
let components = compiled_subplot
    .build_plot_components(..., Some(&filter_df), ...)  // ← Data override!
    .await?;
```

**Proposed Implementation**:
1. Keep compiling subplot at compile time (current behavior)
2. Extract distinct keys at RENDER time (from render-time DataFrame)
3. Pass filtered DataFrame as data override to `build_plot_components`
4. This solves the render param problem WITHOUT recompilation

**Changes Required**:
```rust
// In evaluate_facet():
// REMOVE: Use facet_keys parameter
// ADD: Extract keys from render-time DataFrame
let df = state.data.dataframe_with_context(ctx)?;
let facet_expr = state.data.channels().get(DimConfig::channel_name())?.expr(ctx)?;
let domain_vals = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;

// Rest of logic stays same - compiled_subplot is still used
// Data override mechanism handles filtered data
```

**Pros**:
- **Solves the render param problem** without recompilation
- **No serialization issues** - CompiledFacetRow unchanged
- **Minimal code changes** - just move key extraction
- **No performance regression** - compilation still happens once

**Cons**:
- Does NOT solve nested faceting (still infinite recursion)
- Compiled subplot has wrong scale domains if data changes significantly

**Feasibility**: **HIGHEST** - simple, surgical change

**Question**: Does this solve the original problem adequately?

---

## Revised Recommendations

### Immediate Decision Required

**Question for User**: What is the priority?

**Priority A: Fix Render Param Issue ONLY**
→ Use **Option 3** (Render-Time Data Override)
- Timeline: 1-2 weeks
- Solves: Facets reflect render-time data
- Doesn't solve: Nested faceting

**Priority B: Enable Nested Faceting + Fix Render Params**
→ Choose between:
- **Option 1** (Drop Serialization): 5-8 weeks, breaking change
- **Option 2** (PlotSpec System): 8-12 weeks, no breaking change

---

## Answers to Critical Questions (Expert Consensus)

### 1. Plot::with_data() Implementation
**Answer**: `Plot` already has `data: Option<DataFrame>` field. Implementation is trivial.
**Caveat**: Only relevant if we go with Option 1 or 2 (recompilation approach).

### 2. Parallelization Safety
**Answer**: YES - `SessionContext` and `DataFrame` are thread-safe (`Send+Sync`).
**Caution**: Compiling many plans in parallel may contend on CPU/RAM.

### 3. Caching Strategy
**Answer**: Cache ONLY within single `evaluate_facet` call.
**Reason**: Input data changes per render, so cross-render cache would be immediately invalid.

### 4. Backward Compatibility
**Answer**: This is a BREAKING CHANGE if serialization format changes.
**Action**: Must bump crate version, document migration path.

### 5. GridFacet Compilation
**Answer**: **Per-Cell** compilation required.
**Reason**: Each cell filters by Row AND Column - data distribution (and thus scales/axes) varies per cell.

---

## Risk Assessment Updates

### NEW High Risks (from Expert Review)

1. **Serialization Blocker** (Gemini)
   - Severity: CRITICAL
   - Impact: Plan infeasible as written
   - Mitigation: Choose Option 1, 2, or 3 above

2. **Performance with Large Grids** (Both)
   - Severity: HIGH
   - Example: 10x10 grid = 100 compilations per frame
   - Mitigation: Benchmark, consider compilation caching, or use Option 3

3. **Generic Type Handling** (Gemini)
   - Severity: MEDIUM
   - Issue: `CompiledFacetRow<InnerC>` vs `dyn CompiledMark` trait object
   - Mitigation: Requires careful type erasure or `Any` casting

### Confirmed Risks

- SessionContext thread safety: ✅ SAFE (both experts)
- Timeline accuracy: ✅ REALISTIC (if blocker solved)

---

## Top Priorities for Implementation

Based on expert feedback, address these BEFORE starting implementation:

### 1. **DECIDE ON SERIALIZATION** (CRITICAL)
Choose one:
- Drop it (Option 1)
- Implement PlotSpec (Option 2)
- Use simpler data override approach (Option 3)

**User input required**: What's the use case for serializing `CompiledPlot`? Can we break it?

### 2. **Clarify Requirements** (IMPORTANT)
Which problems MUST we solve?
- ✅ Facets reflect render-time data (not compile-time)
- ❓ Nested faceting support
- ❓ Serialization support

**If only first checkbox**: Use Option 3 (much simpler)

### 3. **Prototype Generic Handling** (if Option 1/2)
Before full implementation, verify:
- Can store `Arc<Plot<InnerC>>` or `Arc<PlotSpec>` in way that allows `CompiledFacetRow` to remain `dyn CompiledMark`
- Type erasure strategy works

---

## Recommended Next Steps

1. **User Decision**: Review Options 1, 2, 3 and choose based on priorities
2. **If Option 3 chosen**: Create simplified plan (1-2 weeks scope)
3. **If Option 1/2 chosen**: Address serialization strategy first, then proceed with original plan
4. **Prototype**: Build small proof-of-concept for chosen option before full implementation
5. **Performance Baseline**: Benchmark current facet rendering before changes

---

## Final Verdict

**Original Plan**: Architecturally sound but technically blocked by serialization

**Path Forward**: Depends on priorities:
- **Quick fix for render params**: Option 3 (1-2 weeks)
- **Full solution inc. nesting**: Option 1 or 2 (5-12 weeks)

**Expert Confidence**: Both experts confident in analysis; Gemini's serialization catch is critical and accurate.
