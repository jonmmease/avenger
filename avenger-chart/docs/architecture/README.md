# Avenger Chart Architecture

These documents describe the current internal architecture of `avenger-chart`.
They are development references, not user-facing book chapters.

User-facing chart documentation lives in `avenger-chart/book/src`.

## Documents

- `crate-boundaries.md`: chart-layer crate boundaries, dependency graph,
  owned APIs, and extension contracts.
- `facet-system.md`: built-in row/column facet runtime, partition tree,
  measurement pipeline, coordination pipeline, placement, and sharing model.
- `positioned-subplots.md`: coordinate-positioned `Subplot<Coord>` compile and
  runtime path, partitioned positioned subplot behavior, and child-frame
  integration.
- `architecture-docs-outline.md`: additional current-system architecture
  sections to add.

## Documentation Rules

- Describe how the current system works.
- Prefer type names, trait names, function names, and module paths.
- Do not use source line-number anchors.
- Keep implementation plans out of current architecture references.
- Delete or move stale planning notes instead of preserving them in the
  architecture directory.
