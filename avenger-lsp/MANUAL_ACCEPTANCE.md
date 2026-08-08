# Native LSP and Zed acceptance

Run this checklist in an interactive Zed session before claiming a language
server release candidate. Record the Avenger commit, `avenger-zed` commit, Zed
version, OS, architecture, launch path, and result. Automated transcript tests
do not satisfy these UI rows.

## Setup

Install `/Users/jmease/repos/avenger-zed` as a Zed dev extension, open the
Avenger worktree, and open representative chart, data, and definition files.
The source-workspace launch should be:

```sh
cargo run --release -p avenger-lang-cli --bin avenger -- lsp
```

Repeat startup once with an explicit `lsp.avenger-lsp.binary.path`, and from an
ordinary non-monorepo worktree with `avenger` on `PATH`.

## Checklist

- [ ] Tolerant diagnostics and outline symbols update through incomplete
  braces, strings, SQL, and rapid edits without restarting the server.
- [ ] Structural completion covers declarations, native kinds, properties,
  enums, `$` bindings, struct fields, import paths, and channels. Exercise
  `param <SQL expression> as <name>`, `store|selection`, `mark group`,
  `legend.overlay`, every no-`as` slot/variable/field header, and both output
  forms.
- [ ] Contextual completion distinguishes parallel frame `dimensions:` from
  mark-owned dimension expressions, limits channel-slot bodies to `default:`,
  offers predicate-member fields, and filters `set` operations by the resolved
  scalar/store/selection target.
- [ ] SQL completion covers SELECT-first and FROM-first queries, qualified
  columns, CTEs/subqueries, joins, and transform-stage schemas. Confirm that
  columns appear only in `"...` / `alias."...`, use canonical quote escaping,
  and never appear after a bare prefix or `alias.`. Confirm `$` is required for
  scalar params and table-valued stores.
- [ ] Automatic completion is empty in SQL strings/comments, declaration and
  SQL alias binders, unknown structural statements, and unsupported bare
  column positions. Explicit invocation may add conservative fuzzy matches but
  never a syntactically illegal candidate family.
- [ ] Completion details identify Arrow type, nullability, source stage, and
  function signature where known. Quoted-column and `$binding` items remain
  visible while typing because their `filterText` includes the authored prefix.
- [ ] Hover, definition, references, and highlights stay on authored source
  across local imports and unsaved changes.
- [ ] Format Document changes only strict-valid source and preserves comments
  and the file's line-ending convention.
- [ ] With semantic tokens off, Tree-sitter presentation remains complete.
  With `semantic_tokens: "combined"`, resolved identities overlay cleanly.
- [ ] Rename preview/application is versioned, cross-file where appropriate,
  and suppressed for collisions, stale documents, and unsupported symbols.
  Confirm a `selection` rename updates an unprefixed `set` target.
- [ ] Quick fixes work for a close property typo, missing `as`, ambiguous SQL
  qualification, and an unambiguous missing scalar param.
- [ ] Inline definition replaces one imported instance with canonical ordinary
  DSL. Extract definition previews a create-file operation plus versioned chart
  edits and the result compiles. Existing target files are never overwritten.
- [ ] Pin import appears for an unpinned HTTP definition, performs no fetch
  until selected, inserts the verified SHA-256, and rejects a stale document.
- [ ] Rapid typing/save storms, atomic replacement, multiple charts, and
  repeated **Restart Language Server** do not publish stale diagnostics or
  completion results, or leave duplicate server processes/watchers.
- [ ] Closing the worktree and quitting Zed releases the server, filesystem
  watchers, file handles, and child process cleanly.
- [ ] Startup/configuration errors appear once and are actionable; protocol
  stdout contains no Cargo warnings or human log lines after the server starts.

## Result record

```text
Date:
Avenger commit:
avenger-zed commit:
Zed/OS/architecture:
Launch paths tested:
Rows passed:
Rows failed (logs/reproduction):
Tester:
```
