# Scale Sharing Internals

This document explains the internal workings of scale sharing in the avenger-chart facet system. It covers the `ScaleSharing` enum, domain propagation mechanisms, grouping logic, fallback scale handling, and hierarchical sharing via `Level(n)`.

## Overview

Scale sharing enables coordinated visualization across faceted layouts. When multiple subplots display related data, they often need to share axis scales so that:

1. **Visual comparison is accurate** - The same data value appears at the same position across all subplots
2. **Labels are not redundant** - Shared scales only need labels on edge subplots
3. **Empty cells render correctly** - Subplots with no data still show properly scaled axes

The scale sharing system operates at three levels:

- **Channel Level**: Per-channel configuration (x, y) determining how scales behave
- **Facet Level**: Domain propagation for the faceting dimension itself (row, column)
- **Coordination Level**: Cross-facet communication for nested layouts

## ScaleSharing Enum

**Location**: `avenger-chart/src/channel/config_traits.rs:152`

```rust
pub enum ScaleSharing {
    /// Share scales across all facets (one domain for all subplots)
    /// Equivalent to Level(u8::MAX)
    Shared,

    /// Independent scales per facet (each subplot has its own domain)
    /// Equivalent to Level(0)
    Free,

    /// Hierarchical level-based scale sharing for nested facets
    /// Level(0) = Free (independent per cell)
    /// Level(1) = Share with immediate parent facet
    /// Level(N) = Share N levels up in the hierarchy
    /// Level(u8::MAX) = Shared (global across all facets)
    Level(u8),
}
```

### Variant Behaviors

| Variant | Behavior | Use Case |
|---------|----------|----------|
| `Shared` | One scale domain computed from full dataset | Compare values across all panels |
| `Free` | Each subplot computes its own domain | Zoom into per-panel patterns |
| `Level(0)` | Equivalent to `Free` | Explicit free sharing |
| `Level(1)` | Share with immediate parent facet | Per-row or per-column sharing |
| `Level(N)` | Share N levels up in hierarchy | Advanced nested layouts |
| `Level(u8::MAX)` | Equivalent to `Shared` | Explicit global sharing |

### Helper Methods

The `ScaleSharing` enum provides several helper methods for working with level-based sharing:

```rust
impl ScaleSharing {
    /// Convert to numeric level (Free=0, Shared=255)
    pub fn to_level(self) -> u8;

    /// Create from numeric level
    pub fn from_level(level: u8) -> Self;

    /// Returns true for Level(1+) and Shared
    pub fn should_share_with_parent(self) -> bool;

    /// Returns true for Shared and Level(u8::MAX)
    pub fn is_fully_shared(self) -> bool;

    /// Returns true for Free and Level(0)
    pub fn is_free(self) -> bool;
}
```

## Domain Propagation

Domain propagation ensures that subplots with shared scales use consistent domains. This happens in two contexts:

### 1. Data Channel Domains (x, y)

When a data channel (x or y) uses `ScaleSharing::Shared`, the system:

1. **Builds a shared ScaleBuilder** from the full dataset before facet filtering
2. **Computes data extents** (min/max for numeric, unique values for categorical)
3. **Extends per-cell scale builders** with these shared extents

**Key Code Flow** (`facet_evaluation.rs:232-305`):

```
measure_pass()
  |
  +-> Check any_shared = scale_sharing_by_channel has Shared
  |
  +-> If any_shared:
  |     +-> Build shared_scale_builder from full df
  |     +-> Extend with shared_data_extents (from coordination)
  |     +-> Build initial_shared_scales with approximate dimensions
  |
  +-> For each subplot:
        +-> Use initial_shared_scales for Shared channels
        +-> Build free_scale_builder for Free channels (from filtered data)
```

### 2. Facet Dimension Domains (row, column)

When a nested facet has its dimension shared (e.g., inner row facet with Shared rows), the outer facet:

1. **Extracts the full domain** from the complete dataset
2. **Passes it via FacetCoordinationContext.inner_domain**
3. **Inner facet receives** and uses this domain instead of computing from filtered data

**Domain Propagation Diagram**:

