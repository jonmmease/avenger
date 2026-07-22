# Phase 0 timing baseline

Recorded 2026-07-21 on the initial LSP scaffold. These are intentionally not
CI thresholds.

| Operation | Cold | Warm | Note |
| --- | ---: | ---: | --- |
| tolerant syntax | 1.054 ms | p50 65.334 µs; p95 76.666 µs | `valid_chart.avenger`, 500 warm samples, release build. |
| immutable project analysis | 1.091 ms | p50 130.666 µs; p95 145.750 µs | One explicit chart root, fresh session/generation, 100 warm samples, release build. |
| completion | n/a | n/a | Providers begin in Phase 5. |

The remaining explicit pre-implementation rows prevent scaffold/build time
from being mistaken for feature latency. Each owning phase replaces its `n/a`
row with p50/p95 measurements using the frozen fixture corpus before closing
its phase gate. Reproduce the syntax row with `cargo test --release -p
avenger-lang-analysis record_syntax_timing_baseline -- --ignored --nocapture`.
Reproduce the project row by substituting
`record_project_analysis_timing_baseline` in that command.
