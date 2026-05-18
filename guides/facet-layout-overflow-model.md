# Facet Layout and Overflow Model

This document describes the *conceptual* overflow model used by the facet system: how overflow regions at the facet level relate to overflow regions at the subplot level, and how those relationships drive aggregation rules at facet edges.

For the implementation, see `avenger-chart/docs/architecture/facet-system.md` and the `overflow_projection` module. For user-facing facet usage, see `avenger-chart/book/src/docs/coordinate-systems/faceting/`.

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

## Overflow Aggregation Rules

When a facet asks "how much overflow do I have on side X?", it aggregates across its child subplots using rules that depend on whether side X is a *main-axis* edge (facets cells are arranged along it) or a *cross-axis* edge (perpendicular to the cell arrangement):

**For FacetRow** (cells stacked vertically — main axis is vertical):
- **Left**: max across all subplots (every row needs the same left margin)
- **Right**: max across all subplots (every row needs the same right margin)
- **Top**: first subplot's top (only the top row extends above the facet)
- **Bottom**: last subplot's bottom (only the bottom row extends below the facet)

**For FacetCol** (cells arranged horizontally — main axis is horizontal):
- **Left**: first subplot's left (only the leftmost column extends left of the facet)
- **Right**: last subplot's right (only the rightmost column extends right of the facet)
- **Top**: max across all subplots
- **Bottom**: max across all subplots

The edge-specific selection (first/last) on the main axis is what allows facet-level content (labels, titles) to stack *beyond* the subplot overflow rather than overlapping it: only edge subplots' overflow extends to the facet boundary, while interior subplots' overflow becomes internal spacing.

Implementation: `overflow_projection::aggregate_facet_band_overflow_with_policy`.
