# Project Knowledge Guides

These files contain cached analysis and architectural knowledge. Read them when working on related areas:

| File | When to read |
|------|--------------|
| `guides/project-overview.md` | Starting work on the project or need general orientation |
| `guides/codebase-structure.md` | Need to understand project layout and module organization |
| `guides/architecture-patterns.md` | Implementing new features or refactoring existing code |
| `guides/code-style-conventions.md` | Writing new code to match existing style |
| `guides/facet-layout-overflow-model.md` | Working on facet layout or overflow handling |
| `guides/nested-facet-implementation.md` | Working on nested facet features |
| `guides/coordinate-transform-analysis.md` | Working on coordinate transformations |
| `guides/overflow-measurement-analysis.md` | Debugging overflow or measurement issues |
| `guides/avenger-chart-documentation-system.md` | Working on documentation |
| `guides/suggested-commands.md` | Need common development commands |
| `guides/task-completion-checklist.md` | Completing tasks and ensuring quality |
| `guides/debugging-strategy.md` | Choosing between interactive debugging and logging |

## Measurement and Coordination System

| File | Purpose |
|------|---------|
| `guides/measurement-coordination-detailed.md` | Comprehensive 750-line analysis covering entire two-pass measurement algorithm, data flow, coordination context, and design patterns with code examples and line references |
| `guides/measurement-architecture-quickref.md` | Quick reference guide with file locations, key functions, data structures, common modifications, and debugging tips |

### When to Read

- **measurement-coordination-detailed.md**: Need deep understanding of how measurement works, tracing parameter flow, understanding spacing coordination
- **measurement-architecture-quickref.md**: Quick lookup of function locations, data structure fields, spacing keys, common modifications

### Key Topics Covered

- Two-pass algorithm (Pass 1 measurement, Phase 1.5 coordination, Pass 2 rendering)
- Data flow during measurement (domain extraction, scale building, subplot measurement, overflow aggregation, gap computation)
- Coordination context structure and parameter passing
- Scale sharing modes and fallback scale handling
- Recursive nested facet measurement via data_override
- Spacing needs aggregation and re-measurement
- Overflow types (guide-only vs. total)
- Concurrent measurement with semaphore bounding
