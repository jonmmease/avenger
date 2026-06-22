# Facet Follow-Ups

## Current State

The core faceting system is implemented.

Implemented pieces:

- `FacetRow`, `FacetColumn`, and `FacetWrap` coordinate systems,
- `Subplot<FacetRow>`, `Subplot<FacetColumn>`, and `Subplot<FacetWrap>` child
  marks,
- nested facets and nested child-frame coordination,
- `Sharing` levels for domains, guide visibility, legends, and child-frame
  behavior,
- facet child data filtering,
- `FacetDataScope` with filtered, broadcast, and ancestor-level inherited-data
  policies,
- mark builder helpers: `facet_data_scope(...)`, `facet_data_level(...)`, and
  `broadcast_to_facets()`,
- fixed, automatic, and responsive `FacetWrap` column counts,
- visual tests and mdBook docs for facet row/column/wrap, scale sharing,
  customization, and nested facets.

Async mark compilation, shared scales, row/column/wrap layout, nested facet
coordination, and broadcast foreground/background data are current
implementation facts rather than future work.

## Remaining Valid Goals

### `FacetGrid` Convenience

Two-dimensional row-by-column faceting can already be represented with nested
`FacetRow` and `FacetColumn` plots. A `FacetGrid` API may still be worthwhile
as authoring sugar for the common case.

Prefer expansion to nested row/column facets unless implementation experience
shows that a real coordinate system gives meaningfully better diagnostics or
layout behavior. Expansion should preserve:

- existing scale and guide sharing semantics,
- facet labels and ordering,
- empty-cell policies,
- public target paths,
- event-datum and hit-test behavior.

### Data-Scope Polish

`FacetDataScope` currently applies to inherited facet data before mark
evaluation and aggregate-channel preparation. That covers the motivating
broadcast-background use case.

Open polish questions:

- whether explicit mark-local data should ever opt into facet scoping,
- whether positioned subplot partitioning needs equivalent data-source policy
  controls,
- how much of the scope behavior needs user-facing documentation beyond the
  existing examples.

### FacetWrap Refinements

`FacetWrap` is implemented as a real coordinate system. Future work here is
not the feature itself, but edge-case polish:

- additional responsive wrapping examples,
- clearer debug labels for computed row/column slots,
- empty-cell behavior documentation for unusual ordering or filtering cases.

## Alternate Paradigms

- **Nested facets only**: simplest runtime model; add helpers for grid/wrap
  authoring where syntax is verbose.
- **Dedicated coordinates for every facet shape**: clearer type names, but can
  duplicate child-frame measurement and coordination logic.
- **Authoring-time expansion**: good for `FacetGrid`; less attractive for
  `FacetWrap` now that wrap-specific responsive layout is implemented.

## Readiness

`FacetGrid` convenience is ready for a small implementation plan if there is
user demand. The remaining data-scope and wrap items are documentation and
polish work, not architecture blockers.

## Decisions Needed

- Whether `FacetGrid` exists as a public type or as builder/helper sugar.
- How expanded `FacetGrid` target paths and labels are named.
- Whether explicit mark-local data can participate in facet scoping.
- Whether positioned subplot partitioning should expose a separate
  data-source policy.
