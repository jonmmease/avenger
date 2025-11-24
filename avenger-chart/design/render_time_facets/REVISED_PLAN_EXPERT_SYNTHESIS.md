# Expert Review Synthesis: REVISED Render-Time Facets Plan

**Date**: 2025-11-23
**Reviewers**: GPT-5 Codex, Gemini 3 Pro
**Status**: **PROCEED WITH CRITICAL FIXES REQUIRED**

---

## Executive Summary

Both experts agree that the REVISED plan **successfully avoids the serialization blocker** and is **architecturally sound**. However, they identified a **CRITICAL DataFusion API issue** that must be fixed before implementation.

### Verdict: FEASIBLE with Required Modifications

**Key Finding**: The proposed `batch_to_dataframe()` implementation using `register_batch()` has significant problems:
1. **Memory leaks** - temp tables persist in catalog
2. **Performance overhead** - unnecessary catalog operations
3. **API usage errors** - async/sync handling issues

**Solution**: Both experts independently recommend the **same fix**: Use direct MemTable construction instead of temp table registration.

**Timeline Update**: 2-3 weeks is optimistic; **3-4 weeks is realistic**.

---

## 1. Serialization Blocker Analysis ✅

### Expert Consensus: BLOCKER AVOIDED

**Gemini 3 Pro**:
> "After exhaustive analysis... I can confirm that **this approach successfully avoids the serialization blocker** I identified in the first review."

**GPT-5 Codex**:
> "Avoids the serialization blocker by not storing uncompiled Plot"

### Why It Works

- No `Arc<Plot<InnerC>>` storage (was the blocker)
- Only stores `Arc<CompiledPlot>` (already serializable)
- Removing `distinct_keys` doesn't introduce new type issues
- `#[typetag::serde]` trait object system preserved

### Serialization Strategy Recommendation

**Both experts recommend**: Option B with backward compatibility

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,

    // Deprecated field for backward compatibility
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) distinct_keys: Vec<ScalarValue>,
}
```

**Benefits**:
- Reads old serialized format (ignores distinct_keys)
- Writes new format (omits distinct_keys)
- Provides smooth migration path

---

## 2. Critical DataFusion API Issue ❌

### The Problem

The plan's `batch_to_dataframe()` has **multiple critical issues**:

**Issue 1: Memory Leaks** (Both experts)
- `register_batch()` stores tables in SessionContext catalog
- Tables persist until SessionContext drops
- For 10x10 grid = 100 temp tables accumulating in memory
- **No automatic cleanup**

**Gemini**:
> "Temporary tables are NOT automatically cleaned up. The catalog maintains references indefinitely within the SessionContext lifetime."

**GPT-5**:
> "DataFusion's SessionContext maintains a catalog of registered tables. There's NO automatic cleanup when tables are no longer needed."

**Issue 2: Performance Overhead** (GPT-5)
Each `register_batch()` involves:
- Cloning the RecordBatch
- Creating catalog metadata
- Storing in HashMap
- Building logical plan on `table()` call

**Issue 3: API Usage Errors** (Gemini)
```rust
// WRONG in plan (missing async handling):
let df = ctx.table(&temp_name).await?;

