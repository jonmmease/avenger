# Render-Time Facets Implementation

This directory contains the design and implementation plan for moving facet calculations from compile time to render time.

## Quick Start

📄 **Read this first**: [CONSOLIDATED_PLAN.md](./CONSOLIDATED_PLAN.md) - **Authoritative implementation plan with all updates**

This consolidated document includes:
- Current status (Phase 1 complete, Phase 2 ready)
- Complete Phase 2 implementation tasks with refined Option 5
- Expert validation results and conflict resolution
- Risk assessment, timeline, and success criteria

## Problem Statement

The current facet system has two critical issues:

1. **Render Params Problem**: Facet keys are extracted at compile time from the full dataset, but render params can transform the data, causing a mismatch between compile-time keys and render-time data.

2. **Nested Faceting Problem**: Nested facets (e.g., FacetRow inside FacetColumn) cause infinite recursion during compilation because inner facets try to compile with the full dataset instead of filtered data.

## Solution Approach

**Recursive Data Overrides** - Move facet key extraction from compile time to render time and use the existing data override mechanism recursively.

Key changes:
1. Remove `distinct_keys` field from `CompiledFacet*` structs
2. Extract keys at render time from actual data (compile-time OR parent's filtered data)
3. Use existing `data` parameter in `evaluate_from_data()` to pass filtered data recursively

**Timeline**: 2-3 weeks (3 phases)

## Active Documents

### 1. CONSOLIDATED_PLAN.md ⭐ AUTHORITATIVE
**Purpose**: Single source of truth for implementation plan and current status

**Contents**:
- Executive summary with current status
- Complete Phase 2 tasks with refined Option 5 (trait-based approach)
- Expert validation results (GPT-5 Codex & Gemini 3 Pro consensus)
- Conflict resolution (RecordBatch column loss issue)
- Risk assessment (all critical risks resolved)
- Implementation checklist with detailed code examples
- Timeline and success criteria

**Status**: Updated 2025-11-23 - Ready for Phase 2 implementation

### 2. CONFLICT_RESOLUTION.md
**Purpose**: Deep dive on expert conflict and resolution

**Expert reviewers**: GPT-5 Codex (primary), Gemini 3 Pro

**Key findings**:
- ❌ Original concern: RecordBatch only contains facet channel, missing data columns
- ✅ Resolution: Add `wants_full_data_batch()` trait method
- ✅ Critical fixes: param application, empty batch with schema
- ✅ Consensus: Phase 2 feasible with refined approach

**Status**: Resolved - incorporated into CONSOLIDATED_PLAN.md

### 3. NESTED_FACET_DATA_FLOW_INVESTIGATION.md
**Purpose**: Complete data flow trace for nested facets

**Key findings**:
- Mechanism already exists for data overrides
- Just need to use existing `data` parameter (remove underscore)
- No new plumbing required at evaluate_facet level
- BUT: RecordBatch loses columns (fixed by refined Option 5)

**Status**: Investigation complete, findings validated by experts

## Implementation Progress

### ✅ Phase 0: Prototyping (COMPLETE)
- Implemented `batch_to_dataframe()` helper using `SessionContext::read_batch()`
- All 41 facet visual tests pass
- Commit: `54c336c4`

### ✅ Phase 1: Structural Changes (COMPLETE)
- Removed `distinct_keys` from CompiledFacetRow/Col/Grid structs
- Removed key extraction from all compile() methods
- All tests still passing
- Commit: `cf006b42`

### ✅ Phase 1.5: Expert Validation (COMPLETE)
- Deep investigation of nested facet data flow
- Expert reviews by GPT-5 Codex and Gemini 3 Pro
- Critical blocker identified and resolved
- Refined Option 5 validated by both experts

### 🔄 Phase 2: Render-Time Logic (READY TO START) ← WE ARE HERE
- Add `wants_full_data_batch()` trait method
- Modify `evaluate_mark_with_plot_df` for full DataFrame preservation
- Update `evaluate_facet` signature (add data_override parameter)
- Add render-time key extraction
- Update facet evaluate_from_data methods
- **Blockers**: None ✅
- **Timeline**: Week 2

### ⏸️ Phase 3: Testing & Validation (PENDING)
- Update tests for new behavior
- Enable nested facet test
- Performance benchmarking
- Memory leak validation
- Timeline: Week 3

## Archive

The [archive/](./archive/) directory contains the original plan and expert review that were superseded by the revised approach. See [archive/README.md](./archive/README.md) for details on the evolution of the plan.

## Related Work

- **Nested Faceting Analysis**: [../nested_faceting_analysis/](../nested_faceting_analysis/) - Original analysis of the nested faceting stack overflow issue, which led to the lazy compilation approach recommendation (8-week timeline). The current render-time approach is simpler and solves the same problem.

## Key Decisions

1. ✅ **DataFusion Helper**: Use `read_batch()` API (official, validated)
2. 🔄 **Serialization Strategy**: TBD - recommend backward compatibility via serde attributes
3. 🔄 **User-Facing API**: Defer to follow-up work (internal plumbing first)

## Files Modified

### Implementation
- `avenger-chart/src/facet/marks/facet_evaluation.rs` - Added batch_to_dataframe() helper
- `avenger-chart/src/facet/marks/facet.rs` - Will remove distinct_keys (Phase 1)

### Testing
- `avenger-chart/tests/visual_tests/test_nested_facets.rs` - Nested facet test (currently ignored)

## Expert Reviews

Two independent expert reviews were conducted:

1. **GPT-5 Codex** - Architecture and implementation feasibility
2. **Gemini 3 Pro** - Technical correctness and edge cases

Both experts:
- ✅ Confirmed approach avoids serialization blocker
- ✅ Validated recursive data overrides concept
- ❌ Identified critical DataFusion API issue (now resolved)
- ✅ Recommended same fix independently

## Success Criteria

### Must Have ✅
- [ ] Compile succeeds for nested facets (no infinite recursion)
- [ ] Nested facet test passes (FacetRow inside FacetColumn)
- [ ] Facets extract keys from render-time data (not compile-time)
- [ ] All existing facet tests pass (after updates)
- [x] No memory leaks from DataFrame conversions

### Should Have 📋
- [ ] Performance within 10% of current implementation
- [ ] Backward serde compatibility (can read old format)
- [ ] Clear migration guide for breaking changes
- [ ] Benchmark suite for facet performance

### Nice to Have 💡
- [ ] User-facing API for render-time data override (defer to follow-up)
- [ ] Parallelization of key extraction for large grids
- [ ] Optimization: cache DataFrame conversion

## Timeline

```
Week 1: Structural Changes
  └─ Remove distinct_keys from compiled structs
  └─ Remove compile-time key extraction

Week 2: Render-Time Logic
  └─ Update evaluate_facet signature
  └─ Add render-time key extraction
  └─ Update evaluate_from_data methods

Week 3: Testing & Validation
  └─ Update tests
  └─ Enable nested facet test
  └─ Performance benchmarking
  └─ Documentation
```

## Questions?

For detailed implementation questions, see:
- Critical Questions section in REVISED_PLAN.md (lines 545-610)
- Open Questions section in REVISED_PLAN.md (lines 694-759)
- Expert recommendations in REVISED_PLAN_EXPERT_SYNTHESIS.md

For current status, see PROGRESS_UPDATE.md
