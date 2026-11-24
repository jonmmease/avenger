# The avenger engine of Quarto 2

The engine extension that runs the `{avenger-sql}` cells of a Quarto 2 document through `avenger quarto2-engine`, which draws their charts. q2's core writes the cells and their figures from the engine's notebook record of the outputs, so captions, labels, cross-references, and the options that show or hide a cell's code and outputs work as they do for a Python cell. The `avenger` binary embeds the extension, and `avenger quarto2-init` installs it in a project.

- `_extensions/avenger/`: the extension. `src/avenger-engine.ts` is its source, and `dist/avenger-engine.js`, committed, is its bundle, which `q2 call build-ts-extension` builds in this folder, or `npm run build` here.
- `tests/avenger-engine.test.ts`: its tests with a fake `avenger`, `node --test tests/*.test.ts` with Node 24, which runs TypeScript by stripping its types.
- `tests/e2e.mjs`: a render of `tests/fixtures/report.qmd` in a temporary project that takes the extension by `q2 add` of this folder, with `avenger` from `AVENGER` or `PATH`.

A checkout installs the extension in a project, as it is, with `q2 add <PATH_OF_THIS_FOLDER> --no-prompt`.
