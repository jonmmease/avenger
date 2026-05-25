# Derive API

## Goal Review

The goal is valid: authors should be able to derive labels, error bars,
connectors, hulls, annotations, and other helper marks from an already-defined
mark. The current library has strong mark contracts, but no public way for one
mark to generate another after its data has been scaled or summarized.

One possible design is a `Derive<C>` trait that receives scaled data and
returns `Box<dyn Mark<C>>`. That is not directly compatible with the current
pipeline because authoring marks compile before scale building and runtime
measurement. A derived mark that depends on scaled geometry must run during
evaluation, not during `Plot::compile`.

## Current System Fit

Existing pieces that help:

- `CompiledMarkCore` exposes mark metadata used by scale and legend planning.
- `CompiledMark::render_from_data` is already the runtime mark boundary.
- `PlotComponents` separates data marks, guides, legends, titles, subtitles,
  and debug marks before scenegraph assembly.
- Scenegraph groups can contain additional marks generated during evaluation.

Missing pieces:

- A stable geometry table that derived marks can consume.
- A scheduling rule for whether derived marks participate in scale planning,
  legends, clipping, z-index, and hit testing.
- A high-level `Text` mark, which is the most common derived-mark target.

## Recommended Direction

Treat derived marks as evaluation-stage products. The parent compiled mark
should expose an optional derived-output hook that can append scenegraph marks
or produce a second compiled mark invocation after the parent geometry is
known.

Two tiers are likely useful:

- **Scenegraph derivation** for labels/connectors that only need parent
  geometry and do not need their own scales.
- **Compiled child mark derivation** for error bars or summaries that need
  ordinary channel, scale, legend, and guide participation.

The first tier is smaller and should come first.

## Alternate Paradigms

- **Explicit layered marks**: authors can already add multiple marks manually.
  This is enough when the derived data is easy to compute before plotting.
- **Data transform pipeline**: good for error bars and summaries that happen in
  data space, but not for label placement based on rendered geometry.
- **Scenegraph post-processing**: works for annotation overlays, but bypasses
  chart semantics and is hard to make extensible.

## Readiness

Discovery first.

This depends on the same post-scale geometry boundary as
[adjust-api.md](adjust-api.md), and it also depends on a high-level
[text-mark.md](text-mark.md) if the first target is label generation.

## Decisions Needed

- Whether derived outputs are scenegraph marks, compiled marks, or both.
- Whether derived marks can create new scales or legends, or only consume
  parent geometry.
- How derived output is ordered relative to parent marks, guides, legends, and
  debug overlays.
- How derived marks interact with facets, concat, and positioned child frames.
- Whether the extension contract belongs in core or remains a built-in runtime
  feature until proven.
