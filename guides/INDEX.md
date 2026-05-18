# Project Knowledge Guides

These files contain cached analysis and architectural knowledge. Read them when working on related areas:

| File | When to read |
|------|--------------|
| `guides/project-overview.md` | Starting work on the project or need general orientation |
| `guides/codebase-structure.md` | Need to understand project layout and module organization |
| `guides/architecture-patterns.md` | Implementing new features or refactoring existing code |
| `guides/code-style-conventions.md` | Writing new code to match existing style |
| `guides/facet-layout-overflow-model.md` | Conceptual overflow stacking model and edge aggregation rules used by the facet system |
| `guides/facet-invariants.md` | Critical facet system invariants that must be maintained |
| `guides/overflow-measurement-analysis.md` | Debugging overflow or measurement issues |
| `guides/avenger-chart-documentation-system.md` | Working on documentation |
| `guides/suggested-commands.md` | Need common development commands |
| `guides/task-completion-checklist.md` | Completing tasks and ensuring quality |
| `guides/debugging-strategy.md` | Choosing between interactive debugging and logging |

## Facet System Architecture

The authoritative reference for the facet system lives in the avenger-chart crate, not in this guides directory:

- `avenger-chart/docs/architecture/facet-system.md` — Module layout, the four-phase pipeline (tree → coord → coordination → place/render), key data structures, slot-sharing model.

The two facet guides in this directory (`facet-layout-overflow-model.md` and `facet-invariants.md`) supplement that document with the conceptual overflow model and the non-type-enforced invariants respectively.
