# Interior Mutability Elimination Plan

**Date:** 2025-11-14
**Context:** Serialization round-trip test failures due to cached state

## Current Situation

### The Problem

**Test Results:**
- ✅ **Direct rendering:** PASSES (0.9999+ similarity)
- ❌ **After serialization:** 0.8858 similarity

The serialization round-trip fails because the scene graph contains `Arc<Mutex<Option<OverflowSpaceRequirement>>>` caches that get serialized, then deserialized into the wrong state.

### Current Architecture: Interior Mutability Pattern

**Data Flow:**
```
[Pass 1: facet_evaluation.rs]
    ├─> Measures subplot overflow
    ├─> Computes max_overflow
    └─> WRITES to Arc<Mutex<>> via cached_edge_overflow.lock()
                 ↓
    [Serialization to scene graph]
                 ↓
    [Deserialization from scene graph]
                 ↓
[Guide rendering: guide.rs]
    └─> READS from Arc<Mutex<>> via cached_edge_overflow.lock()
```

**Files Involved:**

1. **`facet/marks/facet.rs`** (lines 27-47):
   - Custom serialization/deserialization for `Arc<Mutex<Option<...>>>`
   - `serialize_cached_overflow()` / `deserialize_cached_overflow()`

2. **`facet/marks/facet.rs`** (lines 153-159):
   - `CompiledFacetRow` has `cached_edge_overflow` field
   - Same for `CompiledFacetCol`, `CompiledFacetGrid`

3. **`facet/marks/facet_evaluation.rs`** (line 66):
   - Takes `cached_edge_overflow` parameter
   - Line 747-749: **WRITES** to cache after Pass 1

4. **`facet/guide.rs`** (lines 77-82, 681-684):
   - `FacetRowGuide` and `FacetColumnGuide` **READ** from cache
   - Used to avoid re-measuring overflow during guide layout

**Why This Breaks Serialization:**

When the scene graph is serialized:
1. ✅ The cache is correctly serialized with the measured overflow values
2. ✅ Deserialization recreates the `Arc<Mutex<...>>` with same values
3. ❌ **BUT**: These cached values are from a DIFFERENT rendering context
4. ❌ When guides use stale cached values, they calculate wrong dimensions
5. ❌ Result: 0.8858 similarity instead of 0.9999+

The cache is an **optimization to avoid re-measuring** overflow, but it assumes the same context between measurement and use. Serialization breaks this assumption.

---

## The Solution: Argument Passing via Coordinate Transform

### New Architecture: Pure Functions with State

**Data Flow:**
```
[Pass 1: facet_evaluation.rs]
    ├─> Measures subplot overflow → Vec<OverflowSpaceRequirement>
    ├─> Calls facet_coord.with_measured_padding(gap, overflow_vec)
    └─> Returns updated_coord with overflow_by_facet field set
                 ↓
    [Updated coord stored on compiled mark]
                 ↓
    [Serialization: overflow_by_facet serializes with coord]
                 ↓
    [Deserialization: overflow_by_facet restored correctly]
                 ↓
[Guide rendering: guide.rs]
    ├─> Gets coord from compiled mark
    └─> Reads coord.overflow_by_facet (pure data access, no mutex)
```

**Key Insight:** `overflow_by_facet` **ALREADY EXISTS** on facet coords! (see `facet/coord.rs`)

```rust
// FacetRow, FacetColumn, FacetGrid all have:
pub(crate) overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
```

**We're already passing the data via `with_measured_padding()`**, we just need to make guides READ from there instead of from the cache!

---

## Implementation Plan

### Phase A: Make Guides Read from Coord (Remove Cache Reads)

**Goal:** Guides use `coord.overflow_by_facet` instead of `cached_edge_overflow`

**Changes in `facet/guide.rs`:**

1. **Add coord parameter to guide functions** (or extract from compiled mark)
2. **Replace cache reads:**

