# Facet Layout and Overflow Model

## Core Concept: Overflow Regions Overlap and Stack

The facet layout system uses a model where overflow regions **overlap** with subplot overflow regions, and facet-level content **stacks beyond** the subplot overflow.

## Single-Level Facet Layout

For a FacetColumn with 3 columns:

```
                         FACET TITLE ("Species")
                    ┌─────────────────────────────────┐
                    │ "setosa" "versicolor" "virginica"│  ← Facet labels
┌───────────────────┼─────────────────────────────────┼───────────────────┐
│                   │       FACET PLOT-AREA           │                   │
│ FACET             │ ┌─────────┬─────────┬─────────┐ │ FACET             │
│ of-left           │ │subplot1 │subplot2 │subplot3 │ │ of-right          │
│                   │ │plot-area│plot-area│plot-area│ │                   │
│ (overlaps         │ └─────────┴─────────┴─────────┘ │ (overlaps         │
│  subplot1         │                                 │  subplot3         │
│  of-left)         │                                 │  of-right)        │
└───────────────────┴─────────────────────────────────┴───────────────────┘
```

### Key Principles

1. **Facet plot-area** = bounding box of all subplot plot-areas (excludes subplot overflows)

2. **Edge overflow overlapping**:
   - Facet's of-left overlaps with leftmost subplot's of-left
   - Facet's of-right overlaps with rightmost subplot's of-right
   - Facet's of-top overlaps with top row subplots' of-top
   - Facet's of-bottom overlaps with bottom row subplots' of-bottom

3. **Facet guide content stacks beyond subplot overflow**:
   ```
   facet.of_right = subplot.of_right + facet_labels + facet_title
   ```
   The facet's overflow region is **larger than** the subplot's because facet-level content (labels, titles) stacks beyond.

## FacetRow Overflow Structure

For FacetRow (vertical stack of subplots, labels on side):

```
┌────────────────────┬─────────────┬──────────┬────────────┐
│ subplot1 plot-area │ subplot1    │ "narrow" │            │
│                    │ of-right    │ (label)  │ "Petal     │
├────────────────────┼─────────────┼──────────┤  Width"    │
│ subplot2 plot-area │ subplot2    │ "wide"   │ (title)    │
│                    │ of-right    │ (label)  │            │
└────────────────────┴─────────────┴──────────┴────────────┘
←── FACET PLOT-AREA ─→←────────── FACET of-right ─────────→
                      │
                      ├─ overlaps subplot of-right (y-axis ticks)
                      ├─ facet labels stack beyond
                      └─ facet title stacks beyond
```

## FacetColumn Overflow Structure

For FacetColumn (horizontal row of subplots, labels on top/bottom):

```
                    FACET of-top
              ┌─────────────────────────┐
              │ "Species" (title)       │  ← stacks beyond
              │ "setosa" "versi" "virg" │  ← facet labels stack beyond
              │ subplot of-top (x-ticks)│  ← overlaps subplot of-top
              └─────────────────────────┘
              ┌─────────────────────────┐
              │     FACET PLOT-AREA     │
              │ ┌───────┬───────┬─────┐ │
              │ │ sub1  │ sub2  │sub3 │ │
              │ └───────┴───────┴─────┘ │
              └─────────────────────────┘
```

## Nested Facets (FacetRow inside FacetColumn)

When a subplot IS itself a facet (e.g., FacetRow inside FacetColumn):

1. **Inner FacetRow** computes its total overflow:
   - Measures its Cartesian subplots' overflow (y-axis ticks)
   - Adds its facet labels ("narrow", "medium", "wide")
   - Adds its facet title ("Petal Width")
   - Result: Inner FacetRow's of-right = 50px (example)

2. **Outer FacetColumn** uses inner facet's total overflow:
   - Rightmost column's subplot IS the FacetRow
   - Outer FacetColumn's of-right starts from FacetRow's of-right (50px)
   - Outer adds its own content (if any) on top

3. **The stacking chain**:
   ```
   Cartesian subplot of-right (15px for y-ticks)
   + FacetRow labels (20px)
   + FacetRow title (15px)
   = FacetRow total of-right (50px)
   
   FacetColumn uses this as its rightmost subplot's of-right
   + FacetColumn's own right content (0px typically)
   = FacetColumn total of-right (50px)
   ```

## Current Bug: Hardcoded Fallback

When measuring a nested facet subplot's overflow, the code returns hardcoded values:
```rust
return Ok((30.0, 30.0, 40.0, 20.0));  // (top, bottom, left, right)
```

This breaks the stacking model because:
- The inner FacetRow's actual of-right (50px+) is replaced with 20px
- The outer FacetColumn's of-right becomes 20px
- The "Petal Width" title (which needs ~50px) gets clipped

## Fix Required

The fix must ensure that when measuring a nested facet subplot, we return its **actual computed total overflow** (including its facet labels and title), not hardcoded estimates. This actual value becomes the base that the outer facet's overflow calculation stacks upon.

## Overflow Aggregation Rules

For FacetRowGuide:
- **Left**: first subplot's left (leftmost edge)
- **Right**: last subplot's right (rightmost edge)
- **Top/Bottom**: max across all subplots (all rows need same vertical space)

For FacetColGuide:
- **Left/Right**: max across all subplots (all columns need same horizontal space)
- **Top**: first subplot's top (topmost edge)
- **Bottom**: last subplot's bottom (bottommost edge)

The edge-specific selection (first/last) makes sense because only edge subplots' overflow extends to the facet boundary. Interior subplots' overflow is internal spacing.
