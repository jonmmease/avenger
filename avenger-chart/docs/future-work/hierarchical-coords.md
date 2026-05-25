# Hierarchical Visualizations

## Goal Review

The goal is valid: treemaps, sunbursts, icicles, circle packing, and
dendrograms need hierarchy-aware layout. A hierarchical coordinate system with
repeated `.level(...)` calls is one reasonable direction, but current Avenger
has multiple plausible ways to represent these charts.

The current code does not implement treemap or sunburst charts. It does,
however, have strong primitives that matter:

- core coordinate and guide traits,
- custom mark contracts,
- scale and legend contracts,
- child-frame layout for nested plots,
- scenegraph rectangles, arcs, paths, and text,
- DataFusion-based data preparation.

## Current System Fit

Treemap and sunburst are not ordinary point-coordinate charts. Their primary
layout is a hierarchy-to-geometry algorithm:

- treemap: hierarchy -> nested rectangles,
- sunburst/icicle: hierarchy -> radial or linear partitions,
- dendrogram: hierarchy -> node/link geometry.

The key question is whether that algorithm belongs to a coordinate system, a
data transform, or a specialized mark.

## Recommended Direction

Do not commit to "hierarchical coordinate system" as the only model yet.
Prototype treemap two ways:

1. **Transform plus ordinary marks**: a hierarchy transform outputs leaf
   rectangle columns such as `x0`, `x1`, `y0`, `y1`, depth, path, and label.
   Existing or new `Rect`/`Text` marks render the result in Cartesian or
   `ZeroDCoord` space.
2. **Treemap coordinate plus `TreemapRect` mark**: the coordinate measurement
   owns hierarchy layout, guide containers, and label placement; the leaf mark
   consumes hierarchical geometry.

The transform approach is likely simpler for static treemaps. The coordinate
approach may be better if parent containers are guide-like, if drill-down
interaction is central, or if multiple hierarchy marks share one layout.

Sunburst should probably wait until treemap proves the hierarchy data model,
because it adds arc-specific mark and label complexity.

## Alternate Paradigms

- **Hierarchy transform**: best for composability with existing marks and
  scales. It may duplicate guide/container rendering unless a helper exists.
- **Coordinate system**: best if hierarchy containers are the coordinate space
  and guides. It requires custom measurement and geometry contracts.
- **Single specialized mark**: fastest for a first visual, but hardest to
  combine with legends, labels, and interactions.
- **Child-frame hierarchy**: useful for nested charts inside hierarchy cells,
  but overkill for ordinary treemap leaves.

## Readiness

Ready for a design spike.

The spike should implement a static treemap with one hierarchy input format
and one layout algorithm. It should deliberately choose whether the prototype
is transform-first or coordinate-first, then record what became awkward.

## Decisions Needed

- Which hierarchy input model is v1: repeated level expressions, path strings,
  parent/child edges, or nested records.
- Whether layout algorithms live in chart crates or a lower-level reusable
  crate.
- Whether parent containers are guide marks, ordinary marks, or scenegraph
  groups.
- How labels depend on the future [text-mark.md](text-mark.md) and adjustment
  work.
- How color/size legends relate to internal hierarchy depth and leaf data.
- Whether drill-down interaction is part of v1 or later controller work.
