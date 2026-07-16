# avenger-lang-compiler

Chart-aware asynchronous compiler layer for the Avenger chart language. It
owns registry-driven lowering, DataFusion-backed analysis, catalog and compile
environment seams, stable dataset/stage schema and lineage indexes, and
compiled chart/project artifacts. It depends inward on `avenger-lang-core`;
the public `avenger-lang` facade depends on both.

## Phase 0 landed API map

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
