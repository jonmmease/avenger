# Avenger language server

`avenger-lsp` is the native LSP 3.17 protocol adapter for the Avenger chart
language. The installed entry point is `avenger lsp`; this crate is a library
so protocol transport remains separate from the editor-neutral
`avenger-lang-analysis` APIs.

The server provides incremental UTF-8/UTF-16 document synchronization,
tolerant and semantic diagnostics, document symbols, structural and
DataFusion-backed SQL/expression completion, hover, definition/references,
document highlights, strict-valid formatting, semantic tokens, safe rename,
quick fixes, definition refactors, and remote-import pinning. Ordinary editor
analysis is schema-only and never scans table data, creates a physical plan, or
calls `collect()`.

## Running

From this workspace:

```sh
cargo run --release -p avenger-lang-cli --bin avenger -- lsp
```

The process speaks JSON-RPC over stdin/stdout. Standard output is reserved for
LSP framing; human-readable warnings go to stderr and `window/logMessage`.
`RUST_LOG` controls tracing without printing source or data values by default.

Resource controls are CLI arguments:

```text
--debounce-ms <MILLIS>                 default 120
--max-document-mb <MB>                 default 8
--max-diagnostics <COUNT>              default 200 per document
--max-workspaces <COUNT>               default 32
--max-semantic-tokens <COUNT>          default 100000 per document
--max-concurrent-requests <COUNT>       default 16
--max-analysis-cache-entries <COUNT>    default 64 per workspace
--max-dataset-cache-entries <COUNT>     default 512 per workspace
```

All count and size limits must be greater than zero. Compiler project, source,
syntax, SQL, import, and expansion limits apply in addition to these
transport-level bounds.

## Snapshot and capability model

Every edit publishes immediate tolerant syntax against its exact document
version. Cross-file work is debounced into immutable generations; a newer edit
cancels the preceding task, and generation, document-version, source-revision,
and native-registry-profile checks prevent stale publication. Broken roots
retain compatible last-good semantic information while healthy roots remain
independent.

The server prefers UTF-8 positions when the client offers them and otherwise
uses required UTF-16 positions. Rename and code actions require versioned
`documentChanges`. Extract-definition additionally requires the create-file
resource operation. Pin-import is offered only when the client preserves code
action data and can lazily resolve the `edit` property.

Formatting is intentionally unavailable for invalid source. Rename is limited
to authored, collision-free identities with complete references. Extraction
is offered for a named group only when it can create a sibling
`<name>.mark.avenger` safely; external scalar references become `slot expr`
inputs, while unsupported free references suppress the action. Inline uses the
compiler's canonical single-instance expansion. Pin-import fetches only after
the user chooses the action, validates that the result is an Avenger
definition, and inserts the exact SHA-256. HTTP remains disabled for ordinary
project analysis.

## Testing

```sh
cargo test --release -p avenger-lang-analysis
cargo test --release -p avenger-lsp
cargo test --release -p avenger-lang-cli --test lsp_stdio
cargo clippy --release -p avenger-lang-analysis -p avenger-lsp \
  -p avenger-lang-cli --all-targets --no-deps -- -D warnings
```

The suites cover protocol transcripts, malformed and stale changes, both
position encodings, all compiler `.avenger` fixtures, randomized incremental
edits, cancellation/generation races, resource limits, capability fallbacks,
DataFusion completion without execution, and process-level stdio framing.

See [MANUAL_ACCEPTANCE.md](MANUAL_ACCEPTANCE.md) for the installed-editor
matrix. Live inspector values, table preview windows, DAP, and automatic server
downloads are later milestones.
