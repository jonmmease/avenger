# avenger-physical-graph

The DataFusion functions that build avenger charts in physical plans, for hosts that run those plans natively and in WebAssembly pages.

`functions::MarkInstance` is the aggregate of one bound call of a mark function: its template, which holds the layer's encodings and options without a table, and the fields of the columns of its table. Its accumulator collects the rows of its group, sorts them by the keys of the call's `ORDER BY`, and builds the chart of the layer with the rows as its table. `functions::CompositionInstance` is the aggregate of one bound call of a composition, which composes the charts of the rows of its group. A planner binds each call to one of them, so both hold their options and take only data columns and the keys of their `ORDER BY`.

Every chart that the aggregates build names GoogleSQL's syntaxes for its format patterns, `functions::FORMAT_SYNTAXES`.

`cut::CutInputExec` is the leaf through which a node's physical plan reads the result of a node below it. It holds the plan that scans that result and the scan's position, schema, projection, and limit, so a host that plans a node once replaces the scan with a scan of each computation's result and runs the same plan.

## Dependencies

The crate depends on DataFusion's sub-crates and not on the `datafusion` crate, so its dependencies name the parts of DataFusion that a WebAssembly page needs. Its WebAssembly build enables chrono's `wasmbind`, since DataFusion's metrics read the clock when an operator starts and ends. Its tests run a plan of a lifted chart natively and in a browser:

```bash
cargo test --release -p avenger-physical-graph
```

```bash
wasm-pack test --release --headless --chrome avenger-physical-graph
```

On macOS, the browser build compiles C code for wasm32 with Homebrew's LLVM, through `CC_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/clang` and `AR_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/llvm-ar`.