```rust
// BEFORE (lines 77-82):
let cached_overflow = source.cached_edge_overflow.lock().ok()
    .and_then(|cache| cache.clone());

// AFTER:
let overflow_by_facet = source.coord.overflow_by_facet.as_ref();
```

3. **Update overflow aggregation logic:**
   - Instead of using a single `max_overflow` value for all subplots
   - Use per-facet overflow from `overflow_by_facet: Vec<OverflowSpaceRequirement>`
   - First subplot uses `overflow_by_facet[0].left`
   - Last subplot uses `overflow_by_facet[n-1].right`
   - This fixes the issue mentioned in FACET_STAGE4_FOLLOWUP_PLAN.md!

**Files to modify:**
- `facet/guide.rs` (FacetRowGuide, FacetColumnGuide, GridGuide)
- Ensure guides can access the coord from FacetSource

**Testing:**
- Run visual regression tests
- Verify serialization round-trip now passes
- Check that overflow values are correctly per-facet

---

### Phase B: Remove Cache from Compiled Marks (Delete Interior Mutability)

**Goal:** Delete `cached_edge_overflow` entirely

**Changes:**

1. **`facet/marks/facet.rs`:**
   - Delete `serialize_cached_overflow()` / `deserialize_cached_overflow()` (lines 27-47)
   - Remove `cached_edge_overflow` field from `CompiledFacetRow`, `CompiledFacetCol`, `CompiledFacetGrid`

2. **`facet/marks/facet_evaluation.rs`:**
   - Remove `cached_edge_overflow` parameter (line 66)
   - Delete cache write logic (lines 747-749)

3. **`facet/guide.rs`:**
   - Remove `cached_edge_overflow` from `FacetSource` struct
   - Already done in Phase A

**Benefits:**
- ✅ No more `Arc<Mutex<...>>` interior mutability
- ✅ Serialization round-trip works correctly
- ✅ Thread-safe by default (no locks)
- ✅ Easier to reason about (pure data flow)
- ✅ Matches Phase 7 goals in FACET_REFACTOR_PLAN.md

**Testing:**
- Ensure compilation succeeds
- Full visual regression suite
- Verify memory usage doesn't increase (overflow vecs are small)

---

### Phase C: Store Updated Coord on Compiled Mark

**Goal:** Make the updated coord with `overflow_by_facet` accessible to guides

**Current Issue:** The updated coord from `facet_coord.with_measured_padding()` is created in `facet_evaluation.rs` but may not be stored on the compiled mark.

**Changes:**

1. **Check if `CompiledFacetRow` etc. store the coord:**
   - If yes: Ensure it's the UPDATED coord (after `with_measured_padding()`)
   - If no: Add field to store updated coord

2. **Ensure guides can access it:**
   ```rust
   pub struct FacetSource {
       pub subplot: Arc<CompiledPlot>,
       pub data: DataContext,
       pub user_title: Option<String>,
       pub coord: Box<dyn CoordinateSystemTransform>, // ← Add this!
       // Remove: cached_edge_overflow
   }
   ```

3. **Pass updated coord when creating FacetSource:**
   ```rust
   self.facet_sources.push(FacetSource {
       subplot: facet.compiled_subplot.clone(),
       data: facet.state.data.clone(),
       user_title: facet.facet_title.clone(),
       coord: facet.updated_coord.clone(), // ← From with_measured_padding()
   });
   ```

