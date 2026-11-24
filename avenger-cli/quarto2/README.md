# The avenger engine of Quarto 2

The engine extension that runs the `{avenger-sql}` cells of a Quarto 2 document through `avenger quarto2-engine`, which draws their charts. q2's core writes the cells and their figures from the engine's notebook record of the outputs, so captions, labels, cross-references, and the options that show or hide a cell's code and outputs work as they do for a Python cell. The `avenger` binary embeds the extension, and `avenger quarto2-init` installs it in a project.

- `_extensions/avenger/`: the extension. `src/avenger-engine.ts` is its source, and `dist/avenger-engine.js`, committed, is its bundle, which `q2 call build-ts-extension` builds in this folder, or `npm run build` here.
- `grammars/avenger-sql/`: the grammar that q2 highlights the cells' code with, from tree-sitter-avenger-sql, which `quarto2-init` writes to a project's `_quarto/grammars/avenger-sql/`. `scripts/sync-grammar.mjs --grammar-root PATH` copies it from a checkout of tree-sitter-avenger-sql whose wasm is built (`node scripts/build_wasm.mjs` there), adds the export `tree_sitter_avenger-sql` that q2 looks for, renames the two captures that q2's palette names otherwise, and records the grammar's commit in `UPSTREAM`. `--check` compares a sync with the committed files.
- `tests/avenger-engine.test.ts`: its tests with a fake `avenger`, `node --test tests/*.test.ts` with Node 24, which runs TypeScript by stripping its types.
- `tests/e2e.mjs`: a render of `tests/fixtures/report.qmd` in a temporary project that takes the extension by `q2 add` of this folder, with `avenger` from `AVENGER` or `PATH`.

A checkout installs the extension in a project, as it is, with `q2 add <PATH_OF_THIS_FOLDER> --no-prompt`.
