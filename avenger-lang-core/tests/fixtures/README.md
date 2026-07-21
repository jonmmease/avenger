# Frontend fixtures

These directories are populated phase by phase from the normative DSL spec.
`tokens/` stores the Phase 1 lexical, binding-normalization, SQL-island, and
negative conformance corpus. `parse/` and `resolution/` separate valid and
invalid sources in later phases; `definitions/` covers hygienic expansion.
Fixtures are reviewed inputs and must never be rewritten by a baseline update.
`parse/` contains Phase 2 chart, definition, data-root, and strict negative
files. Its canonical AST, semantic print, and diagnostic outputs are reviewed
under `tests/baselines/parse/`.
The DataFusion expression fixture translates representative column, literal,
predicate, aggregate/window, temporal, array, and placeholder expression shapes
already used by the Rust chart and partial-evaluation code into their SQL forms.
`tokens/tree_sitter_conformance.json` is the machine-readable editor-grammar
contract. The Rust token/query tests execute every row against the strict
frontend; peer Tree-sitter repositories consume a revision-and-hash-pinned copy.
`tree_sitter/sql_island_boundaries.json` closes the four structural SQL-island
contexts and every strict-parser call site. `tree_sitter/structural_sources.json`
explicitly classifies and hashes every `.avenger` source under the language
crates. `tree_sitter_contracts.rs` makes both manifests executable and prevents
new sources or island-bearing syntax from bypassing the editor contract.
