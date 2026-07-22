# Phase 0 timing baseline

Recorded 2026-07-21 on the initial LSP scaffold. These are intentionally not
CI thresholds.

| Operation | Cold | Warm | Note |
| --- | ---: | ---: | --- |
| tolerant syntax | n/a | n/a | Engine begins in Phase 1. |
| immutable project analysis | n/a | n/a | Snapshot seam begins in Phase 2. |
| completion | n/a | n/a | Providers begin in Phase 5. |

This explicit pre-implementation baseline prevents scaffold/build time from
being mistaken for feature latency. Each owning phase replaces its `n/a` row
with p50/p95 measurements using the frozen fixture corpus before closing its
phase gate.
