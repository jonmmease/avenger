# Hierarchical Visualizations

## Goal Review

The goal is valid: treemaps, sunbursts, icicles, circle packing, and
dendrograms need hierarchy-aware layout. A hierarchical coordinate system with
repeated `.level(...)` calls is one reasonable direction, but Avenger has
multiple plausible ways to represent these charts.

The first concrete implementation is `avenger-chart-treemap`: an external
coordinate crate with a `Treemap` coordinate, `TreeRect` mark, treemap guide
headers, breadcrumbs, and hierarchy event datum fields. Sunburst, icicle,
circle-packing, and shared hierarchy abstractions remain future work. The
treemap implementation proves several primitives that matter:

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

Use the treemap coordinate crate as the reference implementation before
extracting shared hierarchy abstractions. It has validated the coordinate-owned
measurement path for cases where parent containers are guide-like, drill-down
interaction is central, and multiple hierarchy marks share one layout.

Sunburst should build on the same event/view-window shape where possible, but
it should remain a separate crate until there is clear evidence for which
hierarchy helpers deserve extraction.

## Alternate Paradigms

- **Hierarchy transform**: best for composability with existing marks and
  scales. It may duplicate guide/container rendering unless a helper exists.
- **Coordinate system**: best if hierarchy containers are the coordinate space
  and guides. This is the path taken by `avenger-chart-treemap`.
- **Single specialized mark**: fastest for a first visual, but hardest to
  combine with legends, labels, and interactions.
- **Child-frame hierarchy**: useful for nested charts inside hierarchy cells,
  but overkill for ordinary treemap leaves.

## Readiness

Ready for a follow-up implementation plan.

The next spike should use the treemap crate as dogfood and implement a second
hierarchical coordinate, most likely sunburst or icicle. That second consumer
should decide whether to extract hierarchy path/view-window/event helpers into
a lower-level crate.

## Decisions Needed

- Which hierarchy input models beyond treemap's repeated path columns are worth
  supporting: path strings, parent/child edges, nested records, or adapters.
- Whether layout algorithms live in chart crates or a lower-level reusable
  crate.
- Whether parent containers are guide marks, ordinary marks, or scenegraph
  groups.
- How labels depend on the future [text-mark.md](text-mark.md) and adjustment
  work.
- How color/size legends relate to internal hierarchy depth and leaf data.
- Whether drill-down interaction should become a reusable hierarchy tool after
  treemap's manual app examples have proven the shape.