// table() is async and must be awaited
```

### The Solution: Direct MemTable Construction

**Both experts independently recommend the SAME approach**:

**GPT-5 Codex version**:
```rust
fn batch_to_dataframe(
    batch: &RecordBatch,
    ctx: &SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let provider = datafusion::datasource::MemTable::try_new(
        batch.schema(),
        vec![vec![batch.clone()]]
    )?;

    let logical_plan = LogicalPlanBuilder::scan(
        "temp",
        Arc::new(provider) as Arc<dyn TableProvider>,
        None
    )?.build()?;

    Ok(DataFrame::new(ctx.state(), logical_plan))
}
```

**Gemini version** (nearly identical):
```rust
async fn batch_to_dataframe(
    batch: &RecordBatch,
    ctx: &SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let provider = MemTable::try_new(
        batch.schema(),
        vec![vec![batch.clone()]]
    )?;

    let plan = LogicalPlanBuilder::scan(
        "temp",
        provider_as_source(Arc::new(provider)),
        None
    )?.build()?;

    Ok(DataFrame::new(ctx.state().clone(), plan))
}
```

**Benefits**:
- No catalog registration (no memory leaks)
- No temp table naming collisions
- Faster (no catalog operations)
- Simpler (no cleanup needed)

**Action Required**: **MUST** replace the plan's `batch_to_dataframe()` with this approach.

---

## 3. Additional Issues Identified

### Scale Domain Synchronization (GPT-5)

**Issue**: Plan assumes "compile-time scale domain doesn't matter" but doesn't validate render-time keys match scale domain.

**Risk**: Render-time data has NEW facet keys not in compile-time domain → scale positions undefined.

**Solution**: Add validation:
```rust
let domain_vals = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;

// Validate against compile-time scale domain
for key in &domain_vals {
    if !scale_domain.contains(key) {
        log::warn!("Render-time facet key {:?} not in compile-time domain", key);
        // Either skip or error based on strategy
    }
}
```

### Empty Facet Handling (Both)

**Edge cases not considered**:
- Empty render-time data (no rows)
- New facet values at render time
- Missing facet values at render time

**Gemini**:
> "What if render-time data has no rows for a compile-time key? What if render-time data has NEW keys not in compile-time data?"

**Recommendation**: Add defensive checks and clear error messages.

### Performance Regression Risk (Both)

**GPT-5**:
> "For a 10x10 grid, that's 100 DataFrame conversions + filters."

**Gemini**:
> "Moving key extraction from compile to render time... Could be 100+ conversions for large grids."

**Mitigation**: Add caching for DataFrame conversions:
```rust
// Add to RenderContext
pub struct RenderContext {
    // ...
    batch_cache: Arc<Mutex<HashMap<*const RecordBatch, DataFrame>>>,
}
```

---

## 4. Timeline Revision

### Original Plan: 2-3 weeks

### Expert Assessment: 3-4 weeks realistic

**GPT-5 Breakdown**:
- Week 1: Structural changes ✅
- Week 2: Render logic + fix DataFrame conversion issue
- Week 3: Testing + performance optimization
- Week 4: Buffer for discovered issues

**Risk factors for delays**:
1. DataFrame conversion performance requiring trait changes (+1 week)
2. Scale domain mismatch issues (+3 days)
3. Memory leak debugging (+2 days)
4. Extensive test failures (+3 days)

**Gemini Assessment**:
> "2-3 weeks is OPTIMISTIC given DataFusion complexities. **Realistic Timeline**: **3-4 weeks** with experienced developer, 4-5 weeks otherwise."

**Recommendation**: **Plan for 3-4 weeks** with early performance testing.

---

## 5. Risk Assessment Matrix

| Risk | GPT-5 Severity | Gemini Severity | Consensus |
|------|----------------|-----------------|-----------|
| **DataFusion API misuse** | HIGH | HIGH | **CRITICAL - MUST FIX** |
| **Memory leaks from temp tables** | HIGH | HIGH | **CRITICAL - USE MemTable** |
| **Performance regression >10%** | MEDIUM | MEDIUM-HIGH | **HIGH - BENCHMARK EARLY** |
| **Serialization breaking change** | MEDIUM | MEDIUM | **MEDIUM - Use serde attrs** |
| **Scale domain sync issues** | MEDIUM (new) | MEDIUM | **MEDIUM - Add validation** |
| **Empty facet handling** | LOW (new) | LOW | **LOW - Add defensive checks** |

### Overall Risk Level

- **GPT-5**: "FEASIBLE with caveats"
- **Gemini**: "MEDIUM-HIGH implementation risk"

**Consensus**: **MEDIUM-HIGH** with required fixes, **HIGH** without fixes.

---

## 6. Implementation Recommendations

### Phase 0: Critical Prerequisites (NEW - 2-3 days)

**Both experts emphasize**: Prototype and benchmark BEFORE full implementation.

**Tasks**:
1. **Implement correct `batch_to_dataframe()`** using MemTable approach
2. **Create performance benchmarks** for current facet rendering
3. **Prototype conversion** with single FacetRow test
4. **Measure overhead** of DataFrame conversion
5. **Decide threshold**: If >10% regression, consider trait signature change

### Phase 1: Structural Changes (Week 1)

**As planned**, but with serialization compatibility:
- Remove `distinct_keys` from structs
- Use `#[serde(default, skip_serializing_if = "Vec::is_empty")]`
- Remove key extraction from compile methods
- **Verify**: Compilation succeeds, no behavioral changes

