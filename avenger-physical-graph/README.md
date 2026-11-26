# avenger-physical-graph

The physical side of avenger's query graphs, which runs natively and in WebAssembly pages: the DataFusion functions that build avenger charts in physical plans, the leaves and kept results through which a node's plan reads the nodes below it, the codec of a graph's plans, and a graph's file and its runs in a page.

`functions::MarkInstance` is the aggregate of one bound call of a mark function: its template, which holds the layer's encodings and options without a table, and the fields of the columns of its table. Its accumulator collects the rows of its group, sorts them by the keys of the call's `ORDER BY`, and builds the chart of the layer with the rows as its table. `functions::CompositionInstance` is the aggregate of one bound call of a composition, which composes the charts of the rows of its group. A planner binds each call to one of them, so both hold their options and take only data columns and the keys of their `ORDER BY`.

Every chart that the aggregates build names GoogleSQL's syntaxes for its format patterns, `functions::FORMAT_SYNTAXES`.

`functions::Combine` is the function of a scalar composition, `layer`, `hconcat`, or `vconcat`, in a page's registry, and `functions::combine` composes the charts of each row for it and for a library's declaration of the composition.

`cut::CutInputExec` is the leaf through which a node's physical plan reads the result of a node below it. It holds the plan that scans that result and the scan's position, schema, projection, and limit, so a host that plans a node once replaces the scan with a scan of each computation's result and runs the same plan.

## Kept results and keys

`kept::Kept` is a node's computed result: some of the columns of its output, its batches, and whether they are in the order of its rows. `kept::substitute` points each `CutInputExec` of a plan at a computation's results through `kept::kept_scan`, which replays ordered batches in one partition with their ordering and spreads others over the session's partitions, and `kept::shapes` gives what of the results decides the properties of their scans, so a host reuses a plan for inputs of the same shapes. `keys::SelectionKey` is the part of a node's key that a selection gives: the fingerprints of the clauses of the owners that the node reads, and the selection's resolution when there are any.

## A graph's file

`page::GraphFile` holds a graph that a host planned for a page: for each node, its result when it reads no selection, or its physical plan, what it reads of the selections, and its result with every selection empty, then the outputs and the document's selections.

`codec::GraphCodec` encodes and decodes the plans with datafusion-proto's messages, with the leaves of cuts, the bound marks and compositions, and the operators of selections as its extensions, so the plan's leaves never carry the results that a host planned them with.

`page::PageGraph::decode` reads a file in `page::task_context`, which holds DataFusion's scalar, nested, aggregate, and window functions and the scalar compositions, and `PageGraph::run` returns each output's batches for the clauses of the selections. A run resolves its nodes from the top down, hits the results that earlier runs and the file's empty selections computed, and computes each miss from the bottom up, binding the operators of its selections to the run's states with avenger-selection-datafusion's `bind_selections`. A plan that names a function outside the context fails to decode, so a host checks each plan against the context before it writes the file.

`decode::decode` decodes a page's plans. Each DataFusion operator decodes its own message, and the decoder dispatches only the operators that the plans of charts hold: projections, filters, aggregates, windows, sorts, limits, unions, cross, hash, and nested-loop joins, unnests, scalar subqueries, the placeholder row, the empty scan, the scan of rows that a plan holds, such as a `VALUES` list, which a graph keeps in the node that reads it, the coalesces, the cooperative wrapper, and the codec's extensions. A page therefore links the decoders of those operators and not of DataFusion's others, its sources of files, or its sinks. Expressions decode with datafusion-proto's parsers. An operator outside the set fails to decode with its name, so the host's check reports it. The decoder implements `ExecutionPlanDecode`, the trait through which an operator decodes its children, expressions, and functions, which DataFusion hides from its documentation and does not count as public, so an upgrade of DataFusion can change it. The tests decode a plan of each operator and of the expressions of charts, and run it to the native plan's rows.

## Dependencies

The crate depends on DataFusion's sub-crates and not on the `datafusion` crate, so its dependencies name the parts of DataFusion that a WebAssembly page needs. Its WebAssembly build enables chrono's `wasmbind`, since DataFusion's metrics read the clock when an operator starts and ends. Its tests run a plan of a lifted chart, the decoder's plans, and the graph of a page that avenger-sql-project's tests write to `tests/output/`, whose baselines are in `tests/baselines/`, natively and in a browser:

```bash
cargo test --release -p avenger-physical-graph
```

```bash
wasm-pack test --release --headless --chrome avenger-physical-graph
```

On macOS, the browser build compiles C code for wasm32 with Homebrew's LLVM, through `CC_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/clang` and `AR_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/llvm-ar`.