```
Outer FacetColumn
    |
    +-> Computes inner_domain from full dataset
    |   (e.g., all species values: ["setosa", "versicolor", "virginica"])
    |
    +-> Creates FacetCoordinationContext with:
    |   - inner_domain: ["setosa", "versicolor", "virginica"]
    |   - inner_scale_sharing: Shared
    |   - enable_empty_cell_fallback: true
    |
    +-> Serializes to params["__facet_coordination"]
    |
    v
Inner FacetRow (for column "A")
    |
    +-> Reads coordination domain via get_inner_domain_for_channel()
    |
    +-> Uses full domain even if column "A" only has ["setosa"]
    |
    +-> Creates empty cells for ["versicolor", "virginica"]
    |
    v
Result: All columns have same row structure
```

**Coordination Domain Extraction** (`facet_evaluation.rs:147-156`):

```rust
let coordination_context = FacetCoordinationContext::from_params(&context.params);
let domain_from_coordination = coordination_context.as_ref().and_then(|ctx| {
    // Only use coordination domain if the channel matches
    ctx.get_inner_domain_for_channel(current_channel)
});
```

## Scale Builder Selection

Scale builders are selected per-subplot based on the `ScaleSharing` mode:

| Mode | Behavior |
|------|----------|
| `Shared` | Use scale builder from full dataset |
| `Free` | Use scale builder from filtered cell data |
| `Level(0)` | Same as Free |
| `Level(n)` where n >= 1 | Use domain from parent at level n |

The actual scale building logic is in `facet_evaluation.rs` within `measure_pass()` and `render_pass()`.

## Fallback Scale Handling

Empty cells (subplots with no data after filtering) need special handling to render axes correctly.

### Problem

When domain propagation creates cells for values not present in filtered data, those cells have empty data. Without intervention, they would have no scales and axes wouldn't render.

### Solution: Fallback Scale Builder

**Location**: `avenger-chart/src/facet/scale_helpers.rs:69-132`

The system maintains a **fallback ScaleBuilder** built from the full dataset:

```rust
// Built during measure_pass when enable_empty_cell_fallback is true
let fallback_builder = if enable_empty_cell_fallback {
    Some(compiled_subplot
        .build_scale_builder_from_dataframe(&ctx, &params, df)
        .await?)
} else {
    None
};
```

### Fallback Usage in build_scales_helper_with_fallback

```
build_scales_helper_with_fallback()
    |
    +-> First, try normal scale building via build_scales_helper()
    |
    +-> If scales.is_empty():
    |     +-> Use fallback_builder to build complete scales
    |
    +-> Else if fallback_builder exists:
          +-> Backfill missing required scales (x, y, x2, y2)
          +-> Only insert if not already present
```

### When Fallback is Enabled

The `enable_empty_cell_fallback` flag is set in `FacetCoordinationContext` when:

1. The outer facet propagates a domain to inner facets
2. Scale sharing mode is `Shared` (creating grid-like layouts)
3. There's potential for data combinations that don't exist

## Level(n) Implementation

Hierarchical scale sharing via `Level(n)` enables sophisticated nested facet layouts.

### Conceptual Model

```
Level 0: Current facet (Free - independent)
Level 1: Immediate parent facet
Level 2: Grandparent facet
...
Level N: N levels up in hierarchy
Level u8::MAX: Root level (fully Shared)
```

### Implementation Details

#### 1. Level Information Tracking

**FacetCoordinationContext Fields** (`coordination.rs:599-651`):

```rust
pub struct FacetCoordinationContext {
    /// Current nesting depth (0 = outermost)
    pub nesting_depth: usize,

    /// Per-channel sharing levels
    pub channel_sharing_levels: HashMap<String, u8>,

    /// Level-based domain lookups: (level, channel) -> extents
    pub level_domains: IndexMap<LevelChannelKey, SerializableDataExtents>,

    /// Position path through hierarchy: [outer_pos, ..., inner_pos]
    pub position_path: Vec<usize>,

    /// Counts at each level: [outer_count, ..., inner_count]
    pub level_counts: Vec<usize>,
}
```

#### 2. Domain Lookup by Level

**Key Lookup** (`coordination.rs:72-103`):

