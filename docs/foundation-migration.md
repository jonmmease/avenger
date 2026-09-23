# Foundation API migration

## Build profiles and dependency resolution

Use `--release` for development runs and tests. The `release` profile favors iteration, while `release-perf` retains optimized distribution settings. Geometry consumers use the published `rstar` 0.13 release from crates.io.