### Phase 2: Render-Time Logic (Week 2)

**Modified from plan**:
- Use **corrected** `batch_to_dataframe()` (MemTable approach)
- Add scale domain validation
- Add defensive checks for empty data
- Update all three facet types (Row, Col, Grid)
- **Verify**: Nested facet test passes without crash

### Phase 3: Performance & Testing (Week 3)

**Enhanced testing requirements**:
- Performance regression tests (<10% threshold)
- Memory leak tests (long-running with many facets)
- Edge case tests (empty data, new keys, null values)
- Serialization compatibility tests
- **Verify**: All tests pass, performance acceptable

### Phase 4: Buffer & Documentation (Week 4)

**New phase for discovered issues**:
- Address any performance issues found
- Document limitations (no user-facing API yet)
- Migration guide for breaking changes
- **Verify**: Ready for release

---

## 7. Alternative Approaches Considered

### Option: Change CompiledMark Trait Signature

**Both experts suggest** considering this if performance is unacceptable:

```rust
async fn evaluate_from_data(
    &self,
    data: Option<DataFrame>,  // Changed from RecordBatch
    scalars: &RecordBatch,
    context: &RenderContext,
    coord: Box<dyn CoordinateSystemTransform>,
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError>
```

**Pros**:
- No conversion overhead
- Cleaner abstraction

**Cons**:
- Breaking change to trait (affects all 8+ mark implementations)
- Requires updating all mark types

**GPT-5 Recommendation**:
> "Consider this for Phase 2. Start with RecordBatch conversion, measure performance, then decide."

**Gemini Recommendation**:
> "Better approach long-term but significant implementation effort."

**Decision**: Defer to follow-up work unless performance unacceptable.

---

## 8. Code-Specific Corrections

### Line 215-234: batch_to_dataframe

**REPLACE** the plan's implementation with expert-recommended MemTable approach (see Section 2).

### Line 283-308: Add Domain Validation

**ADD** after key extraction:
```rust
let domain_vals = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;

// Validate against scale domain
let scale_domain = /* extract from dimension_scale */;
for key in &domain_vals {
    if !scale_domain.contains(key) {
        return Err(AvengerChartError::InvalidInput(
            format!("Render-time facet key {:?} not in scale domain", key)
        ));
    }
}
```

### Line 367-371: Add Caching (Optional Optimization)

**CONSIDER** adding DataFrame conversion cache to avoid redundant conversions.

---

## 9. Success Criteria (Updated)

### Must Have ✅

- [ ] **CRITICAL**: Use MemTable approach for DataFrame conversion (not register_batch)
- [ ] Compile succeeds for nested facets (no infinite recursion)
- [ ] Nested facet test passes (FacetRow inside FacetColumn)
- [ ] Facets extract keys from render-time data (not compile-time)
- [ ] Scale domain validation implemented
- [ ] All existing facet tests pass (after updates)
- [ ] No memory leaks from DataFrame conversions
- [ ] Performance within 10% of current implementation

### Should Have 📋