```rust
pub struct LevelChannelKey {
    pub level: usize,      // Hierarchy level
    pub channel: String,   // Channel name (e.g., "x", "y")
}

// Usage:
let key = LevelChannelKey::new(1, "y");
if let Some(domain) = coord_ctx.level_domains.get(&key) {
    // Use domain from level 1 (immediate parent)
}
```

#### 3. Level-Based Extent Extraction

**During measure_pass** (`facet_evaluation.rs:199-230`):

```rust
let level_based_extents = if let Some(ref ctx) = coordination_context {
    scale_sharing_by_channel
        .iter()
        .filter_map(|(channel, mode)| {
            if let ScaleSharing::Level(n) = mode {
                if *n >= 1 {
                    ctx.get_domain_for_channel(channel)
                        .map(|extents| (channel.clone(), extents.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
} else {
    HashMap::new()
};
```

### Guide Ownership with Level(n)

**GuideOwnership Computation** (`coordination.rs:143-193`):

| Sharing Mode | Guide Behavior |
|--------------|----------------|
| `Level(0)` | Full ownership (all guides shown) |
| `Level(1+)` | Edge only (suppress interior) |

```rust
impl GuideOwnership {
    pub fn compute(position: usize, count: usize, scale_sharing: ScaleSharing, is_row_facet: bool) -> Self {
        match scale_sharing {
            ScaleSharing::Free => GuideOwnership::Full,
            ScaleSharing::Shared => {
                // Edge detection based on facet type
                if position == count - 1 { GuideOwnership::Edge }
                else { GuideOwnership::Suppress }
            }
            ScaleSharing::Level(n) => {
                if n == 0 { GuideOwnership::Full }
                else {
                    // Same as Shared - suppress except at edge
                    if position == count - 1 { GuideOwnership::Edge }
                    else { GuideOwnership::Suppress }
                }
            }
        }
    }
}
```

## Code References

### Key Files

| File | Purpose |
|------|---------|
| `channel/config_traits.rs:152-226` | ScaleSharing enum definition |
| `facet/coordination.rs` | FacetCoordinationContext with level-based fields |
| `facet/scale_grouping.rs` | ScaleGrouping and GroupKey logic |
| `facet/scale_helpers.rs:87-132` | build_scales_helper_with_fallback |
| `facet/marks/facet_evaluation.rs:117-400` | measure_pass domain propagation |
| `facet/context.rs:203-318` | FacetContext label visibility |

### Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `ScaleSharing::to_level()` | `config_traits.rs:184-190` | Convert to numeric level |
| `ScaleSharing::should_share_with_parent()` | `config_traits.rs:209-211` | Check parent sharing |
| `GroupKey::for_cell()` | `scale_grouping.rs:32-67` | Compute grouping key |
| `ScaleGrouping::build()` | `scale_grouping.rs:113-188` | Build grouped builders |
| `build_scales_helper_with_fallback()` | `scale_helpers.rs:87-132` | Scale building with fallback |
| `GuideOwnership::compute()` | `coordination.rs:143-193` | Compute guide ownership |
| `FacetContext::should_show_labels()` | `context.rs:203-239` | Label visibility decision |

## Data Flow Summary

```
User Configuration
    |
    +-> ScaleSharing mode per channel (x: Shared, y: Free, etc.)
    |
    v
measure_pass (Phase 1)
    |
    +-> Extract coordination context
    +-> Check any_shared / any_level_shared
    +-> Build shared_scale_builder if needed
    +-> Extend with shared_data_extents / level_based_extents
    +-> Build fallback_builder for empty cells
    |
    +-> For each subplot:
    |     +-> Filter data by facet value
    |     +-> Build scales via build_scales_helper_with_fallback:
    |     |     +-> Shared channels -> use initial_shared_scales
    |     |     +-> Free channels -> build from filtered data
    |     |     +-> Empty cells -> use fallback_builder
    |     +-> Measure overflow
    |
    v
render_pass (Phase 2)
    |
    +-> Rebuild final shared scales with correct dimensions
    +-> For each subplot:
          +-> Build scales with same logic
          +-> Render marks with coordinated scales
          +-> Show/hide labels based on FacetContext
```

## See Also

- [Facet System Overview](overview.md) - Module structure and core types
- `guides/measurement-coordination-detailed.md` - Full measurement algorithm details
- `guides/measurement-architecture-quickref.md` - Quick reference for modifications
