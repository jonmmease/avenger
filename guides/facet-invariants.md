# Facet System Invariants

This document catalogs the critical invariants in the facet system that must be maintained for correct behavior. These invariants are not currently enforced by the type system and must be maintained through careful coding practices and testing.

## Core Invariants

### 1. Phase 2 Scales Must Be Rebuilt with Final Band Size

| Property | Value |
|----------|-------|
| **Invariant** | Scales must be rebuilt in Phase 2 (render pass) using the final band size, not the measurement-phase band size |
| **Consequence if Violated** | Data misalignment - marks may be positioned incorrectly relative to axes |
| **Current Enforcement** | Comment only in `facet_evaluation.rs` |
| **Location** | `avenger-chart/src/facet/marks/facet_evaluation.rs` |

### 2. Use `scale_input_expr()` for Domain Collection

| Property | Value |
|----------|-------|
| **Invariant** | When collecting domain values for scales, always use `scale_input_expr()` not `expr_for_domain()` |
| **Consequence if Violated** | Domain corruption - literal values incorrectly included in scale domain, causing wrong scale ranges |
| **Current Enforcement** | None - relies on developer knowledge |
| **Location** | `avenger-chart/src/channel/value.rs:309` |

**Details**: 
- `scale_input_expr()` returns NULL for literal branches in conditionals, correctly excluding them from domain computation
- `expr_for_domain()` is for type inference only, not value collection
- Common mistake: Using `expr_for_domain()` when building scale domains

### 3. Scale Storage Uses Base Channel Names

| Property | Value |
|----------|-------|
| **Invariant** | Scales must be stored and retrieved using base channel names (e.g., "y" not "y2") |
| **Consequence if Violated** | Runtime error - scale lookup fails, panic or incorrect rendering |
| **Current Enforcement** | None - convention only |
| **Location** | Scale lookup in `avenger-chart/src/plot/compiled/scales.rs` |

**Details**:
- Channels like `y` and `y2` share the same scale
- The base name is derived by stripping trailing digits: y2 → y, y10 → y
- Phase 3 proposes a `BaseChannelName` newtype to enforce this

### 4. Visibility Logic Must Match in Measure and Render

| Property | Value |
|----------|-------|
| **Invariant** | The visibility decision for guides (axes, labels, titles) must be identical in measure and render passes |
| **Consequence if Violated** | Missing or extra guides - space allocated but nothing rendered, or rendered outside allocated space |
| **Current Enforcement** | Shared function (`should_render_guide()`) |
| **Location** | `avenger-chart/src/facet/guide.rs` |

### 5. FacetContext Position Within Grid Dimensions

| Property | Value |
|----------|-------|
| **Invariant** | `FacetContext.position` must always be less than `FacetContext.grid_dimensions` |
| **Consequence if Violated** | Index out of bounds panic during overflow coordination or rendering |
| **Current Enforcement** | `SubplotIterator` guarantees this for properly constructed iterators |
| **Location** | `avenger-chart/src/facet/subplot_iterator.rs` |

### 6. Execution Order: Pass 2 Assumes Pass 1 Complete

| Property | Value |
|----------|-------|
| **Invariant** | Pass 2 (render) must only execute after Pass 1 (measure) completes for all sibling subplots |
| **Consequence if Violated** | Incorrect aggregate gap calculation - spacing between cells may be wrong |
| **Current Enforcement** | Implicit through async task ordering |
| **Location** | `avenger-chart/src/facet/marks/facet_evaluation.rs` |

### 7. Data Consistency: Handle Empty DataFrames

| Property | Value |
|----------|-------|
| **Invariant** | `measure_overflow()` and related functions must handle empty DataFrames gracefully |
| **Consequence if Violated** | Panic or incorrect layout for empty cells |
| **Current Enforcement** | None - defensive coding only |
| **Location** | Throughout facet measurement code |

**Details**:
- Empty cells can occur with domain propagation in nested facets
- Fallback scales (`enable_empty_cell_fallback`) must be used for empty cells
- Empty DataFrames should produce zero-size marks but valid axes

### 8. Concurrency Limits: Semaphore Bounds

| Property | Value |
|----------|-------|
| **Invariant** | Concurrent subplot processing must respect semaphore limits |
| **Consequence if Violated** | Resource exhaustion - memory/GPU pressure from too many parallel operations |
| **Current Enforcement** | Runtime semaphore in async processing |
| **Location** | `avenger-chart/src/facet/marks/facet_evaluation.rs` |

### 9. Phase Ordering: Measure → Coordinate → Render

| Property | Value |
|----------|-------|
| **Invariant** | The three phases must execute in strict order: measure, coordinate, render |
| **Consequence if Violated** | Undefined behavior - incorrect sizes, positions, or missing data |
| **Current Enforcement** | Implicit through function call structure |
| **Location** | Throughout facet code |

**Details**:
- **Measure (Pass 1)**: Compute overflow requirements, collect domains
- **Coordinate (Phase 1.5)**: Aggregate measurements across subplots, compute shared spacing
- **Render (Pass 2)**: Build final scene graph with coordinated measurements

## Additional Type-Safety Invariants

These are candidates for enforcement via newtypes or stricter types in Phase 3:

### A. Expression Type Separation

| Property | Value |
|----------|-------|
| **Invariant** | Domain type inference expressions and scale domain collection expressions are distinct |
| **Proposal** | `DomainTypeExpr` vs `ScaleDomainExpr` newtypes |
| **Status** | Proposed for Phase 3 |

### B. Position Path Length Matches Nesting Depth

| Property | Value |
|----------|-------|
| **Invariant** | `FacetCoordinationContext.position_path.len()` must equal `nesting_depth + 1` |
| **Consequence if Violated** | Incorrect edge detection for guide visibility |
| **Current Enforcement** | None |
| **Location** | `avenger-chart/src/facet/coordination.rs` |

### C. Inner Channel Consistency

| Property | Value |
|----------|-------|
| **Invariant** | `FacetCoordinationContext.inner_channel` must be "row" or "col" when present |
| **Consequence if Violated** | Logic errors in dimension handling |
| **Current Enforcement** | None |
| **Location** | `avenger-chart/src/facet/coordination.rs` |

## See Also

- `guides/measurement-coordination-detailed.md` - Comprehensive analysis of the two-pass measurement algorithm
- `guides/facet-layout-overflow-model.md` - Overflow handling architecture
- `guides/nested-facet-implementation.md` - Nested facet implementation details
