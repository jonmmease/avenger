# Nested Faceting Analysis and Implementation Guide

This directory contains the complete expert analysis and implementation roadmap for solving the nested faceting stack overflow issue in avenger-chart.

## Documents (Read in Order)

### 00_FINAL_REPORT.md (Start Here)
**Executive summary of expert recommendations**
- Problem analysis and root cause
- Recommended solution: Lazy Compilation
- Implementation roadmap overview
- Risk mitigation strategy
- Next steps and critical success factors

**Read time**: 15-20 minutes

### 01_expert_discussion.md (Detailed Analysis)
**Complete expert panel analysis with design precedents**
- Comprehensive problem analysis
- Three solution approaches compared
- Design precedents from Vega, D3, Observable, Plotly
- Detailed risk mitigation strategy
- Implementation strategy outline with code examples

**Read time**: 30-40 minutes

### 02_key_files_referenced.md (Architecture Reference)
**Architectural overview and file locations**
- Core faceting implementation files
- Data flow architecture
- Type system and traits
- Configuration and state management
- Related modules and dependencies

**Read time**: 20-25 minutes

### 03_implementation_roadmap.md (Detailed Plan)
**Phase-by-phase implementation with specific tasks**
- Phase 1: Structural Foundation (1-2 weeks)
- Phase 2: Lazy Compilation Logic (2-3 weeks)
- Phase 3: Optimization (1-2 weeks)
- Phase 4: Documentation (1 week)
- Each phase includes specific tasks with code examples
- Risk assessment and integration checkpoints

**Read time**: 45-60 minutes

### 04_discussion_summary.txt (Quick Reference)
**Executive summary and quick lookup**
- Key findings and recommendation
- Implementation phases at a glance
- Comparison with alternatives
- Answers to key questions
- Next steps for stakeholders

**Read time**: 10-15 minutes

## Quick Facts

**Problem**: Nested faceting (FacetRow inside FacetColumn) causes stack overflow during compilation

**Root Cause**: Data flow mismatch - inner facet receives full dataset at compile time but filtered data at evaluation time

**Recommended Solution**: Lazy Compilation for Nested Facets
- Defer compilation until filtered data available
- Maintains invariant: "Compiled marks match their data"
- Aligns with industry standards (Vega, D3)

**Implementation Timeline**: 8 weeks
- Phase 1: Structural changes (1-2 weeks)
- Phase 2: Lazy compilation logic (2-3 weeks)
- Phase 3: Optimization (1-2 weeks)
- Phase 4: Documentation (1 week)

**Key Principle**: "Compiled marks should always match the data they were compiled for"

## Implementation Checklist

### Phase 1: Structural Foundation
- [ ] Modify CompiledFacetCol struct
- [ ] Modify CompiledFacetRow struct
- [ ] Implement nesting detection
- [ ] Add unit tests for detection
- [ ] Update CompiledFacetGrid struct

### Phase 2: Lazy Compilation Logic
- [ ] Add Plot::compile_with_data() method
- [ ] Add branching in evaluate_from_data()
- [ ] Implement lazy evaluation helpers
- [ ] Add integration tests
- [ ] Ensure Pass 1/Pass 2 consistency

### Phase 3: Optimization
- [ ] Implement parallel compilation
- [ ] Add caching layer
- [ ] Performance benchmarking

### Phase 4: Documentation
- [ ] Update architecture docs
- [ ] Create example code
- [ ] Document performance
- [ ] Create migration guide

## Critical Files to Modify

1. **avenger-chart/src/facet/marks/facet.rs**
   - Add uncompiled_subplot fields to CompiledFacetCol/Row/Grid
   - Implement nesting detection in compile() methods
   - Add evaluate_with_lazy_compilation() helpers

2. **avenger-chart/src/plot/plot.rs**
   - Add compile_with_data() method to Plot trait

3. **avenger-chart/src/facet/marks/facet_evaluation.rs**
   - Support lazy compilation path in evaluate_facet()
   - Add lazy evaluation variant

4. **avenger-chart/src/render/render_context.rs** (if needed)
   - Add with_filtered_data() method to RenderContext

## Key Design Decisions

1. **Detect nesting at compile time**: Check if inner marks contain facet types
2. **Store uncompiled plot optionally**: Only when nesting detected (opt-in)
3. **Branch in evaluate_from_data()**: Different path for lazy vs eager compilation
4. **Reuse evaluation logic**: Both paths use same measurement/rendering algorithm
5. **Preserve single-level performance**: No impact on non-nested facets

## Risk Mitigation Summary

| Risk | Mitigation |
|------|-----------|
| Compilation cost to render time | Parallelize, cache, lazy detection only |
| Breaking API changes | Keep single-level facets unchanged |
| Serialization issues | Test early, update Serde impls |
| Thread safety | Use Arc/Mutex correctly in parallel path |
| Performance regression | Benchmark regression tests |

## Expected Outcomes

After complete implementation:
- ✅ Nested FacetColumn with FacetRow works
- ✅ Deeply nested facets (3+ levels) supported
- ✅ No stack overflow or panics
- ✅ Single-level faceting unchanged
- ✅ Performance acceptable with optimization
- ✅ Clear code structure with separated lazy/eager paths

## Questions Answered

1. **Best architectural approach?** Lazy compilation deferred until filtered data
2. **Pass filtered data through context?** No, recompile with filtered data
3. **Add data parameter to evaluation?** Not needed, handled in compilation
4. **Maintain Pass 1/Pass 2 consistency?** By design, each level compiles with its data
5. **Industry precedents?** Yes, Vega and D3 both use this pattern

## Timeline

```
Week 1-2:   Phase 1 - Structural foundation
Week 3-5:   Phase 2 - Lazy compilation logic
Week 6-7:   Phase 3 - Optimization
Week 8:     Phase 4 - Documentation
            Release ready
```

## Contact and Feedback

This analysis was prepared based on expert panel review of:
- Current avenger-chart faceting architecture
- Stack overflow reproduction and root cause analysis
- Design patterns from Vega, D3, and Observable
- Rust async/await considerations

For questions or clarifications, refer to the detailed documents or consult with the avenger-chart architecture team.

---

**Analysis Date**: 2025-11-23  
**Status**: Ready for implementation  
**Confidence**: High (aligns with industry standards and proven patterns)
