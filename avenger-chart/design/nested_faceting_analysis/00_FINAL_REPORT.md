# Expert Discussion Report: Nested Faceting in Avenger-Chart

**Status**: Expert recommendations complete and ready for implementation  
**Date**: 2025-11-23  
**Expert Analysis**: Comprehensive architecture review and implementation roadmap

---

## Executive Summary

An expert panel has completed a thorough analysis of the nested faceting stack overflow issue in avenger-chart. The consensus recommendation is to implement **Lazy Compilation for Nested Facets**.

**Key Finding**: The problem stems from a fundamental data flow mismatch—inner facets receive the full dataset at compile time but filtered data at evaluation time, creating infinite recursion.

**Solution**: Defer compilation of nested facets until evaluation time, when filtered data is available. This maintains the critical invariant that **compiled marks always match the data they were compiled for**.

**Timeline**: 8 weeks for full production-quality implementation (4 phases)

**Verdict**: Lazy compilation is not a workaround—it's the correct architectural solution, proven by production visualization systems like Vega and D3.

---

## Problem Analysis

### Root Cause: Data Flow Mismatch

The current architecture has a fundamental contract violation between compilation and evaluation:

```
COMPILE TIME:
Plot<FacetColumn>::compile(full_dataset)
  └─> Facet::compile(full_dataset)
      └─> Plot<FacetRow>::compile(full_dataset)  ← Still full dataset!
          └─> STACK OVERFLOW: Infinite recursion
```

```
EVALUATION TIME (if it worked):
CompiledFacetCol::evaluate_from_data()
  └─> Filter by col_value → filtered_dataset
      └─> Pass filtered_dataset to CompiledFacetRow
          └─> Expects data filtered by parent
```

**The Mismatch**: Code compiled for full dataset fails when given filtered data.

### Current Architecture Issues

1. **Early Compilation**: `Arc<CompiledPlot>` created at plot compile time
2. **Lost Context**: Inner plot unaware of parent facet's filtering
3. **No Lazy Path**: Nested facets can't defer until they have filtered data
4. **Violates Principle**: Compilation depends on runtime filter values

---

## Recommended Solution: Lazy Compilation

### Overview

**Defer compilation of nested facets until evaluation time.**

When the system detects nesting:
1. At evaluation: Filter data FIRST
2. Then: Compile inner plot with filtered data
3. Finally: Evaluate with consistent data context

### Key Principle

> **Compiled marks should always match the data they were compiled for**

This invariant prevents stack overflow and ensures system predictability.

### Hybrid Architecture

**Non-nested facets** (most common case):
- Current eager compilation (unchanged)
- Single compilation pass, multiple evaluations
- Performance: Fast (unchanged)

**Nested facets** (new capability):
- Lazy compilation (compile with filtered data)
- Multiple compilations, one per facet value
- Performance: Acceptable with optimization (Phase 3)

---

## Why This Approach is Best

### Comparison with Alternatives

| Criterion | Lazy Compilation | Data Threading | Template Compilation |
|-----------|------------------|-----------------|----------------------|
| **Arbitrary Nesting** | ✅ Yes | ⚠️ Limited | ⚠️ Limited |
| **Data Consistency** | ✅ Perfect | ✅ Explicit | ❌ Deferred |
| **Compilation Cost** | ⚠️ Runtime | ✅ Upfront | ✅ Single |
| **Code Simplicity** | ✅ Contained | ❌ Pervasive | ❌ Complex |
| **Maintainability** | ✅ Clear | ❌ Threaded | ❌ Generic |
| **Industry Standard** | ✅ Vega/D3 | ❌ Custom | ❌ Custom |

### Industry Precedent

**Vega**: Each facet level has explicit data flow and compilation
- Facets are declarative specs
- Each level transforms progressively
- Supports arbitrary nesting naturally

**D3**: Data binding deferred to visualization time
- No separate compilation phase
- Progressive selection nesting matches hierarchical filtering
- Supports arbitrary group nesting

**Observable**: Progressive data transformation
- Each facet level filters progressively
- Transformation order explicit in specification
- Compiles to Vega marks after transformations complete

---

## Implementation Roadmap

### Phase 1: Structural Foundation (1-2 weeks)
**Low risk, mechanical changes**

- Add `uncompiled_subplot: Option<Arc<Plot<InnerC>>>` to CompiledFacet types
- Implement nesting detection at compile time
- Create unit tests for detection logic
- Deliverable: Build succeeds, no behavioral changes

### Phase 2: Lazy Compilation Logic (2-3 weeks)
**Medium risk, complex control flow**

- Add `Plot::compile_with_data()` method
- Add branching in `evaluate_from_data()` for lazy path
- Implement lazy evaluation helpers
- Create integration tests for nesting scenarios
- Deliverable: Nested facets compile and render correctly

### Phase 3: Optimization (1-2 weeks)
**Medium risk, performance-critical**

- Parallelize nested facet compilation
- Implement result caching
- Performance benchmarking
- Deliverable: Performance meets or exceeds requirements

### Phase 4: Documentation (1 week)
**Low risk, essential for adoption**

- Update architecture documentation
- Create example visualizations
- Document performance characteristics
- Create migration guide
- Deliverable: Complete documentation for users

**Total Effort**: ~8 weeks for production-quality implementation

---

## Answers to Key Questions

