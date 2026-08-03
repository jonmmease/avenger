# Phase 0 timing baseline

Recorded 2026-07-21 on the initial LSP scaffold. These are intentionally not
CI thresholds.

| Operation | Cold | Warm | Note |
| --- | ---: | ---: | --- |
| tolerant syntax | 1.054 ms | p50 65.334 µs; p95 76.666 µs | `valid_chart.avenger`, 500 warm samples, release build. |
| immutable module analysis | 1.091 ms | p50 130.666 µs; p95 145.750 µs | One explicit requested module, fresh session/generation, 100 warm samples, release build. |
| SQL completion | 2.189 ms; 21,242 allocations; 3,168,737 bytes | p50 1.433 ms; p95 1.543 ms; p99 1.648 ms; p50/p95 18,878 allocations and 2,906,087 bytes | FROM-first quoted-member completion, one cold plus 500 exact-fingerprint warm requests, 4 results, release build; end-to-end helper allocation counts include tolerant syntax/result construction; recorded 2026-08-03. |

These rows keep scaffold/build time separate from feature latency. Reproduce
the syntax row with `cargo test --release -p
avenger-lang-analysis record_syntax_timing_baseline -- --ignored --nocapture`.
Reproduce the project row by substituting
`record_project_analysis_timing_baseline` in that command. Reproduce the SQL
row with `cargo test --release -p avenger-lang-analysis --test sql_completion
record_sql_completion_timing_baseline -- --ignored --nocapture`.
