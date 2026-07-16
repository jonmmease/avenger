# Frontend fixtures

These directories are populated phase by phase from the normative DSL spec.
`tokens/` stores the Phase 1 lexical, binding-normalization, SQL-island, and
negative conformance corpus. `parse/` and `resolution/` separate valid and
invalid sources in later phases; `definitions/` covers hygienic expansion.
Fixtures are reviewed inputs and must never be rewritten by a baseline update.
