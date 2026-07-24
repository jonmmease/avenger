# avenger-lang-compiler

Chart-aware asynchronous compiler layer for the Avenger chart language. It
owns registry-driven lowering, DataFusion-backed analysis, catalog and compile
environment seams, stable dataset/stage schema and lineage indexes, and
compiled chart/project artifacts. It depends inward on `avenger-lang-core`;
the public `avenger-lang` facade depends on both.

## Current API map

- `Compiler::load_module_graph_attempt()` loads an ordinary source module and
  its explicit import closure. `Compiler::compile_chart_attempt()` selects one
  named or singleton chart entrypoint from that graph, while
  `Compiler::compile_module_attempt()` compiles every chart entrypoint in the
  requested module. All retain dependency candidates, versions, and watch
  anchors on failure.
- `DefaultSourceLoader` enforces the project-root boundary, canonicalizes
  filesystem and redirect origins, rejects symlink escapes, serves versioned
  bundled `std:` definitions, and bounds capability-gated HTTP reads.
- Module-graph compilation uses typed semantic resolution, canonical
  definition expansion, and asynchronous schema-paired native-registry
  lowering for every selected chart entrypoint.
- Project fingerprints include verified language-source content, the AST
  schema/native-registry versions, and discovered local resource versions.
- `expand_module()` returns valid canonical ordinary DSL plus an expansion
  source map. Imported custom mark, tool, and transform definitions lower
  through the same group, behavior, and pipeline paths as handwritten
  ordinary DSL.
- Definition expansion provides typed slots and defaults, channels, closed
  matches, caller block splicing, private alpha-renaming, exact exports,
  migration metadata, and macro-style diagnostic traces. Built-in widgets are
  retained as native declarations; widget definitions are intentionally not a
  language feature.
- Transform pipelines expose surviving input columns plus declared outputs and
  hide newly generated undeclared intermediate columns at the pipeline
  boundary.

The earlier bootstrap API remains in place:

- The prerequisite's final names are `NativeRegistry`,
  `NativeRegistryBuilder`, `NativeRegistryProfileId`, `ResolvedPlot`, and
  `CompiledPlot`.
- `avenger_lang::stock_registry()` and `register_builtins()` expose the complete
  Phase 6 native language inventory while preserving the host-composed registry
  extension seam.
- The plan's dataset-stage concept landed as `DatasetStageId` plus
  `DatasetStageKind` and `DatasetProvenance`.
- Compiled Rust params retain an explicit physical Arrow type derived from
  their `ScalarValue` default. The DSL declares its type and default separately
  and validates both before lowering.
- `compile_phase0_example()` and `analyze_phase0_empty()` remain hidden test
  harnesses. The public module-graph APIs produce the real artifact and
  analysis contracts from DSL input.
