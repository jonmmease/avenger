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
