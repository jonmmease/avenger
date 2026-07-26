# Optional Avenger Project Manifest

Status: future design; not part of Avenger language version 1 and not an
implementation plan.

## Motivation

Avenger source modules already carry the information that belongs in source:
the language version, exact imports, native-module requirements, definitions,
datasets, and chart declarations. A project can therefore remain a
manifest-less directory and compile an explicitly selected module.

Some inputs instead belong to a project invocation:

- the capability boundary shared by the compiler, CLI, watch process, LSP, and
  editor;
- friendly names for chart entrypoints used by project-wide commands;
- the explicitly selected ambient data modules that distinguish development,
  test, and production environments;
- future project-wide test, documentation, formatter, and linter settings.

Today these inputs can come from host configuration. Without one shared rule,
however, the CLI can treat the entry module's directory as the project root
while an LSP treats the editor workspace as the root. The same source then has
different capability boundaries and ambient inputs.

An optional `avenger.toml` could provide a deterministic root marker and a
small project-level configuration surface without turning source imports into
a package-manager dependency system.

## Proposed Discovery Contract

The prospective discovery order is:

1. an explicit project root or manifest supplied by the host;
2. the nearest ancestor of the selected entry module containing
   `avenger.toml`;
3. the entry module's containing directory when no manifest exists.

Version-control metadata such as `.git` does not participate. The compiler,
CLI, watch process, LSP, Zed integration, and other hosts must use one shared
discovery implementation and expose the effective root in diagnostics and
introspection.

The manifest's directory is the project and default capability root. This does
not change module-relative semantics: imports, themes, and data paths continue
to resolve relative to the module that declares them. An explicit host
override is capability policy and must never cause implicit source-module or
ambient-data discovery.

Projects without a manifest remain fully supported.

## Candidate Initial Surface

```toml
manifest-version = 1

[project]
name = "sales-analytics"
default-chart = "overview"
default-data = "development"

[charts.overview]
module = "charts/overview.avenger"
chart = "overview"

[charts.regional]
module = "charts/regional.avenger"
chart = "regional_sales"

[data.development]
ambient-modules = ["data/local.avenger"]

[data.production]
ambient-modules = ["data/warehouse.avenger"]
```

`manifest-version` versions the TOML contract, not the Avenger source
language. Every `.avenger` module continues to carry its own `avenger 1;`
pragma.

### Project Metadata

`project.name` is optional descriptive identity. Optional `description`,
`authors`, `license`, and `repository` fields could be added when publishing
or generated documentation needs them; they should not affect compilation.

### Chart Entrypoints

Each `[charts.<name>]` entry maps a stable project-local name to:

- a module path relative to the manifest;
- an optional chart selector, required when the module does not have a unique
  selectable chart.

This supports commands such as `avenger watch overview`, deterministic
project-wide test and documentation enumeration, and editor runnables without
scanning arbitrary files for semantic inputs. Direct module-and-selector CLI
forms remain available without a manifest.

A later test/documentation surface could associate an entrypoint with a
baseline image, data configuration, tags, or publication settings. Those
fields should be added only when the corresponding workflow is specified.

### Ambient Data Configurations

Each `[data.<name>]` table names an exact set of ambient data-only source
modules. The selected set is an explicit compiler input:

- paths are relative to `avenger.toml`;
- the set is canonicalized and duplicate-free;
- declaration merging is order-independent;
- collisions are errors and identify every contributing module;
- ordinary source discovery never adds ambient modules;
- the selected set and its module identities participate in analysis and
  cache identity.

`project.default-data` is optional. A project may instead require every host
invocation to select a data configuration explicitly. The eventual CLI name
should avoid the word “profile,” which already denotes a native registry
profile.

## Deliberate Exclusions

The initial manifest should not contain:

- **Source dependencies.** Exact relative, standard, native, and remote
  imports already describe the source dependency graph.
- **The Avenger language version.** It remains explicit in every source
  module.
- **Native registry requirements.** Exact `native:` imports state these, and
  compiled artifacts record the resolved native registry profile.
- **Credentials or secrets.** Data declarations reference environment
  capabilities without embedding values.
- **Capability grants.** Checked-in project configuration may describe or
  request needs, but must not authorize itself to read the environment,
  escape the project root, or access the network.
- **Local performance preferences.** Watch scale, cache budgets, window
  placement, and similar settings belong to user or host configuration.
- **A mandatory inventory of every source file.** Explicit imports and named
  chart entrypoints determine meaningful module closures.
- **Package resolution.** The manifest does not introduce version ranges,
  registries, implicit suffixes, directory indexes, or a lockfile.

## Tooling Consequences

If adopted:

- the core compiler receives an already resolved project configuration rather
  than independently discovering one;
- the CLI and LSP call a shared discovery/configuration crate or module;
- the LSP does not equate an editor workspace folder with an Avenger project
  root unless it contains the selected manifest or is explicitly configured;
- Zed runnables can invoke named chart entrypoints and selected data
  configurations;
- `watch`, project analysis, and language-server caches include the canonical
  manifest, selected chart entrypoint, and selected ambient data set in their
  generation identity;
- diagnostics report the effective manifest/root and name all origins in an
  ambient collision.

Filesystem scanning may still be used to find documents for editor indexing.
It must not make an unreferenced source module semantically active.

## Open Decisions

Before implementation planning:

1. Decide whether the filename is fixed as `avenger.toml` or whether an
   explicit host path may name a differently named manifest.
2. Decide whether a default data configuration is encouraged, discouraged, or
   forbidden for builds that claim reproducibility.
3. Choose the CLI terminology for data selection, such as
   `--data development` or `--environment development`.
4. Decide whether chart baseline paths belong directly on chart entries or in
   a later dedicated test table.
5. Specify how nested manifests behave. The likely rule is nearest-manifest
   ownership, with an explicit root override required to select an outer
   project.
6. Decide whether project-wide file-index exclusions are needed initially or
   can wait for formatter/linter project commands.

## Relationship To The External Review

This direction addresses AV-P1-07 from the external programming-languages
review. The source-module refactor has already removed role-specific
filenames and implicit ambient-file scanning. The remaining objective is to
make project-root, chart-entrypoint, and ambient-data inputs explicit and
shared by every host without weakening Avenger's module-relative resolution
or resolver-free import model.