**Coordination with Serialization:**
- The coord already serializes correctly (it's `#[derive(Serialize, Deserialize)]`)
- `overflow_by_facet` field has `#[serde(skip_serializing_if = "Option::is_none")]`
- This is perfect: it only serializes when set

---

## Benefits of This Refactor

### Correctness
1. ✅ **Serialization works:** No stale cached state
2. ✅ **Per-facet overflow:** Guides can use first/last subplot overflows correctly
3. ✅ **Deterministic:** No race conditions from mutexes

### Architecture
1. ✅ **Pure functions:** Data flows through arguments, not side effects
2. ✅ **Single source of truth:** Coord contains all state
3. ✅ **Composable:** Coords can be cloned/updated immutably

### Maintainability
1. ✅ **No locks:** Simpler code, easier to debug
2. ✅ **Testable:** Can test coord transformations in isolation
3. ✅ **Clear ownership:** No shared mutable state

---

## Risk Assessment

### Low Risk
- Coord serialization already works (tested in `facet/coord.rs`)
- Data is already flowing via `with_measured_padding()`
- Guides already have access to overflow data (just from wrong source)

### Medium Risk
- Need to ensure updated coord is stored on compiled mark
- Guides must handle `None` case gracefully (Phase 7 already plans for this)
- Grid facets have special handling (currently don't use cache, see line 1297)

### High Risk
- **NONE** - This is the INTENDED design from FACET_REFACTOR_PLAN.md Phase 7!

---

## Testing Strategy

### Unit Tests
1. Test coord `with_measured_padding()` creates correct state
2. Test guides read `overflow_by_facet` correctly
3. Test serialization round-trip preserves `overflow_by_facet`

### Integration Tests
1. **Visual regression:** All facet tests must pass
2. **Serialization tests:** `facet_column_custom_spacing` must achieve 0.9999+ after serialization
3. **Edge cases:**
   - Empty `overflow_by_facet` (guides should handle gracefully)
   - Grid facets (two dimensions)
   - Free vs shared scales

### Performance Tests
1. Memory usage (overflow vecs are small, should be negligible)
2. Rendering time (no locks = potentially faster!)

---

## Implementation Order

**Recommended sequence:**

1. **Phase C first** (Store updated coord on compiled mark)
   - Ensures guides have access to coord
   - Minimal risk, foundational change

2. **Phase A** (Make guides read from coord)
   - Can test with BOTH sources available (cache + coord)
   - Verify they produce same results
   - Then remove cache reads

3. **Phase B** (Delete cache entirely)
   - Safe once Phase A is verified
   - Clean up all interior mutability

---

## Alignment with FACET_REFACTOR_PLAN.md

This plan **implements Phase 7 Task 1:**

> "Replace the `cached_edge_overflow` mutexes by reading `overflow_by_facet` directly from the updated coordinate transform, and update `FacetRowGuide`/`FacetColGuide`/`GridFacetGuide` to tolerate missing data gracefully."

This is the INTENDED architecture. We're just accelerating Phase 7 because the serialization issue forces our hand.

---

## Next Steps

**Immediate:**
1. Investigate where updated coord is stored after `with_measured_padding()`
2. Verify guides have access to coord (or can easily get it)
3. Implement Phase C if needed

**Short term:**
4. Implement Phase A (guide reads from coord)
5. Test serialization round-trip
6. Verify 0.9999+ similarity

**Medium term:**
7. Implement Phase B (delete cache)
8. Run full test suite
9. Document the new data flow

---

## Open Questions

1. **Where is the updated coord currently stored after `with_measured_padding()`?**
   - Need to check if `CompiledFacetRow` etc. have a coord field
   - If not, where should we add it?

2. **Do guides currently have access to the compiled facet mark?**
   - Need to trace guide creation to see what data is available
   - May need to pass coord explicitly to guide functions

3. **How do Grid facets handle two dimensions?**
   - They have `row_overflow_by_facet` AND `col_overflow_by_facet`
   - Guides must handle both (already planned in FACET_STAGE4_FOLLOWUP_PLAN.md)

---

## Summary

**Problem:** Interior mutability via `Arc<Mutex<...>>` breaks serialization
**Solution:** Use argument passing via coord's `overflow_by_facet` field
**Status:** Architecture already supports this! Just need to wire it up.
**Risk:** Low - This is the intended design from Phase 7
**Benefit:** Fixes serialization + improves architecture + enables per-facet overflow

Let's eliminate interior mutability and make the system pure! 🎯
