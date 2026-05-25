# Repeat System

## Goal Review

The goal is valid: repeat charts answer a different question from facets.
Facets partition rows by data values. Repeat charts replicate a plot template
over columns, measures, or schema roles.

Nothing in the current runtime directly implements repeat. Some related pieces
exist:

- `HConcat` and `VConcat` can compose explicit child plots.
- Nested facets can create row/column grids from data partitions.
- DataFusion preprocessing can reshape wide data into long data, after which
  ordinary facets can approximate some repeat use cases.
- `ScaleSharing` and child-frame sharing already solve much of the domain,
  guide, and legend ownership problem for nested child plots.

The old repeat note also contained facet scale sharing, axis display, labels,
and data strategy sections. Those belong to current facet/child-frame docs or
to [faceting.md](faceting.md), not to repeat.

## Current System Fit

Repeat should be core-owned layout behavior in `avenger-chart`, not an
external container extension point. The important unresolved question is where
schema substitution happens.

A repeat system needs to:

- accept one or more lists of fields or expressions,
- clone or rebuild a child plot template per repeated slot,
- substitute placeholder channels with the slot's field/expression,
- assign stable child-frame identities,
- feed child plots into existing domain/guide/legend sharing machinery.

The hard part is not layout. It is representing a plot template that can be
compiled multiple times with different channel expressions.

## Recommended Direction

Prefer a template-expansion design over a dedicated low-level runtime at first.
For example:

```rust
RepeatGrid::new()
    .rows(["mpg", "horsepower", "weight"])
    .columns(["mpg", "horsepower", "weight"])
    .subplot(|row_field, col_field| {
        Plot::<Cartesian>::new().mark(
            Symbol::new().x(col(col_field)).y(col(row_field))
        )
    })
```

This avoids inventing placeholder expressions until a more declarative template
API is needed. The builder can expand to `HConcat`/`VConcat` or to a dedicated
repeat coordinate after the ergonomics are proven.

For many cases, a documented long-form transform plus ordinary faceting may be
better than repeat:

```text
wide rows -> fold selected columns -> facet by variable -> plot value
```

That approach should be considered before implementing a full repeat runtime.

## Alternate Paradigms

- **Data reshaping plus facets**: simpler and composable. It works when all
  repeated panels share the same mark structure and value column.
- **Template expansion closure**: idiomatic Rust, type-safe, and compatible
  with current `Plot` builders. It is less serializable.
- **Declarative placeholders**: better for specs and serialization, but
  requires placeholder expression design and clearer error reporting.
- **Dedicated repeat coordinate systems**: can reuse child-frame machinery
  directly, but may duplicate concat/facet layout behavior.

## Readiness

Ready for a design spike.

The spike should compare two small prototypes:

- a closure-based `RepeatGrid` that expands to nested concat,
- a `fold` transform plus `FacetRow`/`FacetColumn` example.

After that comparison, the project can decide whether repeat is a first-class
layout feature or documentation/API sugar around transforms and facets.

## Decisions Needed

- Whether repeat is a distinct runtime container or authoring-time expansion.
- Whether the API prioritizes Rust closures or serializable placeholders.
- How per-variable scale sharing is represented with existing `ScaleSharing`
  levels.
- How diagonal, upper/lower triangle, and asymmetric scatterplot matrices are
  expressed.
- Whether repeated slots can have different coordinate systems or only
  different channel expressions.
