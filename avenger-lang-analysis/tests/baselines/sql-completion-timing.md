# SQL completion timing baseline

Recorded on 2026-07-22 with the release profile on the local macOS development
machine. The ignored `record_sql_completion_timing_baseline` test measures one
cold semantic request and 500 exact-fingerprint warm requests over the
FROM-first catalog fixture. This is an observational baseline, not a CI
threshold.

- Cold: 999 µs
- Warm p50: 798 µs
- Warm p95: 922 µs

The corresponding suite asserts that physical plans and executions remain
zero. Repaired source is never inserted into the authored-analysis cache.
