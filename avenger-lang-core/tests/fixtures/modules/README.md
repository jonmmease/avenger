# ESM-style module-system fixtures

These fixtures define the accepted module-system vertical slices before the
breaking parser migration. They are intentionally not part of the legacy
single-root parser corpus. Add them to strict, resolution, compiler,
Tree-sitter, analysis/LSP, and bundling manifests as the corresponding phases
land.

- `mixed-singleton.avenger`: local data, definition, and anonymous chart.
- `multi-chart.avenger`: two named chart entrypoints sharing one relation.
- `library.avenger` and `consumer.avenger`: mixed exported source module,
  named imports, aliases, and private definition dependency.
- `native-consumer.avenger`: explicit native-module namespace import.
