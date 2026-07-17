# Avenger Chart DSL Syntax

## Status

Draft syntax proposal. This document sketches a dedicated scripting language for
authoring `avenger-chart` charts. The goal is not to encode the Rust builder API
one-to-one, and not to start from JSON or YAML. The goal is a small declarative
DSL that can express all current chart features while remaining pleasant for
chart authors.

The first draft favors a regular canonical form. Shorthands can be added later
once the core grammar is proven. Implementation sequencing — the kernel
cut and its per-phase gates — is
[Implementation Phasing](#implementation-phasing).

Adopted decisions (2026-07-07): a required `avenger 1;` version pragma; SQL
string semantics everywhere with mandatory double-quoted data columns and
bare identifiers reserved for DSL names; `value` as the only unscaled
spelling; reserved helper functions (`channel(x)`, `datum('id')`, ...)
instead of sigil forms, with bare arguments for DSL-space names and strings
for data-space names; `sql:` restricted to query statements; cross-file
reuse via `import` and parameterized `define` with `channel` parameters; a
two-tier kind model in which native built-ins are registered by the host and
custom compound marks, tools, and transform pipelines are authored with
`define` — definitions extend the language but are not required to reproduce
or replace native built-ins; custom SQL transforms via the primitive
`transform sql` (over the reserved `input` relation) and importable
`define transform` pipelines with `output` handle declarations; the native
core `transform pipeline` container is their ordinary editable inline form,
preserving one parent-stage position, sequential child dataflow, and public
output handles; the native core `tool behavior` container likewise provides
the ordinary inline form for defined tools, with stable required identity,
instance-scoped state/events/chrome, exact exports, and containing-plot scale
edits; `match` over enum slots as the only branching construct
in definitions (compile-time, closed arms); block slots (caller-provided
declarations at a marked splice point, with declared `exposes` handles) and
typed `function` slots as the two remaining extension constructs;
two-tier publicity (named declarations in a chart have canonical structural
paths; definition internals are private and cross the boundary only through
explicit, stable `export` aliases, while caller-authored block-slot content
retains caller-owned names beneath the instance); ordinary groups can express
the fully inlined form with `private` declarations, constrained `public`
hoisting, component-boundary `export` aliases, and opaque `component_kind`
provenance — there is no generated-only expansion dialect;
params and stores share one lexical value-binding namespace, with `$path`
required for references, scalar-versus-table checking by use context, relation
bindings valid in SQL `FROM`, and typed `set param`/`set store` l-values
retained; every param declares one physical Arrow `type:`, params are nullable,
and behavioral roles are attached at typed use sites rather than through a
param `kind:`;
registry-free distribution — imports are uniform (`std:`, relative, URL) in
every file however obtained, relative imports resolve against the
importer's location, the transitive closure of exact hash-pinned files is
fetched with no version resolution, integrity is inline (`sha256` on the
import, Merkle-pinning the closure with no lockfile), one definition per
file (chart files plus mark/tool/transform definition files — imports bind
exactly one name, and collection-style bundling cannot exist; themes are
plain CSS files, fetched and pinned like imports; a project is a
manifest-less directory whose root is the default capability boundary, with
every relative resource resolving against its declaring file; `.data.avenger`
catalog files configure a DataFusion-shaped catalog/schema/table hierarchy —
provider-backed catalogs with explicit schema projections, inline `catalog
schemas` and `schema tables` containers, and individually bound tables —
ambient by default and
importable as pinned single-root dataset packs by data files and charts
(never definitions), with credentials via capability-gated `env`
values and `.env`, and catalog-level SQL views spelled
`table sql` — logical by default, materialized per session by opt-in, and
parameterized by defaulted `param` declarations, `$name` scalar placeholders in a
once-planned query rebound per use via data-block properties or named
table-function arguments, chaining over any catalog relation with
placeholder forwarding), `expand` as the only definition flattening
(defined-kind instantiations lowered to native built-in and primitive
declarations, with definition imports removed), and duplicate vendored
definitions are harmless because expansion is instance-namespaced; with the
rejected shapes recorded in
[What Definitions Deliberately Cannot Do](#what-definitions-deliberately-cannot-do);
widgets as opaque schema-registered built-in kinds — composed versus native is
a Rust implementation detail, the DSL may instantiate and reference either,
but language version 1 has no `define widget` form and source expansion leaves
widget declarations intact;
lowercase snake_case kind names throughout (`mark rect`, `mark box_plot`);
a six-node generic AST whose serde encoding is the JSON interchange form —
single-key tagged values, SQL islands as canonical text, closed node set —
under round-trip laws (printing is total and canonical, one layout
engine shared with `avenger fmt`; `parse(print(ast)) == ast`), so specs are produced and
consumed without the Rust library, validated structurally by a frozen
hand-written core JSON Schema (closed fourteen-tag value inventory) and
semantically by a full JSON Schema generated from the authoring schema;
and the EBNF grammar below.

Adopted 2026-07-11: the **authoring schema is a normative v1 language
artifact**, not an LSP implementation detail. A dependency-light
`avenger-chart-schema` meta-model describes declaration bodies, properties,
value shapes, native kind signatures, outputs, parts, enums, and docs. The
initial native registry is hand-authored and serialized as a canonical,
versioned snapshot; semantic validation, resolution, lowering, the LSP,
documentation, CLI introspection, and the generated full JSON Schema consume
that same registry. The structural parser remains generic and schema-free, so
adding a native kind never adds a parser production. Imported definitions
contribute schema fragments validated against the same meta-model. Macro
emission may replace manual entries later only when it reproduces the reviewed
snapshot exactly.

Adopted 2026-07-12: the DSL lowers through the unified Rust semantic model
described in the
[Rust–DSL semantic unification implementation plan](../../../scratch/avenger-lang/rust-dsl-unification-implementation-plan.md).
Compiled params, stores, selections, marks, inline views, and tool instances
use opaque typed identities distinct from source names and public aliases;
lexically scoped state is resolved and hoisted into typed chart registries;
event effects lower to one ordered transactional action vector, including an
explicit cursor effect; param Arrow types are authoritative declarations;
pipelines remain one compiled parent stage; native authoring-schema entries are
paired with erased Rust lowerers; native and defined tools share one resolved
behavior expansion; built-in widgets retain their outer measurement and
presentation contract while composed widget behavior migrates through that
same tool representation; and views are inline lexical scopes, not
reusable/exported resources. V1 scale edits therefore target only the
containing plot.

This is the single reference for the language. The earlier companion
documents — the API/baseline audit and the missing-syntax proposals — have
been folded in: the feature-surface sections below carry their still-valid
content updated to the adopted rules, and the
[Coverage And Validation](#coverage-and-validation) section carries the
audit's inventory method and fixture plan. Every one of the 97 baseline
categories (892 deduped scenarios) has a syntax home in this document.

Adopted 2026-07-09: **filename-as-name** for the one-item file kinds —
the file stem is the canonical name, imports bind it, in-file `as`
binders are optional-and-validated, data catalogs (the plural kind) are
exempt and keep declared names, and one *public* item per file remains
the law (private nested defines recorded as the future pressure valve).
No dbt-style `ref()` sigil: queries are planned, not templated, so bare
table names are semantically resolved and SQL qualification is the
explicit form.

Adopted 2026-07-09: **`data:` is a property, not a declaration.** The
earlier `data as <name>` chart declaration is retired; a chart, group, or
mark sets its data context with an anonymous source block —
`data: { table: 'sales'; }`, `data: { sql: ...; }` — or reads a table-valued
store binding with `data: $brush;`. Named/file/SQL sources always use the block
form (no bare-string shorthand), with the same reserved source
properties as before (`table`, `sql`, `url`, `values`, plus table-param
bindings). Anonymous means private: chart-local relations are never
referenced by name; named shared relations are catalog `table` declarations,
which is also the graduation path.

## Design Principles

- Every file begins with a version pragma: `avenger 1;`.
- Use block-structured declarations for charts, groups, marks, transforms,
  interactions, tools, widgets, and other chart objects.
- Use `as` whenever a typed declaration binds a structural or dataflow name:
  `group as manual_box_plot`, `mark rule as median`, and
  `transform aggregate as stats`. Declaration headers are uniformly
  `[private|public] <keyword> <kind> [as <name>]`; the visibility prefix is
  absent in ordinary authoring and appears when a declaration deliberately
  leaves or re-enters a private structural subtree.
- Use `property: value` consistently inside property blocks.
- Bodies distinguish unordered properties from ordered child declarations:
  `property: value;` entries are unordered configuration; repeated child
  declarations (`mark`, `transform`, `cell`, `when`, `level`, ...) are
  ordered. Container bodies (`chart`, `cell`, `group`, channel config) mix
  both.
- Treat SQL expression snippets as the expression language in expression
  slots, with SQL string semantics everywhere: single quotes are string
  literals, double quotes are identifiers.
- Data columns are always double-quoted (`"horsepower"`); a bare identifier
  is never a column. Bare names belong to the DSL: kinds, properties, enum
  values, transform aliases, reserved namespaces, and helper functions.
- `value` is the only unscaled spelling. A bare expression in a channel slot
  is always scaled; `value '#2563eb'` is a literal visual value.
- Params and stores share one value-binding namespace. `$name` references the
  nearest scalar-valued param or table-valued store, and `$component.alias`
  references an exported binding; the surrounding scalar or relation-valued slot
  checks its kind. In event expressions, param reads may add `@start` or
  `@previous` to select a frozen temporal version (`$width@start` reads “width at
  start”); stores do not admit temporal qualifiers. Positional placeholders are
  rejected. All other DSL-injected
  references are reserved
  helper functions (`channel(x)`, `datum('id')`, `event_coord(x)`, ...),
  which tokenize as ordinary SQL. Qualified binding references use a pre-parse token
  normalization pass, not a custom lexer.
- Keep the DSL lexical surface compatible with DataFusion SQL tokenization.
  The whole file should tokenize through `sqlparser-rs` before the
  Avenger-specific parser interprets declarations and property blocks, and
  the token classes the DSL relies on are specified normatively and pinned by
  a conformance corpus so engine upgrades cannot silently change the
  language.
- Make channel blocks first-class because channel configuration is the main
  authoring atom in `avenger-chart`.
- Make `MarkGroup` explicit as `group`: a data-prep and authoring container,
  not a rendered scenegraph group.
- Reuse crosses files through `import` and parameterized `define`
  declarations (marks, tools, transforms). Native built-in kinds remain
  registered by the host and available through the same authoring schema;
  they do not have to be expressible as definitions. Definitions add custom
  compound marks, tools, and SQL-backed transform pipelines over the
  language-level surface available to their kind. The conditional budget for
  definitions is explicit and closed:
  `channel` parameters rename, `match` over an enum slot selects among
  declared variants at expansion time, SQL `CASE` handles expression logic —
  and nothing else branches. No loops, no string templating: data-driven
  multiplicity belongs to `repeat` and `facet`, and programmatic generation
  belongs to the host-language APIs, which emit DSL.
- Built-in widgets use `widget <kind> as <name>` and the same schema-backed
  registry discipline as other native kinds. The language does not define
  custom widgets and never exposes a widget's composed/native implementation
  tier.
- Preserve a post-v1 path to a canonical DSL form generated from a compiled or
  lowered chart; compiled-chart decompilation is outside the implementation
  plan below.

## Source Header And Versioning

Every file begins with a version pragma, optionally followed by imports:

```avenger
avenger 1;

import 'lib/error_bar.mark.avenger';
import 'lib/wheel_zoom.tool.avenger' as co_zoom;
```

The version names the language dialect, not the library release. Parsers
reject files whose major version they do not support. Files are UTF-8.

**A file contains exactly one thing, and the file name is its name**
(adopted 2026-07-09). After the header, a file holds either one `chart`
declaration (a chart file — data, params, stores, and all other resources
live inside the chart's body), one `define` declaration (a definition
file: a compound mark, tool, or transform). A project is a
collection of such files. Within this language layer, imports target
definitions and data resources; chart files are compilation roots rather than
chart dependencies.
The definition kind is normative from the file's content; the
conventional extensions mirror it: `.avenger` for charts,
and `.mark.avenger`, `.tool.avenger`, `.transform.avenger` for definitions.

For these one-item kinds, **the file stem is the canonical name** (the
dbt-model / single-file-component rule): recognized suffixes strip (the
kind extensions above plus a trailing `@version` tag, which remains
naming-not-mechanism), and imports bind that name — so an import line is
self-documenting without fetching the file. Consequences:

- The declaration's `as` binder is **optional, and when present must
  match the stem** (Java's validation move): drift between file name and
  in-file name is impossible, while `define mark error_bar` stays
  greppable for authors who want the name in the text.
- A stem that is not a valid bare identifier (hyphens, content-addressed
  URL names) requires `as` on the *import*; `as` also remains the rename
  and collision-resolution mechanism as before.
- Every chart file is therefore importable — the former
  anonymous-charts-are-private file rule dissolves (anonymous
  *declarations* inside bodies remain private everywhere).
- The interchange `File` struct carries a loader-populated `name` field,
  since JSON consumers without filesystem context still need it; printing
  never emits a binder the source didn't have, preserving the round-trip
  laws.

The one plural file kind is exempt: a data catalog names things inside
itself (it is configuration, not an item — the dbt `sources.yml`
counterpart to the one-per-file models), and importable packs bind their
single declared catalog or schema root, whose in-file presence is load-bearing
for the pack's internal chains. **One public item per file is the law**; if
definition clusters ever make file sprawl hurt, the designated pressure
valve is *private nested defines* (helpers visible only to the file's
single export — imports, naming, expansion, and the gallery untouched),
currently forbidden by the no-nested-definitions hygiene rule and listed
in [Open Questions](#open-questions) — never multiple exports.
Themes are not definitions — a theme is a plain `.css` file, referenced with
`theme css from` and fetched/pinned like any import. The one plural file
kind is the data catalog (`.data.avenger`) — host configuration naming the
tables available to charts, inherently a set (see
[Data Catalogs](#data-catalogs)).

## Names, Strings, And Columns

One rule set governs every name and string in the file, inherited directly
from SQL:

| Form | Meaning | Example |
| --- | --- | --- |
| `'...'` | string literal | `title: 'Horsepower';` |
| `"..."` | data column reference | `x: "horsepower";` |
| bare identifier | DSL name: kind, property, enum value, alias, namespace, helper function | `scale: linear`, `totals.amount`, `median(...)` |
| `$name` | lexical value-binding reference | `"mpg" >= $min_mpg`, `data: $brush` |
| `$path.to.name` | exported value-binding reference | `"mpg" >= $controls.min_mpg` |
| `$path@start`, `$path@previous` | frozen temporal param read in an event expression | `$width@start + event_coord(x) - start_coord(x)` |
| `<kind> <path>` | typed DSL reference | `selection hover.hovered` |

Data columns are **always** double-quoted, even when unambiguous. This is
what makes the rest of the language unambiguous:

- Reserved SQL words are non-issues: a column named `group` or `order` is
  simply `"group"` or `"order"`.
- A bare identifier is never a column, so `totals.amount` can only be a
  transform-alias output, `repeat.row` can only be the repeat namespace, and
  enum-valued properties (`position: right;`) can never collide with data.
- Struct-field access on a data column uses the quoted base:
  `"totals".amount` is field `amount` of a data column named `totals`, while
  bare `totals.amount` is the alias output. Declaring a transform alias that
  shadows a struct-bearing column name earns an editor warning, and the
  quoted form always reaches the data.

Literal channel values use the `value` prefix — the only unscaled spelling.
A bare expression in a channel slot is always a scaled expression:

```avenger
fill: "region";                 -- scaled: column feeds a scale
fill: value '#2563eb';          -- unscaled literal color
stroke: none;                   -- explicitly absent visual value
text: value 'Total';            -- literal text (not column "Total")
```

`none` is the DSL's absent-visual-value literal; SQL `NULL` remains `NULL`
inside SQL expression slots. The two are distinct: `none` removes a visual
property, `NULL` is a data value.

### Physical Arrow Types

Every param and store field names a physical Arrow type using one canonical,
lowercase type algebra. It is DSL schema syntax, not an SQL expression and not
a logical/semantic type alias:

```text
arrow_type = boolean
           | int8 | int16 | int32 | int64
           | uint8 | uint16 | uint32 | uint64
           | float16 | float32 | float64
           | utf8 | large_utf8 | binary | large_binary
           | date32 | date64
           | time32(second|millisecond)
           | time64(microsecond|nanosecond)
           | timestamp(second|millisecond|microsecond|nanosecond [,'timezone'])
           | duration(second|millisecond|microsecond|nanosecond)
           | interval(year_month|day_time|month_day_nano)
           | fixed_size_binary(length)
           | decimal128(precision,scale) | decimal256(precision,scale)
           | list(arrow_type) | large_list(arrow_type)
           | fixed_size_list(arrow_type,length)
           | struct([field(string,arrow_type) {, field(string,arrow_type)}])
           | map(arrow_type,arrow_type) ;
```

Lengths and decimal precision are positive integer literals; decimal scale is
an integer; a timestamp timezone is a single-quoted IANA name or fixed offset.
Nested list/map elements and struct fields use the Arrow-schema nullability
fixed by this v1 grammar (nullable); store-field nullability remains the
separate trailing `nullable` modifier. Dictionary, union, run-end-encoded, and
view types are not valid v1 param/store types. The authoring schema owns this
closed inventory and canonical printer; aliases such as `double`, `varchar`,
`array`, or SQL `timestamp` are rejected so an authored type maps to exactly one
Arrow `DataType`. Atomic types use the existing semantic `Atom` value and
parameterized types use the existing `Call` value, so `list(float64)` encodes as
`{"call":{"fn":"list","args":[{"atom":"float64"}]}}`; Arrow types add no AST
variant or interchange tag.

Struct types are recursive and preserve field order because Arrow struct field
order is physical schema. Field names are always single-quoted strings inside
the type constructor, so arbitrary Arrow field names do not enter the DSL name
namespace. Names must be non-empty and unique within one struct; an empty struct
is `struct()`. For example:

```avenger
param as pointer_state {
  type: struct(
    field('position', struct(
      field('x', float64),
      field('y', float64)
    )),
    field('labels', list(utf8))
  );
  default: NULL;
}
```

The semantic value is nested existing `Call` nodes (`struct`, `field`, `list`)
with string and `Atom` arguments. The authoring schema validates constructor
names, arity, field uniqueness, and recursion; these calls are type syntax and
do not resolve through the SQL/helper-function namespace. Canonical JSON for
the simple type `struct(field('x', float64))` is:

```json
{"call":{"fn":"struct","args":[{"call":{"fn":"field","args":["x",{"atom":"float64"}]}}]}}
```

### Typed Value Boundaries

Declared Arrow types are exact at every param, store-field, and table-param
boundary. The one ergonomic exception is a syntactic scalar literal in a
destination-typed slot: the compiler constructs it directly as the destination
Arrow scalar when the literal is representable without overflow, truncation, or
invalid parsing. Thus `default: 0;` is valid for `int32`, `float64`, or
`decimal128(10, 2)` without first becoming an `int64`; `NULL` becomes the
destination's typed null. SQL list and struct literals recurse under the same
expected type, and every struct field must match the destination's name, order,
nested type, and fixed v1 nullability.

Numeric syntax remains exact until that destination construction. The semantic
AST stores a numeric literal as its canonical decimal spelling, never as an
`f64`, and interchange JSON uses the tagged string form
`{"num":"9007199254740993"}`. Canonicalization preserves every significant
digit and the sign of negative zero; its precise spelling rules are pinned by
the parser/printer corpus. This permits exact `int64`, `uint64`,
`decimal128`/`decimal256`, and floating-point construction without an
IEEE-754 JSON round trip first.

Before the semantic AST is produced, an SQL expression consisting of exactly
one scalar literal normalizes to the corresponding scalar `Value` variant.
This includes strings, booleans, `NULL`, and signed numeric literals; a unary
sign is folded into the numeric literal. Parenthesized or cast literals and all
other expression shapes remain `Expr` values. This normalization is identical
for parsed source and decoded interchange JSON.

This contextual rule applies only to syntactic literals. A general SQL
expression — including a `$param`, function call, arithmetic result, `CASE`,
scalar subquery, or column expression — undergoes ordinary internal DataFusion
planning and must then have exactly the destination Arrow type. The assignment
or argument boundary inserts no implicit cast. Authors use an explicit SQL
`CAST` when physical types differ, even when DataFusion could normally find a
widening or comparison coercion inside an expression.

The same law governs param defaults, `set param`, store row/patch/key fields,
table-function arguments, and computed cursor expressions (`utf8`). A mismatch
known from schemas is a compile-time error; a dynamic mismatch, invalid runtime
cursor string, or failed explicit cast fails the action and aborts its
transaction. Host adapters receive the declared Arrow schema and may
ergonomically construct native scalar/list/struct inputs against it, but the
result must have the exact declared `DataType` and value shape. They may not
silently narrow, truncate, reorder struct fields, parse strings into another
type, or choose a type from the host value.

## Charts, Plots, And Subplots

One naming law, stated once (revised 2026-07-09): **"plot" names the
construct, "chart" names the document unit that wraps one, and
"subplot" is the collective term for plots in nested position.** A
*plot* is the coordinate-framed renderable atom — the engine's unit
(Rust `Plot<C>`). A *chart* wraps its root plot with the
document furnishings — `title`, `subtitle`, theme, canvas/layout and
resize policy, time/format locales, and the document's state
declarations (Rust `Chart<C>`, a forwarding builder over `Plot<C>`, so
simple charts stay one fluent chain; `compile()` takes the chart).
Nested positions keep their grammar keywords — `cell` in concat,
embedded `plot` in marks, `mark subplot` — and are collectively
*subplots* (Rust's concat wrapper is already named `Subplot`); the
schema narrows what each position may declare. "Plot area"
(`layout.plot`, event `surface: plot`) is the plot-the-unit's own frame within
the chart: a chart is its plot plus chrome.

The heading law: **`title:` heads a chart or a guide-like chrome
object** (axes, legends, facet strip headers, widgets — all already
`title:`); **`subtitle:` is chart-only**, unrepresentable on nested
units rather than merely invalid (it lives on `Chart`, not `Plot`);
**nested plot units take `label:`** — a cell caption is a smaller,
cell-local idea, not a document title. Widget *items* use `label:` as a
channel — the same word split one level down. In Rust lowering, a DSL cell
`label:` maps to `Subplot::caption(...)` (or the equivalent repeat-cell
furnishing). Rust `Subplot::label(...)` retains its older band/facet metadata
role and is deliberately not the caption path.

The wrapper lattice (the same pattern at every position — `Plot` itself
carries **no position-dependent fields**): `Chart<C>` = plot + document
furnishings; `Subplot` = plot + cell furnishings (cell name — the `as`
binder — placement `at { ... }`, `label`, cell sizing) and the
`Plot<Inner>` → `Mark<Outer>` adapter; `mark subplot` is its
own wrapper (key *expression*, `plot_size`, placement channels) around a
bare plot template. The wrap-targets encode the two-altitude law in
types: concat cells take plots (parts of one chart — subtitle stays
unrepresentable inside). Rust rename note: `Subplot`'s cell binder should be
`.name(...)`, reserving `key` for `mark subplot`'s data-driven grouping —
two concepts, currently one word.

## Core Declaration Form

The common declaration shapes are:

```avenger
chart <coord-kind> [as <name>] {
  ...
}

group [as <name>] {
  ...
}

private group as <name> {
  ...
}

public mark <mark-kind> as <name> {
  ...
}

mark <mark-kind> [as <name>] {
  ...
}

transform <transform-kind> [as <alias>] {
  ...
}

widget <built-in-widget-kind> as <name> {
  ...
}
```

The body mode depends on the declaration. `chart` and `group` bodies are mixed
container blocks with ordered child declarations. A `mark` body is mixed only
to admit its optional inline `view` child alongside ordinary mark properties.
Ordinary `transform` kinds, `param`, `scale`, `axis`, `legend`, `tool`,
`widget`, and `selection` bodies are property blocks;
an inline `view` has a mixed body containing its properties and dependent
transforms/render children; `store` has a mixed body with ordered `field`/`row` children; and the
`data:` property takes either an anonymous source block or a table-valued
`$store` binding. The core
`transform pipeline` kind is the transform exception: its mixed body contains
interface `output` declarations and ordered child transforms. The core
`tool behavior` kind is the tool exception: its mixed body contains scoped
state, events, scale edits, nested tools, chrome marks/groups, and interface
exports.

Examples:

```avenger
chart cartesian as sales_by_category {
  group as layers {
    transform aggregate as totals {
      group_by: "category";
      total: sum("amount");
    }

    mark rect as bars {
      x: "category";
      y: 0;
      y2: totals.total;
    }
  }
}
```

`as <name>` has one meaning: bind this declaration under that name. For marks
and groups in a chart, an ordinarily visible name contributes one segment to
the canonical structural path used by event targets and scene queries. Every
visible named ancestor is included; anonymous ancestors contribute no public
segment, though the compiler still assigns them private identities. For
transforms, the name is a dataflow alias used to address outputs.

Canonical structural paths are never suffix-matched in v1. Sibling structural
names must be unique, while the same leaf name may appear beneath different
parents because their full paths differ.

Public paths are aliases, not compiled mark identity. Every compiled mark has
one opaque `MarkId` independent of its source binder and may have zero, one, or
multiple public paths after hoisting and exact exports. Event and scene-query
targets resolve public paths to `MarkId`s during compilation; runtime routing
uses those resolved IDs rather than repeating string lookup. Compiled mark
metadata also retains the optional source name, `component_kind`, and exported
part aliases needed for diagnostics, decompilation, and theme matching.

Named declarations are public by default in ordinary chart groups. The
visibility modifiers are the general-purpose representation needed by inline
definition expansion:

- `private` keeps a declaration's lexical name and compiler identity but
  removes it and its descendants from the external structural namespace.
  Internal references and component-boundary `export` declarations may still
  name it.
- `public` is legal only beneath a `private` structural ancestor. It re-enters
  the public namespace at the nearest non-private named component boundary —
  a group or `tool behavior` — omitting the
  intervening private path. A public group carries its normally visible
  descendants with it. This is path hoisting, so collisions are checked at the
  re-entry point rather than only at the declaration's lexical parent.
- `export <private-path> [as <alias>];` in a group publishes one exact private
  declaration beneath that group. Exporting a group exposes the group target,
  not its descendants. The alias defaults to the source path's last segment.
  Export aliases and ordinary or hoisted public children share one namespace.
- A named group or `tool behavior` may set `component_kind: <kind>;`. This is
  opaque component provenance, not kind instantiation. Its exported mark aliases acquire
  `(component_kind, alias)` part provenance for theme matching. Expansion
  records the definition's canonical declared kind, not a use-site import
  rename; the atom remains valid even when no registry entry or import for that
  kind remains.

`private` and `public` are permitted on declarations with a named public
identity, including marks, groups, params, stores, selections, tools, and
widgets. They
do not alter dataflow visibility: transform aliases remain lexical
names governed by their dataflow scope. Both modifiers are rejected inside a
`define` body: its authored declarations are already private as a unit, and
only definition-header `export` declarations may publish them. The modifiers
spell the equivalent visibility after that definition has been inlined into an
ordinary group.

Anonymous declarations are allowed where the object does not need a public name:

```avenger
mark rule {
  x: 0;
  x2: 1;
}
```

## Block Modes

A body may contain unordered properties, ordered child declarations, or both.
The rule that keeps colon syntax meaningful: `property: value;` entries are
unordered configuration; repeated child declarations are ordered. Unordered
also means unique: a property name may appear at most once in a body — a
duplicate is a parse-time error, never a later-wins override. Container
bodies (`chart`, `cell`, `group`, and channel configuration blocks) mix both
modes:

```avenger
chart cartesian as example {
  title: 'Sales';

  data: {
    table: 'sales';
  }

  group as layers {
    transform aggregate as totals {
      group_by: "category";
      total: sum("amount");
    }

    mark rect as bars {
      x: "category";
      y: 0;
      y2: totals.total;
    }
  }
}
```

All child declarations in a body share one ordered sequence, including across
different child kinds. Transform order defines dataflow, mark/group order may
affect scene order, `cell` order defines placement, `when` order defines branch
priority, and `set` order is imperative. The formatter emits properties first
in ascending ASCII lexical order by property name, then emits every child in
its original relative order; it never groups or sorts children by keyword.
The same property rule applies to built-in, imported-definition, and dynamic
open-map properties and requires no schema or import resolution.

The parser may accept properties interleaved with children because their
relative position is not semantic. Canonicalization moves all properties ahead
of the first child while preserving the complete child subsequence exactly.
Placement laws such as “group transforms precede consuming marks and groups”
are authoring-schema validation constraints, never formatter rewrites.

Property blocks contain unordered properties:

```avenger
property: value;
property: {
  nested_property: value;
}
property: typed_value {
  nested_property: value;
}
```

Block-valued properties may be anonymous objects or typed objects. They do not
need a trailing semicolon after the closing brace.

```avenger
mark rect as box {
  x: stats.q1 {
    scale: linear {
      domain: [0, 36];
      nice: true;
    }
    axis: {
      title: 'Value';
      grid: true;
    }
  }

  fill: value '#bfdbfe';
  stroke_width: 1.5;
}
```

This rule intentionally avoids expression-only blocks in the canonical syntax.
For example, filters use `predicate: ...;` rather than a bare predicate body.

When configuration itself needs ordered repetition, use child declarations
inside the property-like block rather than inventing ordered properties:
`when` branches in a conditional channel, `level` entries in a nested band,
`cell` children in containers, `part` blocks in compound marks. If a body
needs repeated ordered *dataflow*, that belongs in an enclosing declaration
block: mark-local data preparation is modeled canonically as a surrounding
`group` with `transform` declarations, then a child `mark`.

```avenger
group as layers {
  scale_hint {
    channel: y;
    type: band;
  }

  mark symbol as points {
    x: "x";
    y: "y";
  }
}
```

## SQL Expression Slots

Expression-valued properties contain SQL expression snippets. Data columns
are double-quoted; strings are single-quoted; bare names are DSL names:

```avenger
filter: "amount" >= $min_amount and "region" = $selected_region;
x: log("amount" + 1);
visible: "amount" is not null;
label: "category" || ': ' || cast("amount" as varchar);
```

Params and stores are value bindings. Params carry scalar values and stores
carry table values; both are referenced with a named `$path`:

```avenger
param as min_amount {
  type: int64;
  default: 0;
}

store as brush {
  field id: utf8;
  field x: float64;
}

transform filter {
  predicate: "amount" >= $min_amount;
}

mark symbol {
  data: $brush;
  x: "x";
}
```

Only named `$` binding references are valid. Positional placeholders such as `$1`,
`$2`, and `?` are rejected. Params and stores occupy one collision-checked
**value-binding namespace** in each lexical scope: declaring both `param as x`
and `store as x` in the same scope is an error. A `$name` reference first resolves
the nearest binding by name, then checks that binding's value kind against the
use site. It never skips an incompatible nearer binding to find a compatible
outer one, and it never crosses a component boundary by guessing or by
concatenating names.

A qualified read of an exported binding extends the same spelling with a DSL
path:

```avenger
visible: $zoom.enabled;
filter: "amount" >= $controls.min_amount;
label: cast($panel.controls.minimum as varchar);
```

Every segment after `$` is a bare DSL identifier. The first segment resolves
lexically; crossing a component boundary requires an explicit export, and the
final target must be a `param` or `store`. `$name` remains the canonical
lexical form; `$component.alias` and deeper paths are the canonical public
forms. Quoted or numeric path segments are invalid, as are paths that resolve
to selections or other non-value state. A param is valid only in a
scalar-valued position; a store is valid only in a relation-valued position.
Those positions read the binding's current value. A schema slot that explicitly
expects a param/store reference (for example `x_domain_param: $x_domain;` or a
`slot ref` of that kind) retains binding identity instead; it uses the same
spelling and resolved `Binding` node, with dereference-versus-handle semantics
fixed by the slot schema.
For example, an inner table-valued `$data` shadows an outer scalar `$data`, and
using the inner binding in a scalar expression is a type error rather than a
request to fall back to the outer declaration.

Event expressions may qualify a param read with a temporal version:

```avenger
set param width = $width@start + event_coord(x) - start_coord(x);
set param velocity = event_coord(x) - $position@previous;
```

`$param` reads the current transaction's working value at the current routed
owner, including a mutation made by an earlier action. `$param@start` reads the
frozen value captured when the containing `between:` interaction began, resolved
at that gesture's captured start owner. `$param@previous` reads the snapshot
published after the preceding successful invocation of the same event binding,
resolved at that invocation's routed owner. A new `between:` gesture clears its
previous snapshot, so `@previous` is typed `NULL` for the first invocation of
the gesture; it is likewise typed `NULL` before a non-`between` binding has a
previous successful invocation. Rejected or failed invocations do not advance
the previous snapshot.

`@start` is valid only within an event binding that has `between:`; `@previous`
is valid only within an event binding. Both snapshots precede the current
transaction and are unaffected by its actions. Temporal qualifiers apply only
to scalar param reads: they are invalid on stores and in schema slots that retain
binding identity rather than dereference a value. The qualifier must immediately
follow the complete lexical or exported path with no intervening whitespace and
composes with native SQL after normalization:
`$controls.width@start::double`. The `@` spelling is deliberately read as
“width at start”; SQL's single-colon JSON traversal remains available, including
on a temporal value (`$config@start:theme.color`).

Bare qualified names resolve in the DSL namespace. Transform aliases expose
their output fields; reserved namespaces such as `repeat` expose placeholder
fields:

```avenger
transform bin as b {
  field: "amount";
  maxbins: 30;
}

mark rect as bins {
  x: b.start;
  x2: b.end;
}
```

All other DSL-injected references are reserved helper functions, which
tokenize as ordinary SQL calls and are rewritten by the resolver before
DataFusion planning. Channel references use `channel(...)`:

```avenger
mark rect as bars {
  x: "category";
  x2: channel(x) {
    band: 1.0;
  }
}
```

The same mechanism covers every DSL-injected reference. Helper arguments
follow the language's core rule: arguments naming DSL-space things —
channels, enum values, declared ids, exported state paths — are bare identifiers
or bare qualified paths; their helper signature supplies the expected kind.
Arguments naming data-space things — datum fields, store columns — are strings. The reserved
helper namespace:

```text
channel(x)                   reference another channel of the same mark
datum('id')                  event / row datum field
event_coord(x)               event coordinate in a channel's space
start_coord(x)               between-binding start coordinate
event_domain_start(x)        event-time scale domain start
event_domain_end(x)          event-time scale domain end
event_path()                 accumulated drag path of a between-binding
event_facet_value(0)         event facet-path component
legend_value()               legend-surface event value
selection_contains(picked, datum('id'))   selection predicate
item_channel(x)              mark-effect item channel value
item_data('label')           mark-effect source datum field
item_bbox(top)               mark-effect item bounding box
view_x(viewport, pixels)     view-ref field (also view_y)
span(lo, hi)                 construct a domain interval value
span_ordered(a, b)           domain interval with endpoints sorted
polygon(event_path())        scene-query geometry from a drag path
```

The helper names are checked against sqlparser's reserved-for-identifier
inventory. In particular, `interval(...)` is not a helper spelling: `INTERVAL`
enters SQL interval-literal parsing in the pinned parser. The token/expression
corpus reserves `EXISTS`, `INTERVAL`, `STRUCT`, and `TRIM` against future helper
names.

Reserved namespaces (`repeat.row`, `repeat.column_id`, ...) resolve the same
way as transform aliases. `$path` is the only sigil form and references a lexical
or qualified scalar/table value binding. The bare
argument form matters for definitions: channel parameters rename through
bare channel arguments during expansion, which is what lets an imported tool
be channel-generic.

## Comments

The canonical DSL comment styles are SQL-native comments:

```avenger
-- Single-line comment

/*
  Block comment
*/
```

Single-line comments require whitespace after `--` — the tokenizer's
`requires_single_line_comment_whitespace` dialect rule — so `amount--1`
continues to parse as a SQL expression rather than silently starting a
comment (`a---b` likewise stays an expression, `a - (-(-b))`; the
formatter spaces operators anyway).

Doc comments are Haddock-style `-- |` lines and attach to the next
declaration — a `define`, a `slot`, `channel`, `output`, or `export`, a
named internal mark (part documentation), a `match` arm (so completing a
mode value shows what it means), or a `chart` (gallery blurbs). Every
line of a doc block carries the `-- |` prefix — a bare `-- |` is a blank
doc line — so a block's extent is never ambiguous, and the formatter
normalizes the prefix spacing. Content is CommonMark: the first
paragraph is the summary shown in completion; the rest is detail shown on
hover and in generated documentation. Fenced `avenger` code blocks inside
doc comments are runnable examples — `avenger doc` renders them into the
generated pages as images, and `avenger test` compiles them, so a published
definition's documentation carries proven examples:

````avenger
-- | One rule per category showing the min-max span of a measure.
-- |
-- | ```avenger
-- | avenger 1;
-- | import 'error_bar.mark.avenger';
-- | chart cartesian {
-- |   data: { sql: SELECT * FROM 'examples/sales.csv'; }
-- |   mark error_bar { category: "region"; measure: "amount"; }
-- | }
-- | ```
define mark error_bar {
  -- | Grouping expression; one bar per distinct value.
  slot expr as category;
  -- | Measure whose min and max span the bar.
  slot expr as measure;
  ...
}
````

Doc comments need no token class of their own: a `-- |` line is an
ordinary line comment whose content begins with `|`. The tokenizer stays
stock — the parser recognizes the prefix, strips it, and stores the
joined block in the declaration's `doc` field — so the syntax needs no
lexical carve-out at all.

Block comments support nesting (the tokenizer's
`supports_nested_comments` dialect rule):

```avenger
/*
  Outer comment
  /*
    Inner comment
  */
*/
```

The DSL should not support `//` or `#` comments in v1. They are not exposed as
general comment-prefix hooks by `sqlparser-rs`, and they overlap with dialect
specific SQL behavior.

## Data And Params

**`data:` is a property, not a declaration** (adopted 2026-07-09; the
earlier sketches' `data as <name>` declaration is retired). Its value is
an anonymous source block — the standard anonymous block-valued property
form — set at chart, group, or mark level, and admitted by registered
data-encoded widget schemas such as `checkbox_list` and
`radio_button_list`. A chart/group/mark scope inherits its parent's data
context unless it sets its own `data:`; a widget's `data:` is instead its
explicit item relation and lowers through `WidgetItems`. Anonymous means
private, per the hygiene law, so a chart-local relation is never referenced by
name. Named, shared relations belong one level up: the catalog's
`table <kind> as` declarations, which is also the
graduation path when an inline block outgrows its chart.

```avenger
chart cartesian as sales_by_region {
  data: { table: 'sales'; }            -- reference a catalog table

  mark rect { x: "region"; y: "amount"; }
}
```

For named, file, and SQL data, reserved properties select the source — always
the block form, never a bare value:

```avenger
data: { table: 'sales'; }              -- catalog table (never a bare string)

data: {                                -- one-off derivation; `sales` is the
  sql:                                 -- ambient catalog name
    SELECT *
    FROM sales
    WHERE "amount" > 0;
}

data: { sql: SELECT * FROM 'customers.csv'; }   -- chart-owned file

param as selected_region {
  type: utf8;
  default: 'all';
}
```

`values:` is the inline-row source for small, self-contained fixtures and
examples. Its value is a non-empty array of anonymous property-only row blocks:

```avenger
data: {
  values: [
    { category: 'A'; amount: 1; active: true; },
    { amount: 2; active: false; category: 'B'; }
  ];
}
```

Every row must declare exactly the same non-empty property-name set. Row
property order is non-semantic, and canonical lexical property order determines
the Arrow column order. Values are self-contained scalar SQL literals, including
`CAST(literal AS type)` where an explicit type is needed; they cannot refer to
params, columns, subqueries, or helpers. The compiler plans the rows as one
DataFusion `VALUES` relation and uses the pinned common-type coercion across all
rows. An empty array, an all-`NULL`/otherwise ambiguous column, incompatible
values, or a missing/extra field is a compile-time error. Authors use `CAST` to
resolve an ambiguous physical type. Inline values create only the anonymous
data context; they never introduce a catalog table name.

Marks may also select a table-valued store binding as their source
(`data: $brush;` — see
[Tools, Selections, Stores, And Views](#tools-selections-stores-and-views)),
the same property in its reference form.

A store binding is also valid in a SQL relation position:

```avenger
data: {
  sql:
    SELECT *
    FROM $brush
    WHERE "x" > 0;
}
```

Before SQL parsing, token normalization replaces `$brush` with an opaque,
kind-neutral quoted identifier and records the original path and span. After
SQL parsing determines that the occurrence is a relation, name resolution
requires it to be a store and rewrites it to that store's registered relation.
A store in a scalar SQL position is rejected, just as a scalar param in `FROM`
is rejected. Once the store supplies the data context, its fields are ordinary
double-quoted columns (`"x"`), never `$brush.x`.

Store fields fix the relation schema. A successful event state transaction
atomically commits a new row snapshot and monotonically increasing revision for
each store it modified — once per store per transaction, even when several
ordered actions updated that store. Outside an event transaction, one query
evaluation reads exactly one committed revision, including all scans of that
store within the plan. Handler filters also run before their transaction and
therefore read committed store revisions.

An action-local scalar subquery instead reads a stable snapshot of the
transaction's private working stores at that action's position. It sees every
mutation made by preceding actions, all scans of one store within the action see
the same working snapshot, and it never observes a partially applied mutation
from the current action. If an action both reads and writes the same store, its
RHS reads the pre-action working snapshot; the mutation applies only after the
entire RHS succeeds. Unqualified `$store` reads use the current event's routed
owner just like ordinary `$param` reads, independently of an LHS `at start`
modifier. Shared stores resolve to root. Temporal qualifiers remain unavailable
for stores.

Transaction-local working snapshots carry an internal transaction/action
generation distinct from the store's committed revision. Action queries must
include that generation in result-cache identity or bypass the shared result
cache; they may reuse logical and physical plans but may never reuse results
from a committed revision or an earlier working generation. A commit invalidates
every dependent dataflow evaluation and any materialized result whose transitive
plan reads the store. The store's resolved identity and committed revision
participate in ordinary physical-plan evaluation-cache fingerprints, so an
unchanged logical plan may be reused but must execute against the new snapshot.
An aborted transaction publishes no working generation, changes no revision,
and triggers no invalidation. Thus ordinary chart dataflow and rendering see
only committed state until commit, while ordered action evaluation sees the
private state it is constructing.
Runtimes may coalesce invalidations after a commit, but may not serve a result
fingerprinted against an older revision.

The declared `field` children are the store's complete **author-visible
schema**. Runtime metadata used for ownership, provenance, and cache
invalidation — currently including store name, owner key, and revision — stays
below the DSL relation boundary. It is not returned by `SELECT * FROM $store`,
cannot be referenced by authored SQL or channel expressions, and does not
appear in language-server completion, documentation, or `avenger tables`.
Implementations may retain hidden metadata columns in the internal Arrow leaf
and insert a projection before exposing the relation; cache fingerprinting must
observe the metadata-bearing leaf even though authored plans see only declared
fields. The `__avenger_store_` field-name prefix is reserved so a declaration
cannot collide with that internal representation. If ownership or revision
ever becomes useful authoring data, it must gain an explicit language helper
rather than exposing physical columns.

Named tables (`table: 'sales'`, and qualified names inside `sql:`
statements) resolve from the project's data catalog or the host's
registrations — see [Data Catalogs](#data-catalogs).

The `sql` property is a special full-statement SQL slot. Its value is parsed
as one SQL statement, and the SQL semicolon terminates the property. The
compiler should use the SQL tokenizer/parser to find the statement boundary
rather than splitting naively on the first semicolon, so semicolons inside
SQL strings or comments remain valid.

Inside a full `sql:` statement, ordinary SQL identifier rules apply — bare
table and column names are normal there. The mandatory column quoting rule
governs the DSL's *expression slots*, where there is no `FROM` clause to
disambiguate.

Statement policy: only query statements are accepted — standard `SELECT`,
`FROM`-first `SELECT` (both including set operations such as `UNION`), and
`VALUES`. DDL and DML (`CREATE`, `DROP`, `INSERT`, `UPDATE`, `DELETE`, `COPY`,
...) are rejected at parse time, and multi-statement payloads are rejected.
Whether a query may touch the filesystem or network (`FROM 'file.csv'`, URLs)
is not a language question: the system interpreting the file grants or denies
those capabilities.

Language version 1 explicitly supports DuckDB-style `FROM`-first projection
ordering:

```sql
FROM vega.movies AS m
SELECT m.title, m.rating
```

It has the same meaning, logical plan, and output schema as:

```sql
SELECT m.title, m.rating
FROM vega.movies AS m
```

This ordering is particularly useful while authoring: by the time the user
types `SELECT m.`, the language server already knows the relation and alias
scope and can complete columns. The shared Avenger SQL dialect is pinned to the
`sqlparser-rs` Generic-dialect feature surface used by DataFusion, including
`supports_from_first_select`; compiler, language server, formatter, and editor
grammar must agree on it. The formatter preserves the author's standard or
`FROM`-first clause ordering. The DuckDB `FROM relation` shorthand with no
explicit `SELECT` is **not** part of v1; it remains deferred until DataFusion
planning tests establish the intended `SELECT *` semantics.

Resource declarations (`param`, `store`, `selection`, ...) bind names in
the chart scope; `data:` deliberately binds none. A future shorthand may
allow compact forms such as `param selected_region: "all";`, but the
canonical grammar should start with object bodies.

## Data Catalogs

A data file (`.data.avenger`) configures the relations available to the
project's charts. Its namespace is deliberately the same three-level hierarchy
used by DataFusion: a catalog contains schemas, and a schema contains tables.
By default the file is **ambient host configuration**: charts reference
relations by name and stay environment-independent, so swapping the data file
retargets every chart against a different environment without touching one.
Definitions can never declare or import data — a library can never smuggle a
connection.

```avenger
avenger 1;

catalog iceberg as warehouse {
  catalog_type: rest;
  uri: 'https://catalog.example.com';
  warehouse: 's3://analytics-warehouse';
  token: env 'ICEBERG_TOKEN';

  schema namespace as analytics {
    path: ['organization', 'analytics'];
  }
}

schema tables as local {
  table delta as events {
    uri: 's3://analytics/events';
  }

  table parquet as orders {
    path: 'data/orders.parquet';
  }
}

table csv as regions {
  path: 'https://example.com/regions.csv';
}
```

Names have ordinary SQL qualification:

- `regions` means the table `regions` in the session's default schema and
  default catalog. A top-level `table` is registered there.
- `local.orders` means table `orders` in schema `local` of the default
  catalog. A top-level `schema` is registered in the default catalog.
- `warehouse.analytics.orders` means table `orders` in schema (Iceberg
  namespace) `analytics` of catalog `warehouse`. A top-level `catalog` is
  registered in the session's catalog-provider list.

There is no DSL-specific reinterpretation of two-part names: in particular,
`vega.movies` is schema `vega`, table `movies`, under the default catalog. A
catalog named `vega` would require a schema as well, for example
`vega.samples.movies`.

Kinds identify the provider at the level being declared:

- `catalog schemas` is an inline catalog containing `schema` declarations;
  `schema tables` is an inline schema containing `table` declarations. Catalogs
  may contain schemas and schemas may contain tables; neither container skips
  or repeats a level.
- `catalog iceberg` connects an Iceberg catalog. `catalog_type:` selects the
  Iceberg catalog implementation, such as `rest`, `glue`, or `sql`. Its body
  contains one or more explicit `schema namespace` projections. Each projection
  gives a possibly multi-level remote namespace `path:` a local, one-level DSL
  schema alias; tables are discovered beneath that namespace. In the example,
  remote namespace `organization.analytics` is projected as local schema
  `analytics`, so a table `orders` is `warehouse.analytics.orders`.
- Explicit projection is the v1 rule even for a one-level Iceberg namespace:
  `schema namespace as analytics { path: ['analytics']; }`. This keeps catalog
  contents deterministic, supports friendly aliases, and maps Iceberg's
  namespace vector losslessly without inventing a flattening or escaping
  convention. Unlisted namespaces are not visible through that DSL catalog.
- Delta Lake is a table format, not by itself a catalog hierarchy. A directly
  addressed Delta table is therefore `table delta`, backed by delta-rs. A
  service such as Unity Catalog or Glue is modeled as its own `catalog` kind;
  the fact that a table discovered through it uses Delta is provider metadata,
  not a reason to call the catalog `delta`.
- This asymmetry is intentional: the declaration keyword names the hierarchy
  level being registered, while the kind names the adapter used at that level.
- `table parquet`/`csv`/`json` use `object_store` URIs (`s3://`, `gs://`,
  `az://`, and project-relative paths), and `table sql` is a query over the
  configured hierarchy ([Views](#views)). For the file kinds, `path:` accepts
  a single file, a directory, or a glob; directory binds lower to a DataFusion
  listing table, with hive-style partition directories (`year=2024/`) exposed
  as columns.
- **Credentials never live in the file.** Providers use their standard
  credential chains by default; the `env '<NAME>'` value form wires a
  property to an environment variable explicitly. Reading the environment
  is a capability (`--allow-env[=VAR]`), and the CLI loads a project-root
  `.env` file (gitignored by `avenger new`) before resolving them. A lint
  warns when a secret-shaped property carries a string literal.
- All `.data.avenger` files in the project load and merge (collisions at the
  same catalog, schema, or table path are errors); `--catalog <file>` restricts
  the session to specific files — the dev/prod switch.
- Inline `data:` blocks inside charts remain for chart-owned files (the
  chart-package pattern); the catalog is for shared, named, and remote
  tables.
- Catalogs give tooling real schemas: `avenger tables` lists resolved names
  and columns. The compiler and native language server register the same
  providers in isolated DataFusion analysis contexts, logically plan view and
  transform chains without executing them, and ground SQL and encoding
  completion on the resulting exact Arrow schemas.

A schema can be declared inline with `schema tables`:

```avenger
schema tables as vega {
  table json as cars   { path: 'data/cars.json'; }
  table csv  as stocks { path: 'data/stocks.csv'; }

  table sql as cars_clean {
    sql: SELECT * FROM cars WHERE "Horsepower" IS NOT NULL;
  }
}
```

Inside the schema, siblings resolve bare (`FROM cars`); outside, names are
qualified (`FROM vega.cars`). Everything a top-level `table` can do is
unchanged inside a schema — file kinds, `table sql`, params, `materialize:`,
and chains.

When a data file needs to define multiple schemas beneath one non-default
catalog, `catalog schemas` supplies the missing level explicitly:

```avenger
catalog schemas as samples {
  schema tables as vega {
    table json as movies { path: 'data/movies.json'; }
  }

  schema tables as demo {
    table csv as sales { path: 'data/sales.csv'; }
  }
}
```

These tables are `samples.vega.movies` and `samples.demo.sales`. Inline
containers can nest only in this order. A provider-backed catalog may instead
admit provider-specific schema projections such as `schema namespace`; those
children configure which remote schemas are visible and discover their tables
through the provider rather than containing `table` declarations.

### Views

`table sql as <name>` binds a name to a query over the catalog — a view
in the database sense, and the catalog-side analogue of a chart's
`data:` block: shared by every chart instead of owned by one. There is
deliberately no `view` keyword: the kind slot already says where a
relation comes from (a file, an external catalog, a query),
`materialize:` says how it is held, and a consumer writing
`FROM daily_totals` cannot tell the difference anyway:

```avenger
avenger 1;

table parquet as trips { path: 's3://taxi/trips.parquet'; }

table sql as manhattan_trips {
  sql: SELECT * FROM trips WHERE "borough" = 'Manhattan';
}

table sql as daily_totals {
  materialize: session;
  sql:
    SELECT date_trunc('day', "pickup_at") AS "day",
           count(*)                       AS "trips",
           sum("fare")                    AS "revenue"
    FROM manhattan_trips
    GROUP BY 1;
}
```

- The `sql:` slot carries the standard query-only rules and may reference
  any relation in the merged hierarchy — file-backed tables,
  provider-backed catalogs, other `table sql` entries, imported packs — regardless of
  declaration order. Query-over-query forms a DAG; cycles
  are errors. `$name` binding reads resolve only to the table's own
  declared scalar params (below) — chart params and stores never reach the catalog.
  Charts bind table params at the use site, and a bare reference always
  resolves via defaults.
- **A `table sql` is logical by default.** It lowers to a DataFusion
  `ViewTable` and inlines into every referencing plan, so chart predicates
  and projections push through it into the source — the property the
  client-server pushdown and view-dependent rasterization designs depend
  on.
- **`materialize: session;` trades inlining for reuse.** The table is
  computed lazily on first reference, held as Arrow record batches for the
  session, and keyed by its plan fingerprint — folding in source snapshot
  identity (an Iceberg snapshot id, an object-store etag) where the source
  exposes one — so a stale materialization is recomputed, never served.
  Bound param values fold into the fingerprint, so distinct bindings
  materialize independently, with growth governed by host cache policy.
  The property is not `sql`-specific: `materialize: session;` on a
  `table csv` over an https URI parses the file once per session. This is
  the declarative opt-in to the planned physical-plan evaluation cache
  (`physical-plan-evaluation-cache.md`); a persistent `disk` mode is
  anticipated there but not specced here.
- Materialization never changes results, only where and when computation
  happens — which is why a pack declaring it is safe: intent the host may
  honor, like any cache. Packs shipping a cleaned view layer over their
  raw tables is the expected pattern (`vega.cars` raw, `vega.cars_clean`
  typed).
- `table sql` entries appear in `avenger tables` and ground column
  completion exactly like file-backed tables; DataFusion derives their
  schemas from the plan without executing them.
- Schema propagation through a chain of views is DataFusion's job, not a
  parallel Avenger inference system. The project analyzer topologically plans
  the table DAG, records each logical plan's `DFSchema` (column name, physical
  Arrow type, nullability, and qualification), and registers the logical view
  for downstream planning. It never creates a physical plan or calls
  `collect()` merely to answer a schema question. The compiler retains source
  spans, stable dataset IDs, stage identities, and lineage around those
  schemas because DataFusion does not provide DSL provenance by itself.
- A view that outgrows the catalog can graduate to its own importable
  file: a data file declaring exactly one name is importable, and a
  single `table sql` qualifies — the dbt-model promotion path, with no
  new machinery.
- There is deliberately no `ref()` sigil for table references. dbt needs
  one because it templates SQL strings without parsing them; here every
  query is planned (sqlparser-rs/DataFusion), so bare names are
  semantically resolved — existence and schemas validate at compile
  time, the view DAG is derived from resolution itself, and lineage keys
  off plans. The explicit form, when wanted, is SQL's own qualification
  (`warehouse.analytics.orders`, `vega.cars`); environment
  retargeting is the catalog swap, which is `ref()`'s other job.

Params parameterize a `table sql` — the same `param` construct charts
declare, referenced the same way, lowering to the same DataFusion
placeholder. A parameterized table plans **once**, placeholders included;
bindings are applied at execution (`with_param_values`), so changing a
binding — interactively or between uses — rebinds the plan rather than
re-planning it:

```avenger
table sql as borough_trips {
  param as borough { type: utf8; default: 'Manhattan'; }
  param as min_fare { type: int64; default: 0; }

  sql:
    SELECT * FROM trips
    WHERE "borough" = $borough AND "fare" >= $min_fare;
}
```

This is the language's two-mechanism law stated once: **`param` is a
runtime value placeholder — the plan is stable and values rebind; `slot`
is expansion-time structural substitution — it shapes the plan** (columns,
function names, declaration blocks) and exists only in `define` files.
`match` is the litmus test: it branches over enum slots because selecting
structure is an expansion-time act, so a param can never drive a `match`.
Mode-like runtime variation stays inside the query as ordinary SQL over
the placeholder (`date_trunc($granularity, "pickup_at")`, `CASE WHEN`) —
varying values, never plan shape; a genuinely structural mode is a
`define transform` with an enum slot.
Catalog tables take params, never slots, and two rules diverge from chart
params because the catalog must remain a fully resolved, browsable
surface:

- **Every param carries an explicit physical Arrow `type:` and a default.** A
  bare `FROM borough_trips` is always
  valid — it binds the defaults. A query with genuinely required inputs is
  a `define transform`, which also differs in kind: it rewrites an
  upstream `input` relation mid-pipeline rather than acting as a source.
  The type fixes placeholder schema independently of the default.
- **Param values are scalar literals.** A placeholder holds a value, not
  an identifier — a table whose *columns* vary by caller is structural
  parameterization, which is `slot`/define territory.

Charts bind table params at the use site — chart params never appear
inside the catalog; their values flow in from outside. In a `data:`
block, bindings are ordinary properties:

```avenger
data: {
  table: 'borough_trips';
  borough: $selected_borough;
  min_fare: 10;
}
```

In SQL position — chart `sql:` blocks, `transform sql`, and
query-over-query inside the catalog itself — a parameterized table is
called as a table function with named arguments, which is also the only
way to use the same table twice with different bindings:

```avenger
data: {
  sql:
    SELECT 'brooklyn' AS "which", "fare"
    FROM borough_trips(borough => 'Brooklyn')
    UNION ALL
    SELECT 'queens', "fare"
    FROM borough_trips(borough => 'Queens', min_fare => 20);
}
```

Arguments are named-only (`=>`, the standard table-function spelling the
SQL parser already accepts); unbound params take their defaults; a param
name may not collide with the data block's reserved properties (`table`,
`sql`, `url`, `values`). Params and their defaults are part of the
`avenger tables` listing, and the language server completes param names
inside table-function arguments. Binding a chart param to a table param
(`borough: $selected_borough;`) is the interactive path: a param change
rebinds the shared plan and re-executes — no re-planning, which is what
keeps parameterized tables cheap under the physical-plan evaluation
cache.

Chains compose freely — a `table sql` builds on file-backed tables,
provider-backed catalogs, and other `table sql` entries alike, and a parameterized link
forwards its params to the links it calls:

```avenger
table parquet as zones { path: 'data/zones.parquet'; }

table sql as zoned_trips {
  param as borough { type: utf8; default: 'Manhattan'; }

  sql:
    SELECT t.*, z."zone_name"
    FROM borough_trips(borough => $borough) AS t
    JOIN zones AS z USING ("zone_id");
}
```

- **A table-function argument accepts exactly what a scalar binding accepts**: a
  scalar literal or a param `$binding` visible in the calling scope — the calling
  table's own params here, the chart's params in chart files. Forwarding
  composes placeholders: the inlined plan carries the outer placeholder,
  so an entire chain still plans once and rebinds at execution —
  `zoned_trips(borough => 'Queens')` reaches through to the `borough_trips`
  filter. The callee param supplies the expected Arrow type: a literal is
  constructed contextually, while a `$binding` or other nonliteral expression
  must already have that exact physical type under
  [Typed Value Boundaries](#typed-value-boundaries).
- **Materialization is the only boundary in a chain.** Logical links
  inline end to end, so a chart predicate pushes through the whole chain
  into the sources. A `materialize: session;` link computes once and holds
  batches; downstream pushdown stops there — filters apply against the
  held batches, not the original source. Fingerprints compose: a
  materialized link fingerprints its fully-inlined upstream plan, bound
  params, and source snapshot identities, so an upstream edit or a new
  binding re-materializes exactly the links it affects.
- **Chains survive distribution.** Names inside an imported data file
  resolve in that file's own namespace before the import's `as` prefix
  applies, so a pack's internal chains (`FROM cars` inside the vega pack)
  keep pointing at the pack's own tables — project-local names never
  capture them.

### Dataset Packs

Data files are also importable — the same fetch-pin machinery as every
other import. Two importers exist, for two modes of chart:

- **Data files import data files** (catalog composition): the project's data
  configuration pulls in a published pack — one line binding its catalog or
  schema.
- **Chart files may import data files** (the portable mode): a tutorial or
  example chart carries its data reference and runs anywhere. Deployed
  charts should prefer ambient tables; portability is a choice, not the
  default.

A pack is a data file organized as one `schema tables` declaration:

```avenger
-- vega-datasets@2.11.data.avenger, published on a CDN
avenger 1;

schema tables as vega {
  table json as cars   { path: 'data/cars.json'; }
  table csv  as stocks { path: 'data/stocks.csv'; }
}
```

```avenger
avenger 1;

import 'https://cdn.example.com/vega-datasets@2.11.data.avenger'
  sha256 '4c1e...';                 -- binds the pack's schema: vega

chart cartesian as cars_scatter {
  data: { table: 'vega.cars'; }

  mark symbol {
    x: "Horsepower";
    y: "Miles_per_Gallon";
    fill: "Origin";
  }
}
```

A dataset pack is nothing special: a `.data.avenger` file at a URL, hash
pinned, publishable and vendorable like any definition — the natural home
for well-known teaching data (the Vega sample datasets as `parquet`/`json`
tables over CDN URLs), and what doc-comment examples import to be runnable
in any project. Collisions between imported packs and the ambient catalog
are errors, like every other name collision.

Three rules make packs behave predictably:

- **A pack is one name.** A published data file wraps its tables in a
  single `schema tables` declaration (or wraps schemas in one `catalog
  schemas` declaration), so a data-file import binds exactly one name — the
  same rule as every other import — and `as` optionally renames that root
  (`vega` → `v`). Multi-name data files are not
  importable: they are the project's own ambient catalog, merged by the
  directory, never by imports.
- **Relative `path:` values resolve against the data file's own location**
  — a filesystem path locally, the URL base when fetched (the same
  ES-modules rule imports use) — so a pack published beside its data files
  on a CDN just works.
- **The pin covers the catalog, not the data.** `sha256` verifies the pack
  text; reading the tables it names is ordinary data access under the
  host's capability flags (`--allow-net`), same as any other source.

## Groups

`group` maps to `MarkGroup`. It is an authoring and data-preparation container.
It does not create a scenegraph group or independent render surface. Primitive
marks inside groups are flattened for rendering.

```avenger
group as manual_box_plot {
  group as summary {
    transform aggregate as stats {
      group_by: "group";
      q1: approx_percentile_cont("value", 0.25);
      median: median("value");
      q3: approx_percentile_cont("value", 0.75);
    }

    mark rect as box {
      x: stats.q1;
      x2: stats.q3;
    }
  }
}
```

Nested groups are the normal way to express branchy dataflow. A group
inherits its data context from its parent unless it sets its own `data:`.
Transform stages in a group define that group's local data context for
child marks and child groups.

To avoid hidden rewrites, the first implementation should require
group-level transform declarations to appear before child `mark` and `group`
declarations in the same group. The ordered AST still records the submitted
sequence; validation rejects a transform that appears after a consuming render
child, and formatting never moves it to make the file valid. If such
interleaving becomes useful later, it requires explicit lowering semantics
rather than an invisible formatter rewrite.

## Transforms

All transforms use:

```avenger
transform <kind> [as <alias>] {
  property: value;
}
```

For ordinary native transforms, `as <alias>` is always optional. Every stage
has an opaque compiler identity derived independently of the alias. `as`
creates only a caller-visible lexical namespace for schema-declared output
handles; it is required to write `alias.handle`, but never merely to execute the
stage. Without an alias, resulting fields remain available through their actual
quoted column names in the next data context:

```avenger
transform calculate {
  total: "price" * "quantity";
}

mark rect {
  y: "total";
}
```

Transforms with conventional outputs such as `bin`, `stack`, `kde`,
`time_unit`, and `rasterize_2d` document both their result-column names and
their handle schema. Authors may use quoted result columns anonymously or add
an alias for handle references such as `b.start`. Two aliases in the same
lexical dataflow scope may not collide; anonymous stages introduce no lexical
name.

Examples:

```avenger
transform filter {
  predicate: "value" is not null;
}

transform calculate {
  margin: "profit" / "revenue";
  label: "category" || ': ' || cast("amount" as varchar);
}

transform aggregate as totals {
  group_by: ["category", "segment"];
  total: sum("amount");
  count: count(*);
}

transform bin as b {
  field: "amount";
  maxbins: 30;
  nice: true;
}

transform stack as s {
  field: totals.total;
  group_by: "category";
  sort_by: "segment";
  offset: zero;
}
```

Diagnostics may display an alias when present, but resolution allocates an
opaque stage symbol for source maps, provenance, and internal references.
Execution and cache fingerprints derive from the resolved operation, inputs,
and configuration rather than either the symbol spelling or user alias.
Consistently renaming an alias and all its lexical references is alpha-renaming:
it does not change the resolved plan or cache fingerprint. It does change the
source AST and printed text, as any binder rename does. The opaque symbol is
never printed in DSL or exposed as an output namespace.

Transform sharing scope should also be a property, keeping the header regular:

```avenger
transform aggregate as global_totals {
  scope: shared;
  group_by: "category";
  total: sum("amount");
}

transform bin as local_bins {
  scope: level(1);
  field: "amount";
  maxbins: 20;
}
```

### The `sql` Transform

`transform sql` is the primitive escape hatch: one query statement applied to
the current data context, which is addressable inside the statement as the
reserved relation `input` (a registered table of the same name is shadowed
there). The stage's output schema is the query's schema.

`input` is positional: wherever transforms form a pipeline — a group's
transform list, a `define transform` body, or a `transform pipeline` body —
each stage's `input` is the previous stage's result, and the first stage's
`input` is the data context at that point (the group's inherited data, the
definition's instantiation site, or the container pipeline's input).

```avenger
transform sql as ranked {
  query:
    SELECT *,
           row_number() OVER (ORDER BY "amount" DESC) AS rank
    FROM input;
}
```

The `query:` statement follows the same policy as data `sql:` — standard or
`FROM`-first `SELECT` (including set operations), or `VALUES` only — and may
join registered tables (`FROM input JOIN dims ON ...`), subject to the host's
data capabilities.
Together with `define transform`
(see [Imports And Definitions](#imports-and-definitions)), this is how
custom transforms are built and shared without touching Rust.

## Channels

A mark channel is a property. Simple channels use a scaled expression, a
`value` literal, or `none`:

```avenger
x: "amount";
fill: value '#2563eb';
opacity: 0.85;
stroke: none;
```

Configured channels attach a block to the value:

```avenger
x: "amount" {
  scale: linear {
    zero: true;
    nice: true;
  }
  axis: {
    title: 'Amount';
    grid: true;
  }
}

fill: "region" {
  scale: ordinal {
    range: ['#5778a4', '#e49444', '#d1615d'];
  }
  legend: {
    title: 'Region';
    position: right;
  }
}
```

Position-channel configuration lives in the same block:

```avenger
y: "group" {
  scale: band {
    domain: ['Alpha', 'Beta', 'Gamma', 'Delta'];
  }
  axis: {
    title: 'Group';
    grid: false;
  }
  band: 0.26;
}

y2: "group" {
  band: 0.74;
}
```

Conditionals are ordered `when` child declarations inside the channel block —
first match wins — with an optional `otherwise`:

```avenger
fill: "region" {
  when {
    predicate: selection_contains(picked, datum('id'));
    value: '#2563eb';
  }
  otherwise: {
    value: '#cbd5e1';
  }
  legend: {
    title: 'Region';
  }
}
```

Branch payloads are `value: ...;` (literal) or `scaled: <sql-expr>;`
(scaled), mapping to `ConditionalValue`. Channel-level `scale`, `axis`,
`legend`, and coordination properties still apply to scaled branches.

### Scale Blocks

Scale blocks are typed property objects; unknown properties are delegated to
the scale implementation schema:

```avenger
x: "amount" {
  scale: linear {
    domain: [0, 100];
    raw_domain: $x_domain;
    nice: true;
    zero: false;
  }
  axis: {
    title: 'Amount';
    grid: true;
    tick_count: 6;
    format_number: '.2f';
  }
}

fill: "category" {
  scale: ordinal {
    domain: ['A', 'B', 'C'];
    order_by: sum("amount");
    order: desc;
    range: ['#5778a4', '#e49444', '#d1615d'];
  }
  legend: {
    title: 'Category';
    position: right;
  }
}
```

`domain` and `range` map to `ScaleConfigSpec` (`raw_domain` rides on its
`ScaleDomain`, and `order_by`/`order` lower to `ScaleOrderingSpec`); the
remaining per-type options come from the generated language schema.

### Nested Position Channels

Nested categorical positions use the `nested([...])` channel expression with
ordered `level` child declarations:

```avenger
x: nested(["region", "category"]) {
  level 0 {
    scope: shared;
    padding_inner: 0.08;
    axis: { title: 'Region'; }
  }

  level 1 {
    scope: free;
    order_by: sum("amount");
    order: desc;
    axis: { title: 'Category'; }
  }

  boundary: level_band(1, 0.5);
}
```

`scope` maps to `NestScope`; `boundary` accepts `band(<expr>)` or
`level_band(<level>, <expr>)`, matching `PositionBoundary`.

### Colorbar Overlays

Colorbar overlays are legend children hosting marks in the injected colorbar
coordinate space (no position scales or visible legends of their own):

```avenger
fill: "value" {
  scale: linear { domain: [0, 100]; }
  legend: {
    title: 'Value';

    overlay as thresholds {
      mark rule as warning {
        x: 80;
        x2: 80;
        y: 0;
        y2: 1;
        stroke: value '#111827';
        stroke_width: 2;
      }
    }
  }
}
```

## Events

Events follow the same body rule: configuration properties plus ordered
`set` actions. Actions use `=` deliberately — imperative assignment, as
distinct from declarative `:` configuration:

```avenger
on click as select_outlier {
  target: mark manual_box_plot.fence.outlier_layer.outliers;
  filter: datum('value') > $threshold;
  consume: true;

  set param selected_group = datum('group');
  set param selected_value = datum('value');
}
```

The optional `as` binder gives the event binding a stable lexical identity for
diagnostics, `@previous` snapshot ownership, source maps, inspection, and
hot-reload migration. It does not create an event value, a public target path,
or a `$` binding. Anonymous bindings receive an opaque compiler identity and
cannot be named by authored source.

Cursor changes are explicit event effects, peers of state assignments rather
than specially named params:

```avenger
on mark_mouse_enter {
  set cursor = crosshair;
}

on mark_mouse_leave {
  set cursor = default;
}
```

`cursor` has no declaration, value-binding identity, owner path, sharing mode,
revision, or readable `$` form. It cannot be exported or queried. `set cursor`
accepts a cursor-style expression; a bare registered style name is the canonical
literal form, while a computed SQL expression must return `utf8`:

```avenger
set cursor = CASE WHEN $enabled THEN 'grab' ELSE 'default' END;
```

Literal styles are checked statically against the registered `CursorStyle`
inventory; computed non-null strings are checked at runtime. `NULL` publishes
no cursor change, while `default` explicitly resets the application cursor. An
invalid non-null style fails the action and aborts its transaction.

One invocation of one event binding is a **state transaction**. Its `set`
actions execute in source order against a private working state, and every
action sees mutations made by preceding actions in that invocation. Thus a
later `$param` read observes an earlier `set param`, and a later store operation
observes rows written by an earlier `set store`, **when both references resolve
to the same concrete `(binding, owner)`**. The working state contains all owner
copies, but each read still selects its owner independently: ordinary `$binding`
uses the current event route, while LHS `at start` affects only its target.
Temporal param reads are a separate exception: `$param@start` and
`$param@previous` read their frozen pre-transaction snapshots and captured
owners regardless of earlier actions. Chart dataflow queries, materialized
results, and rendering continue to see the previously committed state until the
handler succeeds; only action-local evaluation sees the private working state.

This order is preserved in the Rust lowering as one
`Vec<ChartEventAction>` whose variants are param, store, selection, and cursor
effects. It must not lower to separate per-kind assignment arrays, because
doing so would lose cross-kind source order. State-action variants carry typed
resolved declaration IDs; the cursor variant carries no state identity.

SQL scalar subqueries are valid in action expressions and participate in the
same ordering:

```avenger
set store brush = insert_rows {
  row { id: datum('id'); }
}
set param brush_count = (SELECT count(*) FROM $brush);
```

The second action sees the row inserted by the first. Each action evaluates all
of its expressions against one stable pre-action working snapshot, then applies
its mutation as a unit. A failure in query planning, execution, conversion, or
mutation aborts the whole event transaction.

There is no v1 spelling for a live working-state read from a non-current owner.
For example, if a drag has crossed facets, neither `$width` nor `$width@start`
reads a value just written by `set param width at start = ...`: the first reads
the current owner and the second reads the frozen gesture-start snapshot. Store
update primitives routed `at start` still inspect and mutate their target
store's own pre-action working rows internally, but an RHS `FROM $store` scan
independently uses the current owner. A future cross-owner read must gain an
explicit owner-routing syntax and must not overload temporal `@start`.

Success atomically publishes all param, store, and selection changes, assigns
one new revision to each modified store, then schedules dependency invalidation
and rendering. Cursor effects are staged in the same private transaction and
publish only with a successful commit; they do not themselves trigger rendering
or assign a revision. If any action fails, the entire invocation aborts: none
of its changes, cursor effects, revisions, invalidations, or renders become
visible. Distinct event-
binding invocations are distinct transactions and commit in event-dispatch
order; no transaction implicitly joins another handler for the same event.
This transaction boundary changes observability, not action order — the
working-state semantics make that order load-bearing.

Every handler attempt reports exactly one admission outcome to the event-stream
manager:

| Outcome | Meaning |
| --- | --- |
| `Rejected` | Target/surface routing or the handler filter did not admit the event; no transaction opened. |
| `Committed` | The admitted handler completed successfully and its transaction committed, including a valid transaction with no mutations. |
| `Failed` | Filter or action evaluation failed; any opened transaction aborted. |

Only `Committed` advances the binding's previous event/param snapshot and its
throttle clock. `Rejected` and `Failed` advance neither. An event dropped by
`throttle_ms` never invokes the handler, has no admission outcome, consumes
nothing, and advances no temporal or throttle state. Because rejected attempts
do not reset the clock, the next otherwise eligible event remains immediately
eligible; throttling measures time between successful committed invocations,
not between raw events.

`consume: true` stops dispatch of the current event to later bindings only for
`Committed`. A rejected or failed binding leaves the event available to later
bindings. A successfully admitted no-op is still `Committed` and may consume:
consumption means that the binding successfully handled the event, not that it
mutated state. Bindings are attempted in deterministic registration/source
order, so commits before a consuming binding remain committed and bindings
after it are not attempted. The start and end events that merely transition a
`between:` window do not invoke the outer handler and therefore do not consume
on its behalf; their transition processing is not suppressed by the outer
handler's throttle.

The last `set cursor` action in one committed handler wins. Across committed
handlers for the same event, the last non-null cursor result in deterministic
dispatch order wins; a consuming handler truncates that order normally. If no
committed handler publishes a non-null cursor result, the application cursor is
unchanged. Cursor precedence is therefore event-local and ordered, not a global
priority system or a continuously recomputed binding.

Event routing has three orthogonal, closed v1 properties:

```avenger
target: mark manual_box_plot.fence.outlier_layer.outliers;
target: marks [manual_box_plot.summary.box, manual_box_plot.summary.median];

scope: plot;
scope: subplot detail;
scope: subplots [overview, detail];

surface: plot;
surface: all;
surface: legend fill;
```

`target:` is only a source-mark filter and accepts exactly `mark <path>` or a
non-empty, duplicate-free `marks [<path>, ...]`. Paths resolve through the same
lexical/public mark-path graph used elsewhere. Omitting `target:` accepts any
mark or background event on the chosen surface.

`scope:` is only a structural routing restriction and accepts exactly `plot`,
`subplot <path>`, or a non-empty, duplicate-free
`subplots [<path>, ...]`. `plot` means the lexically containing plot;
subplot paths resolve in that plot's public structural namespace. Omitting
`scope:` has the same containing-plot meaning; explicit `scope: plot` remains
valid but earns a redundant-default warning.

`surface:` is only an interaction-surface restriction and accepts exactly
`plot`, `all`, or `legend <channel>`. `legend <channel>` resolves the containing
plot's legend surface for that channel; an absent or ambiguous legend is a
compile-time error. Omitting `surface:` means `plot`; explicit `surface: plot`
is valid but earns a redundant-default warning. `all` admits both plot and
legend surfaces before the independent target/scope filters are applied.

The three restrictions combine with `AND`. `target: plot` and
`target: legend ...` are invalid; diagnostics suggest `scope: plot` or
`surface: plot|legend ...` respectively. An outer event binding owns `scope:`
and `surface:`. Its `between.start` and `between.end` streams inherit both and
may add their own `target:` mark filter, but may not redeclare `scope:` or
`surface:`. This maps directly to Rust's separate mark ids,
`ChartEventScopeTarget`, and `ChartEventSurfaceTarget` rather than synthesizing
one overloaded selector.

Between-event bindings can be block-valued properties:

```avenger
on cursor_moved as drag_box {
  filter: $enabled and $width@start > 0;
  between: {
    start: mouse_down {
      target: mark manual_box_plot.summary.box;
      filter: $enabled;
    }
    end: mouse_up {
      filter: $enabled;
    }
  }

  set param drag_x at start = event_coord(x);
}
```

The outer event-binding `filter:` is a **handler filter**. Target, scope, and
surface routing happen first; the filter then evaluates once against the last
committed state, before a transaction or private working state exists. Ordinary `$param`
reads use the current event's routed owner, and `@start`/`@previous` use the
frozen snapshots defined above. A scalar subquery may scan `$store`, but it sees
the routed committed store snapshot because no working state exists yet. If the
filter is false or `NULL`, no transaction opens, no action runs, and the
binding's previous-event/param snapshot does not
advance. A filter evaluation error is an invocation failure with the same
non-advancement rule.

The filters nested under `between.start` and `between.end` are **stream
filters**. They decide whether a candidate event starts or ends the interaction
window, so they run before that event changes the window. They may inspect the
candidate event and target and may read ordinary scalar `$param` bindings from
the last committed state, resolved using the candidate event's routed owner.
They may not read stores or use `@start`/`@previous`. A false or `NULL` start
filter does not open a gesture; only after every start filter succeeds does the
runtime capture the gesture's start event, routed scope, and param snapshot. A
false or `NULL` end filter leaves the gesture open. Stream-filter errors reject
the candidate event and report a runtime diagnostic without changing gesture
state.

This boundary makes dynamic tool enablement stable: `$enabled` can prevent the
mouse-down from opening a gesture at all. Stream filters never observe a handler
transaction's private state; event dispatch order means they see only commits
published by earlier successful invocations.

> **Landed Rust contract:** handler and low-level start/end filters receive the
> candidate event's routed committed params while stores and temporal param
> qualifiers remain unavailable there. Handler admission reports
> `Rejected | Committed | Failed`; only committed invocations advance temporal
> and throttle state or consume an event. `ChartEventBinding::set_cursor` is an
> explicit ordered effect staged in the same action transaction, so a later
> failure rolls it back and ordinary params never acquire cursor behavior by
> name or metadata.

An action may select the facet owner of its **left-hand target** with `at`:

```avenger
set store brush at start = replace_rows {
  row { x0: start_coord(x); x1: event_coord(x); }
}
set param drag_x at current = event_coord(x);
set selection picked at start = clear;

set store active_brush at start replacing scopes = replace_rows {
  row { x0: start_coord(x); x1: event_coord(x); }
}
```

Omitting the modifier means `at current`. `current` derives the concrete
param/store owner or selection clause scope from the event's current routed
facet instance; `start` uses the routed instance captured when the containing
`between:` interaction began. `at start` is therefore valid only in an event
binding with `between:`. For a `sharing: shared` param or store, both routes
resolve to root and an explicit `at start` earns a redundant-modifier warning.
If the selected route is absent, the adopted non-shared-write rule applies: the
action is a no-op, never an implicit root write.

`at` applies only to the target before `=`. It does not change RHS evaluation:
start-derived event values remain explicit through helpers such as
`start_coord(x)`, and a start-derived param value uses `$param@start`, while
ordinary `$param` expressions use the handler transaction's current routed
owner and working state. Thus LHS `at start` selects where to write; RHS
`@start` selects which frozen value to read, and neither implies the other.
Actions in one transaction may route different targets to different owner
paths; the transaction still commits them atomically. For selection
updates such as `clear_in_scope`, target routing via `at start` is distinct
from a payload's `scope: level(n)`, which filters clauses within that selection.

Params and stores additionally admit the LHS modifier `replacing scopes` after
the optional `at` route:

```avenger
set store brush at start replacing scopes = replace_rows { ... }
set param active at current replacing scopes = true;
```

At that action's position in the transaction, the modifier removes every
existing concrete owner instance of the target declaration, then applies the
RHS to only the owner selected by `at current|start`. Without it, other owner
instances remain unchanged. Later actions see the resulting working state, and
failure rolls back both the removals and the write. Removed param owners fall
back to the declared default; removed store owners are empty and declared
initial rows are not reseeded. The modifier is invalid for selections, whose
update kinds already distinguish all-clause and in-scope operations. It is
redundant for `sharing: shared` and earns a warning. This is the DSL spelling
of Rust's `replace_scoped_values` assignment behavior.

Event type names use the Rust event names in snake case: `mouse_down`,
`mouse_up`, `click`, `double_click`, `mouse_wheel`, `key_press`,
`key_release`, `cursor_moved`, `mark_mouse_enter`, `mark_mouse_leave`,
`window_resize`, `window_resize_settled`, `canvas_resize`,
`canvas_resize_settled`, `window_moved`, `window_focused`, and
`window_close_requested`. (The Rust enum's remaining variants stay
unexposed: interaction settling surfaces as `settle_exact:`, and the
file-watch event is host tooling.) Binding properties include `target:`,
`scope:`, `surface:`, `filter:`, `throttle_ms:`, `consume:`,
`mode: preview | exact`, and
`settle_exact:`.

Store and selection updates use the same action form with typed update
payloads:

```avenger
set store hover = clear;
set store hover = insert_rows  { row { id: datum('id'); } }
set store hover = replace_rows { row { id: datum('id'); } }
set store hover = upsert_rows  { row { id: datum('id'); x: event_coord(x); } }
set store hover = update_by_key { key { id: datum('id'); } fields { x: event_coord(x); } }
set store hover = delete_by_key { key { id: datum('id'); } }
set store hover = toggle_rows  { row { id: datum('id'); } }

set selection picked = clear;
set selection picked = clear_in_scope { scope: level(1); }
set selection picked = toggle_clauses {
  clause {
    id: datum('id');
    equality {
      dimension id { field: "id"; value: datum('id'); }
    }
  }
}
```

Update kinds mirror `StoreUpdate` and `SelectionUpdate`; `replace_all_clauses`,
`replace_clauses_in_scope`, and `upsert_clauses` follow the same shape as
`toggle_clauses`, while `delete_clauses` and `delete_clauses_in_scope` take
clause ids. Clause predicates support `equality` and `interval`
dimensions (`interval x { from: start_coord(x); to: event_coord(x); }`), and
geometry-driven selection uses the scene-query update kinds —
`replace_all_from_scene_query`, `replace_from_scene_query_in_scope`,
`upsert_from_scene_query`, and `toggle_from_scene_query`, the primitives
that make lasso and box selection definable in the language:

```avenger
set selection picked = replace_all_from_scene_query {
  geometry: polygon(event_path());
  policy: intersects;
  marks: [points];
}
```

Selection predicate evaluation follows the current Rust/DataFusion lowering.
Dimensions within one equality or interval clause combine with `AND`. Complete
clauses combine with `OR` when `combine: union` and with `AND` when
`combine: intersect`. With no clauses, `empty: all` yields true and
`empty: none` yields false; an individual equality or interval clause with zero
dimensions is always false.

An interval dimension lowers to the closed comparison
`field >= from AND field <= to`. Endpoints are not reordered: a reversed pair
normally matches nothing, and authors use `span_ordered(...)` when they want
normalization. A `NULL` equality value or either `NULL` interval endpoint makes
the complete clause false. A null data-field comparison produces SQL `NULL`,
which filtering treats as false. Facet-context comparisons are additionally
ANDed with the row predicate; null facet-context values are omitted, while a
required predicate field unavailable in the current data context makes that
clause false rather than raising a missing-column error.

Captured equality values and interval endpoints retain their evaluated Arrow
types. Their comparisons use ordinary pinned DataFusion comparison planning,
including DataFusion's compatible comparison coercions; incompatible types are
planning errors. This is comparison semantics, not a param/store assignment
boundary, so the exact-assignment rule above does not suppress those native
comparison coercions. Struct equality is supported through Arrow/DataFusion
struct comparison.

Floating-point predicates inherit Arrow's total-order comparison semantics,
not IEEE predicate intuition: `NaN = NaN` is true and NaN orders above finite
values. Closed interval comparisons involving NaN follow that same ordering.
Generic predicate clauses substitute their captured values into the saved SQL
expression, but if any captured value is `NULL`, the complete clause is false
without evaluating the expression—even when the expression could otherwise
handle null. Missing referenced data columns likewise make the generic clause
false.

Selection clauses have strict, scope-stable identity. An authored clause `id:`
is a destination-typed `utf8` expression: string literals are contextual, while
a nonliteral expression must already return `utf8` or use an explicit `CAST`.
`NULL` and the empty string are errors. The complete identity is
`(selection declaration, resolved owner path, id)`; equal ids in different
owner paths are distinct clauses.

Every clause-producing update must resolve all candidate clauses and reject a
duplicate complete identity before mutating selection state. Duplicates are
never last-write-wins replacements or sequential toggles. Scene-query updates
obey the same post-resolution uniqueness rule. Their default tuple identity is
an opaque non-empty UTF-8 id produced from a canonical, collision-free encoding
of ordered field ids and typed Arrow scalar values — never Rust `Debug` text.
An explicit scene-query `clause_id` expression follows the same exact-`utf8`
rule as an authored clause id.

The update operations are:

- `replace_all_clauses` replaces the complete selection after validating unique
  identities across its payload.
- `replace_clauses_in_scope` resolves one target owner path, removes only the
  clauses in that path, and assigns every supplied clause to that same path.
  A clause-level scope that would resolve elsewhere is a scope-mismatch error,
  not an insertion outside the replacement scope.
- `upsert_clauses` replaces the complete predicate and facet context of an
  existing identity or inserts the supplied clause when absent.
- `toggle_clauses` removes an existing identity regardless of the supplied
  predicate details, or inserts the supplied complete clause when absent.
- `delete_clauses` removes each supplied non-empty UTF-8 id from every owner
  path; `delete_clauses_in_scope` removes it only from the one resolved target
  path. Missing ids are valid no-ops.

All ids, scopes, predicates, and facet contexts are evaluated before mutation.
An invalid id, duplicate identity, predicate/type error, or scope mismatch fails
the action and aborts the enclosing transaction without retaining a prefix of
the payload. A valid no-op changes no selection revision but still belongs to a
`Committed` handler transaction.

> **Landed Rust contract:** selection state is keyed by `(owner path, id)`;
> authored and expression ids require exact non-empty UTF-8 values, scene-query
> tuples use the canonical typed encoder, and the runtime prevalidates payload
> uniqueness and in-scope ownership before applying any clause. A failure rolls
> back the enclosing action transaction.

Store primary keys and mutation conflicts have strict, order-independent
semantics. Every `primary_key` field must name a declared non-nullable field;
the ordered field tuple is the row identity and must be unique within every
committed or working store snapshot. An unkeyed store may contain duplicate
rows, but `upsert_rows`, `update_by_key`, `delete_by_key`, and `toggle_rows` are
invalid for it. `insert_rows` and `replace_rows` work for keyed and unkeyed
stores.

Every row supplied to `insert_rows`, `replace_rows`, `upsert_rows`, or
`toggle_rows` is a complete row: missing nullable fields normalize to typed
`NULL`; missing non-nullable fields, unknown fields, type mismatches, and null
key fields are errors. Literal field values are contextually constructed as the
declared field type; nonliteral field, key, and patch expressions must return
that exact physical type or contain an explicit `CAST`. A multi-row keyed
payload must contain unique key tuples
within the payload before it is applied. Duplicate payload keys are errors,
never sequential toggles or last-write-wins behavior.

The individual operations are:

- `insert_rows` appends every row. On a keyed store, conflict with either an
  existing key or another payload row is an error.
- `replace_rows` replaces the complete table and validates key uniqueness across
  the replacement.
- `upsert_rows` requires a key and replaces the complete existing row for each
  key, or appends the complete row when the key is absent.
- `update_by_key` requires a `key` block containing exactly every primary-key
  field and no others. Its non-empty `fields` block may contain only non-key
  fields. It patches the matching row or is a valid no-op when no row matches;
  primary-key changes require an explicit delete plus insert/upsert.
- `delete_by_key` uses the same exact key shape and removes the matching row, or
  is a valid no-op when no row matches.
- `toggle_rows` requires a key. An existing key is removed regardless of the
  payload's non-key values; an absent key inserts the supplied complete row.

All payload expressions are evaluated before the mutation is applied. Any
shape, schema, type, nullability, duplicate-key, or existing-key conflict fails
the action and aborts the enclosing event transaction; no prefix of a multi-row
payload is retained. A valid no-op does not modify the store and therefore does
not by itself assign a new store revision, though the handler transaction still
has the `Committed` admission outcome.

> **Landed Rust contract:** the runtime enforces declared non-nullable primary
> keys, complete normalized rows, committed and within-payload key uniqueness,
> keyed-operation requirements, exact key-block fields, and non-empty
> non-key-only patches. Multi-row keyed operations are prevalidated and obey the
> enclosing transaction's rollback rule.

Value bindings use `$path`; other state references use typed DSL paths
whenever the surrounding syntax does not already fix their kind. Paths may be
lexical names or qualified public aliases:

```avenger
controller: $zoom.domain;
highlight: selection hover.hovered;
data: $brush.points;
```

Properties whose schema already fixes the reference kind may omit the prefix,
as in `selection: hover.hovered;`; the AST still records the resolved reference
kind. Imperative actions carry the kind in their existing prefix and accept a
qualified path. These are typed l-values, not value reads, so they deliberately
do not take `$`:

```avenger
set param zoom.domain = span(0, 100);
set selection hover.hovered = clear;
set store brush.points = clear;
```

Inside the owning component, the lexical forms `set param domain`, `set
selection hovered`, and `set store points` remain canonical. A qualified path
must resolve through explicit exports at every component boundary. Params,
stores, and selections retain their kinds for validation. Params and
stores additionally share one value-binding namespace, while a component's
external aliases occupy the single collision-checked interface namespace
established above.

## Tools, Selections, Stores, And Views

The same object syntax covers interaction state. Every param declares its
physical Arrow type, and stores declare ordered `field` and `row` children:

```avenger
param as x_domain {
  type: list(float64);
  default: NULL;
  sharing: shared;
}

store as hover {
  field id: utf8;
  field x: float64 nullable;
  field y: float64 nullable;
  primary_key: [id];
  sharing: free;

  row { id: 'initial'; x: NULL; y: NULL; }
}

selection as picked {
  empty: none;
  combine: union;
}
```

`param` and `store` declarations share one value-binding namespace within each
scope. Same-name declarations of either kind conflict; nested scopes may
shadow, and `$name` always selects the nearest declaration before scalar/table
type checking. Their declaration bodies and typed mutation actions remain
distinct because scalar replacement and table-row updates have different
schemas.

`type:` is required on every chart, component, tool-owned, and catalog-table
param. It names the exact physical Arrow type carried by the DataFusion
placeholder and runtime `ScalarValue`; defaults never infer or alter it. Params
are nullable: `NULL` means the typed null of the declared Arrow type, including
for non-`NULL` defaults and first-invocation temporal reads. Defaults, host
bindings, table-function arguments, and action assignments follow the exact
[Typed Value Boundaries](#typed-value-boundaries) rule: literals are constructed
under the expected type, while nonliteral expressions require exact physical
type equality or an authored SQL `CAST`.

The DSL semantic model and `CompiledParamSpec` carry this `DataType`
explicitly. The DSL type supplies destination context while checking literals,
`NULL`, and default expressions; placeholder fields, host bindings, and runtime
assignment validation use that declared type. The ordinary Rust `Param` API is
different because its default is already a precisely typed Arrow
`ScalarValue`: `Param::new(name, default)` derives the compiled type from
`default.data_type()` rather than requiring the Rust author to repeat it. DSL
lowering must construct or evaluate the default against the separately declared
type and reject a mismatch before producing the compiled param specification.

Params have no behavioral `kind:`. Consumers impose role-specific type
constraints at the use site. `raw_domain: $x_domain` requires
`list(float64)`. Cursor is not a param role: it is a write-only transactional
event effect spelled `set cursor`, defined in [Events](#events). Removing a
consumer role does not change a param's type or identity, and a compatible param
may serve more than one consumer.

`sharing:` maps directly to Rust's `CoordinationScope` and defaults to
`shared` for both params and stores:

```avenger
sharing: shared;    -- one root-owned value across all facets
sharing: free;      -- independent value owned by each leaf facet cell
sharing: level(1);  -- value owned one logical facet level above the leaf
```

The runtime represents a concrete param or store instance by its declaration
identity plus an **owner path** derived from the active facet path. `free` is
equivalent to `level(0)` and retains the full leaf path; `level(n)` removes `n`
logical facet levels from the leaf path; `shared` resolves to the empty root
path. A level at or above the current logical depth also resolves to root, and
all modes resolve to root in an unfaceted plot. Levels are logical facet
nesting, not physical layout artifacts — internal wrap rows do not add a level.

Declaration identity is an opaque, serialized compiler ID, not the source name,
public export path, or a generated string prefix. Param, store, and selection
references resolve to typed IDs before DataFusion planning or event execution;
author-facing names and paths remain diagnostic and decompilation metadata.
Lexical component and tool scopes are a source-resolution rule rather than a
nested runtime storage layout: after resolution, their state declarations are
hoisted into the compiled chart's typed root registries. The concrete runtime
key remains `(declaration identity, owner path)`, so hoisting does not alter
facet sharing or component-instance isolation.

Reads and event writes use the same owner-path calculation. A param with no
written value at its resolved owner uses its declared default. Store initial
rows seed only the root instance; an as-yet unwritten non-root `free` or
`level(n)` store instance is empty. A non-shared event write with no routed
facet scope is a no-op rather than an implicit root write. Store revisions and
materialization keys are per concrete `(store, owner_path)` instance. For a
raw-domain param, validation additionally requires its sharing to be at least as
broad as the scale domain it controls; native tools may explicitly select a
scope or mirror the target scale's sharing.

> Before implementation, reconsider whether declared store initial rows should
> lazily seed every scoped store instance, and whether an unrouted non-shared
> write should be a runtime error instead of a no-op.

Chart/component params are predeclared within their lexical scope, so defaults
may form an acyclic forward-reference graph:

```avenger
param as upper_limit { type: int64; default: $lower_limit + 10; }
param as lower_limit { type: int64; default: 0; }
```

Defaults are evaluated once in topological order when initial state is built;
later writes to `lower_limit` do not reactively recompute `upper_limit`. Use an
ordinary SQL expression at the consumption site when a derived live value is
intended. Catalog-table params retain literal, self-contained defaults as
specified in [Data Catalogs](#data-catalogs).

Tool kinds come from two sources: native built-ins registered by the host and
imported `define tool` definitions. Both use the same declaration shape and
authoring-schema machinery, so their properties complete identically. Custom
tool definitions compose the language's event-system surface — bindings,
helpers, scale edits, store/selection updates, and marks — without implying
that every native tool must be reproducible from those constructs (see
[Imports And Definitions](#imports-and-definitions)):

```avenger
tool pan_scroll_zoom as zoom {
  x_domain_param: $x_domain;
  y_domain_param: $y_domain;
  scroll_zoom: true;
  zoom_base: 1.02;
  settle_exact: true;
}

tool box_zoom as zoom_box {
  channels: [x, y];
  min_size_px: 6;
}

tool point_selection as pick_points {
  selection: picked;
  fields: [id];
  shift_toggle: true;
  double_click_clear: true;
}
```

Other native or imported tool kinds — including `lasso_selection`,
`box_selection`, and coordinate-specific tools such as `geo_pan_zoom` — use
the same caller syntax. A custom lasso-like definition can, for example, use a
between-binding that accumulates `event_path()` and applies a scene-query
selection update; that example does not constrain how a native lasso tool is
implemented.

### Built-In Widgets

Built-in widgets are a peer declaration family:

```avenger
widget checkbox as show_trend {
  position: right;
  label: 'Show trend';
  default: true;
}

widget slider as min_fare {
  position: bottom;
  min: 0;
  max: 100;
  step: 1;
  default: 20;
}

mark rule {
  visible: $show_trend.checked;
  y: $min_fare.value;
}
```

`widget <kind> as <instance>` instantiates an opaque registered built-in. The
initial registry includes the in-tree `checkbox`, `button`, `checkbox_list`,
`radio_button_list`, `slider`, and `text_input` kinds. Its `WidgetSchema` owns
the legal parent/placement, properties, Arrow-typed generated state, public
param/store/selection exports, parts, and measurement metadata; the registry
pairs that metadata with the erased lowerer. V1
placement uses the containing chart's `top`, `right`, `bottom`, or `left`
guide slot; a kind's schema may further restrict placement. A binder is
required because widget state, parts, inspection identity, and hot-reload
migration all attach to the instance even when an individual kind exports no
state.

The DSL identifiers are authoring aliases over the completed initial Rust
widget implementation. Source uses the language-wide snake_case convention;
the landed Rust artifact kind and CSS host element retain their existing
kebab-case strings:

| DSL kind | Rust/CSS kind | Rust tier | Public state | Existing-state binding |
| --- | --- | --- | --- | --- |
| `checkbox` | `checkbox` | composed | `$instance.checked: boolean` | `checked_param: $param;` |
| `button` | `button` | composed | `$instance.activations: uint64` | `activation_param: $param;` |
| `checkbox_list` | `checkbox-list` | composed | `selection instance.selection` | `selection: selection existing;` |
| `radio_button_list` | `radio-button-list` | composed | `$instance.value: item type` | `value_param: $param;` |
| `slider` | `slider` | composed | `$instance.value: float64` | `value_param: $param;` |
| `text_input` | `text-input` | native | `$instance.value: utf8`; lazy `$instance.cursor_position: uint64` and `$instance.selected_text: utf8` | `value_param: $param;` |

The current Rust `RadioButtonList` always mints its value param; adding its
typed external-param builder/lowering seam is part of the prerequisite
`WidgetSchema` registry work, not a reason to expose an inconsistent DSL
surface. TextInput's two editing-state exports are lazy: semantic resolution
mints and publishes each only when the project references that qualified
export. Merely declaring the widget does not cause cursor/selection writes or
add those params to the compiled interface.

The initial property schemas mirror the landed builders:

- every kind requires `position: top|right|bottom|left;`;
- `checkbox` requires a nonempty `label:` and boolean `default:` and optionally binds
  `checked_param:`;
- `button` requires a nonempty `label:`, defaults `variant: neutral`, accepts
  `variant: accent`, and optionally binds `activation_param:` or declares an
  `action:` block;
- `checkbox_list` and `radio_button_list` require `data:`; `value:` and
  `label:` default to the quoted columns `"value"` and `"label"`; a non-inline
  relation additionally requires a nonempty total `order_by:` expression list;
  CheckboxList optionally binds `selection:`, while RadioButtonList may omit
  `default:` only for nonempty inline static data when `value:` is a source
  column present in the first row or a literal; relational data and computed
  value expressions require an explicit non-null `default:`;
- `slider` requires finite `min:` and `max:` with `max > min`, defaults
  `step: 1`, `default: min`, empty `title:`/`format:`, and optionally accepts
  `throttle_ms:` and `value_param:`; and
- `text_input` defaults to an empty value and placeholder,
  `commit: on_change`, and `debounce_ms: 150`, and optionally accepts
  `value_param:`. `on_enter_or_blur` is the other v1 commit policy.

For list widgets, inline `data.values` lowers to ordered `WidgetItems::Static`.
Other data sources lower to `WidgetItems::DataFrame` with the declared
`order_by` expressions as its nonempty total key. The lowerer then uses the
landed generic `Configured` pipeline to project canonical `__value` and
`__label`, derive a type-preserving item identity, validate non-null uniqueness,
and derive `__order`/`__idx`; it must not add an implicit label sort.
Those generated columns and the `__avenger_widget_` runtime-input prefix are
compiler-private. They do not enter the widget data block's author-visible
schema, expression resolution, language-server completion, or inspector table
schema; the compiler rejects source columns that collide with its reserved
names before lowering.

Button actions reuse the ordinary ordered action language without adding a
general callback surface:

```avenger
widget button as clear {
  position: right;
  label: 'Clear selection';
  action: {
    set selection picked = clear;
    set param query = '';
  }
}
```

The action runs when the Button's monotonic activation count changes and
lowers through the same serializable `ChartAction`/parameter-change reaction
seam as the landed Rust `Button::action`. It has no event datum, event helpers,
route, `between:` state, or `at start`; all referenced mutation targets must be
shared. The Rust–DSL prerequisite converts the current per-kind `ChartAction`
arrays to the language's one ordered action vector before this syntax is
implemented, so the source order above remains load-bearing and atomic.

Runtime/CSS part ids also keep their landed kebab-case spelling. Their
`PartSchema` supplies DSL aliases (`focus_ring` → `focus-ring`,
`selected_box` → `selected-box`, `selected_control` →
`selected-control`, and `value_label` → `value-label`; unchanged names map to
themselves) and records whether each part is targetable/hittable or decorative.
CSS continues to use the runtime names, for example
`checkbox-list#regions::part(selected-box)`; CSS tokens do not enter the DSL
identifier grammar.

Rust's `WidgetCell` concat hosting and explicit-frame hosting are not initial
DSL placement forms. They remain runtime capabilities and can be added later as
new schema-declared placements without changing widget kind syntax, state
exports, or the no-definition/no-expansion decision.

Schema-declared param or store exports use the ordinary qualified value-binding
form (`$show_trend.checked`, `$min_fare.value`); a selection export uses the
ordinary typed selection path. A property or `slot ref` that expects the widget
instance itself uses `widget show_trend`; this is a typed handle, not its current
value. A widget kind may expose a schema-specific
property such as `checked_param: $existing;` or `value_param: $existing;` to
bind a compatible existing declaration instead of generating state. In that
case the instance export aliases the same resolved typed identity; it does not
copy the value or create a second declaration. All type, namespace, scope, and
explicit-export rules are the same as for tools and other component state.

The DSL deliberately has no `define widget`, widget slot, or widget expansion
form. Both Rust `ChartWidget` (composed) and `NativeWidget` implementations are
registered and authored through the same opaque `widget` declaration, so their
implementation tier is neither observable nor branchable in source.
`avenger expand` leaves every widget declaration unchanged. Custom widget
implementation remains a Rust registry extension; authors who need a
language-defined interaction without widget measurement or focus behavior use
`define tool`.

### Inline Views

Views are inline lexical scopes owned by the mark or group whose dependent
transform chain they contain. They are not reusable chart resources and cannot
be declared in one place and attached with a separate `view <name>;` statement.
The optional binder names only that inline scope for `view_x` / `view_y`
helpers and diagnostics; it is not a public path, export target, or typed
reference value. The initial model permits at most one view scope per mark or
group and rejects nested view scopes.

For example, a group view contains its view-dependent transforms and render
children directly:

```avenger
group as viewed_points {
  view cartesian as viewport {
    x_domain: $x_domain;
    y_domain: $y_domain;

    transform filter {
      predicate: "amount" > 0;
    }

    mark symbol as points {
      x: "x";
      y: "y";
    }
  }
}
```

## Manual Box-Plot Composition

This manually authored example illustrates a box-plot-shaped composition: a
root group, a fence branch that feeds inliers and outliers, and a summary
branch for the box and median. It is not the specified lowering of the native
`box_plot` kind, whose implementation remains independent.

```avenger
avenger 1;

chart cartesian {
  data: {
    table: 'observations';
  }

  group as manual_box_plot {
    scale_hint {
      channel: y;
      type: band;
    }

    group as fence {
      transform join_aggregate as fence {
        group_by: "group";
        q1: approx_percentile_cont("value", 0.25);
        q3: approx_percentile_cont("value", 0.75);
      }

      group as inliers {
        transform filter {
          predicate:
            "value" >= fence.q1 - (fence.q3 - fence.q1) * 1.5
            and "value" <= fence.q3 + (fence.q3 - fence.q1) * 1.5;
        }

        transform aggregate as whisker {
          group_by: "group";
          whisker_low: min("value");
          whisker_high: max("value");
        }

        mark rule as whiskers {
          x: whisker.whisker_low;
          x2: whisker.whisker_high;
          y: "group" { band: 0.5; }
          y2: "group" { band: 0.5; }
          stroke: value '#475569';
          stroke_width: 1.5;
          zindex: 1;
        }

        mark rule as lower_cap {
          x: whisker.whisker_low;
          x2: whisker.whisker_low;
          y: "group" { band: 0.32; }
          y2: "group" { band: 0.68; }
          stroke: value '#475569';
          stroke_width: 1.5;
          zindex: 2;
        }

        mark rule as upper_cap {
          x: whisker.whisker_high;
          x2: whisker.whisker_high;
          y: "group" { band: 0.32; }
          y2: "group" { band: 0.68; }
          stroke: value '#475569';
          stroke_width: 1.5;
          zindex: 2;
        }
      }

      group as outlier_layer {
        transform filter {
          predicate:
            "value" < fence.q1 - (fence.q3 - fence.q1) * 1.5
            or "value" > fence.q3 + (fence.q3 - fence.q1) * 1.5;
        }

        mark symbol as outliers {
          x: "value";
          y: "group" { band: 0.5; }
          fill: value '#f97316';
          stroke: value '#ffffff';
          stroke_width: 1.25;
          size: 95;
          zindex: 5;
        }
      }
    }

    group as summary {
      transform aggregate as stats {
        group_by: "group";
        q1: approx_percentile_cont("value", 0.25);
        median: median("value");
        q3: approx_percentile_cont("value", 0.75);
      }

      mark rect as box {
        x: stats.q1 {
          scale: linear {
            domain: [0, 36];
          }
          axis: {
            title: 'Value';
            grid: true;
          }
        }
        x2: stats.q3;
        y: "group" {
          scale: band {
            domain: ['Alpha', 'Beta', 'Gamma', 'Delta'];
          }
          axis: {
            title: 'Group';
            grid: false;
          }
          band: 0.26;
        }
        y2: "group" { band: 0.74; }
        fill: value '#bfdbfe';
        stroke: value '#2563eb';
        stroke_width: 1.5;
        zindex: 3;
      }

      mark rule as median {
        x: stats.median;
        x2: stats.median;
        y: "group" { band: 0.24; }
        y2: "group" { band: 0.76; }
        stroke: value '#1e3a8a';
        stroke_width: 2.2;
        zindex: 4;
      }
    }
  }
}
```

The column named `group` — a reserved SQL word — is unremarkable here
because columns are always quoted, while the bare `fence.*`, `whisker.*`,
and `stats.*` references are unambiguously transform-alias outputs.

The public event paths contain every visible named ancestor:

```avenger
on click as select_outlier {
  target: mark manual_box_plot.fence.outlier_layer.outliers;
  set param selected_group = datum('group');
}
```

Thus the box is `manual_box_plot.summary.box`, while the whiskers are
`manual_box_plot.fence.inliers.whiskers`. Anonymous groups do not add a path
segment. There is no `public: true;` property and no leaf-suffix shorthand.

## Chart Chrome And Layout

Chart-level properties live in the mixed `chart` body:

```avenger
chart cartesian as sales {
  title: 'Sales by region' {
    align: center;
    span: plot;
    syntax: plain;
    font_size: 16;
    font_family: 'Inter';
  }

  subtitle: CASE WHEN $compact THEN 'Compact' ELSE 'Detailed' END {
    syntax: typst;
  }

  layout: {
    canvas: { width: 900; height: 520; }
    plot: { width: auto; height: 340; }
    margins: { left: 56; right: 20; top: 30; bottom: 46; }
    resize: { width: fixed; height: responsive; }
    debug_overlay: components;
  }

  guide: {
    plot_background_color: '#ffffff';
  }

  time: { timezone: 'UTC'; week_start: monday; }
  format: { number_locale: 'en-US'; datetime_locale: 'en-US'; }

  mark rect { x: "region"; y: "sales"; }
}
```

`layout.canvas` and `layout.plot` accept fixed width/height objects,
single-axis constraints, or `auto`; `debug_overlay` maps to
`LayoutDebugOverlayMode` (`off`, `components`, `allocation_demand`, `all`).
Non-channel color-valued properties take plain strings — `value` marks
unscaled *channel* values only.

## Composition

Concat and grid containers use ordered `cell` child declarations; cell
headers are `cell <coord> [as <id>] [at { ... }]`. Per the heading law
([Charts, Plots, And Subplots](#charts-plots-and-subplots)), a cell may
carry a `label:` caption; `title:`/`subtitle:` are chart-body-only:

```avenger
chart hconcat as overview {
  spacing: 12;
  widths: [fr(2), px(280)];
  axis_guide_visibility: outer_edges;

  cell cartesian as left {
    label: 'Totals over time';        -- cell caption (heading law: never title/subtitle)
    mark line { x: "date"; y: "total"; }
  }

  cell cartesian as right {
    mark rect { x: "category"; y: "amount"; }
  }
}
```

`vconcat` uses `heights`. Grid concat adds placement objects and per-track
sizing (`auto`, `px(n)`, `fr(n)` map to `TrackSizing`):

```avenger
chart grid_concat as overview_grid {
  rows: 2;
  columns: 2;
  spacing: { row: 12; column: 16; }
  column_widths: [fr(1), px(260)];
  row_heights: [auto, fr(1)];

  cell cartesian as overview at { row: 0; column: 0; column_span: 2; } {
    mark line { x: "date"; y: "total"; }
  }

  cell cartesian as detail at { row: 1; column: 0; } {
    mark symbol { x: "date"; y: "value"; fill: "category"; }
  }

  cell zerod as badge at { row: 1; column: 1; } {
    mark text { text: value 'Summary'; }
  }
}
```

Wrap concat chooses a column count from `columns:` (fixed) or
`responsive_columns:` (target minimum cell width):

```avenger
chart wrap_concat as small_multiples {
  responsive_columns: 220;
  spacing: 10;

  cell cartesian { mark symbol { x: "horsepower"; y: "mpg"; } }
  cell cartesian { mark rect { x: "cylinders"; y: count(*); } }
  cell polar { mark line { theta: "month"; radius: "sales"; } }
}
```

### Facets

Facet dimensions are properties with configuration blocks owning ordering,
slot sharing, guides, and empty-cell policy. `slots` accepts `free`,
`shared`, or `level(n)`; `empty_cells` accepts `hole`, `empty_subplot`, and
`auto`:

```avenger
chart facet as by_region_segment {
  row: "region" {
    title: 'Region';
    slots: shared;
    empty_cells: hole;
    order_by: sum("sales");
    order: desc;
  }

  column: "segment" {
    slots: free;
    empty_cells: empty_subplot;
  }

  cell cartesian {
    mark rect { x: "category"; y: "sales"; }
  }
}
```

Facet wrap uses `facet:` as the dimension property:

```avenger
chart facet_wrap as cars_by_origin {
  facet: "origin" {
    responsive_columns: 190;
    slots: shared;
    order_by: median("mpg");
    order: desc;
  }

  cell cartesian {
    mark symbol { x: "horsepower"; y: "mpg"; fill: "origin"; }
  }
}
```

Mark-level facet data scope is a normal mark property: `filtered` (default),
`broadcast`, or `level(n)`:

```avenger
mark rule as global_median {
  facet_data_scope: broadcast;
  x: median("value");
  x2: median("value");
}
```

### Repeat

Repeat variables are ordered child declarations whose ids become repeat
placeholder metadata; cells instantiate once per repeat context whose
optional `when:` predicate is true:

```avenger
chart repeat_grid as scatter_matrix {
  variable row as mpg { expr: "mpg"; title: 'MPG'; }
  variable row as hp { expr: "horsepower"; title: 'Horsepower'; }
  variable column as weight { expr: "weight"; title: 'Weight'; }
  variable column as accel { expr: "acceleration"; title: 'Acceleration'; }

  domain_coordination: matrix;

  cell cartesian {
    when: repeat.row_id <> repeat.column_id;
    mark symbol { x: repeat.column; y: repeat.row; fill: "origin"; }
  }

  cell zerod {
    when: repeat.row_id = repeat.column_id;
    mark text { text: repeat.row_title; }
  }
}
```

`repeat_rows`, `repeat_columns`, and `repeat_wrap` use `variable row`,
`variable column`, or `variable item`. The reserved namespace exposes
`repeat.{row,column,item}` plus `_name`, `_id`, `_title`, `_index` variants
and `repeat.cell_id`.

### Subplots

`mark subplot` is a data-driven mark with one embedded `plot` child. Grid
variants place by slot; positioned variants use coordinate channels:

```avenger
mark subplot as mini {
  key: "category";
  grid: { row: "subplot_row"; column: "subplot_col"; column_span: 2; }
  plot_size: { width: 132; height: 102; }

  plot cartesian {
    mark line { x: "time"; y: "value"; }
  }
}

mark subplot as inset {
  x: avg("x");
  y: avg("y");
  width: 140;
  height: 90;
  key: "category";

  plot cartesian {
    mark symbol { x: "local_x"; y: "local_y"; fill: "group"; }
  }
}
```

## Native And Defined Compound Marks

Compound marks also have two sources: native built-in kinds registered by the
host and imported `define mark` definitions. Both use `mark <kind>` at the
instantiation site, expose their caller surface through the authoring schema,
and may offer stable `part` blocks for styling and event targeting. Native
built-ins do not have to lower through, or be reproducible as, DSL definitions.

The built-in statistical marks are therefore available without imports:

```avenger
avenger 1;

chart cartesian as mpg_by_origin {
  data: { table: 'cars'; }

  mark box_plot as mpg_box {
    category: "origin";
    values: "mpg";
    extent: 1.5;

    part box {
      fill: "origin" { legend: none; }
      stroke: value '#1f2937';
      opacity: 0.55;
    }
    part median { stroke: value '#111827'; stroke_width: 2; }
    part whiskers { stroke: value '#374151'; }
    part caps { stroke: value '#374151'; }
    part outliers { size: 28; fill: value '#ffffff'; stroke: "origin"; }
  }

  mark violin as mpg_density {
    band_axis: y;              -- horizontal: rebind the channel parameters
    value_axis: x;
    category: "origin";
    values: "mpg";
    width_normalization: per_violin;

    part body { fill: "origin"; opacity: 0.58; }
  }
}
```

The native `box_plot` schema exposes `band_axis`, `value_axis`, `category`,
`values`, and `extent`, so vertical is the default binding and horizontal is a
rebinding. Its registered public part aliases lower to stable paths
(`mpg_box.box`, `mpg_box.median`, `mpg_box.whiskers`,
`mpg_box.lower_cap`, `mpg_box.upper_cap`, `mpg_box.outliers`). An imported
custom definition may expose an analogous surface with `channel`, `slot`, and
explicit `export` aliases, but no equivalence between that definition and the
native implementation is implied. The group construction in
[Manual Box-Plot Composition](#manual-box-plot-composition) is an illustrative
manual composition, not the required expansion of the built-in kind.

## Mark Effects And Derived Marks

Mark effects are ordered child declarations inside primitive mark blocks.
Expression adjustments assign channels through the item-frame helpers;
transform adjustments bind an output alias and route fields through `apply`:

```avenger
mark symbol as points {
  x: "displacement";
  y: "mpg";
  fill: "origin";

  adjust {
    x: item_channel(x) + 4;
    y: item_channel(y) - 2;
  }

  adjust jitter as jittered {
    axis: x;
    width_px: 18;
    seed: 7;
    apply: { x: jittered.x; }
  }

  derive text as labels {
    text: item_data('name');
    x: item_channel(x);
    y: item_bbox(top) - 4;
    zindex: 5;
  }
}
```

`adjust nudge`, `adjust jitter`, and `adjust dodge` map to the built-in
adjustment transforms. Derived marks are limited to `Symbol`, `Rule`,
`Rect`, and `Text`; derived channels evaluate against the item frame, not
the source data frame.

## Themes And CSS

A theme is a plain `.css` file — there is no theme definition kind, because
the theme system was built so CSS carries everything: custom properties
(addressable as params), `light-dark()`, media queries, subtype selectors,
and cardinality-dependent scale ranges. Composition is multiple `theme css`
declarations merging in source order (matching `Theme::append_css`), and
`theme css from` accepts relative paths, `std:themes/` files, and URLs with
the same `sha256` pinning and capability gating as imports. Theme
declarations are valid **only in chart bodies**: a definition may not
declare CSS, so a library can never inject chart-global styling — it styles
itself through its own mark properties and part defaults, and the chart
author owns the theme. CSS stays in files or string payloads — never raw
blocks — which preserves the whole-file SQL tokenization story.

Themes can target compound-mark parts with Web-Components-style part
selectors:

```css
box_plot::part(median) { stroke: #111827; stroke-width: 2px; }
box_plot::part(outliers) { fill-opacity: 0.6; }
```

The targetable surface is exactly the kind's public part surface in the
authoring schema. For a definition, that is its explicit mark `export`
aliases; for a native built-in, it is the registered part inventory. Events,
caller `part` overrides, and theme selectors therefore share one declared
surface, and definition nesting privacy applies uniformly (an internal
`error_bar`'s `bar` is `ranged_dots::part(bar)` only if exported, and
`error_bar::part(bar)` never matches it). Cascade layering makes part theming
effective: a kind's default styling on its public parts sits at the default
layer, below theme part rules, while
caller `part` overrides sit above them — chart-author explicit values >
caller part overrides > theme part rules > kind part defaults >
general theme rules.

Part selectors are a theme-engine (Rust) capability, but a generic one: the
engine parses `<kind>::part(<name>)`, compiled marks carry an opaque
`(kind, part)` provenance pair populated by lowering for public parts. A
defined-kind instance supplies the pair from its kind plus export alias; an
expanded ordinary group preserves the same information with
`component_kind:` plus its component-boundary mark exports. The matcher compares
strings. Rust never learns specific kinds — a
third-party `candlestick.mark.avenger` is themable as
`candlestick::part(wick)` with no engine changes.

An example chart body:

```avenger
chart cartesian as styled {
  theme css from 'std:themes/light.css';

  theme css from 'theme.css';

  theme css: '
    chart { font-size: 13px; color: #111827; }
    mark.symbol { fill-opacity: 0.8; }
    @media (width <= 640px) { axis label { font-size: 10px; } }
  ';

  mark symbol { x: "x"; y: "y"; }
}
```

## Pattern Fill

Pattern fill is a structured channel value with ordered `layer` children;
scaled patterns use a pattern-valued scale range (array elements may be
`none` or `pattern { ... }` objects):

```avenger
mark rect as bars {
  x: "category";
  y: "value";

  fill_pattern: pattern {
    anchor: plot;
    ink: auto_contrast { opacity: 0.22; }
    layer stripe { angle: 45; spacing_px: 12; stroke_width_px: 1.25; }
    layer stripe { angle: 135; spacing_px: 12; stroke_width_px: 1.25; }
  }
}
```

```avenger
fill_pattern: "scenario" {
  scale: ordinal {
    range: [
      none,
      pattern { layer stripe { angle: 0; spacing_px: 16; } },
      pattern { layer stripe { angle: 45; spacing_px: 12; } }
    ];
  }
  legend: { title: 'Scenario'; }
}
```

Structured pattern fields use numeric pixel and degree values; CSS units
live only inside CSS theme payloads.

## Views, Materialization, And Raster Marks

View-scoped transforms form a mark-local data context: the `view` child
declaration hosts the transforms whose results depend on it. `dim
<alias>.<field>` is a typed value lowering to `RasterDim`; `view_x` /
`view_y` helpers reference `ViewRef` fields:

```avenger
mark uniform_raster_2d as density {
  view cartesian as viewport {
    x_domain: "pickup_x";
    y_domain: "pickup_y";
    stale_policy: retarget_cached;
    throttle_ms: 16;

    transform rasterize_2d as pixels {
      x: "pickup_x";
      y: "pickup_y";
      agg: count;
      by: "passenger_count";

      x_dim: {
        extent: [view_x(viewport, domain_start), view_x(viewport, domain_end)];
        bins: view_x(viewport, pixels);
      }
      y_dim: {
        extent: [view_y(viewport, domain_start), view_y(viewport, domain_end)];
        bins: view_y(viewport, pixels);
      }
    }
  }

  raster: pixels.raster;

  x: dim pixels.x_dim { axis: { title: 'Pickup x'; } }
  y: dim pixels.y_dim { axis: { title: 'Pickup y'; } }

  fill_by: dim pixels.by_dim {
    scale: ordinal { scheme: 'tableau10'; }
    legend: { title: 'Passengers'; }
  }

  opacity_by_total: {
    scale: linear { range: [0.15, 1.0]; }
  }

  smooth: true;
}
```

The `rasterize_2d` output handle exposes `pixels.raster`, `pixels.x_dim`,
`pixels.y_dim`, and, when `by` is configured, `pixels.by_dim`. Per-pixel plane
totals are an internal input to the raster mark's `opacity_by_total` channel;
they are not added as a user-visible table column or transform output.

## Geo Coordinates And Resources

External resources are declarations; tile layers are referenced from `chart
geo` properties. `lon_lat`, `projected`, and `geometry` are the
coordinate-specific channel groups:

```avenger
resource tiles as osm {
  kind: xyz;
  url: 'https://tile.openstreetmap.org/{z}/{x}/{y}.png';
  min_zoom: 0;
  max_zoom: 19;
  attribution: 'OpenStreetMap contributors';
}

chart geo as map {
  projection: mercator;
  center_lon_lat: [-73.9857, 40.7484];
  zoom: 11;

  tiles: osm { zindex: -10; }

  tool geo_pan_zoom as map_pan {
    scroll_zoom: true;
    zoom_base: 1.02;
  }

  mark geo_shape as boroughs {
    geometry: "geom";
    fill: "borough";
    stroke: value '#ffffff';
  }

  mark symbol as stations {
    lon_lat: ["lon", "lat"];
    size: "ridership";
    fill: value '#ef4444';
  }
}
```

### Dynamic Rasterization On Map Tiles

The capstone composition — view-driven rasterization over a tiled Mercator
map — uses no dedicated syntax, only the pieces above working together. One
binding semantic makes it fit: **a `view mercator` inside a geo chart binds
to that chart's viewport** — its x/y domains are the viewport's projected
(EPSG:3857) extents, driven by `geo_pan_zoom` — so the view-scoped
`rasterize_2d` re-runs as the map pans and zooms, with bins tracking the
viewport's pixel size:

```avenger
avenger 1;

resource tiles as osm {
  kind: xyz;
  url: 'https://tile.openstreetmap.org/{z}/{x}/{y}.png';
  attribution: 'OpenStreetMap contributors';
}

chart geo as taxi_density {
  projection: mercator;
  center_lon_lat: [-73.98, 40.75];
  zoom: 11;

  data: { sql: SELECT * FROM 'data/nyc_taxi.parquet'; }

  tiles: osm { zindex: -10; }

  tool geo_pan_zoom as pan {
    scroll_zoom: true;
    settle_exact: true;
  }

  mark uniform_raster_2d as density {
    view mercator as viewport {
      stale_policy: retarget_cached;
      throttle_ms: 16;

      transform rasterize_2d as pixels {
        x: "pickup_x";                 -- EPSG:3857 columns
        y: "pickup_y";
        agg: count;
        by: "passenger_count";
        frame: 'EPSG:3857';

        x_dim: {
          extent: [view_x(viewport, domain_start), view_x(viewport, domain_end)];
          bins: view_x(viewport, pixels);
        }
        y_dim: {
          extent: [view_y(viewport, domain_start), view_y(viewport, domain_end)];
          bins: view_y(viewport, pixels);
        }
      }
    }

    raster: pixels.raster;
    x: dim pixels.x_dim;
    y: dim pixels.y_dim;

    fill_by: dim pixels.by_dim {
      scale: ordinal { scheme: 'tableau10'; }
      legend: { title: 'Passengers'; }
    }

    opacity_by_total: pixels.total {
      scale: linear { range: [0.15, 1.0]; }
    }
  }
}
```

`frame:` tags the raster's CRS so the primitive mark chooses identity
rendering (raster already in the display projection) or warping; CRS-correct
compositing under the adaptive Mercator blend is the mark's concern, not the
language's. `retarget_cached` keeps the last raster on screen — retargeted
to the moving viewport — while the exact result computes, and
`settle_exact` on the tool requests a full-quality pass when interaction
stops.

## Parallel And Zero-D Coordinates

Parallel coordinates declare frame-level dimensions; marks map them through
a `dimensions:` object whose entries accept per-dimension config blocks.
Coordinate-slot overlays host an embedded plot per dimension:

```avenger
chart parallel as cars_parallel {
  dimension as mpg { axis: { title: 'MPG'; } }
  dimension as horsepower { axis: { title: 'Horsepower'; } }
  order: [mpg, horsepower];

  mark parallel_line as lines {
    dimensions: {
      mpg: "mpg";
      horsepower: "horsepower" { scale: linear { nice: true; } }
    }
    stroke: "origin";
    opacity: 0.35;
  }

  mark parallel_axis_overlay as mpg_overlay {
    dimension: mpg;
    width_px: 80;

    plot cartesian {
      mark box_plot { values: "mpg"; }
    }
  }
}
```

Zero-dimensional charts need no special body shape; the schema restricts the
valid mark and channel set:

```avenger
chart zerod as badge {
  mark symbol as status_dot { size: 160; fill: "status"; shape: circle; }
  mark text as label { text: "status_label"; }
}
```

## Imports And Definitions

Reuse crosses files through `import` and `define`, one definition per
file: a file is either a chart or a single parameterized mark, tool,
or transform definition. A project is a collection of such files, and
other projects import its definition files. An import binds exactly one
name — the definition's declared name, or a rename via `as`.

Definitions are the language's custom extension tier, alongside native
built-in kinds registered by the host. A `define mark` builds a custom
compound mark, a `define tool` builds a custom behavior from the exposed
event-system constructs, and a `define transform` builds a custom pipeline
from built-in stages and `transform sql`. Native built-ins — including
compound marks, tools, and transforms — remain valid kinds and do not have to
lower through definitions or be expressible using the definition language.
Definitions are structural templates parameterized by slots and channel
parameters, with `match` over enum slots as the only branching form.

### Defining A Compound Mark

```avenger
-- lib/error_bar.mark.avenger
avenger 1;

define mark error_bar {
  slot expr as category;
  slot expr as measure;
  slot number as cap_width { default: 0.3; }
  export bar;
  export caps;
  export center;

  group {
    transform aggregate as stats {
      group_by: category;
      lo: min(measure);
      hi: max(measure);
      mid: avg(measure);
    }

    mark rule as bar {
      x: category { band: 0.5; }
      x2: category { band: 0.5; }
      y: stats.lo;
      y2: stats.hi;
      stroke: value '#374151';
      stroke_width: 1.5;
    }

    mark rule as caps {
      x: category { band: 0.5 - cap_width / 2; }
      x2: category { band: 0.5 + cap_width / 2; }
      y: stats.lo;
      y2: stats.lo;
      stroke: value '#374151';
    }

    mark symbol as center {
      x: category { band: 0.5; }
      y: stats.mid;
      size: 42;
      fill: value '#111827';
    }
  }
}
```

`slot <shape> as <name>` declarations are the definition's explicit property
schema. The closed v1 shapes are `expr`, `expr_list`, `literal`, `number`,
`string`, `boolean`, `enum`, `function`, `ref`, and `block`. The scalar
refinements (`number`, `string`, `boolean`) accept SQL expressions whose
resolved type matches; `literal` accepts any scalar literal without expression
evaluation. Required slots use `;`; optional slots carry `default:` in a body.
An `enum` slot declares a non-empty, duplicate-free `values:` array and any
default must be a member. A `function` slot declares one of `class: scalar;`,
`class: aggregate;`, `class: window;`, or `class: table;`. A `ref` slot's
`kind:` is one of `mark`, `group`, `param`, `selection`, `store`, `tool`,
`widget`, or `resource`. Inline views are lexical scopes, not reference values. A `block` slot accepts a
declaration body at its splice point, with optional `default:` and `exposes:`;
the splice position's authoring schema still determines which child
declarations are legal there.

A `ref` slot binds a resolved typed path, not an untyped identifier. When the
slot declaration supplies a non-value kind, callers normally use the shorter
path form (`target: points;`); the explicit typed form is available where the
surrounding schema does not fix that kind. Param/store ref slots instead use
the value-binding spelling (`state: $zoom.domain;`, `data_ref: $brush;`) and
validate the resolved binding kind against the slot's declared `kind:`.
Qualified paths must traverse public exports.

Slot references are bare identifiers inside the body — they live in the DSL
namespace like every other bare name, which is why mandatory column quoting
matters: `group_by: category;` is unambiguously the slot, while
`group_by: "category";` is the literal column. Call-site values are resolved in
the caller's lexical scope before structural substitution, and that provenance
survives expansion so definition-local aliases cannot capture caller names.
The definition body is checked against the declared shape; body usage never
infers or changes the public slot signature.

A slot default may reference earlier compatible slots:

```avenger
slot number as band_width { default: 0.6; }
slot number as cap_width { default: band_width / 2; }
```

Only textually earlier compatible slots are visible while resolving a default;
a later-slot reference is an error. The resolver still checks the dependency
graph defensively, though the earlier-only rule makes a valid cycle impossible.

### Channel Parameters

A `channel` parameter binds a *logical* channel to a physical one at the
instantiation site. This is how one definition serves both orientations
without conditionals — orientation is just a channel binding:

```avenger
define mark error_bar {
  channel band_axis: x;      -- logical channel, default binding x
  channel value_axis: y;
  slot expr as category;
  slot expr as measure;

  group {
    transform aggregate as stats {
      group_by: category;
      lo: min(measure);
      hi: max(measure);
    }

    mark rule as bar {
      band_axis: category { band: 0.5; }
      value_axis: stats.lo;
      value_axis2: stats.hi;
      stroke: value '#374151';
    }
  }
}
```

```avenger
mark error_bar as vertical_errs { category: "region"; measure: "amount"; }

mark error_bar as horizontal_errs {
  band_axis: y;
  value_axis: x;
  category: "region";
  measure: "amount";
}
```

The physical channel after `:` is the default binding. Omitting it declares a
required channel parameter: every instantiation must supply
`logical_channel: physical_channel;`, and resolution reports the missing
binding before expansion. Channel parameters never infer a physical channel
from how their logical name is used.

Expansion renames logical channels wherever channel identity appears:

- channel property names on any native or defined kind whose authoring schema
  marks that position as channel-valued, including the interval family —
  binding `value_axis: y;` maps `value_axis2` to `y2`;
- channel-enum property values (`scale_hint { channel: ...; }`,
  `scale_edit { channel: ...; }`, tool `channels:` arrays);
- bare channel arguments to reserved helpers (`event_coord(value_axis)`).

This is the entire mechanism — a declared rename, not macro splicing.
Property-name substitution is available only through `channel` parameters,
never through `slot`.

### Block Slots

A block slot receives *declarations* from the caller, spliced at a marked
position inside the definition and evaluated in the definition's data
context at that point — the mechanism that makes custom compound marks
extensible without forking them. The default content is the slot's default;
a bare `name;` statement marks the splice point:

```avenger
define mark distribution_summary {
  channel band_axis: x;
  channel value_axis: y;
  slot expr as category;
  slot expr as values;
  slot block as outlier_marks {
    default: {
      mark symbol as outliers {
        band_axis: category { band: 0.5; }
        value_axis: values;
      }
    }
  }

  group {
    -- ...fence, whisker, and summary declarations...

    group as outlier_layer {
      transform filter {
        predicate: values < fence.lo or values > fence.hi;
      }
      outlier_marks;
    }
  }
}
```

The caller renders outliers however it likes — the supplied marks see the
definition's filtered outlier rows:

```avenger
mark distribution_summary as mpg_summary {
  category: "origin";
  values: "mpg";

  outlier_marks: {
    mark text {
      band_axis: category { band: 0.5; }
      value_axis: values;
      text: datum('name');
      font_size: 9;
    }
  }
}
```

The other polarity is an empty default — a purely additive extension point.
Here callers annotate *data that exists only inside the definition* (the
weekly aggregate), and their content reorients with the instance's channel
bindings:

```avenger
define mark trend_panel {
  channel time_axis: x;
  channel value_axis: y;
  slot expr as time_col;
  slot expr as measure;
  slot block as annotations { default: { } }

  group {
    transform sql {
      query:
        SELECT date_trunc('week', time_col) AS week, avg(measure) AS avg_value
        FROM input
        GROUP BY 1;
    }

    mark line as trend {
      time_axis: "week";
      value_axis: "avg_value";
      stroke: value '#2563eb';
      stroke_width: 2;
    }

    annotations;
  }
}
```

```avenger
mark trend_panel as rev {
  time_col: "day";
  measure: "revenue";

  annotations: {
    mark rule as target {
      time_axis: min("week");
      time_axis2: max("week");
      value_axis: 120000;
      value_axis2: 120000;
      stroke: value '#dc2626';
      stroke_dash: [4, 3];
    }
  }
}
```

The caller's rule reads `"week"` — a column produced by the definition's
internal aggregation — and its `time_axis2` renames through the instance's
channel binding (family-suffix rule included), so a horizontal instantiation
reorients the annotations untouched. Caller-supplied block content remains
caller-owned: its named declarations form a structural subtree rooted directly
at the definition instance, so `rev.target` is a valid event path. Named
definition-authored ancestors around the splice point are private and do not
become segments of that caller-owned path.

Rules:

- A block slot's `default:` content is definition-authored and resolves in the
  definition's lexical scope. Caller-supplied replacement content resolves in
  a layered lexical environment. The innermost
  read-only layer contains the instance's bound slots and channel parameters;
  those names intentionally shadow same-named bare DSL names in the caller,
  with an editor warning on a collision. The remaining caller lexical scope
  stays visible, and data columns come from the splice point's data context.
- By default, block content cannot reach any definition-internal alias. A
  block slot may hand over specific internal handles with its `exposes:`
  property — the definition declares exactly what crosses the boundary, and
  nothing else leaks (Vue's scoped-slot props are the precedent):

  ```avenger
  slot block as annotations {
    exposes: [fence, stats];
    default: { }
  }
  ```

  Content passed to that slot may reference `fence.lo` or `stats.median`;
  content passed to a slot without `exposes` may not reference any internal
  alias.
- A block slot has exactly one splice point. Passing an empty block
  (`outlier_marks: { }`) removes the default structure; an empty default
  (`slot block as annotations { default: { } }`) makes the slot purely
  additive.
- Public aliases exported by the definition and top-level names contributed by
  caller block content share the instance's external namespace. A collision is
  an error at the instantiation site. Descendants within caller content follow
  ordinary full-path rules, so a caller-provided `group as labels` containing
  `mark text as value` is addressed as `<instance>.labels.value`.
  When expansion places caller content beneath a private definition-authored
  ancestor, it prints `public` on each top-level caller declaration. That
  hoists the caller-owned subtree back to the instance boundary without
  exposing the surrounding implementation structure.
- Guidance: `match` is for closed structural modes the definition owns;
  a block slot is for an open extension point the caller owns. The two
  compose.

### The Caller Surface: Slots Plus Parts

Instantiating a defined mark uses the ordinary mark declaration form:
properties bind slots, and `part` blocks style the definition's public named
marks. Native compound marks may expose an analogous part surface through
their registered schema, but the two implementations are independent:

```avenger
avenger 1;

import 'lib/error_bar.mark.avenger';

chart cartesian as sales_errors {
  data: { table: 'sales'; }

  mark error_bar as errs {
    category: "region";
    measure: "amount";
    zindex: 3;

    part bar { stroke: value '#dc2626'; stroke_width: 2; }
    part center { fill: value '#dc2626'; }
  }
}
```

- Properties bind slots by name; a missing slot without a default is an
  error, and an unknown property is an error — except the generic mark
  properties (`zindex`, `visible`, `facet_data_scope`), which apply to the
  expansion root.
- `part <name> { ... }` targets the mark explicitly exported under `<name>`; its
  properties merge over the definition's, use site winning. Parts cover
  open-ended styling so definitions do not need a slot per styleable
  property. The same public-part surface serves theme part selectors
  (`box_plot::part(median)`; see [Themes And CSS](#themes-and-css)) and
  event targets — one declared surface, three consumers.
- Definition bodies are private by default, including declarations at the
  definition's own level. When a definition instantiates another definition
  internally, the inner instance's parts and state are likewise not reachable
  from outside. The outer definition publishes a stable external alias with
  `export`, optionally renaming (the Web Components `exportparts` rule):

  ```avenger
  define mark ranged_dots {
    slot expr as category;
    slot expr as measure;

    export inner.bar as bar;

    mark error_bar as inner { category: category; measure: measure; }
  }
  ```

  Callers then style `part bar { ... }` and target `<instance>.bar` in
  events; `inner.caps` remains private. `export` covers marks, groups, params,
  stores, selections, and other targetable state. The source path is resolved
  inside the definition, but only the alias is part of its public contract.
  The alias defaults to the source path's final segment, source paths may
  forward-reference body declarations, and aliases must be unique within the
  definition interface. Exporting a group exposes that group as a target; it
  does not implicitly publish the group's private descendants.
- `export` has the same exact-alias semantics in an ordinary group. During
  expansion, definition-header exports become group-local exports whose source
  paths point at the printed private declarations. This is what makes the
  expanded result editable without changing its public interface.
- Internal `as` names still expand hygienically, but do not become public paths.
  If `error_bar` explicitly exports internal marks as `bar`, `caps`, and
  `center`, an instance named `errs` exposes `errs.bar`, `errs.caps`, and
  `errs.center`; restructuring the private tree does not change those paths.
- The expansion inherits the instantiation site's data context and
  participates in the chart's scales like any inline group; definitions may
  not set `data:`.

### Defining Tools And Behaviors

The native core `tool behavior` kind is the non-macro behavioral container and
the canonical result of inlining a tool definition. It is also available
directly to authors:

```avenger
tool behavior as hover {
  component_kind: hover_highlight;
  export hovered;

  private selection as hovered {
    empty: none;
  }

  on mark_mouse_enter {
    target: mark points;
    set selection hovered = replace_all_clauses {
      clause {
        equality {
          dimension id { field: "id"; value: datum('id'); }
        }
      }
    }
  }
}
```

`tool behavior` occupies one tool position and requires `as <instance>`. It is
a lexical and runtime ownership scope, not a data, coordinate, layout, or
rendering group. Params, stores, selections, event bindings, nested tools,
scale edits, and optional chrome marks/groups inside it are owned by that
instance. Internal declarations may be marked `private`, exact group-style
`export` declarations publish state or target aliases beneath the instance,
and export aliases collide with public children in one interface namespace.
Chrome marks inherit the data and coordinate context at the behavior's
instantiation point unless they set their own ordinary properties; the
behavior container itself creates no new context.

Native and defined tools lower to one resolved Rust
`ToolBehaviorExpansion`: an opaque tool-instance ID, typed generated-state
declarations, ordered event bindings, containing-plot scale edits, chrome
marks/groups, exact exports, and component/part metadata. Nested tools expand
recursively into that representation. Generated state identity derives from
the tool-instance ID rather than source-name prefixes; public exports are
aliases to the same underlying typed IDs.

A `scale_edit` inside a behavior targets the containing plot's active scale for
its declared channel by default, regardless of any chrome-group nesting. This
is the only v1 target and may be spelled `target: plot;` explicitly. View
scopes are inline transform/render scopes rather than reusable scale owners, so
they are not valid scale-edit targets. This keeps scale mutation attached to
the tool's use site rather than accidentally retargeting it to its visual
chrome.

A custom tool definition composes the following language-level expansion
content, all instance-scoped:

- `param as` / `store as` / `selection as` declarations (generated state);
- `on` event bindings;
- `scale_edit { channel: ...; ... }` declarations, which apply scale
  configuration in the instantiating chart's scope — this is what lets a
  zoom tool bind its own raw-domain params without the caller wiring
  `raw_domain:` by hand;
- ordinary marks, typically store-backed (`data: $brush;`) for
  drag-rectangle and lasso chrome.

A from-scratch tool built only from these primitives:

```avenger
define tool drag_pan {
  channel axis: x;
  slot enum as button {
    values: [left, middle, right];
    default: left;
  }

  param as domain { type: list(float64); default: NULL; }

  scale_edit {
    channel: axis;
    raw_domain: $domain;
  }

  on cursor_moved {
    between: {
      start: mouse_down { button: button; }
      end: mouse_up;
    }

    set param domain = span(
      event_domain_start(axis) - (event_coord(axis) - start_coord(axis)),
      event_domain_end(axis) - (event_coord(axis) - start_coord(axis))
    );
  }
}
```

`tool drag_pan as pan_x;` pans x; `tool drag_pan as pan_y { axis: y; }` pans y
— the channel parameter renames through the scale edit and the bare helper arguments
alike. Its behavior is exactly the behavior declared here; it makes no parity
claim with a native pan/zoom kind. Geometry-driven custom tools can use the
event system's scene queries:
a lasso-like definition can combine a between-binding accumulating
`event_path()` with a scene-query selection update
(`set selection picked = replace_all_from_scene_query { ... }`). This is an
example of the custom surface, not a required implementation of the native
`lasso_selection` kind. Definitions may also wrap native kinds or imported
definitions, preconfiguring them through slots:

```avenger
-- lib/wheel_zoom.tool.avenger
avenger 1;

define tool wheel_zoom {
  slot number as base { default: 1.05; }

  param as x_domain { type: list(float64); default: NULL; }
  param as y_domain { type: list(float64); default: NULL; }

  tool pan_scroll_zoom {
    x_domain_param: $x_domain;
    y_domain_param: $y_domain;
    scroll_zoom: true;
    zoom_base: base;
  }
}
```

```avenger
-- lib/hover_highlight.tool.avenger
avenger 1;

define tool hover_highlight {
  slot ref as target { kind: mark; }
  export hovered;

  selection as hovered {
    empty: none;
  }

  on mark_mouse_enter {
    target: mark target;
    set selection hovered = replace_all_clauses {
      clause {
        equality {
          dimension id { field: "id"; value: datum('id'); }
        }
      }
    }
  }

  on mark_mouse_leave {
    target: mark target;
    set selection hovered = clear;
  }
}
```

```avenger
avenger 1;

import 'lib/wheel_zoom.tool.avenger';
import 'lib/hover_highlight.tool.avenger';

chart cartesian as explorer {
  data: { table: 'cars'; }

  tool wheel_zoom as zoom;
  tool hover_highlight as hover { target: points; }

  mark symbol as points {
    x: "horsepower";
    y: "mpg";
    fill: "origin" {
      when {
        predicate: selection_contains(hover.hovered, datum('id'));
        value: '#dc2626';
      }
      otherwise: { scaled: "origin"; }
    }
  }
}
```

Params, stores, and selections declared inside a definition expand under the
instance namespace, so two instances of the same tool never collide, but they
remain private unless exported. The explicit `export hovered;` above creates
the public `hover.hovered` alias. A `ref` slot accepts only the declared reference kind
(`target: points;` above); typed expression slots accept caller expressions,
including compatible scalar `$binding` reads and data columns. There is no
untyped "any property value" slot.

Every defined-tool instantiation requires an explicit `as` binder, whether or
not that definition currently declares state. The binder is its stable
expansion, event-ownership, diagnostics, and hot-reload identity; adding state
to a later library version therefore cannot silently change the call-site
contract. Native tool schemas declare whether they are stateless. Only a native
tool registered as stateless may use the anonymous `tool <kind>;` form; native
tools with generated or externally targetable state require `as` for the same
reason as defined tools.

### Defining Transforms

A transform definition is a named, slotted pipeline of transform stages —
built-ins, other imported transform definitions, and `transform sql` stages.
`output` declarations are its public handle schema: the fields an
instantiation alias exposes, each mapping to a column of the pipeline's
result (bare when the column has the same name, explicit when re-exporting
an internal stage's field).

A defined-transform instantiation requires `as <alias>` exactly when its
definition declares one or more public `output` handles. An output-free
definition may be instantiated anonymously. This requirement is independent
of whether the caller happens to reference an output: the definition's public
interface and its expanded source shape must remain stable.

The native core `transform pipeline` kind is the corresponding non-macro
container and the canonical result of inlining a transform definition. It is
also available directly to authors:

```avenger
transform pipeline as shares {
  output share: calc.share;

  transform sql as calc {
    query:
      SELECT *,
        "amount" / sum("amount") OVER () AS share
      FROM input;
  }
}
```

A pipeline occupies exactly one transform position in its parent dataflow.
Its child transforms run in source order: the first receives the pipeline's
input relation, each later child receives its predecessor's result as `input`,
and the final child relation is the pipeline's output relation passed to the
next parent stage or mark. Nested pipelines follow the same rule. A pipeline
with no child transform is an error.

Rust lowering preserves this boundary with one serializable
`CompiledPipelineTransform` inside the parent's `DataTransformStage`; it does
not flatten the child stages into the parent `DataContext`. The wrapper applies
its internal `Vec<DataTransformStage>` sequentially, reports their referenced
tables, and owns the pipeline's external cache/materialization identity.
Output handles are compile-time interface metadata rather than persisted child
alias namespaces.

Child stage aliases are lexical to the pipeline and never escape it. `output`
declarations create the only public handles on the pipeline binder; the source
may be a final-result column or an internal alias field whose lineage survives
in the final result. Every declared output is validated against the final
schema. A pipeline containing outputs must have an `as` binder, output names
must be unique, and downstream expressions use `<binder>.<output>` exactly as
they do for a transform definition. Outputs do not project or rename the
relation by themselves; they name fields already present in the final result.
An instantiation-level `scope:` is retained on the pipeline and supplies the
default scope for child stages that do not set their own.

An output-free `transform pipeline` may omit `as`; it still occupies one parent
stage and has an opaque compiler identity. A pipeline with any `output`
declaration requires a binder, as shown above.

```avenger
-- lib/share_within.transform.avenger
avenger 1;

define transform share_within {
  slot expr as measure;
  slot expr_list as partition_keys;
  output share;

  transform sql {
    query:
      SELECT *, measure / sum(measure) OVER (PARTITION BY partition_keys) AS share
      FROM input;
  }
}
```

```avenger
-- lib/binned_counts.transform.avenger
avenger 1;

define transform binned_counts {
  slot expr as field;
  slot number as maxbins { default: 30; }
  output start: b.start;
  output end: b.end;
  output count;

  transform bin as b {
    field: field;
    maxbins: maxbins;
  }

  transform aggregate {
    group_by: [b.start, b.end];
    count: count(*);
  }
}
```

```avenger
avenger 1;

import 'lib/share_within.transform.avenger';

chart cartesian as region_shares {
  data: { table: 'sales'; }

  group as shares {
    transform share_within as s {
      measure: "amount";
      partition_keys: ["region", "year"];
    }

    mark rect {
      x: "region";
      y: s.share;
      fill: "year";
    }
  }
}
```

Slot substitution inside `query:` statements follows the declared shape.
`expr` slots splice one expression into the statement AST before planning;
`expr_list` slots splice a comma-separated expression list in list positions
such as `PARTITION BY`, `GROUP BY`, a `SELECT` list, or `IN (...)`.
Everything else in a statement follows ordinary SQL rules. The resolver warns
when a slot name shadows a column of the incoming context; rename the slot or
quote the column.

On a defined-transform instantiation, `scope:` becomes the expanded
pipeline's `scope:` property and sets the coordination scope for every child
stage that does not declare its own. Output declarations are validated against
the pipeline's actual final schema at compile time, and they are what the alias
namespace and editor completion expose (`s.share` above).

Slot names must parse as identifiers inside statements, so SQL reserved
words (`order`, `end`, `group`) cannot name slots; the resolver rejects them
with a rename suggestion.

A `function` slot binds a function name from the declared registry class and
splices it only in call position — the construct that keeps open sets of
aggregations from becoming one `match` arm per function:

```avenger
define transform rolling {
  slot expr as measure;
  slot expr as order_key;
  slot function as agg {
    class: aggregate;
    default: avg;
  }
  slot number as preceding { default: 6; }
  output rolled;

  transform sql {
    query:
      SELECT *,
        agg(measure) OVER (
          ORDER BY order_key
          ROWS BETWEEN preceding PRECEDING AND CURRENT ROW
        ) AS rolled
      FROM input;
  }
}
```

`transform rolling as r { measure: "sales"; order_key: "date"; agg: median; }`
validates `median` as an aggregate function at instantiation; binding a
non-function, or a function from the wrong class, is an error.

### Modes: `match` Over An Enum Slot

Some transforms need different logic per mode. When the modes differ only in
*expressions*, no construct is needed: a mode slot splices as a literal, so a
SQL `CASE WHEN mode = 'center' ...` constant-folds at planning time. When
the modes differ in *stages*, `match` selects among closed variants at
expansion time. An `enum` slot declares its entire domain; `match` must contain
exactly one arm for every declared value, with no extra or default arm.
Editors complete mode values from the slot's `values:` list and use the arm
docs for per-value detail.

`simple_stack` is an illustrative custom SQL pipeline — one shared windowing
stage, then a per-mode finishing stage. It is intentionally not the native
`stack` transform and carries no compatibility claim with it:

```avenger
define transform simple_stack {
  slot expr as measure;
  slot expr_list as partition_keys;
  slot expr as order_key;
  slot enum as mode {
    values: [zero, center, normalize];
    default: zero;
  }
  output start;
  output end;

  transform sql {
    query:
      SELECT *,
        sum(measure) OVER (
          PARTITION BY partition_keys ORDER BY order_key
        ) AS __simple_stack_end,
        sum(measure) OVER (PARTITION BY partition_keys) AS __simple_stack_total
      FROM input;
  }

  match mode {
    zero {
      transform sql {
        query:
          SELECT *,
            __simple_stack_end - measure AS start,
            __simple_stack_end AS "end"
          FROM input;
      }
    }
    center {
      transform sql {
        query:
          SELECT *,
            __simple_stack_end - measure - __simple_stack_total / 2 AS start,
            __simple_stack_end - __simple_stack_total / 2 AS "end"
          FROM input;
      }
    }
    normalize {
      transform sql {
        query:
          SELECT *,
            (__simple_stack_end - measure) / __simple_stack_total AS start,
            __simple_stack_end / __simple_stack_total AS "end"
          FROM input;
      }
    }
  }
}
```

```avenger
transform simple_stack as s {
  measure: "amount";
  partition_keys: ["category"];
  order_key: "segment";
  mode: center;
}

mark rect { x: "category"; y: s.start; y2: s.end; fill: "segment"; }
```

The arms use the shared base through ordinary pipeline chaining: `match`
splices the selected arm's stages in place, so the expanded pipeline is
exactly *base stage → arm stage*, and the arm's `FROM input` reads the base
stage's result — `__simple_stack_end` and `__simple_stack_total` are ordinary
columns of its input relation. A stage written after the `match` block would consume
the arm's output the same way. Instantiating with `mode: center;` and
`measure: "amount"` expands to nothing more than:

```avenger
transform pipeline as s {
  output start;
  output end;

  transform sql {
    query:
      SELECT *,
        sum("amount") OVER (
          PARTITION BY "category" ORDER BY "segment"
        ) AS __simple_stack_end,
        sum("amount") OVER (PARTITION BY "category") AS __simple_stack_total
      FROM input;
  }

  transform sql {
    query:
      SELECT *,
        __simple_stack_end - "amount" - __simple_stack_total / 2 AS start,
        __simple_stack_end - __simple_stack_total / 2 AS "end"
      FROM input;
  }
}
```

The wrapper is semantically one parent stage, so the following mark still
resolves `s.start` and `s.end`; emitting the two SQL stages as siblings would
lose the definition instance's output-handle namespace.

`match` is compile-time selection, distinct from runtime `when` branches on
channels: exactly one arm's declarations splice into the body during
expansion. Intermediate columns use a `__<define>_` prefix by convention to
avoid colliding with input columns, and expansion qualifies them by
*instance* id (`__s1_simple_stack_end`), so instantiating the same definition
twice in one pipeline cannot collide. At a defined-transform or explicit
`transform pipeline` boundary, the compiler automatically projects the final
relation to the input fields plus the columns named by its `output`
declarations. Newly created undeclared columns are implementation-local and do
not appear in the following data context, schema completion, lineage exports,
or `SELECT *` outside the boundary. A plain `transform sql` in an ordinary
group has no such component boundary and exposes its query schema normally.
There is therefore no author-facing `except:` property for hiding generated
intermediates.

`match` works in every definition kind and at any depth within one — inside
a group, a mark body, or an event body — with each arm contributing whatever
items are valid at that position: transform stages, marks, event bindings,
actions, scale edits, or properties. An empty arm splices nothing, which is
how optional structure is spelled. A mark definition selecting structure
(sketch):

```avenger
define mark distribution_summary {
  channel band_axis: x;
  channel value_axis: y;
  slot expr as category;
  slot expr as values;
  slot enum as outliers {
    values: [show, hide];
    default: show;
  }

  group {
    -- ...fence, whisker, and summary declarations as in the low-level example...

    match outliers {
      show {
        group as outlier_layer {
          transform filter {
            predicate: values < fence.lo or values > fence.hi;
          }
          mark symbol as outliers {
            band_axis: category { band: 0.5; }
            value_axis: values;
          }
        }
      }
      hide { }
    }
  }
}
```

A tool definition selecting just the action inside a shared event binding:

```avenger
define tool click_picker {
  slot ref as target { kind: mark; }
  slot ref as sel { kind: selection; }
  slot enum as mode {
    values: [toggle, replace];
    default: toggle;
  }

  on click {
    target: mark target;

    match mode {
      toggle {
        set selection sel = toggle_clauses {
          clause { equality { dimension id { field: "id"; value: datum('id'); } } }
        }
      }
      replace {
        set selection sel = replace_all_clauses {
          clause { equality { dimension id { field: "id"; value: datum('id'); } } }
        }
      }
    }
  }
}
```

Multiple `match` blocks over different slots compose in one definition, and
`match` composes with channel parameters — the custom distribution-summary
sketch above is orientation-generic and outlier-optional at once.

This keeps the conditional budget of the language explicit: `channel`
parameters rename, `match` selects among declared closed variants, SQL
`CASE` handles per-row and constant-foldable expression logic — and nothing
else branches.

### The Standard Library

The host may bundle reusable DSL-authored definitions through the `std:`
scheme — one definition per file, imported individually, with no privileged
semantics beyond distribution. This definition library is separate from the
native built-in registry: built-in marks, tools, and transforms require no
import, while `std:` supplies optional custom compositions and examples.
There is no requirement that a native built-in have a corresponding standard
definition or that the two be behaviorally equivalent:

```avenger
avenger 1;

import 'std:marks/error_bar';
import 'std:marks/trend_panel';
import 'std:tools/hover_highlight';
import 'std:transforms/share_within';
```

- `std:marks/`, `std:tools/`, and `std:transforms/` hold bundled definitions
  that are useful as reusable compositions, examples, or starting points.
  Their exact inventory is not the native built-in inventory.
  `std:transforms/` definitions are pipelines over built-in stages and
  `transform sql`; `std:themes/` holds plain CSS files
  (`light.css`, `dark.css`, ...) referenced with
  `theme css from 'std:themes/dark.css';` rather than imported.
- Definition-library imports are explicit and per-definition; there is no
  implicit prelude.
  Decompilation emits the imports it needs.
- `std:` resolution is host-provided (bundled resources, no filesystem
  capability required) and versioned with the language: `avenger 1` pins
  stdlib 1.
- Standard definitions use snake_case kind names (`error_bar`, not
  `ErrorBar`), matching every other kind family.
- Decompilation preserves native built-in kinds as native kinds. Explicit
  definition expansion applies only to imported definitions; it does not
  synthesize a DSL implementation for a native built-in.

### What Definitions Deliberately Cannot Do

The extension constructs are bounded by design, and the rejected shapes are
recorded so the boundary holds under pressure:

- **No iteration over slot lists.** Repeated structure per data element is
  data-driven multiplicity: `fold` the fields into rows and write one mark
  (a dumbbell plot is a fold plus one rule and one symbol), or use `repeat`
  and `facet`. A loop construct would duplicate what the data layer does
  better.
- **No definition-valued slots.** A slot cannot receive a transform or mark
  *definition* (higher-order macros); block slots receive concrete
  declarations, and composition happens by importing and instantiating.
- **No multi-relation transform outputs.** A transform definition produces
  one data context; branching consumption belongs to sibling groups at the
  mark level.
- **No runtime dispatch in definitions.** `match` resolves at expansion;
  runtime behavior variation is expressed with params and event-binding
  `filter:` predicates (a two-click gesture is two bindings filtered on a
  state param).
- **No slot-derived identifiers** outside the two declared mechanisms
  (channel parameters and typed `function` slots). Output column names, mark
  names, and property names are never assembled from slot values.

### Project Layout

A project is a directory of files — there is no manifest. Two rules are
normative; everything else is convention.

- **All relative resources resolve against the declaring file**: imports,
  `theme css from` paths, and data file paths inside SQL alike. A chart at
  `charts/regional/chart.avenger` reading `FROM 'regions.parquet'` means its
  sibling file, regardless of the working directory the host compiles from
  (the host rebases data paths before execution). Only charts are affected —
  definitions declare neither data nor themes.
- **The project root is the default capability boundary.** The root is
  whatever the host is given (or discovers, such as the VCS root). By
  default a chart may read files within the project; absolute paths, paths
  escaping the root, and URLs require explicit host grants.

The recommended convention, which scales down to a flat directory for small
projects:

```text
sales-analytics/                  -- project root
  charts/
    revenue_trend.avenger
    revenue_trend.png             -- blessed baseline: sibling of its chart
    executive_overview.avenger
    executive_overview.png
    regional/                     -- a chart that owns its data
      chart.avenger
      chart.png
      regions.parquet
  marks/
    trend_panel.mark.avenger
    error_bar.mark.avenger
  tools/
    wheel_zoom.tool.avenger
  transforms/
    share_within.transform.avenger
  themes/
    corporate.css
  catalog.data.avenger            -- named tables: iceberg/delta/files (see Data Catalogs)
  .env                            -- credentials, gitignored
  data/                           -- shared data files
    orders.parquet
```

Directory names carry no semantics — imports are explicit paths — so
by-kind directories, per-chart packages (a subdirectory holding one chart
plus its co-located data, the natural unit to archive or share), and flat
layouts are all equally valid. Definition files under `marks/`, `tools/`,
and `transforms/` are the project's importable surface for other projects.

Every chart file doubles as an example: its blessed baseline is a sibling
`.png` (the spec+image pair), which makes charts simultaneously the
project's test suite (`avenger test`) and its documentation gallery
(`avenger doc`) — and lets git forges show the rendered image next to the
source.

### Distribution And The Ecosystem

Import paths take three source forms:

| Form | Example | Resolution |
| --- | --- | --- |
| Standard definition library | `import 'std:marks/error_bar';` | bundled with the language, versioned by the pragma |
| Relative path | `import 'lib/trend_panel.mark.avenger';` | files in the project, relative to the importer |
| URL | `import 'https://charts.example.dev/error_bars@1.2.0.avenger';` | fetched once, hash-verified, cached |

The distribution model is deliberately lightweight, built on one structural
fact: **there is no diamond-dependency problem.** Nothing crosses a
definition boundary except the language-level channel vocabulary and
explicitly exported, instance-namespaced names, so two copies of a library
at different versions expand to independent structures that cannot collide.
Duplicates are harmless; therefore vendoring is safe; therefore no version
resolver needs to exist.

Rules:

- **Imports are uniform.** Every file — chart, local library, or fetched
  library — may import `std:`, relative paths, and URLs. A file behaves
  identically however it is obtained. Relative imports resolve against the
  importer's location: a filesystem path locally, the URL base when fetched
  (the ES-modules rule), so multi-file libraries work from raw repository
  links.
- **There is nothing to resolve, only a closure to fetch.** URL imports name
  exact files — no version ranges exist — so transitive dependencies
  degenerate to recursively fetching a closure of pinned files: fetch,
  verify, recurse, reject cycles. No resolver, no version solving. Because
  duplicates are harmless, deep graphs cannot conflict; the worst case is
  redundant bytes.
- **Integrity is inline; there is no lockfile.** A URL import carries its
  content hash in the import itself:

  ```avenger
  import 'https://charts.example.dev/error_bars@1.2.0.avenger'
    sha256 '9f2ab34c...' as eb;
  ```

  Because imports are uniform, a published library's own URL imports carry
  their hashes too, so pinning the root import transitively pins the entire
  closure — a Merkle tree. This also carries the *author's* tested closure
  inside the artifact, which a consumer-side lockfile cannot do. Identity
  is the hash, so any mirror is equivalent (mirror lists are host
  configuration), the fetch cache is content-addressed, and an upgrade is a
  source diff showing the new URL and hash together. Dev mode permits
  hashless URL imports with a warning; `avenger pin` fetches and writes
  hashes into source; locked mode (CI, publishing, untrusted specs)
  requires the full closure to be pinned and refuses mismatches. Whether
  network access is permitted at all remains the host's capability
  decision, like every other resource.

  Pin requirements are a mode, not a file property: dev mode permits
  hashless imports everywhere (own and transitive), locked mode requires
  the whole closure. The warnings differ, though — an unpinned import in
  your own file has a one-keystroke fix, while an unpinned import inside a
  *fetched* file cannot be fixed in place (vendor it or get upstream to
  pin). In practice the authoring flow makes hashless imports transient:
  pasting a URL into an import triggers fetch-verify-and-pin in the editor
  (inserting the `sha256` and offering the definition's name for `as`), and
  `avenger add <url>` does the same from the command line.

  Pinning edits source under strict guarantees: compilation, fetching, and
  rendering never mutate source — `pin` runs only as an invoked command or
  an accepted editor action; the edit is one token added or updated on the
  import line, deterministic and idempotent; and `avenger pin --print`
  emits the pinned lines without writing for paste-preferring workflows.
  (Precedent: Nix's inline fetcher hashes with source-rewriting updaters,
  pip-compile's generated `--hash` lines, HTML Subresource Integrity, and
  manifest-editing commands like `go get` and `cargo add`.) Inline pinning
  also improves trust-on-first-use review: the URL and hash change together
  in a one-line diff where reviews actually happen, rather than in lockfile
  churn.
- **Flattening is a courtesy, not a requirement.** With one definition per
  file there is nothing to collect — `avenger expand` (see below) is the
  only flattening, and unexpanded sources at raw URLs are fully importable.
- **Versioning is convention.** `@1.2.0` in a filename or URL is naming, not
  mechanism; the language checks only the `avenger` major version of the
  imported file. Authors who want upgrades re-point the URL and re-pin.
- **Publishing is putting files somewhere.** There is no registry, account,
  or publish step; a raw-file URL on any host (including a Git forge at a
  pinned tag) is a published library. Copying a definition into your own
  file with an attribution comment remains a first-class alternative —
  harmless by the no-diamond property, reviewable because definitions are
  small declarative text.

### Expansion

One definition per file eliminates collection-style bundling outright — a
multi-definition file cannot exist, so there is no inlining operation and
none of the linker work it would need (reconciling kind names across
origins). Distribution is the closure of single-definition files, each
hash-pinned. The one flattening that remains is the one with no
name-reconciliation problem:

`avenger expand` is **source-level inline-definition expansion**: every imported definition
instantiation is replaced by its expansion — slots substituted, `match` arms
resolved, channel parameters renamed, block slots spliced, part overrides
merged. Native built-in marks, tools, and transforms remain native declarations;
expansion never attempts to reconstruct them in the definition language.
Built-in widgets likewise remain opaque `widget` declarations; there are no
widget definitions or widget-expansion products. Each
defined-mark instance is emitted as an ordinary group named by the instance
id. The group records the original kind as opaque `component_kind:`
provenance. Definition-authored declarations print as `private`; resolved
definition exports print as group-local `export` aliases; and caller-authored
block-slot declarations print normally, or as `public` when they must cross a
private ancestor. Thus a public path such as `mpg_box.box` survives even if
its private source mark was nested more deeply, without exposing that private
tree. A defined-transform instance becomes one native `transform pipeline`
stage with its resolved `output` declarations and sequential child stages;
this preserves both the parent dataflow position and handles such as
`s.share`. A definition with outputs preserves its required alias on that
pipeline; an output-free anonymous definition expands to an anonymous
output-free pipeline. A defined-tool instance becomes one native `tool behavior` with the
same required instance binder, `component_kind` provenance, private owned
state and bindings, resolved exports, containing-plot scale edits, and chrome
marks left in place. Instance-scoped params, stores, and selections keep their
authored lexical names in expanded source; inline view binders remain local to
their owning view bodies. State references inside the component remain lexical,
and group/behavior exports provide the only external qualified aliases. The compiler assigns opaque unique symbol ids after
resolution, but those ids are never printed as DSL names and cannot collide
with caller-authored identifiers.
The output contains no imports needed solely for expanded definitions —
`std:` definition imports included — and no `define`, `slot`, `channel`,
`match`, or `exposes` constructs from those expansions. Resolved `export`
declarations remain because they are ordinary group interface declarations,
and resolved `output` declarations remain because they are ordinary
`transform pipeline` interface declarations; neither is macro machinery.
Resolved exports on `tool behavior` remain for the same reason. Native
built-in declarations and semantic resource imports (such as dataset packs)
remain. `component_kind: error_bar;` on the instance group and `export ... as
bar;` together preserve `error_bar::part(bar)` after the definition import is
gone. The equivalence property below forces this to be right, since a themed
chart whose expansion lost either fact would compile differently.
Expansion has no name-reconciliation problem at all: the definition
namespace — where any collection operation's conflicts would live — is
deleted, and every remaining name is instance-scoped by construction
(mark groups and tool behaviors retain lexical instance boundaries, exported
state qualifies through public aliases, and generated intermediate columns use
opaque resolved identities), so two versions of the same library expand side by side
without contact. This is the archival form (it freezes
rendering semantics against imported-definition evolution), the debugging
form (what a chart actually lowers to), and the minimal-runtime form (a
host can execute it with the definition machinery entirely absent).

For example, an IDE's **inline definition** action may produce this ordinary,
hand-editable group (irrelevant mark details abbreviated):

```avenger
group as errs {
  component_kind: error_bar;
  export body.bar as bar;
  export body.center as center;

  private group as body {
    mark rule as bar {
      x: "category";
      y: "lo";
      y2: "hi";
    }
    mark symbol as center {
      x: "category";
      y: "mid";
    }

    public mark text as annotation {
      x: "category";
      y: "hi";
      text: value 'high';
    }
  }
}
```

The private marks retain internal identities `errs.body.bar` and
`errs.body.center`, but those are not public event paths. Consumers see the
exact aliases `errs.bar` and `errs.center`, plus the hoisted caller-owned path
`errs.annotation`. `component_kind` makes the exported marks continue to match
`error_bar::part(bar)` and `error_bar::part(center)`.
Expansion preserves declaration order, nesting, and data contexts. It attaches
visibility modifiers to declarations in place; it must not invent an extra
data-bearing group merely to create privacy, because that could change
transform scope, scale contribution, or layout semantics.

Runtime constructs survive expansion untouched — params, conditional `when`
branches, event bindings, views, and `transform sql` stages are semantics,
not macros. Expansion is semantics-preserving by construction, and the
guarantee is testable: compiling a chart and compiling its expansion must
produce identical results (see
[Coverage And Validation](#coverage-and-validation)).

Expansion also surfaces as language-server code actions at finer
granularity — expand one instantiation at the cursor, or extract a
selection into a new definition file (see
[Language Server](#language-server)).

What this trades away, consciously: automatic dedup and one-command upgrade
propagation across an ecosystem — the benefits a version resolver buys, at
the cost of being one. A future `pkg:` scheme can layer naming and
discovery over the same fetch-pin-cache mechanism without changing the
language.

### Name Binding And Declaration Order

Name binding is category-based rather than uniformly textual. Entering a
lexical scope performs a predeclaration pass for identities whose existence is
independent of execution order: named marks, groups, params, stores,
selections, tools, resources, and events. Their complete bindings are
therefore available throughout that scope, including before their textual
declaration. Duplicate bindings are diagnosed during predeclaration before any
body is resolved. Params and stores share one value-binding namespace, so the
same scope cannot declare both `param as state` and `store as state`; the other
typed namespaces and public-interface collision rules determine remaining
conflicts.

An inline view binder is established only while resolving that view's body and
is available to its `view_x` / `view_y` helpers. It does not enter the parent
scope's predeclared resource namespace.

A nested scope may shadow an outer value binding. `$name` resolves the nearest
binding by name before checking whether the use requires a scalar param or a
table store; kind-directed fallback to an outer declaration is forbidden.

Predeclaration permits forward references in event targets, typed references,
qualified value-binding paths, tool target properties, and exact `export`
declarations. Private declarations are predeclared internally even though they
do not enter the public interface. A forward reference changes neither child
execution/render order nor source identity.

Transform aliases and data columns are deliberately different:

- A transform alias is not visible in its own body and becomes visible only
  after that stage in the containing dataflow scope. Later transforms and
  render children may use it; earlier siblings may not.
- Each stage's result columns become the next stage's current data context.
  Quoted columns and alias handles therefore follow the same sequential
  dataflow boundary.
- Nested pipelines create nested sequential alias scopes; their child aliases
  never escape the pipeline.
- `output` declarations in a transform definition or `transform pipeline` are
  interface declarations resolved after the complete body. They may
  forward-reference an internal stage alias, but the referenced lineage must
  survive in the final relation. This exception does not make that alias
  available to earlier executable stages.

Catalog relation declarations are predeclared as one merged relation namespace
regardless of source order. After SQL name resolution, dependencies among
`table sql` declarations must form a DAG; self-reference and multi-table cycles
are errors reported with the dependency path.

Value bindings in a lexical scope are likewise predeclared before param defaults
are resolved. Defaults may reference another compatible scalar param in that scope,
including one declared later. Default dependencies must be acyclic and are
evaluated in topological order when initial state is constructed; a default is
initialization, not a reactive binding after construction. Catalog-table params
retain their stricter existing contract: every default is a self-contained
scalar literal checked against the separately required Arrow `type:`, so
table-param defaults have no dependency graph.

Definition slot defaults intentionally retain the simpler textual rule: a slot
default may reference only an earlier compatible slot. Later-slot references
are errors even when they would be acyclic. This makes definition interfaces
read top-to-bottom and avoids a second dependency scheduler in macro expansion.

### Hygiene And Resolution Rules

- Expansion is purely structural: no loops, recursion, or string
  templating, and the only branching is `match` over an enum slot, resolved
  at expansion time. Definitions may not define other definitions. A
  definition body may instantiate native built-in kinds and other imported
  definitions; acyclic imports keep expansion finite.
- **Full value-binding hygiene**: `$name` inside a definition body may reference
  only params or stores declared within that same definition. External state arrives
  through an explicitly compatible typed slot — for example, a caller may
  bind `$zoom_enabled` to a `boolean` expression slot or a `ref` slot whose
  kind is `param` or `store`. After instantiation or source expansion, lexical `$name`
  continues to resolve inside the component boundary; caller code reads an
  explicitly exported value binding with `$instance.alias`. A library file
  can never silently depend on chart state.
- **No styling side effects**: `theme css` declarations are valid only in
  chart bodies. A definition styles itself through its own mark properties
  and part defaults and can never inject chart-global CSS.
- **Two-tier publicity**: within a chart, every ordinarily visible named
  structural declaration is public at its full canonical path, including every
  visible named ancestor; anonymous and explicitly `private` declarations are
  not. A `public` declaration may re-enter at the nearest non-private named
  component boundary (group or `tool behavior`) only when nested beneath a
  private ancestor. Boundary-local exact exports and public children share one
  collision-checked interface. Across a
  definition boundary, every definition-authored declaration is private unless
  explicitly published by `export`, whose alias creates the stable
  `<instance>.<alias>` interface. Caller-authored block-slot content retains
  caller-owned structural names beneath the instance; expansion uses `public`
  only where private definition structure would otherwise hide it. Block
  content sees the caller scope, read-only instance slot/channel bindings,
  splice-point data columns, and only the internal handles declared by
  `exposes`.
- **Native kind names are reserved within their kind namespace.** An import
  that would bind the name of a native mark, tool, or transform must use `as`
  to choose a distinct name. Imported definitions never silently shadow
  native kinds, which keeps parsing, decompilation, and expansion
  deterministic.
- `import 'path';` binds exactly one name — **the file stem** (filename-
  as-name, adopted 2026-07-09; a matching in-file binder is optional and
  validated); `import 'path' as eb2;` renames it, which is also how two
  same-named items from different sources coexist, and is *required* when
  the stem is not a valid bare identifier. (A data file is importable only
  when it declares exactly one name — in practice a `schema tables` or
  `catalog schemas` root; data declarations keep their declared names; see
  [Dataset Packs](#dataset-packs).) Two imports binding the same name are
  an error at the import lines. Only the file's single item is importable;
  there is no transitive re-export.
- Import paths resolve relative to the importing file (filesystem path
  locally, URL base when fetched), and the imported file's `avenger` major
  version must match. Source forms, closure fetching, and hash pinning are
  specified in
  [Distribution And The Ecosystem](#distribution-and-the-ecosystem);
  filesystem and network access remain host capability decisions, as with
  data sources.

### Diagnostics And Tooling

Errors inside expanded definitions report both locations, macro-backtrace
style:

```text
error: scale 'band' requires a discrete domain
  --> lib/stat_marks.avenger:18  (mark rule as bar, in define mark error_bar)
  instantiated at chart.avenger:9  (mark error_bar as errs)
```

The language server loads imported definitions through the same schema
machinery as built-ins: slots complete as properties with their defaults,
export aliases complete inside `part` blocks and event targets, and a
defined kind is indistinguishable from a native one in the editor.

## Grammar

The complete surface grammar in EBNF. Terminals come from the shared SQL
tokenizer; the token classes the DSL relies on are specified normatively and
pinned by a conformance corpus:

```text
ident     unquoted word          DSL names: kinds, properties, enum values,
                                 aliases, namespaces, helper functions
column    "double quoted"        data column reference (SQL identifier)
string    'single quoted'        string literal
number    SQL numeric literal
binding   $ident { . ident }
          [ temporal ]            lexical/exported value read; temporal suffix is param-only
temporal  @start | @previous      one adjacent Generic-dialect word token
punct     { } [ ] ( ) : ; , = .
```

Comments (`--` followed by whitespace; nested `/* ... */`) and whitespace are
trivia, except that `-- |` doc lines are captured into the adjacent
declaration's `doc` field rather than dropped. `sql_expr` and `sql_query` are
islands parsed by sqlparser's `Parser` under `AvengerSqlDialect` from the
normalized token stream; the resolver then
rewrites value-binding paths, bare qualified names, and reserved helper
functions.

```ebnf
file          = version , { import } , ( chart | define | data_file ) ;
data_file     = data_bind , { data_bind } ;
                     (* .data.avenger: catalog config. Data files import
                        only data files; chart files may import data files
                        (portable mode); definition files may not. *)
data_bind     = catalog_bind | schema_bind | table_bind ;
catalog_bind  = "catalog" , ident , bind , body ;
                     (* catalog schemas as samples; schema children are
                        valid on catalog schemas and on provider kinds whose
                        authoring schema defines projections. *)
schema_bind   = "schema" , ident , bind , body ;
                     (* schema tables as vega; table children are
                        valid on schema tables only. schema namespace is a
                        provider projection and contains no tables. *)
table_bind    = "table" , ident , bind , body ;
                     (* table parquet as orders; param children are
                        schema-valid on table sql only *)
version       = "avenger" , number , ";" ;
import        = "import" , string , [ "sha256" , string ] ,
                [ "as" , ident ] , ";" ;

chart         = "chart" , kind , [ bind ] , body ;
kind          = ident ;             (* imports bind one name; `as` renames *)
bind          = "as" , ident ;

define        = "define" , ( "mark" | "tool" | "transform" ) ,
                ident , "{" , { slot | channel_param | output | export } ,
                { item } , "}" ;
slot          = "slot" , slot_shape , bind , ( body | ";" ) ;
slot_shape    = "expr" | "expr_list" | "literal" | "number"
              | "string" | "boolean" | "enum" | "function"
              | "ref" | "block" ;
channel_param = "channel" , ident , [ ":" , ident ] , ";" ;
output        = "output" , ident , [ ":" , sql_expr ] , ";" ;
export        = "export" , qual , [ "as" , ident ] , ";" ;
                     (* define headers and group bodies; source paths may
                        forward-reference body declarations. The alias
                        defaults to the source's terminal name and must be
                        unique in the containing public interface. *)
match_block   = "match" , ident , "{" , { match_arm } , "}" ;
                     (* any depth within a define; compile-time; exactly one
                        arm per value of the matched enum slot; arms may be
                        empty and contribute items valid at that position *)
match_arm     = ident , "{" , { item } , "}" ;
splice        = ident , ";" ;
                     (* define bodies only: splice point of a block slot *)

resource      = param | store | selection | res | theme ;
                     (* data is a property: `data: { ... }` *)
param         = "param" , bind , body ;
store         = "store" , bind , body ;
selection     = "selection" , bind , body ;
res           = "resource" , ident , bind , body ;   (* resource tiles as osm *)
theme         = "theme" , "css" ,
                ( "from" , string , [ "sha256" , string ] | ":" , string ) ,
                ";" ;
                     (* chart bodies only — never in define bodies; themes
                        are plain CSS files, and `from` URLs fetch and pin
                        like imports *)

(* Bodies mix unordered properties with ordered child declarations. Which
   properties and children a given body accepts is schema-driven. *)
body          = "{" , { item } , "}" ;
item          = property | child ;
child         = [ visibility ] , child_decl ;
visibility    = "private" | "public" ;
child_decl    = param | table_bind | resource | group | mark
              | transform | tool | widget | view | event | cell | plot
              | variable | part | level | adjust | derive | overlay
              | layer | when | field | row | key | fields | action
              | scale_edit | scale_hint | dimension | match_block | splice
              | export | output ;

group         = "group" , [ bind ] , body ;
mark          = "mark" , kind , [ bind ] , body ;
transform     = "transform" , kind , [ bind ] , body ;
tool          = "tool" , kind , [ bind ] , ( body | ";" ) ;
widget        = "widget" , kind , bind , body ;
view          = "view" , kind , [ bind ] , body ;
event         = "on" , ident , [ bind ] , body ;
event_target  = "mark" , qual
              | "marks" , "[" , qual , { "," , qual } , [ "," ] , "]" ;
event_scope   = "plot" | "subplot" , qual
              | "subplots" , "[" , qual , { "," , qual } , [ "," ] , "]" ;
event_surface = "plot" | "all" | "legend" , ident ;
cell          = "cell" , kind , [ bind ] , [ "at" , body ] , body ;
plot          = "plot" , kind , body ;
variable      = "variable" , ident , [ bind ] , body ; (* variable row as mpg *)
part          = "part" , ident , body ;
level         = "level" , number , body ;
adjust        = "adjust" , [ kind , [ bind ] ] , body ;
derive        = "derive" , kind , [ bind ] , body ;
overlay       = "overlay" , [ bind ] , body ;
layer         = "layer" , ident , body ;
when          = "when" , body ;
field         = "field" , ident , ":" , arrow_type , [ "nullable" ] , ";" ;
row           = "row" , body ;
scale_edit    = "scale_edit" , body ;      (* tool definitions/behaviors:
                                               edit a containing-plot scale *)
scale_hint    = "scale_hint" , body ;      (* groups: scale-type hint *)
dimension     = "dimension" , bind , body ;  (* parallel charts: frame dimensions *)
key           = "key" , body ;             (* update payloads: delete_by_key, update_by_key *)
fields        = "fields" , body ;          (* update payloads: update_by_key *)
action        = "set" , ( state_action | cursor_action ) ;
state_action  = ( "param" | "store" | "selection" ) , qual ,
                [ "at" , ( "current" | "start" ) ] ,
                [ "replacing" , "scopes" ] , "=" ,
                ( sql_expr , ";" | ident , ( body | ";" ) ) ;
cursor_action = "cursor" , "=" , sql_expr , ";" ;
typed_ref     = ref_kind , qual , ";" ;
ref_kind      = "mark" | "group" | "selection"
              | "tool" | "widget" | "resource" ;

property      = ident , ":" , value ;
value         = body                                 (* anonymous object *)
              | ident , body                         (* typed object: linear { ... } *)
              | typed_ref
              | array , ";"
              | "value" , sql_expr , terminator      (* unscaled literal value *)
              | "dim" , qual , ( body | ";" )        (* raster dimension handle *)
              | "pattern" , body
              | "env" , string , ";"                 (* environment variable, capability-gated *)
              | "none" , ";"
              | sql_query , ";"                      (* the `sql:` property only *)
              | sql_expr , terminator ;              (* default expression slot *)
terminator    = body | ";" ;                         (* config block or semicolon *)
array         = "[" , [ elem , { "," , elem } , [ "," ] ] , "]" ;
elem          = body | sql_expr | "value" , sql_expr | "pattern" , body | "none" ;
qual          = ident , { "." , ident } ;

sql_expr      = ? one sqlparser expression accepted by AvengerSqlDialect ? ;
sql_query     = ? one standard SELECT, FROM-first SELECT, or VALUES statement ? ;
arrow_type    = ? one canonical physical Arrow type from Physical Arrow Types ? ;
```

Grammar notes:

- Declaration headers are uniformly `[visibility] <keyword> <kind> [as
  <name>]`; ordinary declarations omit visibility, and `cell` places its
  optional `at { ... }` after the binding.
- `private` and `public` are optional declaration modifiers, not declaration
  kinds. The grammar shows their token position; the authoring schema permits
  them only on declarations with a named public identity. `public` additionally
  requires a private structural ancestor and hoists to the nearest non-private
  named group. A `group` accepts `export` children and the optional opaque
  `component_kind:` provenance property. Definition headers accept the same
  exact `export` declaration, which expansion moves to the instance group.
- `transform pipeline` is the one transform kind with a mixed body containing
  ordered child transforms and `output` declarations. Its children form a
  sequential sub-dataflow while the container occupies one stage in the
  parent. The schema rejects non-transform executable children, requires at
  least one child stage, requires an `as` binder when outputs are present, and
  resolves every output against the final child relation. Other transform
  kinds retain property-only bodies and optional binders. Instantiating a
  defined transform follows the same resolved rule: non-empty declared outputs
  require `as`; an output-free definition may remain anonymous.
- `tool behavior` is the one tool kind with a mixed body containing owned
  state, events, scale edits, nested tools, optional chrome marks/groups, and
  exact exports. It always requires `as`. It creates a lexical/runtime
  instance boundary but no data, coordinate, or layout context. `scale_edit`
  has exactly one v1 target: the containing plot at the behavior's use site,
  written explicitly as `target: plot;` when desired. Other tool kinds retain
  schema-specific property bodies or the registered stateless semicolon form.
- `widget` always names a registered built-in kind and always requires a
  binder. It has a schema-specific property body and is legal only in the
  parents/guide positions declared by its `WidgetSchema`. `widget` is not a
  valid `define` target and has no language-level expansion form.
- A `channel` declaration without `: ident` is a required channel parameter;
  the instantiation must bind it. The optional identifier after `:` is its
  default physical channel.
- `slot` follows that same header law: the kind is its closed `slot_shape`, and
  the `as` name is the public property. A trailing `;` declares a required
  slot. A body may declare `default:`; `enum` additionally requires `values:`,
  `function` requires `class:`, `ref` requires `kind:`, and `block` may declare
  `exposes:`. The authoring schema rejects properties not valid for the
  declared shape. Slot shapes are explicit and never inferred from body use.
- Which `value` production a property uses is selected by the property's
  schema — an expression slot never parses as a typed object, `sql_query` is
  reachable only from the `sql` property, and enum-valued properties accept
  bare identifiers as `sql_expr` atoms that the resolver checks against the
  enum. A `typed_ref` records an explicit reference kind plus a lexical or
  qualified path; when a property schema already fixes the kind, its shorter
  bare `qual` form lowers to the same typed reference node. Action prefixes
  similarly fix the kind and accept `qual`. Params and stores are the exception:
  reads use `$qual` and lower to a typed scalar/table binding node; their
  imperative `set param` and `set store` targets remain kind-prefixed l-values.
  An action's optional `at current|start` modifies that l-value's routed owner,
  never its RHS; the authoring schema permits `start` only under `between:`.
  `replacing scopes` is a second LHS modifier, valid only for params and stores,
  that clears every concrete owner copy before writing the routed target.
  `set cursor` is the one write-only effect action: it has no target path, `at`,
  or `replacing scopes` modifier. The grammar lists the union of forms.
- Every `param` body requires exactly one `type:` parsed as `arrow_type` and one
  `default:` SQL scalar expression. `kind:` and inferred types are invalid.
  `sharing:` is optional and defaults to `shared`; chart/component defaults may
  use the acyclic param-default dependency rule, while catalog-table defaults
  remain self-contained scalar literals.
- An outer `event` body's `target:`, `scope:`, and `surface:` properties parse
  only as `event_target`, `event_scope`, and `event_surface`, respectively;
  omission supplies unrestricted marks, containing-plot scope, and plot surface.
  A `between.start` or `between.end` stream body may use `target:` plus its
  stream filter/target properties, inherits outer scope/surface, and rejects its
  own `scope:` or `surface:`.
- An event's optional `as` binder supplies diagnostic, source-map,
  hot-reload-migration, and `@previous` snapshot identity only. It introduces
  no value binding or public target.
- `data.values` is the only schema position that accepts an array of anonymous
  `body` elements. Those bodies must contain only scalar-literal properties and
  no children; all other arrays admit only the element shapes declared by their
  authoring schema.
- Keywords are contextual. `value`, `pattern`, `dim`, `env`, and
  `none` are recognized only in value-prefix position (immediately after
  `:`);
  `group`, `level`, `part`, and the other declaration keywords only in
  declaration-head position. A property may therefore be named `value`
  (conditional branch payloads are) without colliding with the `value`
  prefix.
- Ordered semantics: all child declarations preserve one cross-kind source
  order (`transform` dataflow, mark/group scene order, `cell` placement,
  `when` branch priority, `level` index order, `set` action order); properties
  are unordered within their body. A schema may reject particular child-kind
  sequences but the formatter never repairs one by reordering it.

## Coverage And Validation

The coverage unit is the deduped visual-baseline inventory:

```sh
find avenger-chart/tests/baselines avenger-chart/tests/baselines_svg \
  -type f \( -name '*.png' -o -name '*.svg' \) |
  sed 's#^avenger-chart/tests/##; s#^baselines_svg/##; s#^baselines/##; s#\.png$##; s#\.svg$##' |
  sort -u
```

That yields 892 logical scenarios across 97 baseline categories, and every
category has a syntax home in this document: most need only generated
property schemas over the core block shapes; the feature-surface sections
above cover the families that needed dedicated syntax; raster, view, tile,
and CSS-cardinality families additionally depend on runtime materialization
and resource semantics that the syntax merely names. Native built-in kinds
are exercised directly through their registered DSL schema; coverage does not
require reimplementing them as definitions. The widget crate's built-in
inventory is an additional schema/interaction/visual fixture family: each
registered widget kind is instantiated directly, and no widget-definition or
widget-expansion fixture exists.

To prove coverage, a DSL fixture suite runs parallel to the visual tests:

1. For each baseline category, at least one canonical `.avenger` fixture that
   lowers to the same public API feature family.
2. For families with many variants, parameterized fixtures generated from the
   same schema tables the LSP uses.
3. Parse each fixture, lower to `CompiledPlot`, render through the existing
   visual helpers, and compare against the same baseline image.
4. **Post-plan/deferred:** once compiled-chart decompilation exists, add a
   `CompiledPlot -> DSL` smoke test per Rust visual test plus a parse → lower →
   decompile → parse property test. This is not a gate for the language/compiler
   plan or its fixture coverage closure.
5. Parser tolerance and LSP tests stay separate from rendering tests so
   syntax diagnostics can evolve without re-blessing images.
6. An expansion equivalence property over the fixture subset that imports
   custom definitions: `compile(chart)` and `compile(expand(chart))` produce
   identical results. Dedicated custom compound-mark, tool, and SQL-transform
   fixtures pin the definition system's lowering to the compiler's own
   semantics; native-only fixtures need no artificial definition rewrite.
   The comparison includes the public path/state interface and resolved
   `(kind, part)` provenance, not only rendered pixels. Separate parser and
   resolver fixtures pin `private`, constrained `public`, group-local exports,
   and their collision diagnostics in handwritten groups. Transform-definition
   fixtures also assert that expansion produces one `transform pipeline` at
   the original stage position, preserves the final relation, and exposes the
   same output-handle map. Transform fixtures cover anonymous native stages,
   quoted-column consumption, aliased handle consumption, required binders for
   output-bearing definitions/pipelines, anonymous output-free pipelines, and
   alpha-renaming invariance of semantic/cache identity. Tool-definition fixtures assert one `tool behavior`
   with the same explicit instance identity, owned-state namespace, exported
   interface, event bindings, containing-plot scale edits, and rendered chrome;
   a missing `as` on a defined or stateful native tool is a negative fixture.
   Expanded state declarations must retain their lexical source names while
   resolving to the same opaque compiler ids. Positive and negative fixtures
   cover qualified typed paths, export-kind mismatches, cross-boundary access
   without an export, lexical `$name`, qualified `$instance.alias`, deeper
   paths, invalid quoted/numeric segments, non-binding path targets, wrong
   scalar/table use, same-scope param/store collisions, and nearest-binding
   shadowing without kind-directed fallback. Param-schema fixtures require an
   explicit canonical physical Arrow type for every param, reject `kind:` and
   type aliases/inference, preserve typed `NULL`, validate defaults/host values/
   table arguments/action results against the declared type, round-trip nested
   type spellings, reject unsupported Arrow types, and check `raw_domain:`
   use-site type constraints. Typed-boundary fixtures pin contextual primitive,
   list, struct, and `NULL` literals; range/shape failures; exact nonliteral
   param/store/table-argument/cursor results; absence of boundary-inserted casts;
   explicit-cast success/failure; exact host `DataType`; and compile-time versus
   transactional runtime diagnostics. Struct fixtures cover empty, flat, and
   recursively nested structs; ordered and arbitrary string-named fields;
   duplicate/empty-name rejection; nested list fields; nested `Call` interchange
   round trips; and default, assignment, host-binding, and temporal typed-`NULL`
   validation against the complete struct schema. Temporal-binding fixtures cover
   lexical and exported `$param@start`/`$param@previous`, first-invocation typed
   `NULL`, reset at a new gesture, current-working-value versus frozen-snapshot
   reads, previous-owner routing, failed-invocation non-advancement, native cast
   and JSON-access composition, invalid store qualifiers, invalid temporal names,
   non-event use, `@start` without `between:`, and forbidden whitespace before
   the suffix.
   Numeric-literal fixtures preserve `9007199254740993`, decimal128/decimal256
   precision, exponent forms, and negative zero through source/AST/JSON
   round trips; reject destination overflow and invalid decimal scale without
   first converting through `f64`. Literal-normalization fixtures distinguish a
   lone signed scalar literal from parenthesized, cast, and compound SQL
   expressions.
   Resolver fixtures separately pin forward structural/state/event references,
   duplicate predeclarations, sequential transform/column visibility, deferred
   output-interface resolution, acyclic and cyclic param defaults, order-free
   table DAGs, table-cycle diagnostics, and the earlier-only slot-default rule.
   Runtime state fixtures pin event-transaction working-state reads, ordered
   multi-action updates to one store, atomic cross-kind commit, rollback on
   action failure, one revision per modified store, post-commit invalidation,
   and the absence of intermediate chart-query/render observations. Action-
   query fixtures pin committed reads in handler filters, working-store reads
   after preceding actions, one stable working snapshot across repeated scans
   within an action, same-store RHS reads before the current mutation, routed
   current-owner reads independent of LHS `at start`, transaction/action cache
   generation isolation, and rollback after query planning/execution/conversion
   failure. Store-mutation fixtures pin primary-key declaration validation,
   complete-row normalization, typed nullable omission, payload and existing-key
   conflicts, duplicate payload rejection before mutation, keyed-operation
   rejection on unkeyed stores, exact key blocks, non-empty non-key-only patches,
   full-row upsert replacement, missing update/delete no-ops, toggle-by-key
   behavior independent of non-key payload values, replacement uniqueness,
   action/transaction rollback, and no revision for a valid no-op. Selection-
   mutation fixtures pin exact non-empty `utf8` ids, contextual string ids,
   complete `(selection, owner, id)` identity, equal ids across owners, canonical
   non-debug scene-query tuple ids, duplicate payload rejection before mutation,
   whole- and in-scope replacement, scope-mismatch rejection, complete-clause
   upsert, predicate-independent toggle, cross-scope versus in-scope deletion,
   missing-id no-ops, atomic rollback, and no revision for a valid no-op.
   Predicate fixtures pin dimension-AND, union-OR/intersect-AND, both empty
   behaviors, zero-dimension false, inclusive non-normalized intervals,
   `span_ordered` normalization, captured/data-field/facet-context nulls,
   missing-column false, native DataFusion comparison coercion and incompatible-
   type errors, struct equality, Arrow total-order NaN equality/ranges, and the
   generic-predicate any-null short circuit. Filter fixtures
   pin handler-filter evaluation against routed committed current/start/previous
   params before transaction creation, false/`NULL`/error non-advancement of the
   previous snapshot, stream-filter reads of candidate-event routed committed
   params, rejection of stores and temporal qualifiers in stream filters,
   snapshot capture only after a successful start filter, an unmatched start
   leaving the gesture closed, an unmatched end leaving it open, and stream-
   filter errors leaving gesture state unchanged. Event-routing fixtures pin the
   closed singular/plural mark target forms, containing/subplot scope forms,
   plot/all/legend surfaces, all three defaults, AND composition, duplicate and
   empty-list rejection, public-path and legend-channel resolution, invalid
   overloaded `target: plot|legend` diagnostics, redundant explicit defaults,
   and start/end inheritance with stream-local mark narrowing but rejected
   stream-local scope/surface. Admission fixtures pin the
   `Rejected | Committed | Failed` outcomes, successful mutating and no-op
   commits, consume-only-on-commit propagation in deterministic binding order,
   aborted/rejected continuation to later bindings, previous-snapshot and
   throttle-clock advancement only on commit, throttled events invoking and
   consuming nothing, rejected events not delaying the next eligible event, and
   between start/end transitions remaining independent of outer-handler consume
   and throttle. Cursor-effect fixtures pin the absence of a declaration/`$`
   binding/owner/export path, canonical literal and computed-`utf8` forms,
   static and runtime style validation, `NULL` as no publication, `default` as
   reset, last-action and last-committed-handler precedence, consume truncation,
   rollback on later failure, no rerender/revision from cursor alone, and removal
   of param-name/registration behavior. Every registered cursor style,
   including `default`, also appears in the Phase-1 token/expression corpus so
   keyword-adjacent spellings cannot drift. Runtime fixtures also pin
   default/current versus captured-start target routing, mixed target routes in
   one transaction, invalid `at start` outside `between:`, redundant shared-
   target warnings, and no-op missing routes. Cross-owner fixtures pin that
   ordered visibility is keyed by concrete `(binding, owner)`, ordinary reads
   remain current-routed after an `at start` write, temporal reads remain frozen,
   RHS store scans remain current-routed, and an `at start` store primitive
   internally sees its target owner's pre-action working rows. They cover
   `replacing scopes`
   removal-before-write ordering, later-action visibility, rollback, param-
   default fallback, empty non-reseeded stores, invalid selection use, and the
   redundant-shared warning. Sharing fixtures
   pin logical owner paths for `free`/`level(n)`/`shared`, root saturation,
   per-owner param defaults and store revisions, root-only store initial rows,
   and no-op unrouted non-shared writes. Store-relation fixtures assert that
   `SELECT *` and schema tooling expose only declared fields, authored access to
   the reserved metadata prefix fails, and hidden revision metadata still
   invalidates physical-cache entries.

## Implementation Phasing

Everything in this reference is one language, but not one project. The
kernel is the smallest dependency-ordered spine that makes the language
real — each phase has a verifiable gate, and each phase builds only on
the one before it. Everything else is explicitly sequenced after the
kernel and blocks nothing in it.

The language phases begin only after the
[Rust semantic-unification prerequisite](../../../scratch/avenger-lang/rust-dsl-unification-implementation-plan.md)
is complete. That prerequisite delivers typed opaque state IDs, explicit param
types, one ordered transactional event-action vector, cursor effects, compiled
mark aliases/provenance, the pipeline transform, inline-view identity, unified
tool behavior, widget migration through the new tool behavior without losing
measurement/presentation metadata, and schema entries paired with native
lowerers. Its completion
gate includes a registry-driven programmatic chart that compiles and renders
without a handwritten DSL kind switch. Parser work must not introduce
temporary DSL-only runtime representations while that prerequisite is open.

0. **Freeze the kernel authoring-schema projection.** Select the language
   kernel entries from the already unified Rust native registry and emit the
   canonical versioned authoring-schema snapshot.
   The snapshot is the normative semantic inventory; generated JSON Schema,
   docs, completion data, and later macro output are derived views. Gate: the
   snapshot validates against the meta-schema, round-trips canonically, and
   its focused conformance corpus proves every declared body mode and value
   shape. The registry grows with later feature phases, but its model and
   identity rules are frozen here.

1. **Token conformance corpus + the Avenger dialect.** The two dialect
   hooks (`requires_single_line_comment_whitespace`,
   `supports_nested_comments`) over the stock sqlparser tokenizer, and
   the golden token-stream corpus pinning the token classes the language
   relies on, including reserved helper-name exclusions and every registered
   cursor style ([Parser Architecture](#parser-architecture)). The smallest
   deliverable, and it de-risks the foundational bet first. Gate: the
   corpus is green against the workspace's sqlparser.

2. **Parser, AST, printer.** The strict-mode tokenize-then-parse parser
   with SQL islands, the lossless CST, the separate generic semantic AST,
   `-- |` doc-comment capture, the canonical DSL/SQL printer, `avenger fmt`
   over the trivia-preserving concrete tree, and the JSON interchange encoding
   with the frozen core schema.
   Gate: the [round-trip laws](#round-trip-laws) hold over a
   hand-written corpus — `parse(print(ast)) == ast`, `fmt` is a
   fixpoint, and `fmt(text)` with comments stripped equals
   `print(parse(text))`; property-order permutations collapse to one semantic
   AST and one lexical DSL spelling without loading schemas or imports, JSON
   object-key permutations decode equally,
   child-order permutations remain distinct across keywords, DSL/JSON
   round-trips preserve the child array exactly, and the reviewed SQL-unparse
   corpus is unchanged.

3. **Resolver + authoring-schema integration.** Names, scopes, scalar/table `$bindings`,
   predeclaration, forward-reference categories, sequential dataflow aliases,
   param-default and table dependency DAGs, validation, alias fields,
   reserved-helper rewriting, event targets,
   visibility/hoisting and group-export alias graphs, imported-definition
   schema fragments, and schema-driven property checking
   over the normative registry ([Authoring Schema Source](#authoring-schema-source)).
   Gate: the diagnostics contract — unknown names and properties report
   the valid set for their position.

4. **Lowering + the fixture suite.** AST → `Chart`/`Plot` authoring
   model → the existing compiler, plus `avenger check` and
   `avenger render`. Native built-in compound marks, tools, and transforms
   lower directly through their registered schemas at this stage; registered
   built-in widgets lower as opaque widget instances at this stage; this includes
   the core `transform pipeline` and `tool behavior` containers needed by later
   source expansion.
   Gate: the
   [Coverage And Validation](#coverage-and-validation) fixture suite
   renders against the existing baseline images, and the completed native
   inventory produces the reviewed v1 schema snapshot with no undocumented
   entries.

5. **Definitions + expansion.** `import` (relative and `std:` paths),
   `define` with slots, channel parameters, `match`, block slots,
   `export`/`exposes`, the first custom definition fixtures, and `avenger
   expand`. Gate: the expansion equivalence property for charts that use
   imported definitions — `compile(chart) == compile(expand(chart))` — over
   dedicated custom mark, tool, and SQL-transform fixtures. Native built-in
   fixtures remain native and are not rewritten as definitions, and widget
   declarations remain untouched because widgets have no definition form.

6. **Data catalogs.** `.data.avenger` files, catalog/schema/table bindings,
   `table sql` views with params and table-function calls, and
   `avenger tables`. URL-based dataset packs wait for distribution.

Sequenced after the kernel, blocking nothing above: distribution (URL
imports, hash pinning, `add`/`pin`/`update`/`vendor`), the language server
and tolerant parsing mode, editor grammars (Zed, VS Code, CodeMirror),
the online editor, `avenger doc`/`watch`/`editor`, and the agent
affordances (`avenger prompt`, the `--format json` contracts). Each of
these consumes kernel surfaces — the schema, the printer, the resolver —
and adds none.

## Command-Line Tool

One binary, `avenger`, organized around the project workflows. The governing
rule mirrors the pin guarantees: **verbs that mutate source are few,
explicit, and named** (`add`, `pin`, `update`, `vendor`, `fmt`, and `expand
-o`); everything else is read-only.

The authoring loop:

| Command | Purpose |
| --- | --- |
| `avenger check [paths]` | Parse, resolve, validate (strict mode). `--locked` for CI; static by default, `--data` also validates against live schemas. |
| `avenger render <chart> -o out.png` | Compile and render (`.png`/`.svg`/`.pdf`); `--param k=v` overrides; `--watch`. |
| `avenger watch <chart>` | Open one native chart window with dependency-aware hot reload while editing in another editor; preserves compatible state and the physical-plan cache across reloads. Project/gallery mode is deferred. |
| `avenger editor [chart\|project]` | The full native playground: collapsible file browser + editor + live chart, one window — see below. |
| `avenger fmt [paths]` | Canonical formatter; `--check` for CI. Shares one printer with `expand` output and decompilation. |

Imports and distribution (the mutating verbs):

| Command | Purpose |
| --- | --- |
| `avenger add <url\|std:path> [file] [--as name]` | Fetch, verify, and insert a pinned import. |
| `avenger pin [paths] [--print]` | Pin unpinned URL imports. |
| `avenger update [name\|--all]` | Re-fetch mutable-URL imports and refresh hashes — a deliberate re-pin, surfaced as a diff. |
| `avenger vendor <import>` | Copy a remote definition file into the project and rewrite the import to relative. |

Flattening, introspection, and testing:

| Command | Purpose |
| --- | --- |
| `avenger expand <file> [-o out]` | Source-level inline-definition expansion; concrete definition instantiations become editable ordinary groups with visibility, exports, and component provenance, while native built-in kinds and semantic resource imports remain. |
| `avenger info [path]` | The doc-query surface over the entire language schema — native built-ins and imported definitions alike, `--format json` throughout. Bare `info` lists namespaces (marks, transforms, tools, scales, helpers, events); drill by path: `info marks --coord geo`, `info mark rect`, `info mark rect.x` (one channel's option suffixes), `info transform bin`, `info std:marks/error_bar` or any file/URL (slots, parts, outputs, doc comments — inspect before importing). |
| `avenger schema [--format json\|json-schema]` | The entire machine-readable language schema in one dump — `json` for tooling and big-context agents that load the reference once, `json-schema` to compile it into the generated full validator for the AST interchange form (the frozen core schema ships with the spec). |
| `avenger doc [-o dir] [--open] [--single-page]` | Generate the project's documentation site from three sources it already has — see below. |
| `avenger deps <chart>` | The import closure as a tree with pin status and origins; the supply-chain review. |
| `avenger table <chart> --at <alias\|mark>` | Print the data context at a named point in the pipeline; anonymous transforms have opaque compiler identities and are inspected through a following named stage/mark or an editor source-position action. |
| `avenger tables [--format json]` | List the catalog's resolved table names and schemas (`table sql` views included) — the source for column completion and the first thing an agent should read. |
| `avenger ast <file>` | Convert between the two encodings: `.avenger` text to interchange JSON, or interchange JSON back to canonical text (byte-identical to `avenger fmt`). |
| `avenger test [--bless]` | Every chart is an example: compile, render, and fuzzy-perceptually compare each chart against its sibling `.png` baseline (exact matching is brittle across GPUs; threshold configurable). Failures emit actual and diff images; a missing baseline fails with a hint; `--bless` (re)generates baselines for all or changed charts. Also compiles doc-comment examples (visual doctests). |

Deferred live-session inspection commands share the versioned inspection
protocol hosted by a running `avenger watch` process:

| Command | Purpose |
| --- | --- |
| `avenger inspect [--chart <path>\|--session <id>] [--object <id>]` | Open a separate native inspector process for current params, stores, datasets, selections, lineage, and bounded table previews. |
| `avenger snapshot --session <id> -o <bundle>` | Capture committed revisioned state plus metadata and optional render; large datasets remain metadata/bounded previews unless explicitly requested. |
| `avenger dap [--chart <path>\|--session <id>]` | Adapt inspection/debug domains to DAP for explicit pause/break semantics; disconnecting leaves the watch session running. |
| `avenger mcp [--chart <path>\|--session <id>]` | Expose read-only observation tools for an agent: describe state, preview data, render, and wait for a committed revision. |

These commands are follow-on work, not part of the initial watch milestone.
They are separate processes/clients; the watch process remains the runtime
owner, and the LSP is another read-only inspection client rather than a second
state authority.

`avenger editor` is a native playground built with egui over the existing
wgpu stack — the `avenger-egui`/`avenger-chart-egui` crates already provide
the shared-device chart panel (the Rerun architecture: UI and a custom
renderer sharing one wgpu device via paint callbacks). The editor widget is
a thin layer over raw `egui::TextEdit` and its `layouter` callback — the
ecosystem has no LSP-ready code editor, and `egui_code_editor`-style crates
bring their own highlighting that would fight the real parser's. Semantic
tokens feed the `LayoutJob` (cached by buffer hash), diagnostic spans carry
`TextFormat` underlines in the same job, hover and completion anchor to
galley cursor positions, and the only fiddly part is intercepting
completion-popup keys before `TextEdit` consumes them. Three properties make it cheap and good:
the preview pane is a live `PlotSession` rendered by the real renderer —
params, tools, and view-driven rasterization fully interactive, not a
re-rendered image; the language service runs in-process (`avenger-lang` as
a library — diagnostics, completion, hover, and formatting are function
calls, and the editor highlights from the real parser's semantic tokens,
the one surface with no lexical grammar at all); and the whole environment
ships as the single static CLI binary. Panels: collapsible file
browser, editor, live chart, diagnostics, schema docs (`info` inline), and
the project gallery. `avenger watch` is the one-chart preview window with your
own editor filling the editing role. Both `watch` and `editor` share two
hot-reload semantics: reloads are import-graph-aware
(saving a definition file, theme, or data file reloads every chart whose
closure includes it), and session state survives recompiles — params,
stores, selections, and view domains carry over where names still match, so
a zoomed viewport stays put while a color is tweaked. Scope is deliberately
a playground, not an IDE — chart-sized files, no multi-cursor ambitions;
serious project work pairs a real editor with `avenger lsp` and
`avenger watch`.

`avenger doc` composes a static site from three sources, zero configuration:

1. **README.md** at the project root becomes the front-page prose
   (CommonMark; `--readme <path>` overrides). Links to project files rewrite
   to their documentation targets: a link to
   `marks/error_bar.mark.avenger` points at that definition's reference
   section, and a link to `charts/revenue_trend.avenger` becomes its gallery
   entry, image included.
2. **The chart gallery**, built from blessed baselines and each chart's
   `-- |` blurb — no rendering at doc time, so output is deterministic and
   by construction in sync with what `avenger test` verified.
3. **The definition reference**: one section or page per mark, tool, and
   transform — schema tables (slots and defaults, channel parameters,
   outputs, public parts, exports), doc-comment text, and doc examples
   rendered as images.

Single-page mode emits README → gallery → reference as one HTML file;
multi-page mode uses README plus the gallery as the index with a page per
definition. Output is plain static files, publishable anywhere.

Scaffolding and plumbing: `avenger new <project>` and
`avenger new mark|tool|transform <name>` generate the layout and definition
templates; `avenger lsp` launches the language server over stdio.

The CLI is a host, so the capability model surfaces as flags: defaults are
reads within the project root only, with explicit grants beyond —
`--allow-read=<path>`, `--allow-net[=host]` (URL imports, tiles, remote
data), `--allow-env[=VAR]` (catalog credentials; a project-root `.env` is
loaded before resolution) — plus `--locked`, `--offline` (cache-only
fetches), `--catalog <file>` to select data catalogs (the dev/prod switch),
and `--root <dir>` to set the project root and capability boundary
explicitly.

### Agent Authoring

Coding agents have no pretraining on this language; everything they know
arrives in context, and their errors are predictable — pattern-matching to
lookalike languages (SQL, HCL, Vega-Lite, CSS) exactly where this language
deliberately diverges. Three affordances address this:

**The language card.** `avenger prompt` emits the canonical agent primer,
versioned with the toolchain so it can never drift from the compiler the
agent is talking to: five or six complete golden examples spanning the
surface, the traps list below, the construct reference, and the workflow
loop — a few hundred lines, designed for injection into an agent's context
before the first line is written. Agents learn novel syntax from complete
examples plus explicit invariants, not from grammars.

**The traps list** — each entry contradicts a lookalike prior:

- Double quotes are *data columns*; single quotes are strings
  (`x: "mpg"`, `title: 'MPG'`) — the inverse of every familiar language.
- Literal visual values need `value` (`fill: value '#4682b4'`; bare
  `'red'` is a scaled string, `"red"` is a column reference).
- Bare identifiers are language-space names — kinds, properties, enums,
  aliases, slots — never columns.
- Helper arguments: DSL-space names bare (`channel(x)`, `event_coord(x)`),
  data-space names as strings (`datum('id')`).
- `avenger 1;` first; one chart or one define per file; `;` terminates a
  property unless a `{ }` config block follows; channel config attaches
  after the value (`x: "hp" { axis: { title: 'HP'; } }`).
- No loops or conditionals: repetition is data (`fold`, `repeat`,
  `facet`); modes are `match`, in definitions only.

**The loop, machine-legible.** `avenger check --format json` follows a
diagnostics contract: every diagnostic carries a rule id, a span, what was
expected at that point (schema-driven — an unknown property lists the
block's valid properties), and a mechanical `fix:` snippet where one exists,
with messages phrased in the card's own language so every error reinforces
the primer. `avenger render --quick -o p.png` produces a low-scale preview
cheap enough to inspect on every iteration; `avenger table --at <alias>`
shows the data mid-pipeline (agents debug data problems from tables, not
pixels); `avenger fmt` keeps diffs canonical. Machine-readable variants
(`--format json`) exist wherever an agent would parse output.

**Discovery is layered over one schema.** Proactive: `avenger info` is the
list-and-drill doc query (`info marks --coord geo` → `info mark rect` →
`info mark rect.x`), and `avenger schema --format json` dumps the whole
reference for agents that load it once. Reactive: the diagnostics contract
doubles as completion — a plausible guess compiles into an error carrying
the exact valid set for that position, often the cheapest path for an
agent. Positional completion exists through `avenger lsp` for
LSP-speaking harnesses but is deliberately secondary: between context-free
`info` and context-exact diagnostics, agents rarely need cursor
coordinates, which is their weakest skill. The `avenger prompt` card names
these commands, so the primer teaches lookup rather than memorization.

## Lowering Model

The recommended implementation path is:

1. Parse the DSL into a stable DSL AST.
2. Resolve names, SQL snippets, params, aliases, channel references, and event
   targets. Assign opaque typed identities to state declarations, marks, inline
   views, and tool instances; resolve authored paths to those identities; and
   hoist lexically scoped state into the compiled chart registries without
   changing its owner-path sharing semantics.
3. Lower schema-validated native declarations through registry entries that
   pair their normative authoring schema with an erased Rust lowerer. The
   registry, not a second DSL-specific kind switch, constructs the Rust
   authoring objects. A chart file's root
   lowers to `Chart` (document furnishings — title/subtitle, theme,
   canvas, locales, state declarations) wrapping a `Plot`; nested
   positions (cells, embedded plots) lower to bare `Plot`s inside their
   wrappers (`Subplot` for cells); then `MarkGroup`, native built-in marks,
   transform builders, channel values, scales, guides, params, stores,
   selections, tools, built-in widgets, and event bindings. **Coordinate-kind properties
   (`rows:`, `projection:`, `responsive_columns:`, ...) lower to the
   coordinate-system type's builder calls, never to `Plot`/`Chart`
   methods** (see rust-authoring-wrappers.md — the wrappers carry no
   coordinate surface beyond `with_coord`/`configure_coord`).
4. Compile using the existing chart compiler and the unified ordered event,
   pipeline-transform, tool-behavior, and compiled-identity representations.

Compiled-chart decompilation is a post-plan capability, not a v1 compiler gate.
If added later, the compiled root artifact (`CompiledPlot` in the current Rust
API; a `CompiledChart` rename is explicitly deferred) should produce a
canonical, fully elaborated DSL form only when enough compiled metadata is
retained. Exact authoring-source recovery should instead preserve the original
DSL AST or source map.

## AST And Interchange Form

Step 1 of the lowering model names "a stable DSL AST". Its shape is the
payoff of the grammar's uniformity — every declaration is
`keyword kind? as name? { props; children }` — so the tree needs six
generic node types, not a node type per language feature:

The parser maintains two deliberately separate representations:

- A lossless concrete syntax tree owns source tokens, whitespace, comments,
  original SQL-island text, recovery nodes, and byte ranges. Editors,
  diagnostics, comment-preserving formatting, and exact source recovery use it.
- The strict semantic AST below owns only valid language structure and parsed
  SQL semantics. It is the input to resolution/lowering and the value encoded
  by interchange JSON. It contains no recovery nodes or source spelling.

Source spans live in an external `AstSourceMap` keyed by stable semantic node
IDs. They are not fields on semantic nodes and therefore cannot affect
equality, hashing, serialization, or round-trip laws. Host-constructed trees
may populate the same map with host-frame provenance.

```rust
struct File {
    version: u32,                    // the `avenger 1;` pragma
    name: Option<Name>,              // filename-derived canonical name (loader-populated;
                                     // None for plural data catalogs)
    imports: Vec<Import>,            // source, sha256 pin, rename
    root: Root,                      // Chart(Decl) | Define(Decl) | Data(Vec<Decl>)
}

struct Decl {
    visibility: Visibility,              // default | private | public
    keyword: Keyword,                // chart | mark | transform | table | param | on | ...
    kind: Option<Name>,              // symbol, sql, parquet, cartesian, ...
    name: Option<Name>,              // the `as` binder
    props: PropertyMap<Name, Value>, // unique unordered semantic map
    children: Vec<Decl>,             // one semantic cross-kind order
    doc: Option<String>,             // attached `-- |` doc comment
}

enum Visibility { Default, Private, Public }

enum Value {
    Str(String), Num(NumericLiteral), Bool(bool), Null,
    Column(String),                  // "Horsepower"
    Atom(Name),                      // lone bare identifier: retarget_cached, median
    Expr(SqlExpr),                   // parsed semantic expression only
    Query(SqlQuery),                 // sql: statement island
    Binding(BindingKind, Vec<Name>, BindingTime),
                                      // $name or $component.alias[@time]
    Ref(RefKind, Vec<Name>),         // selection hover.hovered, mark layers.points
    Visual(Box<Value>),              // value <literal/expression>
    Dim(Vec<Name>),                  // dim pixels.x_dim; exactly two segments
    Pattern(Box<Value>),
    Env(String),
    None,
    Array(Vec<Value>),
    Block(Option<Box<Value>>, Body), // head value + body; Body = props + children
    Call(Name, Vec<Value>),          // channel(x), datum('id'), list(float64)
}

struct NumericLiteral {
    canonical_decimal: String,       // exact digits; never converted through f64
}

enum BindingKind { Param, Store }
enum BindingTime { Current, Start, Previous } // Current is omitted in source/JSON
```

Every feature in this document is an instance of `Decl` — `table sql` with
params, `catalog schemas` and `schema tables` containers, `match` arms,
effects, block-slot content.
Typed slots need no special node: `slot expr as category;` is a `Decl` with
`keyword = slot`, `kind = expr`, and `name = category`; configured slot fields
such as `default`, `values`, `class`, `kind`, and `exposes` are ordinary
properties in its body.
Visibility is likewise generic declaration metadata rather than a new node:
the parser records a `private` or `public` prefix, the schema decides whether
that declaration may carry it, and the resolver constructs the public alias
graph. The default variant is omitted in text and interchange output.
`PropertyMap` has name-to-value equality and hashing: insertion or source order
is not semantic. The CST separately records authored order and comment anchors.
Canonical DSL printing uses ascending ASCII lexical name order for every key,
so it is total without authoring schemas, import loading, plugins, or network
access, and any two equal semantic maps print identically. `children`, by contrast, is one semantic sequence: equality,
hashing, JSON, printing, expansion, and lowering all preserve it exactly across
child kinds.
**Validity lives in the authoring schema, not the node types**: the schema
checks keyword/kind/property combinations over a generic tree, which is
the inversion that keeps the AST closed while the language grows. A lone
bare identifier parses as `Atom`; whether it names an enum member, a slot,
or a function is the resolver's schema-directed decision (the bare-name
law). `Binding` retains the resolved scalar/table kind, every `$path`
segment, and its temporal version; `Ref` retains the expected non-value state kind and every path segment
even when a schema-fixed property omits the surface prefix. `Expr` and `Query`
retain parsed semantic SQL only. Original island
spelling, whitespace, comments, and token ranges belong to the concrete syntax
tree and source map, not semantic AST identity.

`Block` is the one composite: an optional *head* value plus a body. All
three surface shapes lower to it — a bare block (`data: { ... }`, no
head), a typed object (`scale: linear { ... }`, an `Atom` head), and a
configured value (`x: "amount" { scale: ... }`,
`title: 'Sales' { align: center; }`, `x: dim pixels.x_dim { axis: ... }` —
the value being configured is the head, whatever its variant). Whether a
head names a kind or is a value under configuration is, once again, the
schema's call, not a structural distinction.

`AstSourceMap` is diagnostic provenance rather than AST identity: parsed node
IDs map to source spans, nodes constructed by a host API (the Python bindings)
may map to the host frame that created them, and diagnostics render either. It
is not part of interchange JSON or semantic equality. The CST remains the
authoritative source representation for parsed text.

### The JSON Encoding

Serde over these nodes defines the interchange form. Four rules:

- JSON string, boolean, and null scalars encode directly: `"Manhattan"`,
  `true`, `null`. Numeric literals always use an exact tagged string such as
  `{"num":"12"}` or `{"num":"-0"}`; plain JSON numbers are rejected.
- Every other value is a single-key tagged object: `{"col": "Horsepower"}`,
  `{"atom": "retarget_cached"}`,
  `{"binding": {"kind": "param", "path": "borough"}}`,
  `{"binding": {"kind": "param", "path": "width", "time": "start"}}`,
  `{"binding": {"kind": "store", "path": ["controls", "brush"]}}`,
  `{"ref": {"kind": "selection", "path": ["hover", "hovered"]}}`,
  `{"env": "ICEBERG_TOKEN"}`, `{"expr": "..."}`, `{"query": "..."}`. The
  inventory is closed — fourteen tags: `num`, `col`, `atom`, `binding`, `expr`,
  `query`, `value`, `dim`, `ref`, `pattern`, `env`, `none`, `block`,
  `call` — pinned by the core schema below. The `block` tag carries the
  optional head beside `props` and `children`
  (`{"block": {"head": {"col": "amount"}, "props": ...}}`); a typed
  object is simply a `block` whose head is an `atom`. Imports encode as
  `{"import": "<specifier>", "sha256": "...", "as": "..."}`.
  The prefix tags have deliberately distinct payload shapes: `value` and
  `pattern` contain another value, `dim` contains one two-segment dotted name,
  `env` contains a non-empty string, and `none` is exactly `true`. A one-segment binding path uses the compact string payload; a qualified path
  uses an array of two or more path segments. `kind` is always explicit in
  interchange even though source spelling and contextual typing make it
  unnecessary in the DSL. `time` is omitted for the ordinary current value and
  is `"start"` or `"previous"` for a temporal param read; semantic validation
  rejects a temporal store binding.
- A non-default declaration visibility encodes as `"visibility": "private"`
  or `"visibility": "public"`; omission means the ordinary default. Group
  exports remain ordinary child declarations, and `component_kind` remains an
  ordinary property, so expansion adds no interchange-only node or tag.
- SQL islands serialize as canonical SQL text (the unparser's output),
  never as DataFusion ASTs — compact, readable, re-parsed on load under
  the shared-tokenizer contract, and independent of DataFusion's internal
  types. This is exactly the SQL spelling used by the canonical DSL printer;
  raw source spelling and SQL comments are CST trivia and never enter JSON.
- Props encode as ordinary JSON objects; member order has no semantic meaning
  and decoders may return keys in any order. Canonical JSON bytes follow RFC
  8785 lexical object-key ordering recursively. DSL property names are ASCII,
  so this agrees with the canonical DSL property order. Duplicate object members are rejected by the
  interchange decoder rather than accepted with first- or last-wins behavior;
  a duplicate textual DSL property is likewise a parse error.
- Children encode as one JSON array in semantic cross-kind order. Array order
  is preserved by ordinary JSON implementations and must not be regrouped by
  declaration keyword during encoding or decoding.

```avenger
table sql as borough_trips {
  param as borough { type: utf8; default: 'Manhattan'; }

  sql: SELECT * FROM trips WHERE "borough" = $borough;
}
```

```json
{
  "decl": "table", "kind": "sql", "name": "borough_trips",
  "children": [
    { "decl": "param", "name": "borough",
      "props": { "default": "Manhattan", "type": { "atom": "utf8" } } }
  ],
  "props": {
    "sql": { "query": "SELECT * FROM trips WHERE \"borough\" = $borough" }
  }
}
```

A configured channel shows the head in play — the same `block` tag at
both altitudes:

```avenger
x: "amount" {
  scale: linear { nice: true; }
}
```

```json
{
  "x": {
    "block": {
      "head": { "col": "amount" },
      "props": {
        "scale": { "block": { "head": { "atom": "linear" },
                              "props": { "nice": true } } }
      }
    }
  }
}
```

Interchange JSON carries content and doc comments; provenance and trivia
stay out (exact-source recovery preserves the original text or a source
map, as the lowering model notes). The `version` field carries the pragma,
so the JSON is exactly as versioned as the text — the same major-version
gate applies to both. The optional top-level `name` carries the loader-derived
canonical file name when source context is available. A pathless parse leaves
it absent; loading a named one-item file validates or supplies it, while plural
data-catalog files keep it absent.

### The Core Schema

Two JSON Schemas validate the interchange form, at two layers:

- **The core schema** (below) is structural: it accepts exactly the
  well-formed trees — the node shapes, the closed tag inventory, ident and
  hash lexical rules. It is hand-written, about a hundred lines, and
  frozen per language major version: adding language features never
  changes it.
- **The full schema is generated.** `avenger schema --format json-schema`
  compiles the authoring schema into a conditional JSON Schema over
  keyword/kind pairs — which properties `mark symbol` accepts, which kinds
  `table` has, enum memberships. It revs with the language, and external
  validators wanting semantic depth regenerate it, as Altair regenerates
  from Vega-Lite — but here as an optional deepening over a frozen core
  rather than as the format itself.

Core membership means "parses": the JSON analogue of tokenizing plus
declaration-shape checking. It deliberately does not attempt what JSON
Schema cannot express — name resolution (`FROM trips`, `$borough` against
declared value bindings), scalar/table kind checking, SQL island validity
(islands are opaque strings here), DAG acyclicity, sibling-name uniqueness,
import closures. `avenger check`
remains the validator of record; the schemas exist so foreign tools can
reject malformed trees early and cheaply.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://avenger.dev/schemas/ast-core-1.json",
  "title": "Avenger AST interchange form, core schema (language major version 1)",
  "type": "object",
  "properties": {
    "version": { "const": 1 },
    "name": { "$ref": "#/$defs/name" },
    "imports": { "type": "array", "items": { "$ref": "#/$defs/import" } },
    "root": {
      "oneOf": [
        { "$ref": "#/$defs/decl" },
        { "type": "array", "items": { "$ref": "#/$defs/decl" }, "minItems": 1 }
      ]
    }
  },
  "required": ["version", "root"],
  "additionalProperties": false,
  "$defs": {
    "name": { "type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$" },
    "import": {
      "type": "object",
      "properties": {
        "import": { "type": "string", "minLength": 1 },
        "sha256": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
        "as": { "$ref": "#/$defs/name" }
      },
      "required": ["import"],
      "additionalProperties": false
    },
    "decl": {
      "type": "object",
      "properties": {
        "decl": { "$ref": "#/$defs/name" },
        "kind": { "$ref": "#/$defs/name" },
        "name": { "$ref": "#/$defs/name" },
        "visibility": { "enum": ["private", "public"] },
        "doc": { "type": "string" },
        "props": { "$ref": "#/$defs/props" },
        "children": { "type": "array", "items": { "$ref": "#/$defs/decl" } }
      },
      "required": ["decl"],
      "additionalProperties": false
    },
    "props": {
      "type": "object",
      "propertyNames": { "$ref": "#/$defs/name" },
      "additionalProperties": { "$ref": "#/$defs/value" }
    },
    "body": {
      "type": "object",
      "properties": {
        "head": { "$ref": "#/$defs/value" },
        "props": { "$ref": "#/$defs/props" },
        "children": { "type": "array", "items": { "$ref": "#/$defs/decl" } }
      },
      "additionalProperties": false
    },
    "value": {
      "oneOf": [
        { "type": ["string", "boolean", "null"] },
        { "type": "array", "items": { "$ref": "#/$defs/value" } },
        { "$ref": "#/$defs/tagged" }
      ]
    },
    "tagged": {
      "type": "object",
      "minProperties": 1,
      "maxProperties": 1,
      "properties": {
        "num": {
          "type": "string",
          "pattern": "^-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?$"
        },
        "col": { "type": "string", "minLength": 1 },
        "atom": { "$ref": "#/$defs/name" },
        "binding": { "$ref": "#/$defs/binding" },
        "expr": { "type": "string", "minLength": 1 },
        "query": { "type": "string", "minLength": 1 },
        "value": { "$ref": "#/$defs/value" },
        "dim": {
          "type": "string",
          "pattern": "^[A-Za-z_][A-Za-z0-9_]*\\.[A-Za-z_][A-Za-z0-9_]*$"
        },
        "ref": { "$ref": "#/$defs/ref" },
        "pattern": { "$ref": "#/$defs/value" },
        "env": { "type": "string", "minLength": 1 },
        "none": { "const": true },
        "block": { "$ref": "#/$defs/body" },
        "call": { "$ref": "#/$defs/call" }
      },
      "additionalProperties": false
    },
    "call": {
      "type": "object",
      "properties": {
        "fn": { "$ref": "#/$defs/name" },
        "args": { "type": "array", "items": { "$ref": "#/$defs/value" } }
      },
      "required": ["fn"],
      "additionalProperties": false
    },
    "ref": {
      "type": "object",
      "properties": {
        "kind": {
          "enum": ["mark", "group", "selection", "tool", "widget", "resource"]
        },
        "path": {
          "type": "array",
          "items": { "$ref": "#/$defs/name" },
          "minItems": 1
        }
      },
      "required": ["kind", "path"],
      "additionalProperties": false
    },
    "binding": {
      "type": "object",
      "properties": {
        "kind": { "enum": ["param", "store"] },
        "path": {
          "oneOf": [
            { "$ref": "#/$defs/name" },
            {
              "type": "array",
              "items": { "$ref": "#/$defs/name" },
              "minItems": 2
            }
          ]
        },
        "time": { "enum": ["start", "previous"] }
      },
      "required": ["kind", "path"],
      "additionalProperties": false
    }
  }
}
```

The instance corpus for this schema includes accept/reject and bidirectional
round-trip cases for every one of the fourteen tags, including exact `num`
spelling, two-segment `dim`, string `env`, boolean-true `none`, compact and
qualified binding paths, and headed/headless blocks. It also covers well-formed
chart, data, and define files, optional loader-supplied `name`, and rejects
plain JSON numbers, two-key tagged objects, unknown tags, provenance fields,
invalid tag payload shapes, duplicate members, and malformed hashes. It belongs
to the same conformance corpus that pins the formatter.

### Round-Trip Laws

- **Printing is total.** Every constructible strict AST prints. Every `Expr`
  and `Query`, whether parsed from source, decoded from JSON, or constructed by
  a host API, prints through the same pinned SQL unparser. Nothing a host API
  can build lacks a text spelling.
- **Printing is canonical.** Equal trees produce byte-identical files, so
  generated artifacts diff cleanly against hand-written ones. `avenger
  fmt` shares the printer's one layout engine but runs it over the
  trivia-preserving concrete tree. Valid SQL islands are unparsed canonically;
  their original whitespace and spelling are discarded. DSL and SQL comments
  survive because the CST anchors them to declarations, properties, or SQL
  token/nodes and the formatter reinserts them at deterministic normalized
  positions. Property-attached comments move with their property when lexical
  ordering changes its position. Leading comments anchor to the item
  that follows them, trailing same-line comments to the item before. The link is testable:
  `fmt(text)` with comments stripped equals `print(parse(text))`. Both
  outputs are pinned by the conformance corpus; the layout engine is
  thereby load-bearing, not cosmetic.
- **`parse(print(ast)) == ast`**, using semantic equality that ignores
  provenance. Text, host-language nodes, and JSON are three encodings of one
  strict semantic tree with one lowering; raw spelling and trivia are properties
  of the CST, not counterexamples to the law. The corresponding interchange
  law is `decode_json(encode_json(ast)) == ast`; decoding reparses canonical SQL
  text into the same semantic SQL structure.
- **SQL canonicalization is pinned.** A golden corpus covers expression and
  statement islands, comments, quoted identifiers, qualified scalar/table
  binding reads, and dependency-sensitive syntax. An `sqlparser` or DataFusion
  upgrade that changes canonical unparse output requires an explicit reviewed
  language-snapshot update; it cannot silently churn DSL, JSON, expansion, or
  semantic hashes.
- **Property order is neither identity nor semantics.** Equal property maps
  compare and hash equally regardless of insertion or source order. The CST
  remembers authored order for editor operations, while both `print(ast)` and
  `avenger fmt` emit all properties in ascending ASCII lexical order. Canonical
  JSON uses the corresponding RFC 8785 key order; decoding any input order
  reconstructs the same semantic map.
- **Child order is identity and semantics.** The canonical printer first emits
  canonical properties, then walks `children` without sorting or grouping.
  Child-order permutations are different ASTs even when the declarations have
  different keywords; validation may reject an order, but formatting cannot
  change it.

### What The JSON Form Buys

Producing or consuming a chart spec requires no Avenger code at all — a
JSON library suffices; the Rust library enters only to validate, compile,
or render. That is the foundation for the client-server protocol, notebook
widgets and editors that patch charts structurally, spec diffing, cache
keys — and any language can grow an authoring API without linking the
engine. Because the node set is closed, the format does not churn as the
language grows: new features are new keyword/kind strings validated by the
introspectable authoring schema, never new JSON shapes, so a fixed parser
written today reads every future chart. This is the deliberate inversion
of the Vega-Lite arrangement, where the JSON schema *is* the language and
every feature revs it along with every generated binding.

## Parser Architecture

A promising implementation strategy is tokenize-then-parse: tokenize the whole
DSL file with `sqlparser-rs`, then let the DSL parser borrow SQL parsing on
demand.

The workspace already depends on DataFusion 54 and `sqlparser` 0.62. The
dependency-light frontend's normative SQL-island entry point is sqlparser's
`Parser` under `AvengerSqlDialect`; DataFusion's `DFParser` is used only later
at the compiler/planning boundary. The useful frontend APIs are:

- `Tokenizer::new(&dialect, source).tokenize_with_location()` to produce
  `Vec<TokenWithSpan>`.
- `Parser` initialized with the existing token buffer rather than a second raw
  source parse.
- `Parser::parse_expr()` for expression slots and
  `Parser::parse_statement()` for full-statement `sql:` slots.
- The parser's consumed-token position to return the first unconsumed token to
  the enclosing DSL parser.

The outer parser can then keep a cursor into the shared token buffer:

```text
tokens = sqlparser_tokenize(source)
parse_declaration_block(tokens, cursor)
```

This means every DSL surface feature must first be valid input to the
sqlparser tokenizer using one shared `AvengerSqlDialect`. That dialect is
a compatibility-preserving specialization of `sqlparser-rs`'s
`GenericDialect`: it changes Avenger's comment behavior and explicitly retains
the Generic feature surface required by the frozen corpus, including
`supports_from_first_select() == true`. The strict compiler and language server
must call the same tokenization and SQL-island parsing entry points rather than
letting a host-selected `SessionContext` dialect reinterpret source. Parsed
sqlparser ASTs then enter DataFusion planning through the compiler's pinned
adapter. The Avenger parser can reinterpret token
sequences such as `chart cartesian as sales_by_category { ... }` as DSL
declarations, but it should not require a second lexer or syntax that
`sqlparser-rs` cannot tokenize. New punctuation, string forms, comments, and
sigils should be accepted only after checking how they tokenize under the
pinned dialect.

The Avenger dialect is a thin `Dialect` implementation over the stock
tokenizer. Its Avenger-specific comment hooks are
`requires_single_line_comment_whitespace()` (so `--` opens a comment only
before whitespace, keeping `amount--1` an expression) and
`supports_nested_comments()` (nested `/* ... */`). It must also preserve every
Generic-dialect capability claimed by the language, rather than falling back
to the `Dialect` trait's more restrictive defaults; `FROM`-first `SELECT` is a
mandatory conformance case. Doc comments need no hook at all: a `-- |` doc line
is an ordinary comment token whose content begins with `|`; the parser strips
the prefix and stores the joined block in the declaration's `doc` field. All
of these behaviors are pinned by the token and query conformance corpora.

For ordinary DSL structure, the parser matches `Token::Word`, braces, colons,
semicolons, brackets, strings, and numbers directly. Whitespace and comments
are `Token::Whitespace`, so the DSL can support the canonical `-- ...` and
`/* ... */` comment forms directly from the shared SQL token stream.

Expression-valued properties should not be parsed by scanning for the first
semicolon or brace. Instead, parse one SQL expression from the current token
position, advance the outer cursor by the SQL parser's consumed token count,
then inspect the next DSL token:

```text
x: amount { ... }
```

In this example, SQL consumes `amount`, then the DSL parser sees `{` and parses
the channel configuration block. If a SQL expression itself contains braces,
such as a dictionary/map literal accepted by the chosen SQL dialect, SQL
consumes those braces before the DSL parser resumes. This avoids making `{`
ambiguous by convention.

Full-statement SQL properties use the same idea:

```avenger
data: {
  sql:
    SELECT *
    FROM "customers.csv"
    WHERE amount > $min_amount;
}
```

After `sql:`, call `parse_statement()` from the current token position. The SQL
parser stops before the statement separator, and the DSL parser then requires
one semicolon to terminate the property. This lets SQL own semicolons inside
strings and comments while preserving the native, unquoted SQL feel. A v1
compiler should reject or clearly diagnose attempts to put multiple SQL
statements in one `sql:` property.

The statement parser must admit a first significant token of `FROM` as well as
`SELECT`, `WITH`, or `VALUES`. `sqlparser-rs` represents a complete
`FROM relation SELECT ...` query with an ordinary `Select` AST plus a
`FromFirst` flavor marker. Avenger passes the normalized `from` and
`projection` fields to the same DataFusion planner path as standard ordering;
it does not rewrite the source string. Query-only validation rejects
`FromFirstNoSelect` in language version 1.

The compiler corpus parses every accepted island through the frontend entry
point and then verifies that its canonical SQL is accepted and planned by the
pinned DataFusion adapter with the same expression/query shape. DataFusion may
add planning semantics, but it cannot silently define a second source grammar;
any frontend/planner acceptance mismatch is a release-blocking conformance
failure.

The schema-free structural parser chooses the value production from local
syntax and the reserved property-name contract: `sql:`/`query:` enter a query
island, a leading block/array/prefix enters its structural value, and remaining
expression positions enter an expression island. Authoring schemas validate
whether that parsed shape is legal for the containing declaration afterward;
they are not needed for parse/print round trips. Channel values, filter
predicates, calculate outputs, aggregate expressions, sort keys, and visibility
conditions are SQL expression slots. Selectors such as
`target: mark manual_box_plot.fence.outlier_layer.outliers;`, typed values such as
`scale: linear { ... }`, arrays of DSL names, and nested property objects are
DSL values.

Value bindings fit this model with a kind-neutral normalization. `sqlparser-rs`
tokenizes `$foo` as a placeholder. The DSL rejects positional placeholders such
as `$1`, `$2`, and `?`. For `$zoom.domain`, the
stock tokenizer produces a placeholder token for `$zoom`, followed by `.` and
the word `domain`. Under the Generic dialect, an adjacent temporal suffix such
as `@start` is one word token whose spelling includes `@`. Before an SQL island
is parsed, Avenger's token-normalization pass recognizes a named placeholder,
zero or more `. ident` pairs, and an optional exact `@start` or `@previous`
suffix. It records the full DSL path, temporal version, and source span, then
substitutes one unique opaque quoted identifier token. A quoted identifier is
legal in both scalar-expression and table-relation positions, so SQL parsing can
determine the occurrence's syntactic role without first resolving the binding
kind. A side table preserves the original path, version, and source span.

After SQL parsing, resolution follows the recorded path through the lexical
value-binding namespace and component exports. A scalar occurrence must resolve
to a param and is rewritten to `Expr::Placeholder`; its temporal version selects
the current, gesture-start, or previous-invocation event input. A relation
occurrence must be unqualified, resolve to a store, and is rewritten to that
store's registered relation. The resolved binding kind and compiler id then join
the side-table entry. A store used in a scalar position, a param used as a
relation, or any temporal store occurrence is a kind error at the original
`$path` span. Context validation separately enforces the event-only rules for
`@start` and `@previous`.

This is not a lexer extension: the original source still tokenizes completely
through `sqlparser-rs`, and `$foo.bar` has no competing valid meaning in an SQL
island. `@start` and `@previous` are reserved only as adjacent suffixes of a
complete `$path`; other `@` tokens retain their SQL meaning. Trivia around `.`
may be accepted, but the canonical printer emits no spaces around `.` or `@`.
Quoted identifiers and numeric tokens are never path segments.
Synthetic identifier spellings live only in the normalized token buffer, are
distinguished by a side table rather than a reserved source prefix, and never
appear in diagnostics, serialized SQL, or printed DSL.

Every other DSL-injected reference — `channel(x)`, `datum('id')`,
`event_coord(x)`, `item_channel(x)`, and the rest of the reserved helper
namespace — parses as an ordinary SQL function call and is rewritten by the
resolver before DataFusion planning. No lexer extensions are required beyond
what the SQL tokenizer already produces, which is also why the
mandatory column-quoting rule costs nothing at this layer: quoted identifiers
and bare compound identifiers are both native SQL, and the resolver simply
assigns them to the data and DSL namespaces respectively.

Because the surface language is defined over a third-party tokenizer, the
token classes the DSL relies on (words, quoted identifiers, strings, numbers,
placeholders, punctuation, comments) must be specified normatively and pinned
by a conformance corpus of golden token streams. DataFusion and `sqlparser`
upgrades run against that corpus, so an engine upgrade can never silently
change the language. The corpus includes `$name`, `$component.alias`, deeper
paths, `@start`/`@previous`, cast and JSON-access composition, trivia around `.`,
and rejected whitespace before `@` and quoted/numeric path segments, alongside
the normalized synthetic-token side-table expectations.

`TokenWithSpan` carries line and column spans. The frontend maintains a
line/column-to-byte-offset index alongside the shared token buffer, so every
token also has an exact half-open UTF-8 byte span for diagnostics, source maps,
and lossless binding side-table entries.

## Zed Editor Support

Start editor support with Zed only. Zed wants a Tree-sitter grammar and query
files, so the first editor artifact should be a `tree-sitter-avenger` grammar
that mirrors the DSL surface syntax closely enough for highlighting,
indentation, bracket matching, and SQL injections.

The Tree-sitter grammar is not the compiler parser. The compiler should still
use the `sqlparser-rs` tokenize-then-parse strategy described above. The
Tree-sitter grammar can be more tolerant so partially typed charts still
highlight well.

Initial Zed extension shape:

```text
tree-sitter-avenger/
  grammar.js
  queries/
    highlights.scm
    injections.scm
    brackets.scm
    indents.scm

tree-sitter-avenger-sql/
  grammar.js
  queries/
    highlights.scm

avenger-zed/
  extension.toml
  languages/
    avenger/
      config.toml
      highlights.scm
      injections.scm
      brackets.scm
      indents.scm
      outline.scm
      runnables.scm
      tasks.json
    avenger-sql/
      highlights.scm
```

The SQL grammar can start as an adapted Tree-sitter SQL grammar rather than a
from-scratch grammar. The adaptation should match the SQL accepted by
DataFusion closely enough for highlighting, while also accepting Avenger's SQL
island shapes:

- Full SQL statements after `sql:`, including standard `SELECT`,
  `FROM relation SELECT ...`, set operations, and `VALUES`. `FROM relation`
  without an explicit `SELECT` remains editor recovery syntax, not valid v1
  source.
- SQL expression fragments in channel values, filters, transform outputs,
  sort keys, visibility conditions, and event filters.
- Named bindings such as `$min_amount`, including the temporal param suffixes
  `$width@start` and `$width@previous`.
- Reserved helper functions such as `channel(...)` and `datum(...)`,
  highlighted as functions (no dedicated tokens are needed).
- SQL comments using `-- ...` and `/* ... */`.

This likely means the injected SQL language should be named something like
`avenger-sql` rather than plain `sql`. A stock SQL grammar may highlight full
`SELECT` statements well, but it may not accept a top-level expression fragment
like `"amount" >= $min_amount and "region" = $selected_region`. The adapted
grammar can have a tolerant top-level rule such as:

```javascript
source_file: $ => repeat(choice(
  $.statement,
  $.expression,
  $.binding_ref,
))
```

The adapted SQL grammar is for editor highlighting only. The compiler remains
the source of truth: sqlparser under `AvengerSqlDialect` parses the frontend
islands, and DataFusion receives them only at the planning boundary.

The SQL grammar must give `FROM`-first queries the same stable relation,
alias, projection, and clause nodes as standard ordering wherever the adapted
upstream permits. Highlight and recovery fixtures must include an incomplete
`FROM vega.movies AS m SELECT m.` because that is the authoring shape that
motivates the syntax guarantee.

The first grammar can parse declarations, property blocks, SQL expression
islands, full-statement `sql:` properties, comments, strings, params, and
channel references. The excerpt below is schematic: it names root/import/data
and body rules omitted for space. The peer Tree-sitter/Zed implementation plan
owns the complete stable node contract and corpus. A sketch:

```javascript
module.exports = grammar({
  name: "avenger",

  extras: $ => [
    /\s/,
    $.comment,
  ],

  word: $ => $.identifier,

  externals: $ => [
    $.block_comment,
    $._sql_expression,
    $._sql_statement,
  ],

  rules: {
    source_file: $ => seq(
      $.version_directive,
      repeat($.import_statement),
      choice($.chart_declaration, $.definition_root, $.data_root),
    ),

    version_directive: $ => seq("avenger", $.number, ";"),

    definition_root: $ => seq(
      "define",
      choice("mark", "tool", "transform"),
      field("name", $.identifier),
      $.definition_block,
    ),

    declaration: $ => seq(
      optional($.visibility_modifier),
      choice(
        $.chart_declaration,
        $.group_declaration,
        $.mark_declaration,
        $.transform_declaration,
        $.param_declaration,
        $.store_declaration,
        $.selection_declaration,
        $.tool_declaration,
        $.widget_declaration,
        $.view_declaration,
        $.event_declaration,
        $.cell_declaration,
        $.export_declaration,
        $.output_declaration,
        $.config_declaration,
      ),
    ),

    visibility_modifier: $ => choice("private", "public"),

    export_declaration: $ => seq(
      "export",
      field("source", $.qualified_name),
      optional($.as_clause),
      ";",
    ),

    output_declaration: $ => seq(
      "output",
      field("name", $.identifier),
      optional(seq(":", field("source", $.sql_expression))),
      ";",
    ),

    chart_declaration: $ => seq(
      "chart",
      field("coordinate", $.identifier),
      optional($.as_clause),
      $.declaration_block,
    ),

    group_declaration: $ => seq(
      "group",
      optional($.as_clause),
      $.mixed_block,
    ),

    mark_declaration: $ => seq(
      "mark",
      field("kind", $.identifier),
      optional($.as_clause),
      $.mixed_block,
    ),

    transform_declaration: $ => seq(
      "transform",
      field("kind", $.identifier),
      optional($.as_clause),
      $.mixed_block,
    ),

    param_declaration: $ => seq("param", $.as_clause, $.property_block),

    store_declaration: $ => seq("store", $.as_clause, $.mixed_block),

    selection_declaration: $ => seq(
      "selection",
      $.as_clause,
      $.property_block,
    ),

    tool_declaration: $ => seq(
      "tool",
      field("kind", $.identifier),
      optional($.as_clause),
      choice($.mixed_block, ";"),
    ),

    widget_declaration: $ => seq(
      "widget",
      field("kind", $.identifier),
      $.as_clause,
      $.property_block,
    ),

    view_declaration: $ => seq(
      "view",
      field("kind", $.identifier),
      optional($.as_clause),
      $.mixed_block,
    ),

    event_declaration: $ => seq(
      "on",
      field("event", $.identifier),
      optional($.as_clause),
      $.property_block,
    ),

    cell_declaration: $ => seq(
      "cell",
      field("coordinate", $.identifier),
      optional($.as_clause),
      optional(seq("at", $.property_block)),
      $.declaration_block,
    ),

    config_declaration: $ => seq(
      field("name", $.identifier),
      $.property_block,
    ),

    as_clause: $ => seq("as", field("name", $.identifier)),

    qualified_name: $ => seq(
      $.identifier,
      repeat(seq(".", $.identifier)),
    ),

    declaration_block: $ => seq("{", repeat($.declaration), "}"),

    mixed_block: $ => seq(
      "{",
      repeat(choice($.declaration, $.property)),
      "}",
    ),

    property_block: $ => seq("{", repeat($.property), "}"),

    property: $ => seq(
      field("name", $.identifier),
      ":",
      field("value", choice(
        $.object_value,
        $.array,
        $.typed_block_value,
        $.sql_statement_value,
        $.sql_expression_value,
      )),
    ),

    object_value: $ => $.property_block,

    typed_block_value: $ => seq(
      field("type", $.identifier),
      $.property_block,
    ),

    sql_expression_value: $ => seq(
      $.sql_expression,
      optional($.property_block),
      optional(";"),
    ),

    sql_statement_value: $ => seq($.sql_statement, ";"),

    sql_expression: $ => $._sql_expression,
    sql_statement: $ => $._sql_statement,

    array: $ => seq(
      "[",
      optional(seq($.array_value, repeat(seq(",", $.array_value)), optional(","))),
      "]",
    ),

    array_value: $ => choice(
      $.string,
      $.number,
      $.boolean,
      $.binding_ref,
      $.identifier,
    ),

    binding_ref: $ => token(/\$[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*(@(start|previous))?/),
    identifier: $ => /[A-Za-z_][A-Za-z0-9_]*/,
    number: $ => /-?([0-9]+(\.[0-9]*)?|\.[0-9]+)([eE][+-]?[0-9]+)?/,
    string: $ => /'([^'\\]|\\.)*'/,
    column: $ => /"([^"\\]|\\.)*"/,
    boolean: $ => choice("true", "false"),
    comment: $ => choice($.line_comment, $.block_comment),
    line_comment: $ => token(seq("--", /[ \t][^\n\r]*/)),
  },
});
```

The sketch uses external tokens for `sql_expression` and `sql_statement`
because those boundaries are the hard part. V1 uses a delimiter-aware external
scanner that balances parentheses/brackets and SQL lexical states and stops
before the DSL semicolon or channel/config block. It does not reimplement
sqlparser semantics; the injected adapted SQL grammar parses the contained
tokens, while compiler conformance remains authoritative.

The same external scanner should handle nested `block_comment` tokens because
Tree-sitter regex tokens are not a good fit for nested `/* ... */` comments.
Line comments can remain a simple regex token that requires whitespace after
`--`.

`sql_statement_value` should only be used for the `sql` property. Tree-sitter
cannot easily enforce property-specific schemas in the grammar alone, so the
highlight queries should match `property` nodes with name `sql` and treat their
value as a statement. Other expression-like property values can be treated as
SQL expressions for highlighting.

Initial `highlights.scm` sketch:

```scheme
[
  "chart"
  "group"
  "mark"
  "transform"
  "define"
  "catalog"
  "schema"
  "table"
  "param"
  "store"
  "selection"
  "tool"
  "widget"
  "view"
  "on"
  "cell"
  "as"
  "private"
  "public"
  "export"
  "output"
] @keyword

(comment) @comment
(string) @string
(number) @number
(boolean) @boolean

(binding_ref) @variable.parameter

(property name: (identifier) @property)
(as_clause name: (identifier) @label)

(chart_declaration coordinate: (identifier) @type)
(cell_declaration coordinate: (identifier) @type)
(mark_declaration kind: (identifier) @type)
(transform_declaration kind: (identifier) @function)
(tool_declaration kind: (identifier) @type)
(widget_declaration kind: (identifier) @type)
(view_declaration kind: (identifier) @type)

[
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

[
  ":"
  ";"
  ","
] @punctuation.delimiter
```

Initial `injections.scm` sketch:

```scheme
((sql_statement) @injection.content
 (#set! injection.language "avenger-sql"))

((sql_expression) @injection.content
 (#set! injection.language "avenger-sql"))
```

If SQL injection over-highlights non-SQL DSL values, narrow the query by
property name for the first pass:

```scheme
((property
  name: (identifier) @_name
  value: (sql_statement_value (sql_statement) @injection.content))
 (#eq? @_name "sql")
 (#set! injection.language "avenger-sql"))
```

The adapted SQL `highlights.scm` should use ordinary SQL captures for keywords,
operators, functions, identifiers, strings, numbers, and comments, plus an
Avenger capture for value-binding references:

```scheme
(binding_ref) @variable.parameter
```

Initial `brackets.scm` and `indents.scm` can be small:

```scheme
("{" @open "}" @close)
("[" @open "]" @close)
```

```scheme
(declaration_block "}" @end) @indent
(property_block "}" @end) @indent
(array "]" @end) @indent
```

The first extension associates only `.avenger` with the `avenger` language and
configures comment toggling for
`--` and `/* ... */`.

## VS Code And Web Highlighting

VS Code and the web editor have different packaging surfaces, but they do
not need different Avenger highlighting semantics. Zed uses Tree-sitter
directly; VS Code's fast syntax layer is TextMate grammar based; the web
editor and all embedded editors use CodeMirror 6, whose Lezer parser is
Tree-sitter's close kin. The shared strategy:

1. Define a common token taxonomy for Avenger and Avenger SQL.
2. Implement a TextMate grammar for VS Code instant highlighting.
3. Use CodeMirror 6 with a Lezer grammar for the web editor and every
   embedded editor (docs pages, notebooks); SQL islands delegate to the
   Lezer SQL grammar through `parseMixed`.
4. Use LSP semantic tokens later as the precise layer everywhere.

The baseline VS Code extension shape:

```text
vscode-avenger/
  package.json
  language-configuration.json
  syntaxes/
    avenger.tmLanguage.json
    avenger-sql.tmLanguage.json
  snippets/
    avenger.code-snippets
```

`package.json` should contribute two language ids:

- `avenger` for `.avenger` files.
- `avenger-sql` for injected SQL expression and statement scopes.

The top-level TextMate scope names should be:

```text
source.avenger
source.avenger-sql
```

`language-configuration.json` should configure:

```json
{
  "comments": {
    "lineComment": "--",
    "blockComment": ["/*", "*/"]
  },
  "brackets": [["{", "}"], ["[", "]"], ["(", ")"]],
  "autoClosingPairs": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
    ["\"", "\""]
  ],
  "surroundingPairs": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
    ["\"", "\""]
  ]
}
```

The TextMate grammar should be deliberately lexical. It should provide useful
color immediately, but it should not try to be the compiler parser. The LSP can
correct and enrich it with semantic tokens.

Initial `avenger.tmLanguage.json` responsibilities:

- Highlight declaration keywords: `chart`, `group`, `mark`, `transform`,
  `param`, `store`, `selection`, `tool`, `view`, `on`, `facet`, `cell`, `as`, plus the
  visibility/interface words `private`, `public`, and `export`.
  `output` is likewise a declaration keyword in definition and pipeline
  interfaces.
- Highlight declaration kinds: coordinate kinds, mark kinds, transform kinds,
  selection/tool/view kinds.
- Highlight property names before `:`.
- Highlight comments, strings, numbers, booleans, braces, brackets,
  semicolons, and commas.
- Highlight `$binding` reads wherever they appear, with semantic coloring
  distinguishing scalar params from table stores.
- Scope `sql:` values as `source.avenger-sql`.
- Scope known SQL expression slots as `source.avenger-sql` until a semicolon,
  channel configuration block, or likely next property.

A TextMate sketch:

```json
{
  "scopeName": "source.avenger",
  "patterns": [
    { "include": "#comments" },
    { "include": "#strings" },
    { "include": "#sql-statement-property" },
    { "include": "#sql-expression-property" },
    { "include": "#declarations" },
    { "include": "#properties" },
    { "include": "#refs" },
    { "include": "#literals" },
    { "include": "#punctuation" }
  ],
  "repository": {
    "comments": {
      "patterns": [
        { "name": "comment.line.double-dash.avenger", "match": "--[ \\t].*$" },
        { "name": "comment.block.avenger", "begin": "/\\*", "end": "\\*/" }
      ]
    },
    "strings": {
      "patterns": [
        { "name": "string.quoted.single.avenger", "begin": "'", "end": "'" },
        { "name": "variable.other.column.avenger", "begin": "\"", "end": "\"" }
      ]
    },
    "declarations": {
      "patterns": [
        {
          "name": "keyword.control.declaration.avenger",
          "match": "\\b(chart|group|mark|transform|data|param|store|selection|tool|view|on|facet|cell|as|private|public|export|output)\\b"
        }
      ]
    },
    "properties": {
      "patterns": [
        {
          "name": "variable.other.property.avenger",
          "match": "\\b[A-Za-z_][A-Za-z0-9_]*(?=\\s*:)"
        }
      ]
    },
    "refs": {
      "patterns": [
        { "name": "variable.parameter.avenger", "match": "\\$[A-Za-z_][A-Za-z0-9_]*" }
      ]
    },
    "sql-statement-property": {
      "patterns": [
        {
          "begin": "\\b(sql)(\\s*:)",
          "beginCaptures": {
            "1": { "name": "variable.other.property.avenger" },
            "2": { "name": "punctuation.separator.key-value.avenger" }
          },
          "end": ";",
          "patterns": [{ "include": "source.avenger-sql" }]
        }
      ]
    },
    "sql-expression-property": {
      "patterns": [
        {
          "begin": "\\b(x|x2|y|y2|fill|stroke|opacity|visible|predicate|filter|order_by|sort_by|domain|enabled)\\b(\\s*:)",
          "beginCaptures": {
            "1": { "name": "variable.other.property.avenger" },
            "2": { "name": "punctuation.separator.key-value.avenger" }
          },
          "end": "(?=;|\\{|\\}|\\n\\s*[A-Za-z_][A-Za-z0-9_]*\\s*:)",
          "patterns": [{ "include": "source.avenger-sql" }]
        }
      ]
    }
  }
}
```

This grammar intentionally approximates SQL expression boundaries. TextMate
cannot reliably balance arbitrary SQL expressions, channel config blocks, and
DSL recovery cases. That is acceptable for the first colorization layer.

`avenger-sql.tmLanguage.json` should be an adapted SQL grammar, not just a
generic full-statement SQL grammar. It should support both full statements and
top-level expression fragments. It should highlight:

- SQL keywords, operators, functions, identifiers, strings, numbers, comments.
- `$binding` as a parameter token, including `@start`/`@previous`; semantic
  analysis distinguishes params and stores and highlights the temporal suffix
  as a modifier.
- Reserved helper functions (`channel`, `datum`, `event_coord`, ...) as
  ordinary SQL functions, optionally with a distinct capture.
- DataFusion-oriented function names such as `approx_percentile_cont`, `date_bin`,
  `regexp_match`, and nested/struct functions as ordinary SQL functions.

The web editor uses CodeMirror 6, and the decisive fit is Lezer's
mixed-language parsing: a small Lezer grammar parses the DSL shell, and
`parseMixed` delegates expression and statement islands to the Lezer SQL
grammar — the same island architecture the compiler uses, incrementally
parsed, with no second hand-maintained tokenizer. CodeMirror's modular core
keeps the editor at a fraction of Monaco's size (matching the two-bundle
load choreography), works on touch devices, and is where sophisticated web
IDEs have converged (JupyterLab 4, Chrome DevTools, Replit).

```text
codemirror-avenger/
  avenger.grammar        -- Lezer grammar for the DSL shell
  sql-islands.ts         -- parseMixed wiring to @codemirror/lang-sql's Lezer grammar
  providers.ts           -- lint/autocomplete/hover/semantic-decoration sources
                            backed by the avenger-lang worker RPC
```

Editor intelligence wires to `avenger-lang-analysis.wasm` through CodeMirror's native
extension points — lint sources for diagnostics, autocomplete sources,
`hoverTooltip`, and decorations for semantic tokens — over the same
editor-specific RPC the language service exposes. The Lezer grammar is for
keystroke-latency highlighting and structure only; the worker remains the
semantic source of truth.

Semantic highlighting should be layered on top of these lexical grammars. The
LSP semantic token legend should use mostly standard token types so themes work
well:

```text
namespace      chart/group paths
type           mark kinds, coordinate kinds, scale kinds
function       transform kinds and SQL functions
property       DSL properties and transform output fields
parameter      $bindings (scalar params and table stores)
variable       data columns and aliases
enumMember     channels, event names, enum-like option values
operator       SQL and DSL operators where supplied semantically
```

Useful modifiers:

```text
declaration    declarations introduced by `as`
readonly       generated transform fields
defaultLibrary built-in mark/transform/property names
deprecated     deprecated properties or transforms
modification   param/store updates in event handlers
```

The editor stack should therefore be:

```text
VS Code:
  TextMate grammar now
  LSP semantic tokens later

Web (editor site, docs embeds, notebooks):
  CodeMirror 6 + Lezer grammar, SQL islands via parseMixed
  avenger-lang-analysis.wasm providers (lint/complete/hover/semantic decorations)

Compiler/LSP semantics:
  avenger-lang + sqlparser-rs, not TextMate or Lezer
```

## Online Editor

An online editor in the style of the Vega editor should be able to run entirely
client-side. The recommended architecture is to compile the `avenger-lang`
analysis core to WebAssembly and run it inside a Web Worker, then connect
CodeMirror 6 to that worker through its native extension points (see
[VS Code And Web Highlighting](#vs-code-and-web-highlighting) for the
editor-technology decision).

The browser shape:

```text
browser main thread
  CodeMirror 6 editor
  chart preview
  diagnostics panel
  examples/data panels

web worker
  avenger-lang-analysis.wasm
  document store
  LSP-ish request router

Rust/Wasm core
  sqlparser-rs tokenization
  tolerant DSL parse
  SQL island parse
  name/scope resolution
  diagnostics, completions, hover, symbols, semantic tokens
```

Compile the analysis core to Wasm, not a traditional stdio language server
unchanged. Normal LSP servers often assume process I/O, stdio, filesystem
access, sockets, or a Tokio runtime. Browser Wasm works best when I/O is
factored out and the host passes document text, schema metadata, and example
data into pure analysis APIs.

The crate split should support both native and browser deployments while
retaining the language/compiler plan's dependency boundaries:

```text
avenger-lang-core
  Dependency-light parser, AST, resolver, schemas, spans, and source maps.
  Native + wasm target; no filesystem, stdio, socket, process, DataFusion,
  chart-rendering, or GPU assumptions.

avenger-lang-compiler
  Native async project loading plus DataFusion logical analysis and chart
  lowering. Exposes ProjectAnalysis and compiled artifacts.

avenger-lang
  Public facade over core/compiler features.

avenger-lang-analysis                 # added with the later LSP milestone
  Tolerant editor tree and complete/hover/symbol/semantic-token APIs.
  Consumes DatasetSchemaIndex as data; native + wasm target.

avenger-lsp-native
  Desktop/server adapter.
  stdio or socket LSP transport.
  Uses avenger-lang-analysis plus avenger-lang-compiler project analysis and
  the optional running-chart inspection client.

avenger-lsp-wasm
  Browser adapter.
  Web Worker message transport.
  Calls avenger-lang-analysis.wasm with host-supplied source/schema data.
  Can expose real LSP JSON-RPC or a smaller editor-specific RPC.
```

The editor integration has two viable levels:

- Full LSP-style integration using a browser language client and
  worker-hosted server. This maximizes reuse with VS Code/Zed-style LSP
  clients.
- Direct CodeMirror sources backed by `avenger-lang-analysis.wasm`: lint,
  autocomplete, hover tooltip, and semantic-token decorations registered as
  ordinary extensions. This is simpler for the first online editor if
  multi-editor LSP reuse is not yet needed.

The full editor is a **two-bundle architecture** with opposite constraints,
and the repository's existing wasm-pack browser examples have already
de-risked the heavy half (the complete chart stack — DataFusion, typst
labels, wgpu rendering — runs under Wasm today):

```text
avenger-lang-analysis.wasm  small, instant-loading (worker #1)
  tokenizer + tolerant parser + resolver + generated schema
  optional bundled std: definition sources (versioned text)
  diagnostics, completions, hover, semantic tokens, formatter
  source-level expansion/refactorings, sha256 verification
  -- no DataFusion execution, no GPU; consumes a serialized dataset-schema index

avenger-runtime.wasm     heavy, lazy-loaded (worker #2 + OffscreenCanvas)
  chart compile (lowering -> CompiledPlot) + DataFusion execution
  scales, layout, scenegraph, PlotSession (events, tools, views,
  preview evaluation and retarget_cached previews)
  avenger-wgpu (WebGPU/WebGL2), the Typst-based avenger-text stack
```

The load choreography is the point of the split: the language service gives
full editor intelligence immediately while the runtime streams in; the
first preview renders when it arrives, and diagnostics never wait on it.
The JS host owns everything capability-shaped — fetch (imports, tiles,
remote data, with CORS as the browser's capability boundary), OPFS/memory
for uploaded CSV/parquet feeding DataFusion, fonts, and worker transports;
Wasm does no I/O. For data too large to bring client-side, the runtime
bundle keeps interaction and rendering local while pushing plans to a
server per
[client-server-architecture.md](client-server-architecture.md) — the same
`PlotSession` seam, only the execution location changes.

The native analyzer obtains dataset schemas by using DataFusion itself. The
small browser analyzer need not embed the full DataFusion/catalog stack: the
runtime worker or host runs the same project-analysis contract and sends its
serialized schema index across the worker boundary as plain data:

```typescript
type DataSourceSchema = {
  name: string;
  columns: Array<{ name: string; dataType: string; nullable: boolean }>;
};
```

The online editor can then provide column completions and type diagnostics
without reading files or executing queries in the worker.

## DataFusion Evaluation Cache

Evaluation caching is not part of the DSL syntax itself, but it is an important
runtime goal for interactive chart authoring and ordinary chart sessions with
changing params, stores, selections, and view domains. The dedicated
physical-plan-first design is tracked in
[physical-plan-evaluation-cache.md](physical-plan-evaluation-cache.md).

## Language Server

An `avenger-lang` language server should use the compiler-oriented
tokenize-then-parse architecture, not the Tree-sitter grammar, as its semantic
source of truth. Tree-sitter makes editing feel good in Zed: syntax
highlighting, indentation, bracket matching, and SQL injections. The language
server should answer semantic questions using the same parser, SQL parsing, and
resolver that will eventually lower to `avenger-chart`.

A shared source/crate layout should build on the compiler split rather than
moving lowering back into the dependency-light frontend:

```text
avenger-lang-core/
  lexer.rs        # sqlparser-rs tokenization and source maps
  syntax.rs       # lossless CST, trivia, recovery nodes, source anchors
  parser.rs       # Avenger token cursor parser + semantic projection
  ast.rs          # strict semantic DSL AST
  sql.rs          # SQL island parsing via sqlparser-rs with the shared dialect
  resolver.rs     # names, scopes, value bindings, aliases, event paths
  diagnostics.rs

avenger-lang-compiler/
  analysis.rs     # DataFusion logical planning -> DatasetSchemaIndex/lineage
  lowering.rs     # strict resolved project -> avenger-chart authoring model

avenger-lang-analysis/       # introduced by the future LSP plan
  completion.rs
  hover.rs
  symbols.rs
  semantic_tokens.rs

avenger-lsp-native/
  main.rs         # stdio/socket transport + compiler/inspector adapters

avenger-lsp-wasm/
  lib.rs          # Web Worker transport over dependency-light analysis
```

The parser should support two modes:

- Strict mode for compiling, tests, and save/build validation.
- Tolerant mode for live LSP analysis while the user is mid-edit.

Tolerant mode is not a different language, but it does not manufacture an
invalid strict AST. It always produces the lossless CST, whose recovery nodes,
synthesized missing delimiters, and diagnostics retain enough structure for
editor features. A best-effort analysis projection may mirror portions of the
semantic tree, but it is a distinct non-serializable `AnalysisTree`; only a
source with no blocking syntax or SQL errors produces the strict `File` AST
accepted by resolution, canonical interchange, hashing, or lowering.

Tolerant mode can still start with `sqlparser-rs` tokenization. The language
server should try normal tokenization first. If tokenization fails because of an
unterminated string, unterminated block comment, or other lexical error, it can
keep the tokens emitted before the error, attach a lexical diagnostic, and
resume from a recovery point if one can be found.

The Avenger parser should recover at DSL sync points:

- In declaration blocks, sync at declaration starters such as `chart`, `group`,
  `mark`, `transform`, `param`, `store`, `selection`, `tool`, `view`, `on`,
  `facet`, `cell`, `private`, `public`, `export`, `output`, or at `}`.
- In property blocks, sync at a plausible `identifier:` pair or at `}`.
- If a property semicolon is missing before the next `identifier:`, synthesize
  the missing semicolon and emit a diagnostic.
- If a block is missing `}`, synthesize a closing brace at EOF and emit a
  diagnostic.
- If a declaration header is malformed, keep an `error_declaration` node and
  continue parsing the next declaration.

SQL islands should be parsed strictly when possible, then recovered locally if
they fail:

```text
x: amount + ;
```

For this example, the CST and analysis tree keep the surrounding mark and
property plus an `SqlError` recovery node containing the raw token range for
`amount +`, and report the SQL diagnostic on that span. `SqlError` is not a
`Value` variant in the strict AST, cannot serialize to interchange JSON, and
cannot reach resolution or lowering.

SQL island recovery boundaries depend on the slot:

- Expression property: stop at a top-level `;`, top-level `{`, `}`, EOF, or a
  plausible next `identifier:`.
- Channel value: stop at a top-level `{` so `x: amount { ... }` still separates
  the SQL value from channel configuration.
- Full-statement `sql:` property: stop at the statement semicolon, `}`, or EOF.

The analysis pipeline should be:

1. Tokenize with `sqlparser-rs`, preserving spans.
2. Parse the DSL token stream in tolerant or strict mode.
3. Normalize each qualified binding token sequence, including an adjacent
   `@start` or `@previous`, inside an SQL island to a unique kind-neutral quoted
   identifier, retaining a synthetic-token-to-path/version/source-span side
   table.
4. Parse SQL expression and statement islands with sqlparser under
   `AvengerSqlDialect`; use DataFusion only for subsequent native planning.
5. Resolve names, scopes, value-binding paths, transform aliases, event targets,
   views, tools, selections, stores, and channel references; use each binding's
   SQL AST role to rewrite params to placeholders and stores to relations.
6. In native analysis, register project catalogs and schema-only state
   relations in an isolated DataFusion `SessionContext`; logically plan the
   dataset/view/transform DAG in dependency order and retain exact output
   schemas for every named dataset and pipeline stage without physical
   planning or execution. Browser analysis consumes the serialized form of
   the same schema index.
7. Produce and lower the strict semantic AST only when no blocking CST,
   schema, or SQL error remains. Tolerant recovery nodes never lower.

### DataFusion-backed schema analysis

Column analysis is required language-server infrastructure, not an optional
runtime enhancement. The native LSP calls the same execution-free project
analysis API used by compilation. That API returns stable dataset/stage IDs,
source locations, lineage, and exact Arrow schemas derived from DataFusion
logical plans. It registers file/catalog providers, typed stores as
schema-only relations, and preceding logical views; then reads the plan or
`DataFrame` output `DFSchema`. It never calls `collect()` merely to serve an
editor request.

The resulting schema index drives both SQL and DSL completion:

- SQL completion observes CTE, subquery, relation, and alias scope and offers
  qualified columns, including struct fields and the output of preceding
  views.
- Encoding and transform-expression completion uses the schema at that exact
  pipeline stage, so columns added or removed by an earlier transform appear
  correctly.
- Ambiguous joins offer qualified names rather than guessing, and completion
  items carry physical Arrow type and nullability.
- Custom table providers and transforms must expose a planning-time schema or
  logical-plan contract; a runtime-only schema is reported as unavailable
  rather than guessed.

DataFusion is the schema and type authority, but it is not an autocomplete
engine. Completion is requested while source is commonly invalid. The LSP
uses its last successful project-analysis snapshot plus the tolerant syntax
tree to identify cursor scope. For SQL it may replace the cursor with a unique
sentinel identifier, parse the patched island, and inspect the resulting AST
without trying to plan the nonexistent sentinel column. `FROM`-first syntax
makes the common `FROM vega.movies AS m SELECT m.` case especially reliable
because the relation scope precedes the incomplete projection.

The detailed incomplete-input recovery, query-scope, candidate, ranking,
caching, and test design—and the prior-art survey behind it—is tracked in
[sql-completion.md](sql-completion.md).

Each analysis generation owns an isolated `SessionContext` and immutable
provider snapshot. Cache parsed sources, provider metadata, logical plans, and
schema indexes by dependency fingerprint; debounce and cancel superseded
editor generations. Do not share or mutate the watch process's current
`SessionContext`.

### Optional live-session enrichment

A native LSP may also connect read-only to a matching running chart through the
versioned Avenger inspection protocol exposed by `avenger watch`. This is an
overlay on static analysis, never a prerequisite for it. Stable source/runtime
IDs let the LSP associate committed runtime objects with declarations and
provide:

- current scalar values, row/selection counts, and scoped-instance summaries
  as refreshed inlay hints;
- bounded table previews, runtime schemas, lineage, last-update revision, and
  trigger details in hover;
- source-mapped execution, provider, event-handler, and hot-reload migration
  diagnostics;
- editor actions such as **Open in Inspector**, snapshot, preview, or adopt a
  current scalar value as an authored default.

Editors that implement LSP CodeLens can show **Open table/store in Inspector**
above the declaration. The command asks the LSP to launch
`avenger inspect --session <opaque-session-id> --object <opaque-object-id>`
directly, without a shell. In Zed, CodeLens is user-configurable and off by
default, so the same command should remain reachable through the code-action
menu. Inlay hints remain compact and read-only; table contents belong in the
separate inspector window.

All runtime results carry session ID, reload generation, and committed
revision. Updates are coalesced to avoid flicker, scoped values never collapse
misleadingly to one value, and live LSP hints are suppressed while DAP owns a
paused debug view. The planned DataFusion schema remains available when no
watch session exists. If the planned and observed schemas differ, hover shows
both and the LSP emits a schema-drift diagnostic rather than silently replacing
the static result.

The LSP can offer:

- Diagnostics for unknown declaration kinds, mark kinds, transform kinds,
  properties, channels, params, stores, transform aliases, event targets, duplicate
  names, invalid or redundant visibility modifiers, public-path/export
  collisions, missing required tool or output-bearing transform binders,
  unknown/anonymous transform handle namespaces, invalid `scale_edit`
  targets, typed-reference kind mismatches, private cross-boundary state access,
  forward references to sequential aliases/columns, value-binding collisions,
  scalar/table binding kind mismatches, invalid temporal binding qualifiers or
  contexts, param-default and table
  dependency cycles, later-slot default references, invalid block modes, group
  transform ordering violations, SQL parse errors, and invalid placeholders
  such as `$1` or `?`.
- Completion for declaration keywords, mark kinds, transform kinds, property
  names, channel names, scale and guide options, lexical and qualified
  `$binding` paths with scalar/table type information, typed state paths,
  reserved helper functions, predeclared forward event/structural/state
  targets, currently visible transform aliases, and DataFusion-derived columns
  in SQL queries, encoding expressions, and transform expressions at their
  exact pipeline stage.
- Hover for marks, transforms, properties, params, stores, data columns, SQL expression
  types, aliases, selections, tools, widgets, and event paths — with doc comments as
  the content for definitions (slots, parts, outputs, `match` arms; mode-value
  completion shows the arm's doc) and schema docs for native built-ins at the same
  granularity, down to channel option suffixes and enum values.
- Go to definition and references for `as` bindings, lexical and qualified
  `$binding` paths, typed state paths, transform alias
  fields such as `stats.median`, inline view binders, tools, widgets, selections, and mark paths such
  as `manual_box_plot.fence.outlier_layer.outliers`.
- Document symbols for charts, groups, marks, transforms, params, stores,
  views, tools, widgets, selections, and events.
- Semantic tokens for precise highlighting beyond Tree-sitter, especially for
  resolved scalar/table bindings, aliases, generated fields, and unknown or deprecated names.
- Formatting for DSL structure and valid SQL islands through the canonical
  printer/unparser, with CST-anchored DSL and SQL comments reinserted at stable
  normalized positions. Invalid islands remain byte-preserved until repaired.
- Code actions such as add missing param declaration, add missing `as` binding,
  qualify an ambiguous alias field, or convert an unknown property into a
  suggested known property.
- Flattening as refactorings, at finer granularity than the CLI command:
  **inline definition** (replace one imported mark, tool, or transform
  instantiation with its ordinary group, `tool behavior`, or `transform
  pipeline` expansion — slots substituted, `match` resolved, channels renamed — the
  "eject from the library" workflow and the fastest way to see what a
  construct lowers to), the inverse **extract definition** (select a group,
  create a new definition file with slots inferred from the selection's
  free names, replace the selection with an instantiation, and add the
  import), and **pin import** (fetch an unpinned URL import and insert its
  `sha256`, the quickfix paired with the dev-mode warning).
- Import authoring is completion-driven: `std:` and project-relative paths
  complete inside import strings, and pasting a URL triggers
  fetch-verify-and-pin — inserting the hash and offering the fetched
  definition's name for the `as` clause — so hashless imports are transient
  editing states rather than a workflow. `avenger add <url>` is the
  command-line equivalent.

## Authoring Schema Source

The authoring schema is the normative semantic inventory of language major
version 1. The semantic validator, resolver, lowerer, language server,
documentation generator, CLI introspection, and full interchange validator
share it as their source of truth for declaration placement, properties, value
shapes, channels, transform output handles, parts, enum values, required
properties, defaults, and docs. The structural parser and generic AST are
deliberately independent of native-kind inventory. The generated full JSON
Schema is a derived validator over the AST; it is not the authoring schema's
source.

For native kinds, the in-process registry pairs each normative schema entry
with an erased Rust lowerer for that same kind and coordinate context. The
serializable schema crate remains dependency-light and contains metadata only;
the facade-side lowering registry owns constructors for marks, transforms,
tools, widgets, coordinates, scales, guides, and other native objects. Registry
construction rejects duplicate entries, and CI checks schema channel/property
inventories against the Rust implementations. A DSL frontend therefore cannot
maintain a second native-kind switch whose behavior drifts from schema,
documentation, or Rust authoring.

The native registry is an injected, immutable host capability. The stock
language facade provides the canonical built-in registry; a third-party Rust
host may use the same public `NativeRegistryBuilder` to compose those entries
with explicit registration functions for its own marks, compound marks,
coordinates, transforms, tools, and opaque widget kinds. Built-ins and
extensions use the same schema-plus-lowerer entry types. Registry construction
rejects duplicate keys, missing coordinate prerequisites, and
schema/lowerer mismatches before project analysis begins.

For an existing typed coordinate pack, the builder's
`register_mark::<C>` and `register_tool::<C>` seams add compatible lowerers
after the coordinate has been registered. A host can therefore register the
stock built-ins first and then call independent extension registration
functions without replacing or reconstructing the built-in coordinate pack.

Coordinate-specific registration remains typed inside `CoordinatePack<C>`:
compatible marks and tools lower while `C` is known, and only a complete root
chart or child-plot operation crosses the object-safe erasure boundary. This
allows a host linked with third-party chart crates to compile mixed built-in
and custom coordinate trees without adding parser productions or forking the
compiler. Compatibility is explicit per registered `(coordinate, mark)` pair;
registering a coordinate does not make every built-in mark compatible with it.

Each finalized composed registry has a deterministic
`NativeRegistryProfileId` derived from the language major and canonical native
schema. Compiled artifacts and caches record that profile, and a host must be
linked with the implementations represented by it. Runtime-loaded Rust dynamic
libraries, linker discovery, grammar extensions, and a stable Rust plugin ABI
are outside v1. Source-level `define` declarations remain portable project
content and do not mutate the host-native profile.

Documentation is a schema requirement, not an afterthought, at every
granularity: the entity (mark kind, transform kind, tool kind, widget kind,
scale type, coordinate, reserved helper, event type) and each of its members — a channel per
(coordinate, mark) pair, each channel option suffix, each mark base
property, each transform property and output field, each scale option, and
**each enum value** (`empty_cells: hole` hover-explains what `hole`
does). For definitions, docs come from `-- |` doc comments; for native
built-ins, the initial implementation authors Avenger-language docs directly
in the schema entries beside their registration/lowerer code. Rust API docs may
describe the execution API separately; capturing them through schema-emitting
macros is an optional later deduplication, not a prerequisite. Both definition
and native docs use the same CommonMark format with the same summary/detail
split, so hover,
`avenger info`, and generated documentation render native and defined kinds
identically. A completeness lint runs in CI: a public schema
node without a non-empty doc fails the build — `missing_docs`, extended to
the language surface. The language's own reference site is generated from
the same schema, so hover text, CLI output, and published reference can
never disagree. This schema should describe the authoring DSL, not the
compiled serialization shape. Compiled structs are useful for execution, but
they often contain generated implementation fields or omit authoring concepts
such as repeated aggregate measures, channel configuration blocks, and transform
output handles.

The adopted architecture is a small, dependency-light authoring-schema
meta-model shared by the chart crates and `avenger-lang`:

```text
avenger-chart-schema
  SchemaVersion
  DeclarationSchema
  KindSchema
  MarkSchema
  TransformSchema
  ToolSchema
  WidgetSchema
  PropertySchema
  ChannelSchema
  ChildRule
  TransformOutputSchema
  PartSchema
  ExportSchema
  ValueShape
```

Minimum schema contents:

- Declaration kinds, their legal parents and children, and their allowed body
  mode: ordered declarations, unordered properties, raw payload, or mixed body;
  their binding class (scope-predeclared, sequential dataflow, or non-binding);
  their collision domain and value kind, including the shared param/store
  value-binding namespace and scalar-versus-table distinction;
  which declarations admit `private`/`public`, the required ancestry for public
  hoisting, and which component boundaries admit interface exports and
  `component_kind` provenance. `ChildRule` validates legal cross-kind
  sequences and placement constraints over the single ordered child list; it
  never authorizes formatter reordering.
- Each property's value shape — SQL expression, SQL query, literal, atom,
  scalar/table binding, typed reference, array, anonymous block, typed block,
  or configured value —
  plus requiredness, default, multiplicity, and nested block schema. A schema
  may include a non-semantic presentation rank for documentation tables and
  completion lists, but canonical DSL/JSON printing, AST equality, and hashing
  must ignore it and always use lexical property ordering.
- The closed slot-shape inventory and each shape's configuration contract:
  defaults, enum domains, function classes, reference kinds, block exposure,
  and caller/block hygiene.
- Mark kinds, coordinate compatibility, supported channels, channel defaults,
  extra mark-level properties, and public part aliases. Native parts are
  registered aliases; defined parts are derived only from explicit mark
  exports.
- Transform kinds, properties, default values, required values, output
  handles, and materialization behavior. The native `pipeline` entry additionally
  declares its mixed body, sequential child-stage rule, non-empty constraint,
  final-relation output validation, lexical internal aliases, and single-stage
  behavior in its parent dataflow. Transform schemas distinguish actual result
  columns from optional lexical output handles and declare binder policy:
  ordinary native stages are optional, while pipelines and defined transforms
  require a binder exactly when their public output set is non-empty. Compiler
  identity is not authoring-schema surface and never derives from the binder.
- Tool kinds, properties, generated/private state, supported coordinate
  contexts, and explicitly exported public state, parts, or event targets;
  whether a native kind is stateless and therefore admits an anonymous
  semicolon form. The native `behavior` entry additionally declares its
  required binder, mixed owned-content body, non-data/layout scope,
  containing-plot-only `scale_edit` target, exact exports, and chrome
  inheritance rules.
- Widget kinds, legal parents and guide positions, properties, generated typed
  state, existing-state binding properties, public param/store/selection
  exports, public parts, and intrinsic measurement/presentation metadata, plus
  the registry contract that pairs each schema with an erased built-in lowerer.
  The initial entries also record the snake_case authoring kind/part aliases
  separately from the landed kebab-case runtime/CSS identities, lazy-versus-
  eager state exports, Button's ordered action-block shape, and list-widget
  item-source/total-order requirements. `WidgetSchema` deliberately has no
  definition-slot or expansion contract and does not reveal whether Rust
  implements the kind as `ChartWidget` or `NativeWidget`.
- Scale, axis, legend, guide, theme, pattern, layout, selection, store,
  event, tool, widget, resource, and view property schemas.
- Native kind namespace membership, enum values, typed reference targets, and
  aliases used by the human-facing DSL, including exact group-export aliases,
  hoisted public paths, collision domains, and opaque component-kind part
  provenance. Typed state references record their required kind and path;
  `$name` is lexical, while `$component.alias` records a qualified exported
  value-binding path, its param/store kind, and its resulting scalar or table type.
- CommonMark docs on every node above, enum values included, enforced by a
  completeness lint.

The registry serializes to one canonical, versioned snapshot for each language
major (`authoring-schema-1.json`). That snapshot is a reviewed release artifact
and the portable contract used by tools that do not link Avenger. Serialization
uses the same RFC 8785 JSON canonicalization and is round-tripped in CI; any unexpected snapshot diff fails the
build. Adding a native kind or property is therefore a language-surface change
even when it does not change the meta-model.

Registry completion is tracked family by family rather than inferred from a
single vertical slice: Cartesian, polar, parallel, geo, treemap,
concat/repeat/facet/subplot, marks, transforms, tools, widgets, guides, scales,
layout, resources, and state/events. Every registered public node must pass the
documentation completeness lint before that family is considered complete.
Rust semantic unification supplies the meta-model, paired-lowerer mechanism,
and one complete bootstrap slice; the language/compiler's full-native-surface
phase owns incremental completion of this inventory. Generic parser work does
not wait for every family.

Imported `define` files contribute schema fragments after parsing: each typed
slot becomes a property with exactly its declared `ValueShape`, channel
parameters become renameable channel positions, transform outputs and export
aliases become completion/validation surfaces, and enum slots supply the domains that `match`
blocks must exhaust. Those fragments must validate against the same meta-model
before an imported kind enters the registry. They may add names but cannot
change the meaning of native entries or the v1 meta-model.

The ordinary `avenger-chart` rendering path does not depend on the LSP. The
schema crate is tiny, dependency-light, and Wasm-friendly. Existing chart
crates may expose their schema entries behind a `language-schema` feature;
normal rendering builds can leave entry collection and snapshot serialization
disabled while still sharing the meta-model types where public contracts need
them.

Macros may later reduce duplication by emitting schema metadata next to the API
they already generate. This is a source-code migration, not a schema migration:
macro output must reproduce the reviewed canonical snapshot exactly. For
example, `define_common_mark_channels!` and
`define_position_channels!` can continue to add methods like `fill(...)`,
`x(...)`, and `y(...)` to the concrete mark authoring types, while also emitting
feature-gated `ChannelSchema` constants for the same declarations.

Conceptually:

```rust
define_common_mark_channels! {
    Symbol {
        fill: {
            with_config: ColorChannelConfig,
        },
        size: {},
    }
}
```

continues to generate builder methods on `Symbol<C>`:

```rust
impl<C> Symbol<C> {
    pub fn fill<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("fill", value.into())
    }
}
```

and, when the schema feature is enabled, can also expose metadata:

```rust
#[cfg(feature = "language-schema")]
impl<C> Symbol<C> {
    pub fn common_channel_schemas() -> &'static [ChannelSchema] {
        &[/* fill, size, ... */]
    }
}
```

Coordinate-specific channels follow the same pattern on specialized mark types:

```rust
impl Symbol<Cartesian> {
    pub fn x<V: Into<ChannelValue>>(self, value: V) -> Self { ... }
    pub fn y<V: Into<ChannelValue>>(self, value: V) -> Self { ... }
}
```

The LSP-facing registry can then collect built-in schemas from the same crates:

```rust
pub fn builtin_mark_schemas() -> Vec<MarkSchema> {
    vec![
        Symbol::<Cartesian>::cartesian_mark_schema(),
        Rect::<Cartesian>::cartesian_mark_schema(),
        Symbol::<Polar>::polar_mark_schema(),
    ]
}
```

Transforms need a similar schema path, but their metadata should be attached to
the authoring transform, not only the compiled transform. For example, `Bin`
should advertise properties such as `field`, `maxbins`, `nice`, `base`,
`extent`, and `name`, plus output handles such as `start`, `end`, and `index`.
`Aggregate` advertises repeated grouping and measure forms rather than only the
compiled `group_by` and `measures` fields. The initial transform registry is
hand-authored, then may become macro-assisted once the snapshot and meta-model
are stable.

The first implementation uses a manual native registry:

```rust
pub const BUILTIN_TRANSFORMS: &[TransformSchema] = &[
    transform_schema! {
        name: "filter",
        properties: [
            predicate: SqlExpr { required: true },
        ],
        outputs: none,
    },
    transform_schema! {
        name: "bin",
        properties: [
            field: SqlExpr { required: true },
            maxbins: SqlExpr,
            nice: Bool,
            base: Integer,
            extent: Object,
            name: Identifier,
        ],
        outputs: handles ["start", "end", "index"],
    },
];
```

The manual source is simple to bootstrap and keeps the language bundle small;
the canonical snapshot and validation tests control its drift. Tests compare
the registry against public chart APIs that already expose partial metadata:

- For marks, instantiate each built-in mark/coordinate pair and compare schema
  channel names against `supported_channels()` or the macro-generated channel
  descriptor functions.
- For common mark channels, compare defaults, required flags, and
  `allow_column_ref` where those concepts exist in `ChannelDescriptor`.
- For transforms, compile representative authoring transforms in tests and
  verify that documented output handles correspond to real output methods or
  generated output names.
- For enum-valued properties, test that the schema names round-trip through the
  parser/lowering layer.
- For docs/examples, regenerate the canonical authoring-schema snapshot and
  fail CI on unexpected changes.

Over time, channel and transform macros may absorb manual declarations so the
builder API and registry are emitted together. That refactor is accepted only
when it produces no unintended snapshot change; the versioned snapshot remains
the normative artifact throughout.

Schema-aware analysis is part of the first useful native LSP milestone. It uses
the compiler's DataFusion-backed project-analysis API to propagate exact schemas
through catalog tables, SQL views, and chart transform stages without executing
queries. Configured source providers may perform capability-gated metadata or
schema inference (for example Parquet metadata or bounded CSV/JSON sampling),
and unavailable providers produce explicit schema-unavailable diagnostics.
An optional running-chart inspection connection supplies observed values and
schemas as a revisioned overlay; it does not replace planned schema analysis.

## Open Questions

- Should resource declarations such as params and data get compact shorthands
  in v1, or remain object declarations until the grammar settles?
- Should anything beyond `define` declarations become importable as a chart
  dependency (for example, named data declarations)?
- Private nested defines — the designated pressure valve if one-item
  file sprawl ever hurts: a helper `define` visible only to the file's
  single export (one *public* item per file stays the law; imports,
  naming, expansion, and the gallery untouched). Currently forbidden by
  the no-nested-definitions hygiene rule; open until real usage shows
  the need.
- Should a future shorthand auto-import the bundled definition library (a prelude), or
  do explicit per-definition imports (`import 'std:marks/error_bar';`) stay mandatory?
- When and how does a `pkg:` naming/discovery layer arrive over the
  fetch-pin-cache mechanism — a community index of URLs first, a real
  registry later, or never?
- Should hash algorithm agility beyond `sha256` be specified now (the
  keyword position permits future algorithms) or deferred until needed?
- Pattern ranges are the one theming surface CSS cannot express (patterns
  are structured DSL values). If themeable pattern defaults prove necessary,
  do patterns gain CSS syntax, or does a minimal theme definition kind
  return?
- Should transform-definition outputs support slot-derived names (a
  calculate-style transform whose output column names the caller chooses),
  or do fixed `output` declarations cover the practical cases?
- Should a block slot ever allow multiple splice points (the same caller
  content stamped at several positions), or does single-splice stay the
  rule?
- Should `part` overrides at instantiation sites be allowed to attach mark
  effects (`adjust`, `derive`) to a definition's internal marks, or only to
  restyle properties?
- Should reserved helper functions gain dotted sugar (`event.coord.x` for
  `event_coord(x)`) once the parser can keep it unambiguous?
- **An inline Rust macro** (recorded 2026-07-10; wanted): `chart!(r#"…"#)`
  as a full `Chart` constructor in Rust source. Verbatim token-tree form is
  ruled out **by design** — the SQL-flavored surface (single-quoted strings,
  `--` comments, `DATE '…'` literals) does not survive the Rust lexer, and
  the DSL will not bend to a host language. The sqlx-style string-literal
  proc macro is fully compatible: compile-time parse + schema validation,
  then (leaning) an **embedded validated AST** lowered at runtime through
  the same interpreter as file compilation — one lowering implementation,
  no macro/file drift; builder-codegen (`quote!` to `Chart::new()…`) is a
  recorded later optimization. Prerequisites are the DSL toolchain itself
  (parser + schema as library crates, the serializable AST); macro-specific
  opens: error spans (whole-literal + line/col on stable;
  `Literal::subspan` is nightly), Rust-value interpolation (trailing
  bindings — `chart!(r#"…"#, region = &region)` — for `Param`/`DataFrame`
  handles, mirroring component-fn arguments; DSL `$bindings` stay DSL-level),
  and editor highlighting via injection (the sqlx tree-sitter/LSP trick).
  The runtime companion is the primitive (the same parse→validate→lower
  path hot reload and file loading use; the macro merely hoists the
  fallible half to compile time), and it needs **no erased chart type**
  (settled 2026-07-10): the runtime currency is
  `parse_chart(src) -> Result<ParsedChart>` (the validated AST) and
  `ParsedChart::compile(ctx) -> CompiledPlot` — the lowering builds the
  typed `Chart<C>` inside a per-kind match arm and compiles it
  immediately, so `C` never escapes; erasure lives where it already
  exists (the AST before, the coordinate-erased `CompiledPlot` after),
  the same erase-at-the-compile-boundary move as
  `SubplotChildPlotSpec::compile_boxed`.
  `Chart::<C>::from_dsl` (runtime kind-checked, typed) is optional
  sugar for hosts that know the kind; a mutable `DynChart` builder is
  deliberately not planned — documents are closed scopes, and hosts
  retarget via catalog/params at runtime, not builder mutation. The
  macro's second advantage beyond validation stands: it reads the kind
  at compile time and emits a statically typed `Chart<C>`. Crate homes
  (updated 2026-07-13 to match the implementation plan): the strict parsed and
  resolved representation lives in `avenger-lang-core` and is re-exported
  ergonomically by `avenger-lang`; `ParsedChart::compile`/project lowering live
  in `avenger-lang-compiler`, which pulls `avenger-chart`, coordinate crates,
  providers, and DataFusion. The proc-macro and future Wasm analysis layer use
  the dependency-light core, while native compilation and native LSP schema
  analysis use the compiler facade. Hosts that need third-party Rust kinds
  inject their statically composed native registry into that compiler facade;
  the parser/core boundary does not change.
