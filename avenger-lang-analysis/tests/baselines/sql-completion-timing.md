# SQL completion timing baseline

Recorded on 2026-08-03 with the release profile on the local macOS development
machine. The ignored `record_sql_completion_timing_baseline` test measures one
cold semantic request and 500 exact-fingerprint warm requests over the
FROM-first catalog fixture. This is an observational baseline and ratchet, not
a wall-clock CI assertion.

- Cold: 2.189 ms; 21,242 allocations; 3,168,737 allocated bytes
- Warm p50: 1.433 ms; 18,878 allocations; 2,906,087 allocated bytes
- Warm p95: 1.543 ms; 18,878 allocations; 2,906,087 allocated bytes
- Warm p99: 1.648 ms
- Result count: 4
- Cache behavior: 1 miss followed by 500 hits

The measured warm p95 is below the 25 ms semantic-completion target, and the
cold request is below the 100 ms target. The corresponding suite asserts that
physical plans, scans, collections, and executions remain zero. Repaired
source is never inserted into the authored-analysis cache. Separate unit tests
enforce the cache's entry and byte limits, LRU behavior, generation-sensitive
identity, concurrent access, and cancellation behavior.

Allocation counts cover the complete editor-neutral request helper, including
tolerant syntax analysis and result construction, rather than only the cached
SQL-analysis lookup. They are recorded as a ratchet for later profiling; they
are not a release assertion while latency remains comfortably within budget.
