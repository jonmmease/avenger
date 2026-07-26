# Avenger Language Future Work

This directory contains durable design notes for the Avenger language and its
project tooling that are not yet normative or scheduled for implementation.
It is separate from
[`avenger-chart/docs/future-work`](../../../avenger-chart/docs/future-work/README.md),
which owns the Rust chart API, chart semantics, marks, coordinates, guides,
layout, and rendering design.

Topics belong here when their primary owner is one or more of:

- Avenger source modules, imports, names, or project structure;
- the structural grammar, AST, authoring schema, or compiler;
- the CLI, language server, editor integrations, or project-wide workflows;
- source-level distribution, reproducibility, or capability configuration.

Detailed implementation plans and active checklists may continue to live in
`scratch/avenger-lang/`. Once a design here becomes normative, its accepted
contract should move into the canonical language specification and the
future-work note should either become a short historical pointer or be
archived.

## Current Notes

| Document | Status |
| --- | --- |
| [project-manifest.md](project-manifest.md) | Future design: an optional `avenger.toml` that acts as a deterministic project-root marker and names chart entrypoints and ambient-data configurations. |

## Follow-Up Todo

- [ ] **AV-P1-08 — consolidate SQL-island boundaries.** Create one normative
  `SQL Island Boundaries` section in the canonical DSL specification; make its
  five-context boundary table, balancing and terminator rules, lone-identifier
  typed-object rule, and quoted-column requirement authoritative. Rewrite the
  current Block Modes account to point there, and keep the Rust
  `SqlIslandContext` inventory, strict parser corpus, Tree-sitter corpus, and
  editor recovery fixtures as executable mirrors. This is specification
  consolidation and test hardening, not a syntax redesign.
