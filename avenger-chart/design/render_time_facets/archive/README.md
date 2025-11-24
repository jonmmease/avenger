# Archive: Original Plan Documents

This directory contains the original planning documents that have been superseded by the revised plan.

## Files

### PLAN.md
**Date**: 2025-11-23
**Status**: Superseded by REVISED_PLAN.md

**Original approach**: Store `uncompiled_subplot: Arc<Plot<InnerC>>` and recompile at render time.

**Why superseded**: Expert review (GPT-5 Codex and Gemini 3 Pro) identified a critical serialization blocker:
- `CompiledFacetRow` must be serializable (uses `#[typetag::serde]`)
- `Plot<InnerC>` is NOT serializable (contains trait objects, async methods, closures)
- Storing uncompiled Plot in compiled struct would break serialization

### EXPERT_REVIEW_SYNTHESIS.md
**Date**: 2025-11-23
**Status**: Superseded by REVISED_PLAN_EXPERT_SYNTHESIS.md

**Summary of expert findings**:
1. ✅ Architectural approach sound
2. ❌ Critical serialization blocker identified
3. 💡 Three options proposed:
   - Option 1: Drop serialization requirement
   - Option 2: Implement PlotSpec system
   - Option 3: Use render-time data overrides (simpler)

**User decision**: Pursue enhanced Option 3 (recursive data overrides), which became REVISED_PLAN.md

## Current Active Documents

The current active planning documents are in the parent directory:

1. **REVISED_PLAN.md** - Complete implementation plan using recursive data overrides
2. **REVISED_PLAN_EXPERT_SYNTHESIS.md** - Expert review of revised plan with critical DataFusion API fix
3. **PROGRESS_UPDATE.md** - Current progress summary

## Evolution of the Plan

1. **Initial Plan** (PLAN.md): Store uncompiled Plot, recompile at render time
   - Timeline: 5-8 weeks
   - Risk: Serialization blocker

2. **Expert Review** (EXPERT_REVIEW_SYNTHESIS.md): Identified blocker, proposed alternatives
   - Critical finding: Can't store uncompiled Plot in serializable struct
   - Recommendation: Consider Option 3 (data overrides)

3. **Revised Plan** (REVISED_PLAN.md): Recursive data overrides approach
   - User insight: Use existing data override mechanism recursively
   - Timeline: 2-3 weeks
   - Solves: Both render params AND nested faceting
   - No serialization issues

4. **Revised Expert Review** (REVISED_PLAN_EXPERT_SYNTHESIS.md): Validated approach, fixed DataFusion issue
   - ✅ Avoids serialization blocker
   - ❌ Found DataFusion API misuse (register_batch memory leaks)
   - ✅ Recommended using read_batch() instead

5. **Current Status** (PROGRESS_UPDATE.md): DataFusion helper implemented
   - Phase 0 complete
   - Ready for Phase 1 structural changes

## Key Lesson Learned

The progression from original plan → expert review → revised plan → implementation demonstrates the value of:
1. Expert review before implementation
2. Prototyping critical components early
3. Iterative refinement based on feedback
4. Following official API patterns over custom solutions
