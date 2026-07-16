# avenger-lang-compiler

Chart-aware asynchronous compiler layer for the Avenger chart language. It
owns registry-driven lowering, DataFusion-backed analysis, catalog and compile
environment seams, stable dataset/stage schema and lineage indexes, and
compiled chart/project artifacts. It depends inward on `avenger-lang-core`;
the public `avenger-lang` facade depends on both.

## Phase 3 landed API map

- `Compiler::load_file_project_attempt()` loads one chart plus ambient data and
  its import closure; `load_project_graph_attempt()` deterministically
  discovers all project charts and ambient data files. Both retain dependency
  candidates, versions, and watch anchors on failure.
- `DefaultSourceLoader` enforces the project-root boundary, canonicalizes
  filesystem and redirect origins, rejects symlink escapes, serves versioned
  bundled `std:` definitions, and bounds capability-gated HTTP reads.
- `compile_file_attempt()` and `compile_project_attempt()` use the same Phase 3
  graph, then intentionally stop at the Phase 4 semantic-validation boundary.
- Project fingerprints include verified language-source content, the AST
  schema/native-registry versions, and discovered local resource versions.

The earlier bootstrap API remains in place:

- The prerequisite's final names are `NativeRegistry`,
  `NativeRegistryBuilder`, `NativeRegistryProfileId`, `ResolvedPlot`, and
  `CompiledPlot`.
- The current stock inventory is intentionally the prerequisite bootstrap
  registry. `avenger_lang::stock_registry()` and `register_builtins()` retain
  their API while Phase 6 grows that inventory.
- The plan's dataset-stage concept landed as `DatasetStageId` plus
  `DatasetStageKind` and `DatasetProvenance`.
- Compiled Rust params retain an explicit physical Arrow type derived from
  their `ScalarValue` default. The future DSL still declares its type and
  default separately.
- `compile_phase0_example()` and `analyze_phase0_empty()` are temporary hidden
  harnesses. Phase 5 replaces them with real DSL input without changing the
  artifact and analysis contracts.
