# avenger-chart-datafusion

The DataFusion side of avenger's charts, for hosts that run plans natively and in WebAssembly pages: the functions that build charts, and the codec of the logical plans that call them.

## Functions

`functions::MarkInstance` is the aggregate of one bound call of a mark function: its template, which holds the layer's encodings and options without a table, and the fields of the columns of its table. Its accumulator collects the rows of its group, sorts them by the keys of the call's `ORDER BY`, and builds the chart of the layer with the rows as its table. `functions::CompositionInstance` is the aggregate of one bound call of a composition, which composes the charts of the rows of its group. A planner binds each call to one of them, so both hold their options and take only data columns and the keys of their `ORDER BY`. `functions::Combine` gives the scalar compositions, `layer`, `hconcat`, and `vconcat`, as functions without state, and `functions::combine` composes the charts of each row for them.

Every chart that the aggregates build names GoogleSQL's syntaxes for its format patterns, `functions::FORMAT_SYNTAXES`.

## Codec

`codec::PlanCodec` writes and reads a logical plan that builds charts, for a host that sends a plan to where it runs, such as a page. DataFusion's messages hold the plan, and the codec adds what they do not know:

- A mark's aggregate travels as the bytes of its template and its fields, and a composition's as the bytes of its options.
- A function or an extension node of the host that planned the plan travels through that host's `codec::HostCodec`. A session holds its hosts as `codec::Hosts`, an extension of its configuration, so the session that writes a plan and the session that reads it find the same hosts.
- A scan travels as the position of its table in the tables that the codec holds.

A function without state travels as its name, and decodes from the functions of the session that reads the plan. A plan must hold the aggregates, not the declarations that a front end such as avenger-sql binds to them when it simplifies a plan's expressions.

## Dependencies

The functions depend on DataFusion's sub-crates. The codec depends on datafusion-proto and on the `datafusion` crate, whose logical plans it writes. The tests run a plan of a lifted chart and a round trip of the codec natively and in a browser, and enable chrono's `wasmbind` in WebAssembly, since DataFusion's metrics read the clock when an operator starts and ends:

```bash
cargo test --release -p avenger-chart-datafusion
```

```bash
wasm-pack test --release --headless --chrome avenger-chart-datafusion
```

On macOS, the browser build compiles C code for wasm32 with Homebrew's LLVM, through `CC_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/clang` and `AR_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/llvm-ar`.