### Q1: What's the best architectural approach?
**A**: Lazy compilation of nested facets, deferred until filtered data available.
- Matches industry patterns (Vega, D3)
- Supports arbitrary nesting depth
- Maintains data/code consistency

### Q2: Should we pass filtered DataFrame through compilation context?
**A**: No. Recompile inner plot with filtered DataFrame as input.
- More explicit than threading through context
- Each level controls its own constraints
- Simpler API and cleaner data flow

### Q3: Should we add a data parameter to evaluation methods?
**A**: Not needed. Lazy compilation approach handles it naturally.
- Data threading happens at compilation time
- Evaluation methods work with compiled structure
- Cleaner separation of concerns

### Q4: How do we maintain consistency across measurement and rendering passes?
**A**: By design. Each facet level compiles with exact data it will evaluate.
- Pass 1 and Pass 2 use identical data and parameters
- Nested facets each compile with their filtered data
- No data transformation between compilation and evaluation

### Q5: Are there precedents in visualization libraries like Vega/D3?
**A**: Yes. Both support arbitrary nesting via progressive data transformation.
- Vega: Declarative facet specs with explicit data flow
- D3: Deferred data binding at visualization time
- Lazy compilation is the standard pattern in visualization systems

---

## Risk Mitigation Strategy

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|-----------|
| **Compilation cost moves to render time** | Medium | High | Parallelize, cache, lazy detection |
| **Breaking API changes** | Low | High | Keep single-level facets unchanged |
| **Serialization issues** | Medium | Medium | Test serialization early (Phase 1) |
| **Thread safety in parallel** | Medium | High | Use Arc/Mutex patterns correctly |
| **Performance regressions** | Medium | Medium | Benchmark regression tests |

---

## Critical Success Factors

1. **Maintain Invariant**: Compiled marks always match their data
2. **Support Deep Nesting**: 3+ nesting levels work correctly
3. **Zero Regression**: Single-level faceting performance unchanged
4. **Clear Separation**: Lazy and eager paths clearly distinguished
5. **Comprehensive Tests**: All nesting scenarios covered

---

## What This Means for Implementation

### Structural Changes (Phase 1)

```rust
// Modified struct to support optional lazy compilation
pub struct CompiledFacetCol {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) uncompiled_subplot: Option<Arc<Plot<Cartesian>>>, // NEW
    // ... existing fields
}
```

### Lazy Compilation Branch (Phase 2)

```rust
// In evaluate_from_data()
if let Some(uncompiled) = &self.uncompiled_subplot {
    // Lazy path: compile with filtered data for each facet value
    self.evaluate_with_lazy_compilation(uncompiled, ...).await
} else {
    // Eager path: use current single-compilation approach
    evaluate_facet(...).await
}
```

### Key Method to Implement

```rust
// Add to Plot trait
impl<C: CoordinateSystem> Plot<C> {
    pub async fn compile_with_data(
        self,
        df: &DataFrame,
        session_context: &SessionContext,
    ) -> Result<Arc<CompiledPlot>, AvengerChartError> {
        // Compile with specific data context
    }
}
```

---

## Next Steps for Stakeholders

### Immediate (This Week)
1. Review expert recommendations with architecture team
2. Validate approach aligns with project goals
3. Discuss potential concerns or edge cases

### Short-Term (Next Sprint)
1. Design detailed API for Phase 1 changes
2. Get team feedback on structure modifications
3. Begin Phase 1 implementation

### Medium-Term (Following Sprints)
1. Implement Phase 2 (lazy compilation logic)
2. Conduct integration testing
3. Run performance benchmarks
4. Collect feedback from early adopters

### Long-Term
1. Implement Phase 3 (optimization)
2. Complete Phase 4 (documentation)
3. Release in minor version update

---

## Conclusion

The nested faceting problem reveals a deeper architectural tension between:
- **Early compilation** (current approach)
- **Late data binding** (needed for nesting)

The solution is a **hybrid strategy**:
- Keep early compilation for simple facets (fast path)
- Add lazy compilation for nested facets (correct path)

This approach is:
- ✅ Architecturally sound (matches industry standards)
- ✅ Practically implementable (clear 4-phase plan)
- ✅ Backwards compatible (existing code unchanged)
- ✅ Performance viable (optimization path defined)
- ✅ Maintainable (clear separation of concerns)

**Expert consensus**: Lazy compilation is not a workaround—it's the **correct architectural solution** for faceted visualization systems.

---

## Supporting Documents

Four detailed supporting documents accompany this report:

1. **nested_faceting_expert_discussion.md** (Full analysis)
   - Complete problem analysis
   - Solution architecture with code examples
   - Design precedents from Vega/D3/Observable
   - Detailed risk mitigation

2. **key_files_referenced.md** (Architecture reference)
   - File locations and dependencies
   - Data flow diagrams
   - Type system hierarchy
   - Current implementation patterns

3. **implementation_roadmap.md** (Detailed plan)
   - Phase-by-phase implementation tasks
   - Code examples for each task
   - Risk assessment per phase
   - Success criteria and checkpoints

4. **DISCUSSION_SUMMARY.txt** (Executive summary)
   - Quick reference guide
   - Key findings summary
   - Recommendations to stakeholders

---

**Report Complete**
All supporting analysis and implementation guidance available.
Ready to proceed to Phase 1: Structural Foundation.