- [ ] Backward serde compatibility (can read old format)
- [ ] Clear migration guide for breaking changes
- [ ] Benchmark suite for facet performance
- [ ] Defensive handling of edge cases (empty data, new keys)
- [ ] Performance regression tests

### Nice to Have 💡

- [ ] DataFrame conversion caching
- [ ] User-facing API for render-time data override (defer to follow-up)
- [ ] Parallelization of key extraction for large grids

---

## 10. Expert Recommendations Summary

### GPT-5 Codex Final Recommendation

> **PROCEED with modifications**
>
> The plan is fundamentally sound but requires:
> 1. Fix DataFrame conversion approach (critical)
> 2. Add scale domain validation (important)
> 3. Implement proper serialization compatibility (important)
> 4. Plan for 3-4 weeks instead of 2-3 (realistic)
> 5. Add comprehensive benchmarking (critical)

### Gemini 3 Pro Final Recommendation

> **PROCEED WITH CAUTION**
>
> Fix the DataFusion API approach first (recommend direct DataFrame construction without temp tables), then follow the plan with the suggested modifications.
>
> The 2-3 week timeline is optimistic - budget 3-4 weeks minimum and have a fallback plan if performance is unacceptable.
>
> **Success Probability**: **65-70%** with the recommended fixes, dropping to **40%** if DataFusion issues aren't addressed properly.

---

## 11. Final Verdict

### Can We Proceed? **YES, with required fixes**

**What MUST change before implementation**:

1. ✅ **Replace `batch_to_dataframe()`** with MemTable approach (CRITICAL)
2. ✅ **Add scale domain validation** (IMPORTANT)
3. ✅ **Use serde backward compatibility** for distinct_keys (IMPORTANT)
4. ✅ **Plan 3-4 weeks, not 2-3** (REALISTIC)
5. ✅ **Add Phase 0 for prototyping** (CRITICAL)

**What stays the same**:

- ✅ Remove distinct_keys from compile time
- ✅ Extract keys at render time
- ✅ Use recursive data overrides
- ✅ Structural changes to facet.rs
- ✅ Overall architectural approach

**Success probability with fixes**: **65-75%** (both experts agree)

---

## 12. Recommended Next Steps

### Immediate (This Week)

1. **Review this synthesis** with stakeholders
2. **Decide**: Proceed with fixes or explore alternatives?
3. **Create Phase 0 branch** for prototyping

### Short-Term (Next Week)

1. **Implement correct `batch_to_dataframe()`** using MemTable
2. **Create baseline benchmarks** for current facet performance
3. **Prototype with FacetRow** to validate approach
4. **Measure conversion overhead** and assess acceptability

### Medium-Term (Weeks 2-4)

1. **Full implementation** if prototype successful
2. **Comprehensive testing** including edge cases
3. **Performance optimization** if needed
4. **Documentation and migration guide**

---

## Conclusion

The REVISED plan successfully addresses the original serialization blocker and provides an elegant solution via recursive data overrides. However, both experts independently identified a **critical DataFusion API issue** that would cause memory leaks and performance problems.

**With the expert-recommended fixes**, this plan is:
- ✅ Architecturally sound
- ✅ Technically feasible
- ✅ Solves both render params AND nested faceting
- ✅ Avoids serialization blocker
- ⚠️ Requires 3-4 weeks (not 2-3)
- ⚠️ Success probability 65-75% with proper implementation

**The path forward is clear**: Implement the MemTable-based DataFrame conversion, add validation, prototype early, and proceed with cautious optimism.

---

## Supporting Documents

- **REVISED_PLAN.md**: Original detailed implementation plan
- **GPT-5 Codex Review**: Full expert analysis (Task output)
- **Gemini 3 Pro Review**: Full expert analysis (Task output)
- **Original PLAN.md**: First approach with serialization blocker
- **EXPERT_REVIEW_SYNTHESIS.md**: Synthesis of original plan reviews

**All expert recommendations have been incorporated into this synthesis.**
**Ready for stakeholder decision and Phase 0 implementation.**
