# Avenger Chart DSL Version 1 Specification

## Status

Final pre-implementation language-version-1 specification. This document
defines the dedicated scripting language for authoring `avenger-chart` charts.
The goal is not to encode the Rust builder API one-to-one, and not to start
from JSON or YAML. The goal is a small declarative DSL that can express the
planned chart surface while remaining pleasant for chart authors. A later
change to a settled source form is a versioned language change, not an
implementation detail.

Language version 1 favors a regular canonical form. Shorthands may be proposed
for a later language version once the core grammar is proven. Implementation sequencing — the kernel
cut and its per-phase gates — is
[Implementation Phasing](#implementation-phasing).

Adopted decisions (2026-07-07): a required `avenger 1;` version pragma; SQL
string semantics everywhere with mandatory double-quoted data columns and
bare identifiers reserved for DSL names; explicit `encoded` and `direct`
channel modes; SQL-shaped contextual accesses (`channel.x`, `event.coord.x`,
`datum."field"`, ...) for compiler-provided values; operation functions retain
bare DSL-space arguments and strings only where their signature explicitly
accepts data-space names; `sql:` restricted
to query statements; cross-file
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
declarations at a marked splice point, with declared `exposes` handles) as the
remaining structural extension construct;
two-tier publicity (named declarations in a chart have canonical structural
paths; definition internals are private and cross the boundary only through
explicit, stable `export` aliases, while caller-authored block-slot content
retains caller-owned names beneath the instance); ordinary groups can express
the fully inlined form with `private` declarations, constrained `public`
hoisting, component-boundary `export` aliases, and opaque `component_kind`
provenance — there is no generated-only expansion dialect;
scalar params, stores, and selections share one collision-checked lexical
state-binding namespace; `$path` reads are checked by use context, store
relations are valid in SQL `FROM`, selections retain typed references and
current-row predicates, and
target-first `set <path>` resolution selects the update algebra; scalar params
bind a required row-free SQL initializer before `as` and take their exact Arrow
type from DataFusion planning, while stores and selections are peer state
declaration categories with category-specific bodies;
registry-free distribution — imports are uniform (`std:`, `native:`, relative,
URL) in every module however obtained, relative imports resolve against the
importer's location, and named or namespace clauses bind explicit exports
rather than filenames; the transitive closure of exact hash-pinned source
modules is fetched with no version resolution and inline `sha256` integrity
(Merkle-pinning the closure with no lockfile); one `.avenger` module may mix
charts, mark/tool/transform definitions, and DataFusion-shaped
catalog/schema/table declarations, while ambient datasets remain an explicit
host policy rather than a filename role; themes are plain CSS files fetched
and pinned like other resources; a project is a manifest-less directory whose
root is the default capability boundary, with every relative resource
resolving against its declaring module; provider-backed catalogs use explicit
schema projections, inline `catalog schemas` and `schema tables` containers,
and individually bound tables, with credentials via capability-gated `env`
values and `.env`, and catalog-level SQL views spelled
`table sql` — logical by default, materialized per session by opt-in, and
parameterized by scalar `param <expression> as <name>` declarations,
`$name` scalar placeholders in a
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
hand-written core JSON Schema (closed sixteen-tag value inventory) and
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
explicit cursor effect; compiler-inferred param Arrow types are authoritative contracts;
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

Adopted 2026-07-09: **`data:` is a property, not a declaration.** The
earlier `data as <name>` chart declaration is retired; a chart, group, or
mark sets its data context with an anonymous source block —
`data: { table: sales; }`, `data: { sql: ...; }` — or reads a table-valued
store binding with `data: $brush;`. Named/file/SQL sources always use the block
form (no bare-string shorthand), with the same reserved source
properties as before (`table`, `sql`, `url`, `values`, plus table-param
bindings). Anonymous means private: chart-local relations are never
referenced by name; named shared relations are catalog `table` declarations,
which is also the graduation path.

Adopted 2026-07-22: **declaration headers distinguish instances, declared
members, and keyed entries.** Runtime/chart instances use
`<category> <type> as <name>` when a runtime kind must be stated (`mark symbol
as points`, `mark group as layers`). Scalar params instead use the same
source-to-alias order as SQL projections: `param 640 as width`. Members whose parent
already establishes their role omit `as` and put type/shape before name
(`slot expr measure`, `field float64 x`, `variable row mpg`). Entries whose
parent establishes both role and type use only their key (`equality { id {
... } }`, `dimensions: { mpg: { ... } }`). `as` otherwise retains its true
source-to-alias meaning for imports, exports, and explicit transform outputs.
Scalar params require a header initializer; stores and selections are complete
declaration categories spelled `store as <name>` and `selection as <name>`;
mutation is target-resolved through one `set <path>` form. Logical dataflow groups are the language-owned
`mark group` kind. Continuous-colorbar overlays are the `overlay:` mark-block
property of a standard legend; there is no `container` declaration family.
Definition channels are `slot channel`,
expression adjustment is `adjust expr`, and nested Arrow struct fields are
type-first `field(<type>, '<name>')` constructors.

Adopted 2026-08-08: **`param` exclusively binds a scalar SQL expression.**
Stores and selections are peer declarations, not param types in the
expression position: `store as rows { ... }` and `selection as picked {
... }`. A declaration supplies an implementation kind only when the category
admits meaningful alternatives; `store` and `selection` are already complete
categories. Scalar params, stores, and selections retain one collision-checked
state-binding namespace, `$path` reads, target-resolved `set`, component
exports, runtime identities, and hot-reload behavior. The removed `param
store` and `param selection` forms are diagnosed but not accepted.

Adopted 2026-07-24: **every `.avenger` source file is a static module**.
A module contains an ordered mixture of chart entrypoints, reusable
mark/tool/transform definitions, and table/schema/catalog declarations.
Items are private unless prefixed by top-level `export`; imports always use
an explicit named or namespace clause; filenames and basename segments have
no semantic naming, category, discovery, or ambient-data role. One chart may
be anonymous, while every chart in a multi-chart module must be named.
Module-local lookup is category-aware, export names are module-globally
unique, definitions retain defining-module hygiene, and module/source graphs
are acyclic. Defined marks cannot capture datasets directly or transitively;
defined transforms may explicitly depend on non-`input` relations and carry
those provider/capability dependencies. Native extensions are imported
schema-described `native:` modules already supplied by the host. Source
modules may be bundled into one ordinary import-free module through
deterministic private alpha-renaming and bound-reference rewriting. This
decision is source-breaking and replaces the earlier filename-as-name,
one-public-item-per-file, plain-import, specialized-suffix, and
collection-style-bundling designs.

Adopted 2026-07-26: **transforms that accept caller-named SQL expressions use
an ordered projection-list value.** The standard property is `expressions:` and each
custom output is written `<sql-expression> AS <dsl-name>`, reusing the
select-list grammar between SQL `SELECT` and `FROM` without using open
user-named property maps. `aggregate`, `join_aggregate`, `scalar_aggregate`,
`calculate`, and `window` require named projection items; `select` accepts
source columns plus explicitly aliased computed expressions. Projection aliases
are exact, case-sensitive DSL output binders, not DataFusion-normalized SQL
identifiers. A constrained `slot outputs` carries the same named projection
shape through a custom transform definition and makes its caller-authored
aliases that instance's public output handles.

Adopted 2026-08-02: **every expression-driven channel states its evaluation
mode explicitly.** `encoded <sql-expression>` applies the channel policy
registered by the active mark and coordinate profile, including a scale when
that policy is scale-bearing. `direct <sql-expression>` uses the evaluated
value in the channel's output space and bypasses scale creation, domain
inference, and scale application. Both accept the same arbitrary scalar SQL
expressions; neither means “constant.” Ordered conditional branches use the
parallel `encoded:` and `direct:` properties. Bare channel expressions, the
old `value <expression>` qualifier, and the old conditional `scaled:` and
`value:` properties are not part of language version 1.

Adopted 2026-08-08: **a selection binding is the Boolean predicate for the
current chart row.** In a scalar row-expression island, `$picked` or a
qualified `$component.picked` evaluates the referenced selection against the
current row, using that selection's `empty` and `combine` policy. This form is
valid in channel predicates and other row expressions with an effective data
schema. It is not a scalar state snapshot: it is invalid in event expressions,
param initializers, and full SQL queries, and it cannot take `@start` or
`@previous`. Event-time equality membership remains the distinct
`selection_contains(selection, datum."<field>")` operation.

## Design Principles

- Every file begins with a version pragma: `avenger 1;`.
- Use block-structured declarations for charts, marks, transforms,
  interactions, tools, widgets, and other chart objects.
- Use `as` for a named instance or a genuine source-to-alias mapping:
  `mark group as manual_box_plot`, `mark rule as median`, and
  `transform aggregate as stats`. Declared members omit it:
  `slot expr measure`, `field float64 x`, and `variable row mpg`. A keyed
  child whose parent fixes its role and type uses only the key.
- Use `property: value` consistently inside property blocks.
- Bodies distinguish unordered properties from ordered child declarations:
  `property: value;` entries are unordered configuration; repeated child
  declarations (`mark`, `transform`, `cell`, `when`, `level`, ...) are
  ordered. Container bodies (`chart`, `cell`, `mark group`, channel config) mix
  both.
- Treat SQL expression snippets as the expression language in expression
  slots, with SQL string semantics everywhere: single quotes are string
  literals, double quotes are identifiers.
- Data columns are always double-quoted (`"horsepower"`); a bare identifier
  is never a column. Bare names belong to the DSL: kinds, properties, enum
  values, transform aliases, contextual namespaces, and intrinsic operations.
- Every expression-driven channel chooses `encoded` or `direct` explicitly.
  `encoded` invokes the registered channel policy; `direct` bypasses scale and
  domain processing but still accepts row-varying SQL expressions.
- Scalar params, stores, and selections share one state-binding namespace.
  `$name` references the nearest binding and the surrounding scalar,
  relation-valued, or current-row predicate slot checks its kind;
  `$component.alias` references an exported binding. In event expressions,
  scalar param reads may add `@start` or
  `@previous` to select a frozen temporal version (`$width@start` reads “width at
  start”); stores do not admit temporal qualifiers. Positional placeholders are
  rejected. Other compiler-provided values use contextual qualified identifiers
  such as `channel.x`, `event.coord.x`, and `datum."field"` (plus the ordinary
  SQL subscript in `event.facet[1]`). Qualified binding references use a
  pre-parse token normalization pass, not a custom lexer.
- Keep the DSL lexical surface compatible with DataFusion SQL tokenization.
  The whole file should tokenize through `sqlparser-rs` before the
  Avenger-specific parser interprets declarations and property blocks, and
  the token classes the DSL relies on are specified normatively and pinned by
  a conformance corpus so engine upgrades cannot silently change the
  language.
- Make channel blocks first-class because channel configuration is the main
  authoring atom in `avenger-chart`.
- Make `MarkGroup` explicit as `mark group`: a data-prep and authoring
  mark, not a rendered scenegraph group. Continuous-legend injected marks use
  a singular `legend.overlay` property containing local mark declarations.
- Reuse crosses files through `import` and parameterized `define`
  declarations (marks, tools, transforms). Native built-in kinds remain
  registered by the host and available through the same authoring schema;
  they do not have to be expressible as definitions. Definitions add custom
  compound marks, tools, and SQL-backed transform pipelines over the
  language-level surface available to their kind. The conditional budget for
  definitions is explicit and closed:
  `slot channel` parameters rename, `match` over an enum slot selects among
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

## Source Modules, Imports, And Versioning

Every UTF-8 source module begins with one language-version pragma, followed by
all imports and then one or more module items:

```avenger
avenger 1;

import {
  confidence_band,
  summarize as aggregate_summary,
} from './statistics.avenger';

import * as acme
  from 'native:com.acme.avenger.visuals@1';

table parquet as observations {
  path: './observations.parquet';
}

define transform prepare {
  transform aggregate_summary {}
}

export define mark interval {
  mark confidence_band {}
}

chart cartesian as summary {
  data: { table: observations; }
  transform prepare {}
  mark interval {}
}

chart acme.isometric as density {
  data: { table: observations; }
  mark acme.hexbin {}
}
```

The version names the language dialect, not a library release. Parsers reject
unsupported majors. Imports must precede every module item. Items may appear
in any order and are predeclared before bodies are resolved, but the resulting
item dependency graph must be acyclic. Empty and import-only modules are
invalid.

### Module items

The only top-level item categories are:

```avenger
chart cartesian as scatter { ... }

define mark error_bar { ... }
define tool inspect { ... }
define transform summarize { ... }

table parquet as observations { ... }
schema tables as samples { ... }
catalog iceberg as warehouse { ... }
```

`define` introduces a reusable kind and is valid only for marks, tools, and
transforms. Charts and datasets are concrete values or entrypoints, so
`define chart`, `define table`, `define schema`, and `define catalog` are
invalid. Primitive mark/transform instances, params, stores, selections,
events, widgets, and views remain nested declarations rather than module
items. Custom widget definitions remain unsupported.

Every definition and dataset item is named. A module may contain zero, one,
or many charts:

- a sole chart may be anonymous or named;
- when there is more than one chart, every chart must be named;
- an exported chart must be named;
- every top-level chart is directly selectable from its defining module,
  regardless of export visibility.

The identity of a named chart is its canonical module identity plus its
authored name. An unnamed singleton uses a distinguished singleton identity.
Each compiled entrypoint owns independent runtime params, stores, selections,
cursor state, event transactions, migration keys, and inspector state.

### Exports

Module items are private by default. Prefix an item with `export` to make its
existing name importable:

```avenger
export define mark error_bar { ... }
export define transform summarize { ... }
export table parquet as observations { ... }
export schema tables as samples { ... }
export catalog iceberg as warehouse { ... }
export chart cartesian as scatter { ... }
```

Module-local resolution has a separate collision-checked namespace for each
authoring-schema kind category, plus chart, data-binding, and namespace-alias
categories. Same-spelled locals in distinct semantic categories may coexist.
Exports instead use one globally unique name namespace within the producer
module, so a named import is unambiguous before its category is known.

An exported item carries its private transitive dependencies without exposing
their names. References within that closure stay bound in the defining module
and never re-resolve in the consumer. Component `export` declarations inside
a definition remain a separate instance-interface mechanism.

### Imports

Every import has an explicit clause. Named imports bind selected exports:

```avenger
import {
  confidence_band as band,
  error_bar,
} from './intervals.avenger';
```

The source name precedes `as`; the optional second name is the consumer-local
alias. The shorthand `import { error_bar }` materializes the same spelling as
both exported and local name.

A namespace import binds one lexical module alias containing exactly that
module's public export table:

```avenger
import * as intervals from './intervals.avenger';

chart cartesian {
  mark intervals.error_bar {}
}
```

Named imports retain their semantic category. Namespace member lookup is
filtered by the expected category at its use site. Private items are never
members. A namespace alias is reserved across all local categories.

Plain, default, side-effect, empty-clause, dynamic, conditional, and re-export
forms are invalid. Imports name exact relative, `std:`, `native:`, or HTTP
origins; there is no package search, version range, directory index, or
implicit suffix resolution:

```avenger
import { error_bar } from './intervals.avenger';
import { country_names } from 'std:datasets';
import * as acme from 'native:com.acme.avenger.visuals@1';
import { error_bar }
  from 'https://charts.example.dev/intervals.avenger'
  sha256 '9f2ab34c...';
```

`sha256` pins the complete source module bytes. Each transitive source import
carries its own pin. A `native:` import never fetches code; it selects an exact
schema-described module already registered by the host and records that
module's schema and implementation profile.

### Source filenames and ambient data

`.avenger` is the sole source suffix. The basename describes the module's
purpose and never determines an item name, category, import binding, chart
entrypoint, or ambient-data role. Basenames may contain dots, so
`legacy.mark.avenger` is syntactically an ordinary source path, but `.mark`
has no meaning.

Ambient datasets are a host/project-root policy, not a filename convention.
An explicitly designated ambient module may contain only data declarations
and imports needed by those declarations, and contributes only its exported
datasets. An ordinary mixed module is never discovered or activated merely
because of its path.

### Module and item closure

Source import graphs and resolved item dependency graphs are acyclic. An
exported definition retains its private helpers. A defined mark may not
directly or transitively capture a table, schema, or catalog; it receives its
row relation from each use site. A defined transform may join its reserved
`input` relation to explicitly resolved module-local, imported standard, or
ambient datasets. Every non-`input` relation becomes a visible provider,
capability, provenance, and activation dependency. Using such a transform
transitively inside a defined mark is invalid.

Table/schema/catalog paths resolve to stable relation identities in the
defining module. A namespace qualifier such as `samples` in
`samples.warehouse.analytics.orders` is consumed by language resolution and
does not occupy a DataFusion catalog segment. The compiler rewrites resolved
relations to internal table identities before DataFusion planning while
preserving authored paths for diagnostics and tooling.

### Single-file bundling

Bundling does not add a nested module declaration. A bundler loads and resolves
the ordinary module graph, selects a chart entrypoint or module interface, copies the
reachable source items once, deterministically alpha-renames private lexical
bindings, rewrites all bound kind/helper/data/SQL references, removes imports
and unreachable items, and emits one ordinary import-free `.avenger` module.

Bundling preserves selected export names, chart selectors, params, stores,
selections, cursors, public component/target aliases, field and SQL aliases,
explicit IDs, and presentation metadata. The generated module has a new
source-module identity and needs no manifest or source map to compile.

Private lexical names may be alpha-renamed only with all of their resolved
references. Their spelling must never implicitly determine rendered titles,
labels, legend text, accessibility text, scene names, event targets,
interaction keys, state-migration keys, provider capabilities, layout,
dataflow, or cache identity. Presentation defaults may derive from semantic
data names such as columns because those names are not private lexical
bindings. Export names, chart selectors, explicitly host-visible state names,
public target aliases, explicit IDs, and component provenance are observable
and therefore are never subject to unrestricted renaming.

An imported item retains the producer's module/item/export identity alongside
the consumer-local alias. Import renaming changes only the lexical spelling;
it does not change definition identity, inferred presentation, navigation
provenance, or cache keys.

## Names, Strings, And Columns

One rule set governs every name and string in the file, inherited directly
from SQL:

| Form | Meaning | Example |
| --- | --- | --- |
| `'...'` | string literal | `title: 'Horsepower';` |
| `"..."` | data column reference | `x: encoded "horsepower";` |
| bare identifier | DSL name: kind, property, enum value, alias, contextual namespace, intrinsic operation | `scale: linear`, `totals.amount`, `median(...)` |
| `$name` | lexical value-binding reference | `"mpg" >= $min_mpg`, `data: $brush` |
| `$path.to.name` | exported value-binding reference | `"mpg" >= $controls.min_mpg` |
| `$path@start`, `$path@previous` | frozen temporal param read in an event expression | `$width@start + event.coord.x - event.start.coord.x` |
| `<kind> <path>` | typed DSL reference | `selection hover.hovered` |

Bare DSL names use exactly the same unquoted-identifier character profile as
`AvengerSqlDialect` SQL identifiers:

```text
start        Unicode alphabetic | _
continuation Unicode alphabetic | ASCII digit | _
```

This permits names such as `café`, `Δvalue`, and `sales_2026`. The sharing is
lexical, not semantic: DSL names preserve their authored spelling, are
case-sensitive, and are not Unicode-normalized, while unquoted SQL identifiers
retain DataFusion's lowercase normalization rules. Fixed language keywords
remain ASCII and contextual. `$`, `@`, and `#` are never identifier characters:
`$` is reserved for value bindings and dollar-quoted strings, `@` for temporal
suffixes and supported SQL operators, and `#` for possible future operator
syntax. Avenger v1 rejects `a # b`: DataFusion exposes that spelling through a
PostgreSQL parse path that the Generic-derived `AvengerSqlDialect` does not use.
DSL names remain unquoted.

Names beginning with `__av_` are reserved for deterministic compiler output.
They may appear in canonical expanded source on compiler-private declarations,
or in canonical bundled source on link-local top-level items whose generated
names replace module qualification. Ordinary authored binders and SQL
identifiers must not use the prefix. Definition authors spell private SQL
intermediates with the separate `__private_` marker described under
[Defining Transforms](#defining-transforms).

Value-binding path segments use this same identifier profile. The leading `$`,
path dots, and an adjacent optional `@start` or `@previous` provide structure
rather than becoming part of a segment. Consequently `$controls.width`,
`$width@start`, and `$tag$raw text$tag$` are lexically distinct without quoted
binding segments or delimiter lookahead beyond recognition of a dollar-string
opener.

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

Expression-driven channel values always state how their SQL expression enters
the channel:

```avenger
fill: encoded "region";                 -- registered encoding policy
fill: direct '#2563eb';                 -- direct output-space color
opacity: direct coalesce("alpha", 1.0); -- direct may vary by row
stroke: none;                           -- explicitly absent channel
text: direct 'Total';                   -- literal text (not column "Total")
```

`encoded` does not promise that a scale exists: text and coordinate-owned
layout channels may have an identity/no-scale encoding policy. `direct` does
promise that no scale is created or applied and that the expression does not
contribute to scale-domain inference. `none` is the DSL's absent-channel
literal; SQL `NULL` remains `NULL` inside SQL expression slots. The two are
distinct: `none` removes a channel property, while `NULL` is a data value.

### Physical Arrow Types

Every store field and compiler/schema-owned fixed param names a physical Arrow
type using one canonical, lowercase type algebra. Authored scalar params do not
repeat this syntax: DataFusion plans their initializer and its exact Arrow
`DataType` becomes the param contract. The algebra below is DSL schema syntax,
not an SQL expression and not a logical/semantic type alias:

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
           | struct([field(arrow_type,string) {, field(arrow_type,string)}])
           | map(arrow_type,arrow_type) ;
```

Lengths and decimal precision are positive integer literals; decimal scale is
a signed integer; a timestamp timezone is a single-quoted IANA name or fixed
offset. These structural integer positions reuse the common DataFusion number
token and then apply their integer/range constraint. Decimal scale alone wraps
that token in an optional unary `+` or `-` (`decimal128(38,-2)` and
`decimal128(38,+2)` are valid spellings); the sign is not part of the number
token, and the canonical printer omits unary `+`. Length and precision remain
positive and therefore do not admit a leading sign.
Nested list/map elements and struct fields use the Arrow-schema nullability
fixed by this v1 grammar (nullable); store-field nullability remains the
separate trailing `nullable` modifier. Dictionary, union, run-end-encoded, and
view types are not valid v1 store or schema-fixed-param types. The authoring schema owns this
closed inventory and canonical printer; aliases such as `double`, `varchar`,
`array`, or SQL `timestamp` are rejected so an authored store-field or
schema-fixed type maps to exactly one Arrow `DataType`. Atomic types use the existing semantic `Atom` value and
parameterized types use the existing `Call` value, so `list(float64)` encodes as
`{"call":{"fn":"list","args":[{"atom":"float64"}]}}`; Arrow types add no AST
variant or interchange tag.

Struct types are recursive and preserve field order because Arrow struct field
order is physical schema. Field names are always single-quoted strings inside
the type constructor, so arbitrary Arrow field names do not enter the DSL name
namespace. Names must be non-empty and unique within one struct; an empty struct
is `struct()`. For example:

```avenger
store as state {
  field struct(field(float64, 'x'), field(list(utf8), 'labels')) pointer;
}
```

The type precedes each nested field name, matching type-first store fields and
definition slots. Nested field names remain strings so arbitrary Arrow names
do not enter the DSL identifier namespace. The semantic value is nested
existing `Call` nodes (`struct`, `field`, `list`) with type and string
arguments. The authoring schema validates constructor names, arity, field
uniqueness, and recursion; these calls are type syntax and do not resolve
through the SQL/helper-function namespace. Canonical JSON for the simple type
`struct(field(float64, 'x'))` is:

```json
{"call":{"fn":"struct","args":[{"call":{"fn":"field","args":[{"atom":"float64"},"x"]}}]}}
```

The former name-first `field('x', float64)` constructor is invalid. A
colon-based alternative such as `struct(x: float64)` is not part of the
physical type grammar.

### Typed Value Boundaries

Authored scalar-param initializers are the one type-defining boundary: the
compiler plans the row-free SQL expression and uses its planned Arrow
`DataType` without an outer destination cast. Every later consumer of that
param—table arguments, actions, native fixed bindings, and host state—uses the
inferred exact type. Store fields, schema-fixed generated params, cursor
assignments, and other typed destinations continue to apply DataFusion's strict
Arrow `CAST`. There is no literal/nonliteral distinction at those destination
boundaries.

An explicit authored `CAST` controls a scalar param's contract, while a bare
`NULL` or otherwise untyped empty expression is rejected. For example,
`param CAST('0.75' AS DOUBLE) as opacity;` is `Float64`, while `param 0.75 as
opacity;` follows the pinned exact-number normalization and is
`Decimal128(2, 2)`. Representative pinned results are `1` → `Int64`, `1.5` →
`Decimal128(2, 1)`, `1e2` → `Decimal128(1, -2)`, `-0.0` → `Float64`, string
literals and `upper(...)` → `Utf8`, Boolean literals → `Boolean`, and
`CAST(... AS DATE)` → `Date32`. `arrow_cast` supplies exact Arrow types such as
`Binary` that DataFusion's SQL `CAST` type spelling does not support.

At typed consumers, the destination cast intentionally accepts the complete conversion matrix of
the pinned DataFusion 54 / Arrow 58 implementation, including supported string
parsing and lossy numeric conversions. For example, an `int32` store field or
action destination may receive `3.9` (producing `3` under the pinned cast), and
the pinned Boolean string kernel accepts forms such as
`'yes'` and `'no'`. An invalid constant cast is a compilation error. A
value-dependent runtime cast failure fails and rolls back the complete ordered
event transaction; it never silently becomes `NULL` or a skipped assignment.
Authors who want failure-to-null semantics write an explicit inner
`TRY_CAST(...)`, after which the destination's ordinary typed-null admission
rules apply.

`NULL` is strictly cast to a known destination's typed null. Scalar-param
initializers instead spell a typed null explicitly, for example `CAST(NULL AS
DOUBLE)`. DSL list, struct, and map values at declared destinations recursively
apply the same destination rule to their members.
The declared target supplies the otherwise ambiguous shape of empty nested
values, fixed-size-list length, struct field order/nullability, and map key and
value types. Unknown struct fields, missing non-nullable fields, null map keys,
and map-key collisions introduced by casting are errors.

Numeric syntax remains exact until SQL planning and the destination cast. The semantic
AST stores a numeric literal as its canonical decimal spelling, never as an
`f64`, and interchange JSON uses the tagged string form
`{"num":"9007199254740993"}`. Canonicalization preserves every significant
digit and the sign of negative zero; its precise spelling rules are pinned by
the parser/printer corpus. This permits exact `int64`, `uint64`,
`decimal128`/`decimal256`, and floating-point construction without an
IEEE-754 JSON round trip first.

The accepted numeric token syntax is exactly the syntax accepted by the pinned
DataFusion 54 general expression planner under Avenger's pinned parser options,
and it applies everywhere in the DSL—not only inside explicit SQL fragments.
For the pinned sqlparser/DataFusion pair, the decimal token envelope is:

```text
mantissa  := digits [ '.' [ digits ] ] | '.' digits
exponent  := [ 'e' | 'E' ] [ '+' | '-' ] digits
number    := mantissa [ exponent ] [ 'L' ]
signed    := [ '+' | '-' ] number       -- signs are unary SQL operators
```

Thus `1`, `1.25`, `.5`, `1.`, `1.e2`, `1.25E+003`, `1L`, `+.5`, and `-0.`
are accepted wherever a numeric value is allowed, including structural
property values, definition defaults, array elements, and SQL expression or
query islands. Numeric underscores, lowercase `l`, radix-prefixed numeric
integers, and bare `NaN`/`Infinity` are not numeric literal spellings.
`X'...'` is an Arrow binary literal rather than a numeric value and is tested
under the separate delimiter-bearing literal profile; `0x...` remains outside
the frozen Avenger source profile even though sqlparser tokenizes it into that
same binary category.

The shared token syntax does not erase contextual constraints. Structural
positions that require an unsigned, nonnegative, positive, or integer value
still enforce that requirement after exact numeric normalization. In
particular, the language version, `level` declaration or state-sharing index,
Arrow lengths, and decimal precision do not accept a leading sign; decimal
scale is the one signed non-expression numeric position in v1. Ordinary
property values, array elements, defaults, rows, and channel values already
pass through SQL-expression parsing, so their unary signs need no outer-grammar
exception.

The exact AST canonicalizer adapts all planner-accepted number tokens without
rounding: it inserts `0` before a leading decimal point, inserts `0` after a
trailing decimal point so the value remains floating-shaped, lowercases `E`,
normalizes exponent sign/leading zeroes, drops the semantically ignored `L`,
and preserves significant mantissa digits, fractional trailing zeroes, and
negative zero. Examples include `.5` → `0.5`, `1.` → `1.0`,
`1.e+003L` → `1.0e3`, and `-0.` → `-0.0`.

“DataFusion-compatible” is an executable compatibility claim: upgrades rerun a
positive/negative matrix through both Avenger tokenization and DataFusion
logical planning. A spelling is not supported merely because sqlparser can
produce a `Token::Number`; it must reach a DataFusion numeric scalar under the
pinned options. The same checked matrix supplies the structural parser,
Tree-sitter grammars, formatter, and editor fixtures.

Before the semantic AST is produced, an SQL expression consisting of exactly
one scalar literal normalizes to the corresponding scalar `Value` variant.
This includes strings, booleans, `NULL`, and signed numeric literals; a unary
sign is folded into the numeric literal. Parenthesized or cast literals and all
other expression shapes remain `Expr` values. This normalization is identical
for parsed source and decoded interchange JSON.

That AST normalization is an interchange detail, not a semantic distinction at
a typed boundary. A literal, parenthesized literal, `$param`, function call,
arithmetic result, or `CASE` expression is planned by the same pinned
DataFusion profile and receives the same strict destination cast. Exact numeric
AST nodes are normalized to exact `int64`, `uint64`, decimal128/decimal256, or
negative-zero floating source literals before DataFusion plans any scalar
expression, full query, catalog SQL, or editor-analysis probe.

The boundary inventory is closed for v1:

| Typed boundary | Expression environment | Destination |
| --- | --- | --- |
| authored scalar param initializer | row-free SQL and same-scope scalar-param dependencies | DataFusion-planned source type (type-defining; no outer cast) |
| schema-generated param initializer | schema-owned row-free value | schema-fixed Arrow type |
| catalog-table param initializer | self-contained row-free SQL | DataFusion-planned source type |
| table/data-block argument | row-free SQL and visible scalar params | callee param type |
| store initial-row field | row-free SQL and chart scalar params | declared field type |
| `set <scalar-param>` and store row/key/patch RHS | ordered event or param-change environment | target param/field type |
| `set cursor` RHS | event environment | `utf8`, then cursor-style validation |
| compiler-owned filters | their existing stream/action environment | `boolean` |

Ordinary SQL output columns, native mark/channel properties without an exact
Arrow destination, definition slots, enums, names, structural integers, and
selection-specific predicate comparison semantics do not gain an implicit
destination cast. Host `ScalarValue` and `RecordBatch` inputs also remain exact:
they already carry physical Arrow types and must match the compiled interface
without SQL coercion.

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

The surface has three header laws:

1. **Named instances** use `<category> <type> as <name>` (or the
   schema-permitted anonymous form). The type chooses behavior; `as` binds the
   runtime, structural, or dataflow identity.
2. **Declared members** use `<member-category> <shape-or-type> <name>` with no
   `as`. Their parent has already fixed their namespace and role.
3. **Parent-keyed entries** use only `<key> { ... }` when the parent fixes both
   the entry category and value shape.

Imports, exports, and explicit transform outputs are mappings rather than
declarations, so `as` reads consistently as source-to-alias. A declaration
does not add `as` merely because its semantic AST stores a name.

The common instance shapes are:

```avenger
chart <coord-kind> [as <name>] {
  ...
}

mark group [as <name>] {
  ...
}

private mark group as <name> {
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

param <sql-expression> as <name>;

param <sql-expression> as <name> {
  sharing: shared | free | level(<nonnegative-integer>);
}

store as <name> {
  ...
}

selection as <name> {
  ...
}
```

Representative member and keyed-entry forms are:

```avenger
slot expr measure;
slot channel axis { default: x; }
field float64 x nullable;
variable row mpg { expr: "mpg"; }

equality {
  id { field: "id"; value: datum."id"; }
}
```

The body mode depends on the declaration. `chart` and `mark group` bodies are mixed
container blocks with ordered child declarations. A `mark` body is mixed only
to admit its optional inline `view` child alongside ordinary mark properties.
Ordinary `transform` kinds, scalar-param sharing bodies, `scale`, `axis`, `legend`, tools,
widgets, and selections have property blocks;
an inline `view` has a mixed body containing its properties and dependent
transforms/render children; stores have mixed bodies with ordered
`field`/`row` children; and the
`data:` property takes either an anonymous source block or a table-valued
`$store` binding. The core
`transform pipeline` kind is the transform exception: its mixed body contains
interface `output` declarations and ordered child transforms. The core
`tool behavior` kind is the tool exception: its mixed body contains scoped
state, events, scale edits, nested tools, chrome marks/groups, and interface
exports.

A semicolon is only the compact empty-body spelling. For slots, requiredness
means the absence of `default:`; it is not a meaning carried by the semicolon.

Examples:

```avenger
chart cartesian as sales_by_category {
  mark group as layers {
    transform aggregate as totals {
      group_by: "category";
      expressions: sum("amount") AS total;
    }

    mark rect as bars {
      x: encoded "category";
      y: encoded 0;
      y2: encoded totals.total;
    }
  }
}
```

`as <name>` has one meaning: bind this declaration under that name. For marks
and mark groups in a chart, an ordinarily visible name contributes one segment to
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

Named declarations are public by default in ordinary chart mark groups. The
visibility modifiers are the general-purpose representation needed by inline
definition expansion:

- `private` keeps a declaration's lexical name and compiler identity but
  removes it and its descendants from the external structural namespace.
  Internal references and component-boundary `export` declarations may still
  name it.
- `public` is legal only beneath a `private` structural ancestor. It re-enters
  the public namespace at the nearest non-private named component boundary —
  a mark group or `tool behavior` — omitting the
  intervening private path. A public mark group carries its normally visible
  descendants with it. This is path hoisting, so collisions are checked at the
  re-entry point rather than only at the declaration's lexical parent.
- `export <private-path> [as <alias>];` in a mark group publishes one exact private
  declaration beneath that group. Exporting a mark group exposes one mark target
  that addresses all of the group's primitive runtime descendants. The alias
  defaults to the source path's last segment.
  Export aliases and ordinary or hoisted public children share one namespace.
- A named mark group or `tool behavior` may set `component_kind: <kind>;`. This is
  opaque component provenance, not kind instantiation. Its exported mark aliases acquire
  `(component_kind, alias)` part provenance for theme matching. Expansion
  records the definition's canonical declared kind, not a use-site import
  rename; the atom remains valid even when no registry entry or import for that
  kind remains.

`private` and `public` are permitted on declarations with a named public
identity, including marks, mark groups, scalar params, stores, and selections,
tools, and widgets. They
do not alter dataflow visibility: transform aliases remain lexical
names governed by their dataflow scope. Both modifiers are rejected inside a
`define` body: its authored declarations are already private as a unit, and
only definition-header `export` declarations may publish them. The modifiers
spell the equivalent visibility after that definition has been inlined into an
ordinary mark group.

Anonymous declarations are allowed where the object does not need a public name:

```avenger
mark rule {
  x: encoded 0;
  x2: encoded 1;
}
```

## Block Modes

A body may contain unordered properties, ordered child declarations, or both.
The rule that keeps colon syntax meaningful: `property: value;` entries are
unordered configuration; repeated child declarations are ordered. Unordered
also means unique: a property name may appear at most once in a body — a
duplicate is a parse-time error, never a later-wins override. Container
bodies (`chart`, `cell`, `mark group`, and channel configuration blocks) mix both
modes:

```avenger
chart cartesian as example {
  title: 'Sales';

  data: {
    table: sales;
  }

  mark group as layers {
    transform aggregate as totals {
      group_by: "category";
      expressions: sum("amount") AS total;
    }

    mark rect as bars {
      x: encoded "category";
      y: encoded 0;
      y2: encoded totals.total;
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

The concrete syntax resolves the otherwise overlapping `ident { ... }` shape
as a typed object, never as a lone SQL identifier followed by a configuration
body. Configured values therefore use any other expression head, such as a
quoted column, binding, literal, qualified expression, or function call. This
does not exclude a canonical v1 expression: data columns are always quoted,
bindings begin with `$`, and calls retain parentheses. Both shapes still lower
to a `Block` with a head, and the authoring schema decides whether that block is
legal for the particular property.

```avenger
mark rect as box {
  x: encoded stats.q1 {
    scale: linear {
      domain: [0, 36];
      nice: true;
    }
    axis: {
      title: 'Value';
      grid: true;
    }
  }

  fill: direct '#bfdbfe';
  stroke_width: direct 1.5;
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
`mark group` with `transform` declarations, then a child `mark`.

```avenger
mark group as layers {
  scale_hint {
    channel: y;
    type: band;
  }

  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
  }
}
```

## SQL Expression Slots

Expression-valued properties contain SQL expression snippets. Data columns
are double-quoted; strings are single-quoted; bare names are DSL names:

```avenger
filter: "amount" >= $min_amount and "region" = $selected_region;
x: encoded log("amount" + 1);
visible: "amount" is not null;
label: "category" || ': ' || cast("amount" as varchar);
```

Scalar params and stores are value bindings. Scalars carry scalar values and
stores carry table values; both are referenced with a named `$path`:

```avenger
param 0 as min_amount;

store as brush {
  field utf8 id;
  field float64 x;
}

transform filter {
  predicate: "amount" >= $min_amount;
}

mark symbol {
  data: $brush;
  x: encoded "x";
}
```

Only named `$` binding references are valid. Positional placeholders such as `$1`,
`$2`, and `?` are rejected. Scalar params, stores, and selections occupy one
collision-checked **state-binding namespace** in each lexical scope: declaring
`param 0 as x` and `store as x`, or any other category pair, in the
same scope is an error. A `$name` reference first resolves the nearest state
binding by name, then requires the category allowed by the use site. It never skips an incompatible nearer binding to
find a compatible outer one, and it never crosses a component boundary by
guessing or concatenating names.

A qualified read of an exported binding extends the same spelling with a DSL
path:

```avenger
visible: $zoom.enabled;
filter: "amount" >= $controls.min_amount;
label: cast($panel.controls.minimum as varchar);
```

Every segment after `$` is a bare DSL identifier. The first segment resolves
lexically; crossing a component boundary requires an explicit export, and the
final target must be a scalar param, store, or selection. `$name` remains the canonical
lexical form; `$component.alias` and deeper paths are the canonical public
forms. Quoted or numeric path segments are invalid, as are paths that resolve
to other non-value state. A param is valid only in a scalar-valued position; a
store is valid only in a relation-valued position; a selection is valid only as
a Boolean current-row predicate in a scalar row-expression island. Param and
store positions read the binding's current state value, while a selection read
evaluates membership for the current row. A schema slot that explicitly
expects a param/store reference (for example `x_domain_param: $x_domain;` or a
`slot ref` of that kind) retains binding identity instead; it uses the same
spelling and resolved `Binding` node, with dereference-versus-handle semantics
fixed by the slot schema.
For example, an inner table-valued `$data` shadows an outer scalar `$data`, and
using the inner binding in a scalar expression is a type error rather than a
request to fall back to the outer declaration.

Event expressions may qualify a param read with a temporal version:

```avenger
set width = $width@start + event.coord.x - event.start.coord.x;
set velocity = event.coord.x - $position@previous;
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
  x: encoded b.start;
  x2: encoded b.end;
}
```

Compiler-provided scalar values use SQL-shaped contextual property access.
They tokenize as ordinary qualified SQL identifiers (or, for facets, a
qualified identifier plus a SQL subscript) and are rewritten through parsed
SQL AST nodes before DataFusion planning. Channel references use `channel`:

```avenger
mark rect as bars {
  x: encoded "category";
  x2: encoded channel.x {
    band: 1.0;
  }
}
```

The contextual-access inventory is normative:

| Access | Legal scalar-expression context | Arrow result | Nullability and meaning |
| --- | --- | --- | --- |
| `channel.<channel>` | a channel expression on the current mark | referenced channel's exact expression type | preserves the referenced expression's nullability; cycle validation still applies |
| `$<selection-path>` | scalar row expression with an effective data schema | `boolean` | evaluates the selection against the current row; honors the selection's `empty` and `combine` policy |
| `datum."<field>"` | event expression | effective logical hit-row field type | nullable at the event boundary and when a possible target lacks the field |
| `event.coord.<channel>` | event expression | `float64` | current event coordinate in the channel's space |
| `event.start.coord.<channel>` | event expression in a `between` binding | `float64` | nullable until a start coordinate exists |
| `event.domain.<channel>.start` / `.end` | event expression | `float64` | event-time scale-domain boundary |
| `event.path` | event expression in a `between` binding | `list(float64)` | accumulated coordinate path; nullable before one exists |
| `event.facet[n]` | event expression | `utf8` | one-based logical facet-path component; only a positive integer literal is accepted |
| `event.legend.value` | legend-surface event expression | `utf8` | continuous-legend value; null outside a value hit |
| `item.channel.<channel>` | `adjust` or `derive` item frame | registered item-frame channel type | nullable item-frame field; preserves numeric, string, boolean, and other registered physical representations |
| `item.data."<field>"` | `adjust` or `derive` item frame | effective source-data field type | preserves physical Arrow type and source nullability |
| `item.bbox.<edge>` | `adjust` or `derive` item frame | `float32` | nullable item-frame field; edge is `top`, `right`, `bottom`, or `left` |
| `<view>.x.domain.start` / `.end` | owning inline-view scope | `float64` | inline-view x-domain param |
| `<view>.y.domain.start` / `.end` | owning inline-view scope | `float64` | inline-view y-domain param |
| `<view>.x.pixels` / `.y.pixels` | owning inline-view scope | `uint32` | inline-view pixel-count param |

`datum`, `channel`, `event`, and `item` are language-owned contextual roots.
They and all fixed members are case-insensitive in valid scalar DSL expression
islands; canonical formatting emits lowercase. An inline-view binder such as
`viewport` is instead a normal lexical DSL name and participates in rename and
navigation. Channel members are unquoted DSL-space names. Data fields remain
required double-quoted SQL identifiers and use normal `""` escaping.

A recognized contextual root reserves the whole candidate access in its legal
scalar expression island. Bare roots, unquoted data fields, unknown or
over-qualified members, invalid bounding-box edges, and zero, negative, or
computed facet subscripts receive a focused contextual-access diagnostic rather
than falling through to ordinary DSL or SQL name resolution. Full SQL queries
do not reserve these roots: for example,
`SELECT event."value", datum."id" FROM input AS event JOIN rows AS datum`
retains ordinary SQL alias semantics.

The function-shaped contextual spellings (`channel(x)`, `event_coord(x)`,
`item_data('field')`, `view_x(view, pixels)`, and the rest of that family) are
removed and are not compatibility aliases. Diagnostics offer an automatic
rewrite only when the old arguments make the conversion unambiguous.

Operations and value constructors remain calls:

| Operation | Argument kinds | Legal context | Result | Meaning |
| --- | --- | --- | --- | --- |
| `selection_contains(selection, datum."<field>")` | selection, contextual datum field | event scalar expression | `boolean` | selection membership predicate |
| `span(lo, hi)` | two numeric expressions | event scalar expression | `list(float64)` | construct an interval |
| `span_ordered(a, b)` | two numeric expressions | event scalar expression | `list(float64)` | construct and order an interval |
| `polygon(points)` | path/list expression | scene-query geometry slot | scene geometry | construct polygon geometry |
| `rect(x0, y0, x1, y1)` | four numeric expressions | scene-query geometry slot | scene geometry | construct rectangle geometry |
| `circle(cx, cy, radius)` | three numeric expressions | scene-query geometry slot | scene geometry | construct circle geometry |

The resolver and editor analysis consume this operation inventory directly;
it is not a prose-only list. `EXISTS`, `INTERVAL`, `STRUCT`, and `TRIM` remain
reserved SQL forms and are not Avenger operations or completion candidates.

Operation arguments naming DSL-space things are bare identifiers or qualified
paths when the operation signature calls for them. The operation inventory is
checked against sqlparser's reserved-for-identifier inventory. In particular,
`interval(...)` is not an operation spelling: `INTERVAL` enters SQL
interval-literal parsing in the pinned parser. The token/expression corpus
reserves `EXISTS`, `INTERVAL`, `STRUCT`, and `TRIM` against future intrinsic
operation names.

Reserved namespaces (`repeat.row`, `repeat.column_id`, ...) resolve the same
way as transform aliases. `$path` is the only sigil form and references a lexical
or qualified scalar/table value binding. The bare
argument form matters for definitions: channel slots rename through
bare channel arguments during expansion, which is what lets an imported tool
be channel-generic.

## SQL Projection Lists

A projection list is the ordered SQL fragment that normally appears between
`SELECT` and `FROM`. It is a third SQL island beside a scalar expression and a
query:

```avenger
transform calculate as derived {
  expressions:
    "price" * "quantity" AS total,
    coalesce("label", 'unknown') AS display_label,
    23 AS constant;
}
```

The semantic value owns one or more `sqlparser` select items in source order.
Its expressions use the same `AvengerSqlDialect`, value-binding normalization,
contextual-access and intrinsic-operation resolution, definition-slot
substitution, type planning, and canonical
SQL spelling as every other SQL island. Top-level commas separate projection
items; commas and `AS` tokens inside calls, casts, arrays, structs, subqueries,
or comments remain inside the item's expression. A trailing comma is accepted
when the pinned dialect accepts a trailing SQL projection comma, but canonical
formatting removes it.

The generic projection AST can represent the select-item forms supported by
the pinned Avenger SQL parser. An authoring schema narrows those forms at each
property:

- A **named projection** contains only
  `<sql-expression> AS <dsl-name>` items. Every alias is required, explicit
  `AS` is required, quoted aliases and multi-alias forms are invalid, aliases
  are unique, and wildcards are invalid. `aggregate`, `join_aggregate`,
  `scalar_aggregate`, `calculate`, and `window` use this shape.
- A **select projection** accepts an unaliased direct data-column expression or
  an explicitly aliased expression. A computed expression requires
  `AS <dsl-name>`. Wildcards remain the job of `transform sql`; they are
  deliberately invalid in `transform select`, where selecting `*` would add
  no behavior and would make the transform's output inventory input-dependent.

The alias after top-level `AS` is a DSL-owned binder even though SQL provides
its tokenization and expression boundary. It therefore uses the unquoted DSL
identifier profile, preserves authored case, and is not lowercased by
DataFusion. Lowering removes the alias from the parsed expression and supplies
the exact name to the Rust transform builder. When a projection is structurally
spliced into a full SQL query, expansion emits a quoted SQL alias as needed to
preserve that exact physical column name.

Aliases are outputs, not inputs. Every item in one projection list resolves
against the same relation entering the transform; an earlier alias is not
visible to a later sibling. This matches SQL `SELECT` semantics and makes
property-map order irrelevant. The item order determines deterministic output
and handle order. Duplicate aliases are an error before lowering.

The standard property name is `expressions:`:

```avenger
transform aggregate as totals {
  group_by: ["category", "segment"];
  expressions:
    sum("amount") AS total,
    count(*) AS count;
}

transform window as ranked {
  partition_by: "category";
  order_by: "amount";
  expressions:
    row_number() OVER () AS rank,
    sum("amount") OVER () AS running_total;
}

transform select {
  expressions:
    "category",
    "amount" * 2 AS doubled;
}
```

`expressions` is a contextual reserved property name in the schema-free structural
parser, just as `sql` and `query` select query islands. A registry entry may
use `expressions` only with a projection-list shape. For a custom slot whose property
has another name, a top-level `AS` or comma makes the projection syntax
structurally self-identifying; the schema then requires the named projection
shape. A single `slot outputs` value is always self-identifying because its
only valid item has an explicit alias.

The aliases of a named projection, and explicitly aliased items in a select
projection, are the transform's dynamic output handles. An unaliased pass-through
column in `select` does not create a handle. If the transform has an instance
binder, downstream expressions may use `derived.total`; the physical column is
also available as `"total"` where the transform produces a relation column.
`scalar_aggregate` instead publishes same-named derived scalar handles,
preserving its existing non-column result kind. Anonymous transforms still
create the physical columns but introduce no qualified handle namespace.

This syntax deliberately replaces, rather than supplements, the former open
property-map and structured-measure forms:

```avenger
-- Invalid v1 syntax after this adoption:
transform aggregate {
  total: sum("amount");
  measures: [{ name: 'count'; op: count; }];
}
```

Removing wildcard properties restores unknown-property diagnostics:
`group_byy:` is always an unknown property, never an attempted aggregate
measure. The projection shape does not broaden a transform's operation set.
For example, the native aggregate family still accepts exactly the aggregate
function forms its Rust implementation supports; `23 AS constant` is valid for
`calculate` but not for `aggregate`.

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

The exact trigger follows the pinned Rust tokenizer: the character immediately
after the second `-` must exist and satisfy Rust `char::is_whitespace`. This is
the Unicode White_Space predicate, not an ASCII-only space/tab approximation.
Consequently `-- comment`, `--\tcomment`, `--\u{00a0}comment`, and `--` followed
immediately by LF or CRLF start comments, while `--x`, `amount--1`, and bare
`--` at EOF do not. A line comment consumes through the next LF (including the
CR in CRLF); source line indexing likewise treats LF and CRLF as line endings.
A bare CR is whitespace and therefore triggers a comment, but is not by itself
an Avenger line ending, so that comment continues to the next LF or EOF. The
compiler conformance corpus and both Tree-sitter grammars use these same cases
rather than relying on a host regex `\s` definition.

Canonical formatting emits `-- ` for an ordinary empty line comment, including
the trailing ASCII space. Zed's line-comment toggle uses the same `-- ` prefix.

Doc comments are Haddock-style `-- |` lines and attach to the next
declaration — a `define`, `slot` (including `slot channel`), `output`, or `export`, a
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
-- | import { error_bar } from './error_bar.avenger';
-- | chart cartesian {
-- |   data: { sql: SELECT * FROM 'examples/sales.csv'; }
-- |   mark error_bar { category: "region"; measure: "amount"; }
-- | }
-- | ```
define mark error_bar {
  -- | Grouping expression; one bar per distinct value.
  slot expr category;
  -- | Measure whose min and max span the bar.
  slot expr measure;
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

The strict compiler reports an unterminated block comment as a lexical error.
The tolerant editor grammars consume the incomplete comment only through EOF
and expose recoverable error/incomplete-comment structure so preceding syntax
remains usable; repairing the closing `*/` must produce the same tree as a clean
parse.

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
  data: { table: sales; }            -- reference a catalog table

  mark rect { x: encoded "region"; y: encoded "amount"; }
}
```

For named, file, and SQL data, reserved properties select the source — always
the block form, never a bare value:

```avenger
data: { table: sales; }              -- statically resolved catalog relation path

data: {                                -- one-off derivation; `sales` is the
  sql:                                 -- ambient catalog name
    SELECT *
    FROM sales
    WHERE "amount" > 0;
}

data: { sql: SELECT * FROM 'customers.csv'; }   -- chart-owned file

param 'all' as selected_region;
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

Named tables (`table: sales`, and qualified names inside `sql:`
statements) resolve from the project's data catalog or the host's
registrations — see [Data Catalogs](#data-catalogs).

The `table:` property accepts one unquoted relation path, not a string or SQL
expression. Each path segment is a DSL/SQL identifier and resolution consumes
module aliases and imported schema/catalog bindings before provider planning.
For example, `import { vega as samples } from './catalog.avenger';` makes
`table: samples.movies;` refer to the imported `movies` table. Quoted
`table: 'samples.movies';` is invalid: relation identity participates directly
in completion, navigation, dependency analysis, and rename.

The `sql` property is a special full-query SQL slot. Its value is parsed as one
query statement, and the SQL semicolon terminates the property. The
compiler should use the SQL tokenizer/parser to find the statement boundary
rather than splitting naively on the first semicolon, so semicolons inside
SQL strings or comments remain valid.

Inside a full `sql:` statement, ordinary SQL query structure applies, but the
language-wide column rule does not change: every identifier used as a data or
output column is double-quoted. Relation paths, relation aliases, function
names, SQL keywords, and the binder introduced after projection `AS` remain
bare where SQL permits them. Once a projection alias is referenced as an
output column—for example from `ORDER BY` or an enclosing query—it is quoted
like every other column. This keeps column syntax uniform across full queries,
projection lists, and scalar expression islands.

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
SELECT m."title", m."rating"
```

It has the same meaning, logical plan, and output schema as:

```sql
SELECT m."title", m."rating"
FROM vega.movies AS m
```

This ordering is particularly useful while authoring: by the time the user
types `SELECT m."`, the language server already knows the relation and alias
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

`table`, `schema`, and `catalog` are ordinary module items and may appear
beside charts and definitions. Their namespace is deliberately the same
three-level hierarchy used by DataFusion: a catalog contains schemas, and a
schema contains tables. A chart uses module-local data directly or imports an
exported relation by name or namespace. Separately, a host may explicitly
designate one or more data-only modules as **ambient configuration** so charts
can remain environment-independent and a deployment can retarget them without
editing source. No filename or suffix makes a module ambient. Defined marks
may not capture data; defined transforms may name explicit data dependencies,
which remain visible in their resolved capability and activation closure.

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
- Ambient modules are loaded only when the host explicitly designates them.
  Their exported data items merge into the analysis environment; collisions
  at the same catalog, schema, or table path are errors. A CLI option such as
  `--catalog <module>` may select those modules as the dev/prod switch.
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

When a module needs to define multiple schemas beneath one non-default
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
- A view that outgrows its module can graduate to an exported `table sql` in
  another module and be imported explicitly — the dbt-model promotion path,
  with no new machinery.
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
  param 'Manhattan' as borough;
  param 0 as min_fare;

  sql:
    SELECT * FROM trips
    WHERE "borough" = $borough AND "fare" >= $min_fare;
}
```

This is the language's two-mechanism law stated once: **`param` is a
runtime value placeholder — the plan is stable and values rebind; `slot`
is expansion-time structural substitution — it shapes the plan** (columns,
function names, declaration blocks) and exists only in `define` declarations.
`match` is the litmus test: it branches over enum slots because selecting
structure is an expansion-time act, so a param can never drive a `match`.
Mode-like runtime variation stays inside the query as ordinary SQL over
the placeholder (`date_trunc($granularity, "pickup_at")`, `CASE WHEN`) —
varying values, never plan shape; a genuinely structural mode is a
`define transform` with an enum slot.
Catalog tables take params, never slots, and two rules diverge from chart
params because the catalog must remain a fully resolved, browsable
surface:

- **Every scalar param carries a required row-free SQL initializer before
  `as`; its DataFusion-planned Arrow type is the exact placeholder contract.** A
  bare `FROM borough_trips` is always
  valid — it binds the declared initial values. A query with genuinely required inputs is
  a `define transform`, which also differs in kind: it rewrites an
  upstream `input` relation mid-pipeline rather than acting as a source.
  An explicit `CAST` fixes placeholder schema when a particular type is needed.
- **Param values are scalar literals.** A placeholder holds a value, not
  an identifier — a table whose *columns* vary by caller is structural
  parameterization, which is `slot`/define territory.

Charts bind table params at the use site — chart params never appear
inside the catalog; their values flow in from outside. In a `data:`
block, bindings are ordinary properties:

```avenger
data: {
  table: borough_trips;
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
SQL parser already accepts); unbound params take their initializer value; a param
name may not collide with the data block's reserved properties (`table`,
`sql`, `url`, `values`). Params and their initial values are part of the
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
  param 'Manhattan' as borough;

  sql:
    SELECT t.*, z."zone_name"
    FROM borough_trips(borough => $borough) AS t
    JOIN zones AS z USING ("zone_id");
}
```

- **A table-function argument is a row-free scalar SQL expression**. It may use
  literals, operators, scalar functions, `CASE`, parentheses, and visible
  scalar `$param` bindings — the calling table's own params here, the chart's
  params in chart scopes. It may not use columns, stores, subqueries, event/item
  context, or transform outputs. Forwarding
  composes placeholders: the inlined plan carries the outer placeholder,
  so an entire chain still plans once and rebinds at execution —
  `zoned_trips(borough => 'Queens')` reaches through to the `borough_trips`
  filter. The callee param supplies the destination Arrow type and the complete
  expression receives the strict cast from
  [Typed Value Boundaries](#typed-value-boundaries). A binding such as
  `borough => upper($selected_borough)` therefore remains one stable logical
  expression over an outer placeholder and does not require replanning when
  the param changes.
- **Materialization is the only boundary in a chain.** Logical links
  inline end to end, so a chart predicate pushes through the whole chain
  into the sources. A `materialize: session;` link computes once and holds
  batches; downstream pushdown stops there — filters apply against the
  held batches, not the original source. Fingerprints compose: a
  materialized link fingerprints its fully-inlined upstream plan, bound
  params, and source snapshot identities, so an upstream edit or a new
  binding re-materializes exactly the links it affects.
- **Chains survive distribution.** Names inside an imported module resolve
  against that module's lexical bindings before exports are bound in the
  consumer, so a pack's internal chains (`FROM cars` inside the Vega module)
  keep pointing at the pack's own tables — consumer-local names never capture
  them.

### Dataset Packs

Dataset packs are ordinary source modules whose public interface consists of
exported tables, schemas, or catalogs. They use exactly the same explicit
imports and fetch-pin machinery as definitions and mixed modules. A portable
chart imports the data it needs; a deployment may instead supply an explicitly
configured ambient module.

A pack may export one schema containing several tables:

```avenger
-- vega-datasets.avenger, published on a CDN
avenger 1;

export schema tables as vega {
  table json as cars   { path: 'data/cars.json'; }
  table csv  as stocks { path: 'data/stocks.csv'; }
}
```

```avenger
avenger 1;

import { vega } from 'https://cdn.example.com/vega-datasets.avenger'
  sha256 '4c1e...';

chart cartesian as cars_scatter {
  data: { table: vega.cars; }

  mark symbol {
    x: encoded "Horsepower";
    y: encoded "Miles_per_Gallon";
    fill: encoded "Origin";
  }
}
```

A dataset pack is nothing special: it is a `.avenger` module at a URL,
hash-pinned, publishable, and vendorable like any other module. It is the
natural home for well-known teaching data and what runnable documentation
examples import. Collisions between imported relations and ambient relations
are ordinary binding or catalog-path errors.

Three rules make packs behave predictably:

- **Exports are explicit.** A pack may export one or many data items. Consumers
  use named imports (`import { vega as samples } ...`) or a namespace import;
  private helpers remain bound in the producer.
- **Relative `path:` values resolve against the module's own location**
  — a filesystem path locally, the URL base when fetched (the same
  ES-modules rule imports use) — so a pack published beside its data files
  on a CDN just works.
- **The pin covers the catalog, not the data.** `sha256` verifies the pack
  text; reading the tables it names is ordinary data access under the
  host's capability flags (`--allow-net`), same as any other source.

## Groups

`mark group` maps to `MarkGroup`. It is a language-owned core mark and an
authoring/data-preparation container.
It does not create a scenegraph group or independent render surface. Primitive
marks inside groups are flattened for rendering.

```avenger
mark group as manual_box_plot {
  mark group as summary {
    transform aggregate as stats {
      group_by: "group";
      expressions:
        approx_percentile_cont("value", 0.25) AS q1,
        median("value") AS median,
        approx_percentile_cont("value", 0.75) AS q3;
    }

    mark rect as box {
      x: encoded stats.q1;
      x2: encoded stats.q3;
    }
  }
}
```

Nested mark groups are the normal way to express branchy dataflow. A group
inherits its data context from its parent unless it sets its own `data:`.
Transform stages in a group define that group's local data context for child
marks and child mark groups.

To avoid hidden rewrites, the first implementation should require
group-level transform declarations to appear before child `mark` and
`mark group`
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
  expressions: "price" * "quantity" AS total;
}

mark rect {
  y: encoded "total";
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
  expressions:
    "profit" / "revenue" AS margin,
    "category" || ': ' || cast("amount" as varchar) AS label;
}

transform aggregate as totals {
  group_by: ["category", "segment"];
  expressions:
    sum("amount") AS total,
    count(*) AS count;
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

The named-projection transforms share syntax but retain their distinct Rust
semantics:

| Transform | `expressions:` | Result and collision rule |
| --- | --- | --- |
| `aggregate` | optional only when `group_by:` is present | Collapses rows; every item must be one aggregate call supported by the native aggregate implementation. A measure alias may not collide with a grouping output or incoming column. |
| `join_aggregate` | required | Appends grouped aggregate results to every input row. Aliases may not collide with input columns. |
| `scalar_aggregate` | required | Publishes derived scalar handles without adding relation columns. `evaluation:` retains its existing eager/lazy meaning. |
| `calculate` | required | Appends or intentionally replaces same-named input columns. All items read the pre-transform input, including when one alias replaces an input column. |
| `window` | required | Appends window results and rejects aliases that collide with input columns. `partition_by:` and `order_by:` supply defaults to every item as before. |
| `select` | required | Replaces the relation with the ordered projection. Direct columns may omit aliases; computed expressions may not. Duplicate resulting column names are invalid. |

Window projection items use SQL window syntax, including `OVER (...)`. An
empty `OVER ()` delegates missing partition and ordering clauses to the
transform-level `partition_by:` and `order_by:` properties.

`aggregate` with neither grouping keys nor measures is invalid. The other
required properties exclude no-op stages. These requirements are authoring
schema rules; the lower-level Rust builders may continue to represent an empty
builder while the language refuses a semantically empty declaration.

Diagnostics may display an alias when present, but resolution allocates an
opaque stage symbol for source maps, provenance, and internal references.
Execution and cache fingerprints derive from the resolved operation, inputs,
configuration, and physical output schema rather than the opaque stage symbol.
A projection alias names both the public handle and, for relation-producing
transforms, the physical output column. Consistently renaming an alias and all
its lexical references therefore changes the resolved plan schema and cache
fingerprint even when the expression is unchanged. The opaque stage symbol is
never printed in DSL or exposed as an output namespace.

Transform sharing scope should also be a property, keeping the header regular:

```avenger
transform aggregate as global_totals {
  scope: shared;
  group_by: "category";
  expressions: sum("amount") AS total;
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

A mark channel is a property. Simple expression-driven channels use an
explicit `encoded` or `direct` mode; optional channels may use `none`:

```avenger
x: encoded "amount";
fill: direct '#2563eb';
opacity: direct coalesce("row_opacity", 0.85);
stroke: none;
```

`encoded` evaluates its SQL expression and passes the result through the
channel policy registered by the active mark and coordinate profile. On a
scale-bearing channel this includes scale resolution, domain contribution,
and range encoding. On an identity-policy channel it remains encoded even
though no scale object is used. `direct` evaluates the same class of SQL
scalar expression and uses the result directly in the channel's declared
output space. It may reference columns, params, contextual values, or
transform outputs; it is not restricted to literals or constants.

For a position channel, a direct result is already in the output space
declared by the coordinate profile. It is not normalized or implicitly
scaled. The authoring schema states that output type and space when the
profile knows them.

Configured channels attach a block to the value:

```avenger
x: encoded "amount" {
  domain_contribution: infer;
  scale: linear {
    zero: true;
    nice: true;
  }
  axis: {
    title: 'Amount';
    grid: true;
  }
}

fill: encoded "region" {
  scale: ordinal {
    range: ['#5778a4', '#e49444', '#d1615d'];
  }
  legend: {
    title: 'Region';
    position: right;
  }
}
```

`domain_contribution` controls whether the encoded values from this channel
participate in automatic scale-domain inference:

- `infer` is the default;
- `exclude` omits only this channel occurrence from automatic domain
  collection.

Exclusion does not disable the channel's scale. The channel still participates
in scale type inference, scale and guide configuration, rendering, and any
explicit or `raw_domain` domain. Other channels that resolve to the same scale
continue to contribute normally, so primary and secondary position channels
such as `x` and `x2` are controlled independently. A conditional channel has
one `domain_contribution` policy for all of its encoded branches. Encoded pattern
channels such as `fill_pattern` follow the same rule.

An inferred scale whose only matching channels use
`domain_contribution: exclude` is an error. Supply another contributing
channel or configure an explicit or raw domain.

Position-channel configuration lives in the same block:

```avenger
y: encoded "group" {
  scale: band {
    domain: ['Alpha', 'Beta', 'Gamma', 'Delta'];
  }
  axis: {
    title: 'Group';
    grid: false;
  }
  band: 0.26;
}

y2: encoded "group" {
  band: 0.74;
}
```

Conditionals are ordered `when` child declarations inside the channel block —
first match wins — with an optional `otherwise`:

```avenger
fill: encoded "region" {
  when {
    predicate: $picked;
    direct: '#2563eb';
  }
  otherwise: {
    direct: '#cbd5e1';
  }
  legend: {
    title: 'Region';
  }
}
```

The channel head is the fallback branch unless an `otherwise:` block replaces
it. Each `when` requires exactly one `predicate:` and exactly one of
`encoded:` or `direct:`. `when` declarations are evaluated in source order
and the first matching predicate wins. An `otherwise:` block likewise
contains exactly one of the two mode properties. Branch payloads accept full
scalar SQL expressions and modes may be mixed freely:

```avenger
fill: direct '#94a3b8' {
  when {
    predicate: "selected";
    direct: '#2563eb';
  }
  when {
    predicate: "use_category_color";
    encoded: "category";
  }
}
```

Channel-level `scale`, `axis`, `legend`, `domain_contribution`, and other
encoding-policy configuration is legal exactly when at least one effective
branch is encoded. Only effective encoded branches participate in scale type
inference and domain collection. An `otherwise:` replacement makes the head
ineffective for those purposes. A direct-only conditional cannot carry scale
or guide configuration.

### Scale Resolution

This section is normative for a resolved Avenger v1 program. Scale resolution
is a semantic part of the language: it determines which values share a scale,
which data values can be displayed, and how those values map into a coordinate
or visual range. It is not a renderer-aesthetic choice.

Some decisions below are owned by the active native implementation profile.
Those points are called out explicitly. The profile is the deterministic
native schema/implementation profile described in
[Authoring Schema Source](#authoring-schema-source);
compiled artifacts and caches record its identity. A custom profile may add
scale kinds and coordinate- or mark-specific policies, but it must implement
the language-owned ordering and locality rules in this section.

#### Scale creation and identity

A channel occurrence is **scale-bearing** when all of the following hold:

1. its payload has an effective `encoded` branch, rather than `direct`,
   `none`, or an entirely direct conditional;
   payload;
2. the active coordinate profile says that the channel uses a scale; and
3. the channel resolves to a scale kind, either explicitly or by the inference
   procedure below.

`direct` and `none` never create, configure, or contribute to a scale. A
direct branch of an otherwise scale-bearing conditional also does not
contribute a domain value. Coordinate-owned partition/layout inputs, such as
facet dimensions, may use channel-shaped syntax without being scale-bearing;
the coordinate's versioned schema declares that distinction.

Each coordinate profile declares its required position-scale families. In the
stock Cartesian profile these are `x` and `y`; in polar they are `r` and
`theta`. The compiler attempts to realize those scales even when only a
secondary member such as `x2` or `y2` is authored.

Every scale has an identity local to one plot. By default, the identity is the
channel name with a trailing decimal suffix removed, so `x`, `x1`, and `x2`
resolve to the local `x` scale while `fill` resolves to `fill`. A native or
expanded defined mark may provide a different resolved scale identity through
its registered channel mapping. That identity is semantic IR, not a
user-visible label.

All scale-bearing occurrences with the same resolved identity in one plot use
one effective scale. This includes occurrences on different marks and inside
`mark group` descendants: a group is a dataflow boundary, not a scale
namespace. A child `plot`/`cell` begins a new scale namespace. Two child plots
never share a scale object merely because both have a scale named `x`.

Scale-configuration fragments attached to occurrences of the same local scale
are merged before type inference. Resolved marks are traversed depth-first in
semantic child order; properties within one declaration use canonical property
order, so authored property order remains nonsemantic. A later fragment
replaces an earlier explicitly set domain, range, ordering field, or
same-named option while preserving fields it does not set. Multiple explicit
scale kinds for one identity must agree; disagreement is a compile-time error,
not last-writer-wins. A compiler-owned plot scale edit is applied after the
merged channel configuration.

Definition expansion does not change any of these results. Private expansion
groups introduce no new scale namespace, and scale identities, contributor
order, configuration precedence, and diagnostics must be equivalent before and
after expansion.

#### Scale-kind inference

The compiler first determines the effective physical Arrow input type from the
first encoded input with a statically known non-`Null` Arrow type in resolved
mark traversal order. For a conditional channel, only encoded branches
participate in this check; direct branches are typed as `Null` in the
compiler-only domain expression. An Arrow
`List<T>` input is inferred from `T`. Every later contributor must be
compatible with the selected scale's domain type; otherwise compilation or
logical planning fails rather than silently selecting a second scale type.

The effective scale kind is selected in this precedence order:

1. the explicit kind in the merged `scale:` configuration;
2. `ordinal` when no kind is explicit but the merged scale has a discrete or
   pattern range;
3. `nested_band` for a position channel authored with `nested([...])`;
4. a compound mark's registered `scale_hint`; incompatible hints for the same
   scale are an error;
5. the coordinate profile's preference for the channel and Arrow type;
6. the first applicable mark profile preference in resolved traversal order;
7. the stock Arrow-type default below.

The stock defaults are:

| Physical Arrow input | Default scale kind |
|---|---|
| signed/unsigned integer, `float32`, `float64` | `linear` |
| `date32`, `date64`, timestamp | `time` |
| `utf8`, `large_utf8`, `utf8_view`, boolean | `ordinal` |
| struct | none; a position channel must use `nested([...])` |
| any other type | none |

A coordinate preference can refine this table; for example, the stock parallel
coordinate profile prefers `point` for string and boolean dimensions. A mark
preference or `scale_hint` selects a kind only—it does not add a domain value.
If the procedure finds no compatible registered scale kind for a
scale-bearing occurrence, compilation fails with the channel, Arrow type, and
active profile in the diagnostic.

The mapping from a preference to a concrete scale implementation, the input
types and options accepted by that implementation, and any additional
coordinate/mark preferences are native-profile-owned. The stock profile
registers `band`, `linear`, `log`, `nested_band`, `ordinal`, `point`, `pow`,
`quantile`, `quantize`, `sqrt`, `symlog`, `threshold`, and `time`.

#### Local domain contribution

Unless an explicit non-raw `domain:` is configured, the local domain
contributor multiset for a scale contains:

- the encoded input expression of every matching channel occurrence on every
  mark in the plot;
- every matching mark-owned domain source declared by the native profile,
  such as the generated channels of a native compound mark; and
- every encoded branch of a matching conditional channel, represented as one
  conditional expression whose direct branches produce `null`.

Each expression is evaluated against that occurrence's data context after its
group/view transforms. `mark group` descendants remain in the containing
plot's contributor set even when groups use different data contexts. Defined
marks contribute exactly what their expanded ordinary marks contribute.

`domain_contribution: exclude` removes that one occurrence (or the whole
conditional occurrence) only from this automatic contributor multiset. It
does not affect scale identity, type inference, configuration, guides,
rendering, an explicit domain, or `raw_domain`. A native mark's registered
domain source may carry the same policy. If every available matching source is
excluded and neither an explicit domain nor `raw_domain` is configured,
compilation/evaluation fails rather than inventing a data domain.

An explicit interval or discrete `domain:` replaces the automatic contributor
multiset for that local scale. A profile-supplied `DomainExprs` source also
replaces ordinary mark collection with its declared relation/expression
sources. `raw_domain` is different: it is a runtime override and retains the
inferred or explicit domain as its fallback.

After all contributor relations are unioned, the registered scale
implementation reduces them to a local domain. The stock profile uses:

- minimum/maximum over all non-null numeric contributors for ordinary numeric
  domains;
- earliest/latest over all non-null temporal contributors;
- the unique non-null categorical values, sorted by the stock scalar total
  order when no `order_by` is present;
- the configured `order_by` aggregate and direction across all contributing
  rows, with the category value as the deterministic tie-breaker; and
- structured unique paths, followed by the per-level ordering rules, for
  `nested_band`.

The reducer used by a third-party scale kind is native-profile-owned and must
be declared by that scale's registry entry. It still receives the complete
language-defined contributor multiset and may not inspect excluded
occurrences.

#### Domain coordination across plots

Domain coordination shares **extents**, not scale instances or scale
configuration. An ordinary channel can configure its coordination target in
the same channel block:

```avenger
x: encoded "height" {
  domain_scope: shared;
  domain_group: 'height';
}
```

`domain_scope` accepts the common coordination values:

- `free` (equivalent to `level(0)`) keeps one domain per leaf plot;
- `level(n)` coordinates with the logical owner `n` child-frame/facet levels
  above the leaf; and
- `shared` coordinates at the root owner.

`domain_group` is an optional nonempty string containing only ASCII letters,
digits, underscores, or hyphens; periods are reserved for future namespacing.
It may appear only with `domain_scope`. Without it, the resolved local scale
identity is the group, so ordinary `x` scales coordinate with other `x`
scales. A named group allows unlike local identities to coordinate—for
example, one matrix cell's `x` scale with another cell's `y` scale when both
represent `height`. Scaled pattern channels use the same coordination
properties.

When at least one occurrence of a local scale declares a domain scope, the
broadest explicitly declared scope is effective; an undeclared occurrence
does not broaden it. All explicit named groups for that local scale must agree
or compilation fails. With no explicit declaration, the stock facet, repeat,
and child-frame profiles use `shared` with the local scale identity as the
group.

For each `(owner, group)` target, compatible local numeric/temporal extents are
unioned by outer minimum/maximum and categorical extents by unique-value union.
The coordinated extent is then supplied back to each participating local scale,
including an empty child that had no local contributor but belongs to a group
with another contributor. Ordered categorical domains are evaluated at the
coordination owner so every member receives one stable order. Explicit domains
do not export an inferred extent and are not widened by coordination.

Repeat's `domain_coordination: matrix` is the high-level spelling for
by-variable coordination: the compiler assigns the repeat variable id as a
named group to each repeated channel. Any explicit channel coordination must
name the same group and may only narrow, not rename or broaden, the
repeat-generated target.

Facet `slots` are independent of scale-domain coordination. `slots: shared`
coordinates which facet values occupy layout slots; it does not make child
scale domains shared. Conversely, `slots: free` may be combined with shared
child domains. Nested-band `level` declarations likewise separate
cross-cell `scope` from within-scale `nest_scope`.

Coordinates with coupled-domain constraints, such as Cartesian
`unit_aspect` or a fitted geo viewport, may run a profile-owned domain
realization step after compatible extents have been grouped. The profile must
declare which scale kinds and container groups it supports and must preserve
the resolved coordination owners; it may not silently combine otherwise free
domains.

#### Domain normalization, raw override, and range

For the stock numeric continuous scales, final fallback-domain construction
has this order:

1. collect and reduce each local contributor multiset;
2. union compatible extents at each domain-coordination target;
3. apply coordinate/mark geometry-aware domain expansion, including
   radius-aware position padding;
4. apply the scale implementation's normalization—in the stock linear family,
   pixel clip padding, then `zero`, then `nice`; and
5. if `raw_domain` evaluates to a valid override, replace the normalized
   fallback domain with that raw interval.

Temporal scales apply their registered temporal `nice` operation. Categorical
scale padding changes range layout/bandwidth rather than adding domain values.
Other scale-specific normalization is native-profile-owned and documented by
the registered scale schema.

Profile-provided domain-affecting defaults such as Cartesian linear `nice` and
`y.zero` are omitted when the author supplies an explicit non-raw domain.
Author-specified options remain authoritative and can explicitly request
normalization of that domain. Rendering-only defaults such as `round` still
apply. User channel options override coordinate and mark defaults; the
compiler-owned plot scale edit is last.

`raw_domain` is accepted only by numeric-domain, continuous-range scales. A
valid value is a two-element, finite, nondegenerate numeric interval. It is a
literal viewport override: it bypasses fallback-domain normalization. `null`,
partially null, nonfinite, malformed, or degenerate results do not fail an
interaction; they use the normalized inferred/explicit fallback. A raw-only
configuration may use a neutral fallback internally, but that fallback is not
an authored or shareable data domain.

Position-scale ranges are coordinate-owned: Cartesian `x` maps across plot
width, Cartesian `y` maps from plot height to zero, polar `theta` maps to
`[0, 2π]`, and polar `r` maps to half the smaller plot dimension. That binding
overrides a channel-authored numeric range for the same position scale.
For a non-position scale, range precedence is: explicit merged range,
mark-profile default, theme range, then stock channel default. Range selection
does not alter the contributor set or domain ownership.

> **Rust alignment required before this contract is considered implemented.**
> The Rust model already implements the contributor, normalization, raw-domain,
> range, and extent-coordination behavior above. The language lowering still
> needs ordinary-channel `domain_scope`/`domain_group`, agreement diagnostics
> for coordination groups and explicit scale kinds, and scale-kind selection
> from the fully merged effective configuration rather than the first raw
> channel fragment.

### Scale Blocks

Scale blocks are typed property objects; unknown properties are delegated to
the scale implementation schema:

```avenger
x: encoded "amount" {
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

fill: encoded "category" {
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
remaining per-type options come from the generated language schema. Their
merge precedence, inference role, domain semantics, and range ownership follow
[Scale Resolution](#scale-resolution).

### Nested Position Channels

Nested categorical positions use the `nested([...])` channel expression with
ordered `level` child declarations:

```avenger
x: encoded nested(["region", "category"]) {
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

`scope` maps to that nested level's cross-cell domain-coordination scope.
`nest_scope` accepts `free` or `shared` and maps to `NestScope`, controlling
whether child categories reserve common slots inside the one nested scale.
`boundary` accepts `band(<expr>)` or `level_band(<level>, <expr>)`, matching
`PositionBoundary`.

### Colorbar Overlays

Colorbar overlays are local mark blocks hosted by a standard legend in the
injected Cartesian colorbar coordinate space:

```avenger
fill: encoded "value" {
  scale: linear { domain: [0, 100]; }
  legend: {
    title: 'Value';

    overlay: {
      mark group as thresholds {
        mark rule as warning {
          x: encoded 80;
          x2: encoded 80;
          y: encoded 0;
          y2: encoded 1;
          stroke: direct '#111827';
          stroke_width: direct 2;
        }
      }
    }
  }
}
```

An authored legend block has at most one `overlay:` property. The property has
no head or scalar properties and must contain at least one direct `mark`
declaration. Direct transforms, tools, widgets, events, and properties are
invalid; use an inner `mark group` when marks need shared data, a store binding,
a view, or transforms. Imported defined marks are valid children because they
expand through the ordinary mark pipeline.

The block receives no chart-row relation. Each contained mark therefore uses
unit data or an explicit `data:`/store binding. Both orientations expose
Cartesian scales: the gradient axis represents the legend value and the
cross-axis has domain `[0, 1]`. Overlay descendants cannot define position
scales, visible legends, or positioned subplots. They remain noninteractive
and local to the legend: authored names are retained for diagnostics and
provenance, but no chart-level mark paths are published. If multiple channel
legends merge, their overlay mark collections concatenate. An effective
legend with overlays must render a continuous colorbar surface.

## Events

Events follow the same body rule: configuration properties plus ordered
`set` actions. Actions use `=` deliberately — imperative assignment, as
distinct from declarative `:` configuration:

```avenger
on click as select_outlier {
  target: mark manual_box_plot.fence.outlier_layer.outliers;
  filter: datum."value" > $threshold;
  consume: true;

  set selected_group = datum."group";
  set selected_value = datum."value";
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

The unqualified name `cursor` is reserved in the state-binding namespace, so
no scalar param, store, or selection may bind it. This keeps `set cursor`
unambiguously the cursor effect rather than a state assignment.

Literal styles are checked statically against the registered `CursorStyle`
inventory; computed non-null strings are checked at runtime. `NULL` publishes
no cursor change, while `default` explicitly resets the application cursor. An
invalid non-null style fails the action and aborts its transaction.

One invocation of one event binding is a **state transaction**. Its `set`
actions execute in source order against a private working state, and every
action sees mutations made by preceding actions in that invocation. Thus a
later scalar `$param` read observes an earlier scalar `set`, and a later store
operation observes rows written by an earlier store `set`, **when both
references resolve
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
set brush = insert_rows {
  row { id: datum."id"; }
}
set brush_count = (SELECT count(*) FROM $brush);
```

The second action sees the row inserted by the first. Each action evaluates all
of its expressions against one stable pre-action working snapshot, then applies
its mutation as a unit. A failure in query planning, execution, conversion, or
mutation aborts the whole event transaction.

There is no v1 spelling for a live working-state read from a non-current owner.
For example, if a drag has crossed facets, neither `$width` nor `$width@start`
reads a value just written by `set width at start = ...`: the first reads
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

  set drag_x at start = event.coord.x;
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
set brush at start = replace_rows {
  row { x0: event.start.coord.x; x1: event.coord.x; }
}
set drag_x at current = event.coord.x;
set picked at start = clear;

set active_brush at start replacing scopes = replace_rows {
  row { x0: event.start.coord.x; x1: event.coord.x; }
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
start-derived event values remain explicit through contextual accesses such as
`event.start.coord.x`, and a start-derived param value uses `$param@start`, while
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
set brush at start replacing scopes = replace_rows { ... }
set active at current replacing scopes = true;
```

At that action's position in the transaction, the modifier removes every
existing concrete owner instance of the target declaration, then applies the
RHS to only the owner selected by `at current|start`. Without it, other owner
instances remain unchanged. Later actions see the resulting working state, and
failure rolls back both the removals and the write. Removed scalar-param owners
fall back to the evaluated initializer; removed store owners lazily fall back to
their declared initial rows. The modifier is invalid for selections, whose
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
set hover = clear;
set hover = insert_rows  { row { id: datum."id"; } }
set hover = replace_rows { row { id: datum."id"; } }
set hover = upsert_rows  { row { id: datum."id"; x: event.coord.x; } }
set hover = update_by_key { key { id: datum."id"; } fields { x: event.coord.x; } }
set hover = delete_by_key { key { id: datum."id"; } }
set hover = toggle_rows  { row { id: datum."id"; } }

set picked = clear;
set picked = clear_in_scope { scope: level(1); }
set picked = toggle_clauses {
  clause {
    id: datum."id";
    equality {
      id { field: "id"; value: datum."id"; }
    }
  }
}
```

Update kinds mirror `StoreUpdate` and `SelectionUpdate`; `replace_all_clauses`,
`replace_clauses_in_scope`, and `upsert_clauses` follow the same shape as
`toggle_clauses`, while `delete_clauses` and `delete_clauses_in_scope` take
clause ids through a non-empty `ids: [...]` array (and the scoped form also
requires `scope:`). Clause predicates support keyed `equality` and `interval`
dimensions (`x { field: "x"; from: event.start.coord.x; to: event.coord.x; }`). The
parent fixes the child category and predicate type, so the dimension ID is the
complete header; it is not a scoped declaration or `as` binder. Geometry-driven
selection uses the scene-query update kinds —
`replace_all_from_scene_query`, `replace_from_scene_query_in_scope`,
`upsert_from_scene_query`, and `toggle_from_scene_query`, the primitives
that make lasso and box selection definable in the language:

```avenger
set picked = replace_all_from_scene_query {
  geometry: polygon(event.path);
  policy: intersects;
  marks: [points];
  fields: [{ id: 'id'; datum: 'id'; field: "id"; }];
  unique_by: ['id'];
}
```

Scene-query `geometry:` accepts `polygon(points)`,
`rect(x0, y0, x1, y1)`, or `circle(cx, cy, radius)`. `policy:` accepts
`intersects`, `envelope_intersects`, `contained`, `anchor_inside`, or
`centroid_inside`. The required non-empty `fields:` array maps stable selection
dimension ids to mark datum fields and data-field expressions; `datum:`
defaults to the field `id`, while `unique_by:` defaults to all captured field
ids in order. Optional `max_hits:`, `sharing:`, and `clause_id:` configure the
native scene query without exposing renderer-internal row metadata.

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
boundary, so the destination-cast rule above does not suppress those native
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
`NULL`; missing non-nullable fields, unknown fields, failed strict casts, and
null key fields are errors. Every field, key, and patch RHS is a SQL expression
strictly cast to its declared field type; nested members recursively use their
declared destination types. A multi-row keyed payload must contain unique key tuples
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
kind. Imperative actions accept an unprefixed lexical or qualified path. The
compiler resolves the target first; its scalar, store, or selection category
then determines the valid RHS expression or update operation. These are typed
l-values, not value reads, so they deliberately do not take `$`:

```avenger
set zoom.domain = span(0, 100);
set hover.hovered = clear;
set brush.points = clear;
```

Inside the owning component, the lexical forms `set domain`, `set hovered`, and
`set points` remain canonical. A qualified path must resolve through explicit
exports at every component boundary. Scalar params, stores, and selections retain
their categories for validation. All three share the state-binding namespace, while a component's
external aliases occupy the single collision-checked interface namespace
established above.

## Tools, Selections, Stores, And Views

The same object syntax covers interaction state. A scalar `param` maps one
row-free SQL initializer to a name. `store` and `selection` are peer state
declaration categories with category-specific bodies:

```avenger
param CAST(NULL AS DOUBLE[]) as x_domain {
  sharing: shared;
}

store as hover {
  field utf8 id;
  field float64 x nullable;
  field float64 y nullable;
  primary_key: [id];
  sharing: free;

  row { id: 'initial'; x: NULL; y: NULL; }
}

selection as picked {
  empty: none;
  combine: union;
}
```

Scalar params, stores, and selections share one collision-checked
state-binding namespace within each scope. Nested scopes may shadow. `$name`
always selects the nearest declaration before requiring the use site's scalar,
table, or current-row predicate role. Selections also retain typed references
for properties such as `selection: picked`. Their bodies and mutation
operations remain distinct because scalar
replacement, table-row updates, and clause updates have different schemas.

The initializer's DataFusion-planned Arrow type is carried by the placeholder
and runtime `ScalarValue`. Initializers are evaluated once, in dependency
order, and are not reactive definitions. Scalar params remain nullable, but a
bare `NULL` has no concrete type and is rejected; authors write `CAST(NULL AS
DOUBLE)` or `arrow_cast` for Arrow types that SQL cannot name faithfully.
Host bindings, table-function arguments, and action assignments follow the
exact [Typed Value Boundaries](#typed-value-boundaries) rule and must consume or
strictly cast to that inferred type.

Core resolution stores `ParamTypeContract::Inferred` plus the symbolic
initializer and remains DataFusion-free. Compiler analysis produces a
generation-local `ParamTypeIndex<ParamId, DataType>` from the actual compile
environment before catalog or chart planning. `CompiledParamSpec` and the
ordinary Rust `Param` API then derive their exact contract from the resulting
typed `ScalarValue`, so Rust and DSL runtime behavior agree without requiring
the DSL author to repeat the type.

Scalar params have no behavioral `kind:` or authored `type:`. `store` and
`selection` are closed header categories, not metadata. Consumers impose role-specific type constraints at
the use site. `raw_domain: $x_domain` requires
`list(float64)`. Cursor is not a param role: it is a write-only transactional
event effect spelled `set cursor`, defined in [Events](#events). Removing a
consumer role does not change a param's type or identity, and a compatible param
may serve more than one consumer.

`sharing:` maps directly to Rust's `CoordinationScope` and defaults to
`shared` for scalar params and stores:

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
Hot-reload migration uses a separate deterministic `StateMigrationKey` derived
from stable authored structure. Renaming only a public export alias does not
change this key. Renaming a state declaration's source binder or a containing
component/tool instance binder does change the key and therefore intentionally
resets that state on reload. All references are re-resolved to opaque typed IDs
after either rename, so this migration boundary cannot create dangling runtime
references.
Lexical component and tool scopes are a source-resolution rule rather than a
nested runtime storage layout: after resolution, their state declarations are
hoisted into the compiled chart's typed root registries. The concrete runtime
key remains `(declaration identity, owner path)`, so hoisting does not alter
facet sharing or component-instance isolation.

Reads and event writes use the same owner-path calculation. A scalar param with
no written value at its resolved owner uses its evaluated initializer.
Declared store rows are likewise the immutable authored baseline for every
owner. The root instance is seeded eagerly; an as-yet unwritten non-root
`free` or `level(n)` owner reads those rows lazily at revision zero. Its first
effective mutation materializes that owner's rows and advances its revision;
an update equal to the authored baseline remains revision zero. Store revisions
and materialization keys are per concrete `(store, owner_path)` instance.

An admitted non-shared store action must have a routed interaction scope for
its selected `at current` or `at start` surface. If no owner can be derived, the
binding fails with a diagnostic naming the store, sharing mode, assignment
surface, and event surface. The whole transaction rolls back, and failed
admission does not advance consumption, throttling, or `previous` state. This
never silently redirects a misrouted write to the root. Shared stores and
compiler-owned root surfaces always select the root. An evaluated unfaceted
plot scope also resolves `free` and `level(n)` to the root through ordinary
root saturation, so unfaceted interactions remain valid.

For a raw-domain param, validation additionally requires its sharing to be at
least as broad as the scale domain it controls; native tools may explicitly
select a scope or mirror the target scale's sharing.

Chart/component params are predeclared within their lexical scope, so initializers
may form an acyclic forward-reference graph:

```avenger
param $lower_limit + 10 as upper_limit;
param 0 as lower_limit;
```

Initializers are planned, typed, and evaluated once in topological order when initial state is built;
later writes to `lower_limit` do not reactively recompute `upper_limit`. Use an
ordinary SQL expression at the consumption site when a derived live value is
intended. Catalog-table params retain self-contained row-free initializers as
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
between-binding that accumulates `event.path` and applies a scene-query
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
  y: encoded $min_fare.value;
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
    set picked = clear;
    set query = '';
  }
}
```

The action runs when the Button's monotonic activation count changes and
lowers through the same serializable `ChartAction`/parameter-change reaction
seam as the landed Rust `Button::action`. It has no event datum or contextual
event access,
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
mark group as viewed_points {
  view cartesian as viewport {
    x_domain: $x_domain;
    y_domain: $y_domain;

    transform filter {
      predicate: "amount" > 0;
    }

    mark symbol as points {
      x: encoded "x";
      y: encoded "y";
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
    table: observations;
  }

  mark group as manual_box_plot {
    scale_hint {
      channel: y;
      type: band;
    }

    mark group as fence {
      transform join_aggregate as fence {
        group_by: "group";
        expressions:
          approx_percentile_cont("value", 0.25) AS q1,
          approx_percentile_cont("value", 0.75) AS q3;
      }

      mark group as inliers {
        transform filter {
          predicate:
            "value" >= fence.q1 - (fence.q3 - fence.q1) * 1.5
            and "value" <= fence.q3 + (fence.q3 - fence.q1) * 1.5;
        }

        transform aggregate as whisker {
          group_by: "group";
          expressions:
            min("value") AS whisker_low,
            max("value") AS whisker_high;
        }

        mark rule as whiskers {
          x: encoded whisker.whisker_low;
          x2: encoded whisker.whisker_high;
          y: encoded "group" { band: 0.5; }
          y2: encoded "group" { band: 0.5; }
          stroke: direct '#475569';
          stroke_width: direct 1.5;
          zindex: 1;
        }

        mark rule as lower_cap {
          x: encoded whisker.whisker_low;
          x2: encoded whisker.whisker_low;
          y: encoded "group" { band: 0.32; }
          y2: encoded "group" { band: 0.68; }
          stroke: direct '#475569';
          stroke_width: direct 1.5;
          zindex: 2;
        }

        mark rule as upper_cap {
          x: encoded whisker.whisker_high;
          x2: encoded whisker.whisker_high;
          y: encoded "group" { band: 0.32; }
          y2: encoded "group" { band: 0.68; }
          stroke: direct '#475569';
          stroke_width: direct 1.5;
          zindex: 2;
        }
      }

      mark group as outlier_layer {
        transform filter {
          predicate:
            "value" < fence.q1 - (fence.q3 - fence.q1) * 1.5
            or "value" > fence.q3 + (fence.q3 - fence.q1) * 1.5;
        }

        mark symbol as outliers {
          x: encoded "value";
          y: encoded "group" { band: 0.5; }
          fill: direct '#f97316';
          stroke: direct '#ffffff';
          stroke_width: direct 1.25;
          size: direct 95;
          zindex: 5;
        }
      }
    }

    mark group as summary {
      transform aggregate as stats {
        group_by: "group";
        expressions:
          approx_percentile_cont("value", 0.25) AS q1,
          median("value") AS median,
          approx_percentile_cont("value", 0.75) AS q3;
      }

      mark rect as box {
        x: encoded stats.q1 {
          scale: linear {
            domain: [0, 36];
          }
          axis: {
            title: 'Value';
            grid: true;
          }
        }
        x2: encoded stats.q3;
        y: encoded "group" {
          scale: band {
            domain: ['Alpha', 'Beta', 'Gamma', 'Delta'];
          }
          axis: {
            title: 'Group';
            grid: false;
          }
          band: 0.26;
        }
        y2: encoded "group" { band: 0.74; }
        fill: direct '#bfdbfe';
        stroke: direct '#2563eb';
        stroke_width: direct 1.5;
        zindex: 3;
      }

      mark rule as median {
        x: encoded stats.median;
        x2: encoded stats.median;
        y: encoded "group" { band: 0.24; }
        y2: encoded "group" { band: 0.76; }
        stroke: direct '#1e3a8a';
        stroke_width: direct 2.2;
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
  set selected_group = datum."group";
}
```

Thus the box is `manual_box_plot.summary.box`, while the whiskers are
`manual_box_plot.fence.inliers.whiskers`. Anonymous groups do not add a path
segment. There is no `public: true;` property and no leaf-suffix shorthand.

## Chart Chrome And Layout

Chart-level properties live in the mixed `chart` body:

```avenger
chart cartesian as sales {
  param 520.0 as canvas_height;

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
    canvas: { width: 900; height: $canvas_height; }
    plot: auto;
    margins: { left: 56; right: 20; top: 30; bottom: 46; }
    debug_overlay: components;
  }

  guide: {
    plot_background_color: '#ffffff';
  }

  time: { timezone: 'UTC'; week_start: monday; }
  format: { number_locale: 'en-US'; datetime_locale: 'en-US'; }

  mark rect { x: encoded "region"; y: encoded "sales"; }
}
```

`layout.canvas` and `layout.plot` accept fixed width/height objects,
single-axis constraints, or `auto`; `debug_overlay` maps to
`LayoutDebugOverlayMode` (`off`, `components`, `allocation_demand`, `all`).
When a canvas dimension is a bare `float64` parameter reference, a native host
may bind that parameter to virtual-canvas resize input. Constants and compound
expressions remain chart-controlled because a host cannot invert them safely.
This direct binding is the resize declaration; there is no separate
`layout.resize` property.
Non-channel color-valued properties take plain strings. The `direct` mode is
needed only at an expression-driven channel boundary.

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
    mark line { x: encoded "date"; y: encoded "total"; }
  }

  cell cartesian as right {
    mark rect { x: encoded "category"; y: encoded "amount"; }
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
    mark line { x: encoded "date"; y: encoded "total"; }
  }

  cell cartesian as detail at { row: 1; column: 0; } {
    mark symbol {
      x: encoded "date";
      y: encoded "value";
      fill: encoded "category";
    }
  }

  cell zerod as badge at { row: 1; column: 1; } {
    mark text { text: direct 'Summary'; }
  }
}
```

Wrap concat chooses a column count from `columns:` (fixed) or
`responsive_columns:` (target minimum cell width):

```avenger
chart wrap_concat as small_multiples {
  responsive_columns: 220;
  spacing: 10;

  cell cartesian {
    mark symbol { x: encoded "horsepower"; y: encoded "mpg"; }
  }
  cell cartesian {
    mark rect { x: encoded "cylinders"; y: encoded count(*); }
  }
  cell polar {
    mark line { theta: encoded "month"; radius: encoded "sales"; }
  }
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
    mark rect { x: encoded "category"; y: encoded "sales"; }
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
    mark symbol {
      x: encoded "horsepower";
      y: encoded "mpg";
      fill: encoded "origin";
    }
  }
}
```

Mark-level facet data scope is a normal mark property: `filtered` (default),
`broadcast`, or `level(n)`:

```avenger
mark rule as global_median {
  facet_data_scope: broadcast;
  x: encoded median("value");
  x2: encoded median("value");
}
```

All native and defined marks share the following coordinate-independent
properties. These are schema-owned generic mark properties rather than
encoding channels:

- `visible`: a scalar boolean SQL expression;
- `details`: one field name or an ordered array of field names retained for
  interaction details and path partitioning;
- `zindex`: a signed 32-bit integer rendering order;
- `facet_data_scope`: `filtered`, `broadcast`, or `level(n)` where `n` is from
  0 through 255;
- `geometry_space`: `coordinate` (the default) or `display`.

`details` contains data-field names, not arbitrary scalar expressions.
`geometry_space: coordinate` builds geometry in encoded coordinate space and
then projects it; `display` projects channel values first and builds geometry
in display space.

### Repeat

Repeat variables are ordered child declarations whose ids become repeat
placeholder metadata; cells instantiate once per repeat context whose
optional `when:` predicate is true:

```avenger
chart repeat_grid as scatter_matrix {
  variable row mpg { expr: "mpg"; title: 'MPG'; }
  variable row hp { expr: "horsepower"; title: 'Horsepower'; }
  variable column weight { expr: "weight"; title: 'Weight'; }
  variable column accel { expr: "acceleration"; title: 'Acceleration'; }

  domain_coordination: matrix;

  cell cartesian {
    when: repeat.row_id <> repeat.column_id;
    mark symbol {
      x: encoded repeat.column;
      y: encoded repeat.row;
      fill: encoded "origin";
    }
  }

  cell zerod {
    when: repeat.row_id = repeat.column_id;
    mark text { text: encoded repeat.row_title; }
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
    mark line { x: encoded "time"; y: encoded "value"; }
  }
}

mark subplot as inset {
  x: encoded avg("x");
  y: encoded avg("y");
  width: 140;
  height: 90;
  key: "category";

  plot cartesian {
    mark symbol {
      x: encoded "local_x";
      y: encoded "local_y";
      fill: encoded "group";
    }
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
  data: { table: cars; }

  mark box_plot as mpg_box {
    category: "origin";
    values: "mpg";
    extent: 1.5;

    part box {
      fill: encoded "origin" { legend: none; }
      stroke: direct '#1f2937';
      opacity: direct 0.55;
    }
    part median { stroke: direct '#111827'; stroke_width: direct 2; }
    part whiskers { stroke: direct '#374151'; }
    part caps { stroke: direct '#374151'; }
    part outliers { size: direct 28; fill: direct '#ffffff'; stroke: encoded "origin"; }
  }

  mark violin as mpg_density {
    band_axis: y;              -- horizontal: rebind the channel slots
    value_axis: x;
    category: "origin";
    values: "mpg";
    width_normalization: per_violin;

    part body { fill: encoded "origin"; opacity: direct 0.58; }
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
  x: encoded "displacement";
  y: encoded "mpg";
  fill: encoded "origin";

  adjust expr {
    x: item.channel.x + 4;
    y: item.channel.y - 2;
  }

  adjust jitter as jittered {
    axis: x;
    width_px: 18;
    seed: 7;
    apply: { x: jittered.x; }
  }

  derive text as labels {
    text: item.data."name";
    x: item.channel.x;
    y: item.bbox.top - 4;
    zindex: 5;
  }
}
```

`adjust nudge`, `adjust jitter`, and `adjust dodge` are the stock registered
`Adjust` kinds. Their registry entries own property validation, documented
outputs, and native transform construction; `apply:` remains the generic
language routing from a bound adjustment output to target mark channels.
Third-party native modules may export additional registered adjustment kinds
without changing the parser. Derived marks are limited to `Symbol`, `Rule`,
`Rect`, and `Text`. Their property assignments are item-frame expressions,
not ordinary mark-channel slots: they evaluate after the source item's
channels have been scaled and assign the derived primitive's item-space
attributes directly. They therefore do not request or contribute to scales,
and a literal in a `derive` body remains a direct item-space literal without
a mode qualifier. This is a context distinction rather than a second
ordinary-channel spelling; ordinary mark channels and `part` overrides always
state `encoded` or `direct`.

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
third-party `candlestick` definition is themable as
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

  mark symbol { x: encoded "x"; y: encoded "y"; }
}
```

## Pattern Fill

Pattern fill is a structured channel value with ordered `layer` children;
encoded patterns use a pattern-valued scale range (array elements may be
`none` or `pattern { ... }` objects):

```avenger
mark rect as bars {
  x: encoded "category";
  y: encoded "value";

  fill_pattern: pattern {
    anchor: plot;
    ink: auto_contrast { opacity: 0.22; }
    layer stripe { angle: 45; spacing_px: 12; stroke_width_px: 1.25; }
    layer stripe { angle: 135; spacing_px: 12; stroke_width_px: 1.25; }
  }
}
```

```avenger
fill_pattern: encoded "scenario" {
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
        extent: [viewport.x.domain.start, viewport.x.domain.end];
        bins: viewport.x.pixels;
      }
      y_dim: {
        extent: [viewport.y.domain.start, viewport.y.domain.end];
        bins: viewport.y.pixels;
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
chart geo as map {
  resource tiles as osm {
    kind: xyz;
    url: 'https://tile.openstreetmap.org/{z}/{x}/{y}.png';
    min_zoom: 0;
    max_zoom: 19;
    attribution: 'OpenStreetMap contributors';
  }

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
    fill: encoded "borough";
    stroke: direct '#ffffff';
  }

  mark symbol as stations {
    lon_lat: ["lon", "lat"];
    size: encoded "ridership";
    fill: direct '#ef4444';
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

chart geo as taxi_density {
  resource tiles as osm {
    kind: xyz;
    url: 'https://tile.openstreetmap.org/{z}/{x}/{y}.png';
    attribution: 'OpenStreetMap contributors';
  }

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
          extent: [viewport.x.domain.start, viewport.x.domain.end];
          bins: viewport.x.pixels;
        }
        y_dim: {
          extent: [viewport.y.domain.start, viewport.y.domain.end];
          bins: viewport.y.pixels;
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

    opacity_by_total: {
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

Parallel-coordinate dimensions are discovered from the `dimensions:` bindings
owned by parallel marks. A chart may also provide an optional, sparse
`dimensions:` map for frame-level configuration. Both maps use the same stable
logical dimension IDs, but they have deliberately different responsibilities:

- a chart-level entry such as `mpg: { axis: { ... } }` configures the shared
  frame slot and axis for `mpg`; it does not contain or imply a data expression;
- a mark-level entry such as `mpg: "miles_per_gallon"` binds that mark's data
  expression to the logical `mpg` slot and may carry mark-owned channel, scale,
  and domain configuration;
- the chart-level map need not mention every mark-owned dimension. An omitted
  entry receives the default frame and axis configuration, including the
  dimension ID as the default axis title;
- every chart-level entry must be bound by at least one mark. Configuring an ID
  that no mark binds is an error;
- multiple marks may bind different expressions to the same logical dimension
  ID. The ID is the join key for shared frame configuration and scale
  coordination, not an alias for any one expression.

Dimension IDs are also used by `order`, coordinate-slot overlays, axis drag and
display state, and parallel guide event metadata. `order`, when present, names
each discovered logical dimension exactly once. Coordinate-slot overlays host
an embedded plot for the named logical dimension:

```avenger
chart parallel as cars_parallel {
  dimensions: {
    mpg: { axis: { title: 'MPG'; } }
    horsepower: { axis: { title: 'Horsepower'; } }
  }
  order: [mpg, horsepower];

  mark parallel_line as lines {
    dimensions: {
      mpg: encoded "mpg";
      horsepower: encoded "horsepower" { scale: linear { nice: true; } }
    }
    stroke: encoded "origin";
    opacity: direct 0.35;
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

When the defaults are sufficient, the chart-level map may be omitted entirely:

```avenger
chart parallel as compact_parallel {
  mark parallel_line as lines {
    dimensions: {
      mpg: encoded "miles_per_gallon";
      horsepower: encoded "engine_horsepower";
    }
  }
}
```

Here `mpg` and `horsepower` are still the logical dimension IDs. The quoted
right-hand sides are the mark's expressions, and their column names need not
match those IDs.

Zero-dimensional charts need no special body shape; the schema restricts the
valid mark and channel set:

```avenger
chart zerod as badge {
  mark symbol as status_dot {
    size: direct 160;
    fill: encoded "status";
    shape: direct 'circle';
  }
  mark text as label { text: encoded "status_label"; }
}
```

## Imports And Definitions

Reuse crosses module boundaries through explicit named or namespace imports.
A module may contain any number of private or exported mark, tool, and
transform definitions alongside charts and data items. A consumer imports
only the producer's explicit exports; the source filename never creates a
binding.

Definitions are the language's custom extension tier, alongside native
built-in kinds registered by the host. A `define mark` builds a custom
compound mark, a `define tool` builds a custom behavior from the exposed
event-system constructs, and a `define transform` builds a custom pipeline
from built-in stages and `transform sql`. Native built-ins — including
compound marks, tools, and transforms — remain valid kinds and do not have to
lower through definitions or be expressible using the definition language.
Definitions are structural templates parameterized by slots, including
specialized channel slots, with `match` over enum slots as the only branching
form.

### Defining A Compound Mark

```avenger
-- lib/statistics.avenger
avenger 1;

export define mark error_bar {
  slot expr category;
  slot expr measure;
  slot number cap_width { default: 0.3; }
  export bar;
  export caps;
  export center;

  mark group {
    transform aggregate as stats {
      group_by: category;
      expressions:
        min(measure) AS lo,
        max(measure) AS hi,
        avg(measure) AS mid;
    }

    mark rule as bar {
      x: encoded category { band: 0.5; }
      x2: encoded category { band: 0.5; }
      y: encoded stats.lo;
      y2: encoded stats.hi;
      stroke: direct '#374151';
      stroke_width: direct 1.5;
    }

    mark rule as caps {
      x: encoded category { band: 0.5 - cap_width / 2; }
      x2: encoded category { band: 0.5 + cap_width / 2; }
      y: encoded stats.lo;
      y2: encoded stats.lo;
      stroke: direct '#374151';
    }

    mark symbol as center {
      x: encoded category { band: 0.5; }
      y: encoded stats.mid;
      size: direct 42;
      fill: direct '#111827';
    }
  }
}
```

`slot <shape> <name>` declarations are the definition's explicit property
schema. The closed v1 shapes are `expr`, `expr_list`, `literal`, `number`,
`string`, `boolean`, `enum`, `ref`, `block`, `channel`, and the
transform-only `outputs` shape. The scalar
refinements (`number`, `string`, `boolean`) accept SQL expressions whose
resolved type matches; `literal` accepts any scalar literal without expression
evaluation. A slot is required when it has no `default:` property; `;` is only
the compact empty-body form.
An `enum` slot declares a non-empty, duplicate-free `values:` array and any
default must be a member. A `ref` slot's `kind:` is one of `mark`, `param`,
`selection`, `store`, `tool`, `widget`, or `resource`; `mark group` uses the
ordinary `mark` reference category. Inline views are lexical scopes, not
reference values. A `block` slot accepts a declaration body at its splice
point, with optional `default:` and `exposes:`; the splice position's
authoring schema still determines which child declarations are legal there.
Function names are deliberately not slot values in v1. An open-ended callable
choice is expressed as a complete `expr` supplied by the caller; a definition
that owns the call arguments uses a closed `enum` plus `match`.

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
slot number band_width { default: 0.6; }
slot number cap_width { default: band_width / 2; }
```

Only textually earlier compatible slots are visible while resolving a default;
a later-slot reference is an error. The resolver still checks the dependency
graph defensively, though the earlier-only rule makes a valid cycle impossible.

### Channel Slots

A `slot channel` input binds a *logical* channel to a physical one at the
instantiation site. This is how one definition serves both orientations
without conditionals — orientation is just a channel binding:

```avenger
define mark error_bar {
  slot channel band_axis { default: x; }
  slot channel value_axis { default: y; }
  slot expr category;
  slot expr measure;

  mark group {
    transform aggregate as stats {
      group_by: category;
      expressions:
        min(measure) AS lo,
        max(measure) AS hi;
    }

    mark rule as bar {
      band_axis: encoded category { band: 0.5; }
      value_axis: encoded stats.lo;
      value_axis2: encoded stats.hi;
      stroke: direct '#374151';
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

The `default:` physical channel is the default binding. Omitting it declares a
required channel slot: every instantiation must supply
`logical_channel: physical_channel;`, and resolution reports the missing
binding before expansion. Channel slots never infer a physical channel
from how their logical name is used.

Expansion renames logical channels wherever channel identity appears:

- channel property names on any native or defined kind whose authoring schema
  marks that position as channel-valued, including the interval family —
  binding `value_axis: y;` maps `value_axis2` to `y2`;
- channel-enum property values (`scale_hint { channel: ...; }`,
  `scale_edit { channel: ...; }`, tool `channels:` arrays);
- contextual channel members (`event.coord.value_axis`, `channel.value_axis`,
  and `item.channel.value_axis`).

This is the entire mechanism — a declared rename, not macro splicing.
Property-name substitution is available only through `slot channel` inputs,
never through the other slot shapes.

### Block Slots

A block slot receives *declarations* from the caller, spliced at a marked
position inside the definition and evaluated in the definition's data
context at that point — the mechanism that makes custom compound marks
extensible without forking them. The default content is the slot's default;
a bare `name;` statement marks the splice point:

```avenger
define mark distribution_summary {
  slot channel band_axis { default: x; }
  slot channel value_axis { default: y; }
  slot expr category;
  slot expr values;
  slot block outlier_marks {
    default: {
      mark symbol as outliers {
        band_axis: encoded category { band: 0.5; }
        value_axis: encoded values;
      }
    }
  }

  mark group {
    -- ...fence, whisker, and summary declarations...

    mark group as outlier_layer {
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
      band_axis: encoded category { band: 0.5; }
      value_axis: encoded values;
      text: direct datum."name";
      font_size: direct 9;
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
  slot channel time_axis { default: x; }
  slot channel value_axis { default: y; }
  slot expr time_col;
  slot expr measure;
  slot block annotations { default: { } }

  mark group {
    transform sql {
      query:
        SELECT date_trunc('week', time_col) AS week, avg(measure) AS avg_value
        FROM input
        GROUP BY 1;
    }

    mark line as trend {
      time_axis: encoded "week";
      value_axis: encoded "avg_value";
      stroke: direct '#2563eb';
      stroke_width: direct 2;
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
      time_axis: encoded min("week");
      time_axis2: encoded max("week");
      value_axis: encoded 120000;
      value_axis2: encoded 120000;
      stroke: direct '#dc2626';
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
  read-only layer contains the instance's bound slots and channel slots;
  those names intentionally shadow same-named bare DSL names in the caller,
  with an editor warning on a collision. The remaining caller lexical scope
  stays visible, and data columns come from the splice point's data context.
- By default, block content cannot reach any definition-internal alias. A
  block slot may hand over specific internal handles with its `exposes:`
  property — the definition declares exactly what crosses the boundary, and
  nothing else leaks (Vue's scoped-slot props are the precedent):

  ```avenger
  slot block annotations {
    exposes: [fence, stats];
    default: { }
  }
  ```

  Content passed to that slot may reference `fence.lo` or `stats.median`;
  content passed to a slot without `exposes` may not reference any internal
  alias.
- A block slot has exactly one splice point. Passing an empty block
  (`outlier_marks: { }`) removes the default structure; an empty default
  (`slot block annotations { default: { } }`) makes the slot purely
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

import { error_bar } from './lib/statistics.avenger';

chart cartesian as sales_errors {
  data: { table: sales; }

  mark error_bar as errs {
    category: "region";
    measure: "amount";
    zindex: 3;

    part bar { stroke: direct '#dc2626'; stroke_width: direct 2; }
    part center { fill: direct '#dc2626'; }
  }
}
```

- Properties bind slots by name; a missing slot without a default is an
  error, and an unknown property is an error — except the generic mark
  properties (`visible`, `details`, `zindex`, `facet_data_scope`,
  and `geometry_space`), which apply to the expansion root.
- `part <name> { ... }` targets the mark explicitly exported under `<name>`.
  Its body is property-only and may use every value form accepted by the
  target mark property. In particular, a channel override may use a configured
  channel block with ordered `when`/`otherwise` branches. A supplied property
  replaces that property as one complete value; part overrides do not
  recursively merge channel configuration, conditions, scale blocks, or guide
  blocks with the definition's value. Thus the use site wins independently at
  each property name.
- A `part` body cannot contain direct child declarations such as `adjust`,
  `derive`, transforms, marks, tools, event bindings, or widgets. Authors who
  need structural customization must expose a block slot or define another
  component. Parts cover open-ended property styling so definitions do not
  need a slot per styleable property. The same public-part surface serves
  theme part selectors (`box_plot::part(median)`; see
  [Themes And CSS](#themes-and-css)) and event targets — one declared surface,
  three consumers.
- Definition bodies are private by default, including declarations at the
  definition's own level. When a definition instantiates another definition
  internally, the inner instance's parts and state are likewise not reachable
  from outside. The outer definition publishes a stable external alias with
  `export`, optionally renaming (the Web Components `exportparts` rule):

  ```avenger
  define mark ranged_dots {
    slot expr category;
    slot expr measure;

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
    set hovered = replace_all_clauses {
      clause {
        equality {
          id { field: "id"; value: datum."id"; }
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

- scalar `param <sql-expression> as <name>`, `store as <name>`, and
  `selection as <name>` declarations
  (generated state);
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
  slot channel axis { default: x; }
  slot enum button {
    values: [left, middle, right];
    default: left;
  }

  param CAST(NULL AS DOUBLE[]) as domain;

  scale_edit {
    channel: axis;
    raw_domain: $domain;
  }

  on cursor_moved {
    between: {
      start: mouse_down { button: button; }
      end: mouse_up;
    }

    set domain = span(
      event.domain.axis.start - (event.coord.axis - event.start.coord.axis),
      event.domain.axis.end - (event.coord.axis - event.start.coord.axis)
    );
  }
}
```

`tool drag_pan as pan_x;` pans x; `tool drag_pan as pan_y { axis: y; }` pans y
— the channel slot renames through the scale edit and contextual channel members
alike. Its behavior is exactly the behavior declared here; it makes no parity
claim with a native pan/zoom kind. Geometry-driven custom tools can use the
event system's scene queries:
a lasso-like definition can combine a between-binding accumulating
`event.path` with a scene-query selection update
(`set picked = replace_all_from_scene_query { ... }`). This is an
example of the custom surface, not a required implementation of the native
`lasso_selection` kind. Definitions may also wrap native kinds or imported
definitions, preconfiguring them through slots:

```avenger
-- lib/tools.avenger
avenger 1;

export define tool wheel_zoom {
  slot number base { default: 1.05; }

  param CAST(NULL AS DOUBLE[]) as x_domain;
  param CAST(NULL AS DOUBLE[]) as y_domain;

  tool pan_scroll_zoom {
    x_domain_param: $x_domain;
    y_domain_param: $y_domain;
    scroll_zoom: true;
    zoom_base: base;
  }
}
```

```avenger
export define tool hover_highlight {
  slot ref target { kind: mark; }
  export hovered;

  selection as hovered {
    empty: none;
  }

  on mark_mouse_enter {
    target: mark target;
    set hovered = replace_all_clauses {
      clause {
        equality {
          id { field: "id"; value: datum."id"; }
        }
      }
    }
  }

  on mark_mouse_leave {
    target: mark target;
    set hovered = clear;
  }
}
```

```avenger
avenger 1;

import { hover_highlight, wheel_zoom } from './lib/tools.avenger';

chart cartesian as explorer {
  data: { table: cars; }

  tool wheel_zoom as zoom;
  tool hover_highlight as hover { target: points; }

  mark symbol as points {
    x: encoded "horsepower";
    y: encoded "mpg";
    fill: encoded "origin" {
      when {
        predicate: $hover.hovered;
        direct: '#dc2626';
      }
      otherwise: { encoded: "origin"; }
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
result. `output name;` is the same-name shorthand. Any explicit source uses
the source-to-alias form `output <expression> as <public-name>;`.

A definition that intentionally lets its caller choose a variable set of
named expressions may declare one `slot outputs`:

```avenger
export define transform summarize {
  slot expr_list group_by;
  slot outputs measures;

  transform aggregate {
    group_by: group_by;
    expressions: measures;
  }
}
```

```avenger
transform summarize as stats {
  group_by: ["category"];
  measures:
    sum("amount") AS total,
    avg("amount") AS average;
}
```

`slot outputs <name>` is a required, non-empty named projection list. It is
valid only in `define transform`, may occur at most once, has no `default:`,
and must be consumed exactly once in a named-projection position — an
`expressions:` property or a SQL `SELECT` projection splice. Its caller-authored
aliases automatically join the definition's public output-handle set, so the
example exposes `stats.total` and `stats.average`. A definition containing an
outputs slot therefore always requires an instance binder. Static `output`
declarations may coexist with it; names must be unique after the call-site
projection is known.

The compiler validates every dynamic alias against the expanded pipeline's
final relation just as it validates a static `output`. Dropping or renaming a
slot-provided column before the boundary is an error. When the slot is spliced
into `transform sql`, expansion quotes its SQL alias as necessary to preserve
the exact case-sensitive DSL name. Substitution is structured over parsed
select items, never comma-joined text.

This is a deliberately bounded row-polymorphic interface, not a general
identifier macro. The caller must author each complete alias; a definition
cannot compute, prefix, suffix, rename, filter, iterate over, or synthesize
those names. `slot expr_list` remains the non-naming expression-list input.
Definitions that own a closed output interface continue to use ordinary
`output` declarations.

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
  output calc.share as share;

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
they do for a transform definition. At the pipeline boundary, surviving input
columns plus the declared output expressions form the relation visible to the
next parent stage. Declared outputs may rename final-result expressions;
undeclared columns introduced inside the pipeline remain private.
An instantiation-level `scope:` is retained on the pipeline and supplies the
default scope for child stages that do not set their own.

An output-free `transform pipeline` may omit `as`; it still occupies one parent
stage and has an opaque compiler identity. A pipeline with any `output`
declaration requires a binder, as shown above.

```avenger
-- lib/transforms.avenger
avenger 1;

export define transform share_within {
  slot expr measure;
  slot expr_list partition_keys;
  output share;

  transform sql {
    query:
      SELECT *, measure / sum(measure) OVER (PARTITION BY partition_keys) AS share
      FROM input;
  }
}
```

```avenger
export define transform binned_counts {
  slot expr field;
  slot number maxbins { default: 30; }
  output b.start as start;
  output b.end as end;
  output count;

  transform bin as b {
    field: field;
    maxbins: maxbins;
  }

  transform aggregate {
    group_by: [b.start, b.end];
    expressions: count(*) AS count;
  }
}
```

```avenger
avenger 1;

import { share_within } from './lib/transforms.avenger';

chart cartesian as region_shares {
  data: { table: sales; }

  mark group as shares {
    transform share_within as s {
      measure: "amount";
      partition_keys: ["region", "year"];
    }

    mark rect {
      x: encoded "region";
      y: encoded s.share;
      fill: encoded "year";
    }
  }
}
```

Slot substitution inside `query:` statements follows the declared shape.
`expr` slots splice one expression into the statement AST before planning;
`expr_list` slots splice a comma-separated expression list in list positions
such as `PARTITION BY`, `GROUP BY`, a `SELECT` list, or `IN (...)`.
An `outputs` slot splices one named SQL projection list only into a `SELECT`
list or a schema-declared named-projection property and retains each alias as
an output binder.
Everything else in a statement follows ordinary SQL rules. The resolver warns
when a slot name shadows a column of the incoming context; rename the slot or
quote the column.

Definition-private intermediate SQL columns use one fixed logical spelling:
`__private_<suffix>`. The marker applies to parsed column references,
projection aliases, and wildcard column selectors, including their quoted
forms; it is never recognized inside a string literal, comment, relation name,
or unrelated SQL binder. It is valid only inside a definition and the suffix
must be non-empty. Expansion rewrites every such identifier structurally to
the deterministic physical form
`__av_col_<instance-identity>_<suffix>`. For example:

```avenger
export define transform doubled {
  slot expr measure;
  output doubled;

  transform sql {
    query:
      SELECT *, measure * 2 AS __private_doubled
      FROM input;
  }
  transform sql {
    query:
      SELECT *, __private_doubled AS doubled
      FROM input;
  }
}
```

The source definition name is deliberately absent from this convention:
renaming or importing the definition under another name cannot alter its
private-column semantics. A definition instance that owns private
intermediates rejects an incoming column whose name begins with either
`__private_` or compiler-owned `__av_`; callers must project or rename that
column before the instance. Expansion never probes the runtime schema to
freshen names.

On a defined-transform instantiation, `scope:` becomes the expanded
pipeline's `scope:` property and sets the coordination scope for every child
stage that does not declare its own. Output declarations are validated against
the pipeline's actual final schema at compile time, and they are what the alias
namespace and editor completion expose (`s.share` above).

Slot names must parse as identifiers inside statements, so SQL reserved
words (`order`, `end`, `group`) cannot name slots; the resolver rejects them
with a rename suggestion.

There is no `function` slot. When the caller should choose an open-ended
DataFusion function, it supplies the complete expression so ordinary SQL
planning owns function lookup, overload resolution, argument checking, return
typing, and cache identity:

```avenger
define transform rolling {
  slot expr rolled_expr;
  output rolled;

  transform sql {
    query:
      SELECT *,
        rolled_expr AS rolled
      FROM input;
  }
}
```

The caller may bind `rolled_expr:` to
`median("sales") OVER (ORDER BY "date" ROWS BETWEEN 6 PRECEDING AND CURRENT
ROW)`. If the definition must own the measure or call shape, it exposes a
closed `enum` and selects explicitly authored calls with `match`; v1 does not
model function identifiers as typed first-class values.

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
  slot expr measure;
  slot expr_list partition_keys;
  slot expr order_key;
  slot enum mode {
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

mark rect {
  x: encoded "category";
  y: encoded s.start;
  y2: encoded s.end;
  fill: encoded "segment";
}
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
  slot channel band_axis { default: x; }
  slot channel value_axis { default: y; }
  slot expr category;
  slot expr values;
  slot enum outliers {
    values: [show, hide];
    default: show;
  }

  mark group {
    -- ...fence, whisker, and summary declarations as in the low-level example...

    match outliers {
      show {
        mark group as outlier_layer {
          transform filter {
            predicate: values < fence.lo or values > fence.hi;
          }
          mark symbol as outliers {
            band_axis: encoded category { band: 0.5; }
            value_axis: encoded values;
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
  slot ref target { kind: mark; }
  slot ref sel { kind: selection; }
  slot enum mode {
    values: [toggle, replace];
    default: toggle;
  }

  on click {
    target: mark target;

    match mode {
      toggle {
        set sel = toggle_clauses {
          clause { equality { id { field: "id"; value: datum."id"; } } }
        }
      }
      replace {
        set sel = replace_all_clauses {
          clause { equality { id { field: "id"; value: datum."id"; } } }
        }
      }
    }
  }
}
```

Multiple `match` blocks over different slots compose in one definition, and
`match` composes with channel slots — the custom distribution-summary
sketch above is orientation-generic and outlier-optional at once.

This keeps the conditional budget of the language explicit: `channel`
parameters rename, `match` selects among declared closed variants, SQL
`CASE` handles per-row and constant-foldable expression logic — and nothing
else branches.

### The Standard Library

The host may bundle reusable DSL-authored modules through the `std:` scheme,
with no privileged semantics beyond distribution. This definition library is
separate from the native built-in registry: core built-in marks, tools, and
transforms require no import, while `std:` supplies optional custom
compositions and examples. There is no requirement that a native built-in
have a corresponding standard definition or that the two be behaviorally
equivalent:

```avenger
avenger 1;

import { error_bar, trend_panel } from 'std:marks';
import { hover_highlight } from 'std:tools';
import { share_within } from 'std:transforms';
```

- `std:marks`, `std:tools`, and `std:transforms` expose bundled definitions
  that are useful as reusable compositions, examples, or starting points.
  Their exact inventory is not the native built-in inventory.
  `std:transforms/` definitions are pipelines over built-in stages and
  `transform sql`; `std:themes/` holds plain CSS files
  (`light.css`, `dark.css`, ...) referenced with
  `theme css from 'std:themes/dark.css';` rather than imported.
- Definition-library imports are explicit named or namespace imports; there
  is no implicit prelude.
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
- **No computed slot-derived identifiers.** Channel slots substitute channel
  identities, never names. Mark names and property names are never assembled
  from slot values. The one bounded exception is `slot outputs`: its caller
  supplies complete, explicit projection aliases that become output column and
  handle names. Neither the definition nor expansion may compute or rewrite
  those aliases.

### Project Layout

A project is a directory of files — there is no manifest. Two rules are
normative; everything else is convention.

- **All relative resources resolve against the declaring module**: imports,
  `theme css from` paths, and data paths inside SQL alike. A chart at
  `charts/regional/chart.avenger` reading `FROM 'regions.parquet'` means its
  sibling file, regardless of the working directory the host compiles from
  (the host rebases data paths before execution). The rule applies to every
  module item; semantic restrictions separately prevent `define mark`
  declarations from capturing data or themes.
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
  lib/
    marks.avenger                 -- may export several mark definitions
    interactions.avenger          -- tools and supporting definitions
    transforms.avenger
  themes/
    corporate.css
  data/
    catalog.avenger               -- exported catalog/schema/table items
    orders.parquet
  .env                            -- credentials, gitignored
```

Directory names carry no semantics — imports are explicit paths — so
by-kind directories, per-chart packages (a subdirectory holding one chart
plus its co-located data), mixed feature modules, and flat layouts are all
equally valid. A module's explicit `export` items, not its path, are its
importable surface for other projects.

Every chart entrypoint may double as an example. A host can associate each
entrypoint with a blessed `.png`, using the chart selector as part of the
baseline identity when one module contains several charts. Those
source+entrypoint+image triples form the project's test suite (`avenger test`)
and documentation gallery (`avenger doc`).

### Distribution And The Ecosystem

Import paths take three source forms:

| Form | Example | Resolution |
| --- | --- | --- |
| Standard library | `import { error_bar } from 'std:marks';` | bundled with the language, versioned by the pragma |
| Relative path | `import { trend_panel } from './lib/marks.avenger';` | exact module path, relative to the importer |
| URL | `import { error_bar } from 'https://charts.example.dev/intervals.avenger' sha256 '9f2a...';` | fetched once, hash-verified, cached |
| Native host module | `import * as acme from 'native:com.acme.visuals@1';` | selects an exact schema-described capability registered by the host |

The distribution model is deliberately lightweight. Every dependency names an
exact module origin, imports bind explicit exports, and the loaded graph is
acyclic. The loader therefore fetches and verifies a closure rather than
solving package versions. Two distinct module origins or content identities
remain distinct even when they export the same spelling; consumer aliases and
namespace imports resolve any local collision. Defined-kind instances expand
into independent instance-scoped structures, so vendored copies do not create
runtime symbol collisions.

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
  import { error_bar as eb }
    from 'https://charts.example.dev/intervals.avenger'
    sha256 '9f2ab34c...';
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
- **Bundling is a deterministic linker operation, not a source restriction.**
  `avenger bundle` copies one selected module or chart closure into an
  ordinary import-free module, alpha-renames private bindings, and rewrites
  already-resolved references. `avenger expand` is the separate
  inline-definition operation described below.
- **Versioning is convention.** `@1.2.0` in a filename or URL is naming, not
  mechanism; the language checks only the `avenger` major version of the
  imported file. Authors who want upgrades re-point the URL and re-pin.
- **Publishing is putting modules somewhere.** There is no registry, account,
  or publish step; a raw-file URL on any host (including a Git forge at a
  pinned tag) is a published module. Vendoring selected exports into a local
  module with attribution remains a first-class alternative.

### Expansion

`avenger bundle` and `avenger expand` are independent. Bundling links a
resolved source-item closure into one ordinary module while preserving the
selected public interface. `avenger expand` is **source-level
inline-definition expansion**: every reachable defined-kind
instantiation is replaced by its expansion — slots substituted, `match` arms
resolved, channel slots renamed, block slots spliced, part overrides
merged. Native built-in marks, tools, and transforms remain native declarations;
expansion never attempts to reconstruct them in the definition language.
Built-in widgets likewise remain opaque `widget` declarations; there are no
widget definitions or widget-expansion products. Each
defined-mark instance is emitted as an ordinary group named by the instance
id. The group records the original kind as opaque `component_kind:`
provenance. Definition-authored binding declarations print as `private`;
their deterministic names use
`__av_<instance-identity>_<source-name>`. Resolved
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
marks left in place. Instance-scoped params, stores, selections, transform
aliases, and structural binders are alpha-renamed in expanded source; inline
view binders remain local to their owning view bodies. State references inside
the component are rewritten to the corresponding generated binders, and
group/behavior exports provide the only external qualified aliases. The
compiler assigns opaque semantic ids after resolution in addition to these
printable names.
The output removes imports and private source items needed solely for expanded
definitions — `std:` definitions included — and contains no `define`, `slot` (including
`slot channel`), `match`, or `exposes` constructs from those expansions.
Resolved `export`
declarations remain because they are ordinary group interface declarations,
and resolved `output` declarations remain because they are ordinary
`transform pipeline` interface declarations; neither is macro machinery.
Resolved exports on `tool behavior` remain for the same reason. Native
built-in declarations and semantic resource imports (such as dataset packs)
remain. `component_kind: error_bar;` on the instance group and `export ... as
bar;` together preserve `error_bar::part(bar)` after the definition import is
gone. The equivalence property below forces this to be right, since a themed
chart whose expansion lost either fact would compile differently.
The `__av_` prefix is compiler-owned across DSL binders and physical columns.
Ordinary authored declarations and SQL cannot introduce it. Canonical
expanded source uses it on explicitly private declarations and private
physical columns; canonical bundled source also uses it for deterministic
link-local top-level item names. Expansion reports a collision rather than
capturing an already-present generated-form binder. Because the namespace is
reserved, the instance identity is deterministic and expansion does not
perform schema-dependent or process-fresh name generation. Mark groups and tool
behaviors retain lexical instance boundaries, exported state qualifies through
public aliases, and generated intermediate columns use
`__av_col_<instance-identity>_...`, so two versions of the same library expand
side by side without contact. This is the archival form (it freezes
rendering semantics against imported-definition evolution), the debugging
form (what a chart actually lowers to), and the minimal-runtime form (a
host can execute it with the definition machinery entirely absent).

For example, an IDE's **inline definition** action may produce this ordinary,
hand-editable group (irrelevant mark details abbreviated):

```avenger
mark group as errs {
  component_kind: error_bar;
  export body.bar as bar;
  export body.center as center;

  private mark group as body {
    mark rule as bar {
      x: encoded "category";
      y: encoded "lo";
      y2: encoded "hi";
    }
    mark symbol as center {
      x: encoded "category";
      y: encoded "mid";
    }

    public mark text as annotation {
      x: encoded "category";
      y: encoded "hi";
      text: direct 'high';
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
selection into a new or existing definition module (see
[Language Server](#language-server)).

What this trades away, consciously: automatic dedup and one-command upgrade
propagation across an ecosystem — the benefits a version resolver buys, at
the cost of being one. A future `pkg:` scheme can layer naming and
discovery over the same fetch-pin-cache mechanism without changing the
language.

### Name Binding And Declaration Order

Name binding is category-based rather than uniformly textual. Entering a
lexical scope performs a predeclaration pass for identities whose existence is
independent of execution order: named marks, groups, all three state-binding
categories, tools, resources, and events. Their complete bindings are
therefore available throughout that scope, including before their textual
declaration. Duplicate bindings are diagnosed during predeclaration before any
body is resolved. Scalar params, stores, and selections share one
state-binding namespace, so the same scope cannot reuse `state` across
`param 0 as state`, `store as state`, or `selection as state`;
the other
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

Value bindings in a lexical scope are likewise predeclared before scalar-param
initializers are resolved. A header initializer may reference another
compatible scalar param in that scope, including one declared later. Initializer
dependencies must be acyclic and are evaluated in topological order when
initial state is constructed; an initializer is not a reactive binding after
construction. Catalog-table params retain their stricter contract: every
initializer is a self-contained row-free scalar SQL expression, so table-param
initializers have no dependency graph; their types are still inferred by the
same DataFusion planning pass.

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
- **Deterministic alpha-equivalence**: alpha-renaming applies only to
  definition-private binders and `__private_` intermediate columns. Public
  export aliases, declared transform outputs, definition instance names,
  caller-owned block-slot names, imported public names, and user-facing
  component provenance are fixed observations and are never alpha-renamed.
  Renaming a private source binder or private-column suffix together with all
  of its bound uses is semantics-preserving; renaming any fixed observation is
  an API or chart change. Expansion is deterministic for the resolved module,
  instance path, and definition-local seed.
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
  content sees the caller scope, read-only instance slot bindings (including
  channel slots),
  splice-point data columns, and only the internal handles declared by
  `exposes`.
- **Native and source imports obey the same category collision rules.** A
  named import enters the semantic category of the exported item and may not
  collide with an existing same-category binding; the consumer may rename it
  with `as`. A namespace alias is reserved across all categories. Imports
  never silently shadow core or native kinds.
- Every import has an explicit clause. `import { error_bar as eb } from
  './intervals.avenger';` binds one selected export; `import * as intervals
  from './intervals.avenger';` binds the producer's complete public export
  table under one namespace. Filenames create no bindings. Plain, default,
  side-effect, dynamic, conditional, and re-export forms are invalid.
- Import paths resolve relative to the importing file (filesystem path
  locally, URL base when fetched), and the imported module's `avenger` major
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
ident     unquoted word using    DSL names: kinds, properties, enum values,
          AvengerSqlDialect's    aliases, namespaces, intrinsic operations
          identifier characters
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
declaration's `doc` field rather than dropped. `sql_expr`, `sql_projection`,
and `sql_query` are islands parsed by sqlparser's `Parser` under
`AvengerSqlDialect` from the normalized token stream; the resolver then
rewrites value-binding paths, DSL-qualified names, contextual accesses, and
intrinsic operations.

```ebnf
file          = version , { import } , module_item , { module_item } ;
module_item   = [ "export" ] , ( chart | define | data_bind ) ;
                     (* top-level export is module visibility; child export
                        declarations remain component-interface aliases *)
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
import        = "import" , import_clause , "from" , string ,
                [ "sha256" , string ] , ";" ;
import_clause = named_import | namespace_import ;
named_import  = "{" , import_spec , { "," , import_spec } , [ "," ] , "}" ;
import_spec   = ident , [ "as" , ident ] ;
namespace_import
              = "*" , "as" , ident ;

chart         = "chart" , kind , [ bind ] , body ;
kind          = qual ;              (* unqualified or namespace-qualified *)
bind          = "as" , ident ;

define        = "define" , ( "mark" | "tool" | "transform" ) ,
                ident , "{" , { slot | output | export } ,
                { item } , "}" ;
slot          = "slot" , slot_shape , ident , ( body | ";" ) ;
slot_shape    = "expr" | "expr_list" | "literal" | "number"
              | "string" | "boolean" | "enum" | "ref" | "block"
              | "channel" | "outputs" ;
output        = "output" , ( ident | sql_expr , "as" , ident ) , ";" ;
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

resource      = param | res | theme ;
                     (* data is a property: `data: { ... }` *)
param         = "param" ,
                ( sql_expr , bind , ( ";" | scalar_param_body )
                | ( "store" | "selection" ) , bind , body ) ;
scalar_param_body
              = "{" , [ sharing_property ] , "}" ;
sharing_property
              = "sharing" , ":" ,
                ( "shared" | "free" | "level" , "(" , number , ")" ) , ";" ;
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
child_decl    = table_bind | resource | mark
              | transform | tool | widget | view | event | cell | plot
              | variable | part | level | adjust | derive
              | layer | when | field | row | key | fields | action
              | scale_edit | scale_hint
              | selection_clause | equality_predicate | interval_predicate
              | match_block | splice
              | export | output ;

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
variable      = "variable" , ( "row" | "column" | "item" ) , ident , body ;
part          = "part" , ident , body ;
level         = "level" , number , body ;
adjust        = "adjust" , ( "expr" , body | kind , [ bind ] , body ) ;
derive        = "derive" , kind , [ bind ] , body ;
layer         = "layer" , ident , body ;
when          = "when" , body ;
field         = "field" , arrow_type , ident , [ "nullable" ] , ";" ;
row           = "row" , body ;
scale_edit    = "scale_edit" , body ;      (* tool definitions/behaviors:
                                               edit a containing-plot scale *)
scale_hint    = "scale_hint" , body ;      (* groups: scale-type hint *)
selection_clause = "clause" , body ;       (* selection-update payload only *)
equality_predicate = "equality" , "{" , { predicate_entry } , "}" ;
interval_predicate = "interval" , "{" , { predicate_entry } , "}" ;
predicate_entry = ident , body ;           (* parent fixes entry category/type *)
key           = "key" , body ;             (* update payloads: delete_by_key, update_by_key *)
fields        = "fields" , body ;          (* update payloads: update_by_key *)
action        = "set" , ( state_action | cursor_action ) ;
state_action  = qual ,
                [ "at" , ( "current" | "start" ) ] ,
                [ "replacing" , "scopes" ] , "=" ,
                ( sql_expr , ";" | ident , ( body | ";" ) ) ;
cursor_action = "cursor" , "=" , sql_expr , ";" ;
typed_ref     = ref_kind , qual , ";" ;
ref_kind      = "mark" | "selection" | "tool" | "widget" | "resource" ;

property      = ident , ":" , value ;
value         = body                                 (* anonymous object *)
              | ident , body                         (* typed object: linear { ... } *)
              | typed_ref
              | array , ";"
              | channel_mode , sql_expr , terminator (* expression-driven channel *)
              | "dim" , qual , ( body | ";" )        (* raster dimension handle *)
              | "pattern" , body
              | "env" , string , ";"                 (* environment variable, capability-gated *)
              | "none" , ";"
              | sql_query , ";"                      (* reserved `sql:` and `query:`
                                                        properties only *)
              | sql_projection , ";"                 (* reserved `expressions:` or a
                                                        structurally evident outputs slot *)
              | sql_expr , terminator ;              (* default expression slot *)
terminator    = body | ";" ;                         (* config block or semicolon *)
array         = "[" , [ elem , { "," , elem } , [ "," ] ] , "]" ;
elem          = body | sql_expr | "pattern" , body | "none" ;
qual          = ident , { "." , ident } ;
channel_mode  = "encoded" | "direct" ;

sql_expr      = ? one sqlparser expression accepted by AvengerSqlDialect ? ;
sql_projection
              = ? one non-empty sqlparser SELECT projection list ? ;
sql_query     = ? one standard SELECT, FROM-first SELECT, or VALUES statement ? ;
arrow_type    = ? one canonical physical Arrow type from Physical Arrow Types ? ;
```

Grammar notes:

The exceptional structural and mark-effect kinds have the following normative
ownership. “Registered” means the authoring schema and paired Rust lowerer are
one entry in the native registry; “language core” means the structural resolver
and compiler own semantics that cannot be expressed as an ordinary leaf
lowerer.

| Surface form | Inventory and schema owner | Body mode | Lowering owner |
|---|---|---|---|
| `mark group` | language core | mixed recursive mark/dataflow body | recursive `MarkGroup` compiler path |
| `transform pipeline` | registered `Transform` entry | ordered child transforms plus outputs | registered pipeline assembler after generic child lowering |
| `tool behavior` | language core | owned state, events, tools, and chrome marks | behavior-component compiler path |
| `adjust expr` | language core | item-frame channel assignments | expression-adjustment compiler path |
| `adjust nudge`, `adjust jitter`, `adjust dodge`, and imported extension adjustments | registered `Adjust` entries | property-only adjustment configuration plus `apply:` routing | paired adjustment lowerer followed by generic channel routing |

- Named runtime/chart instances use `[visibility] <category> <type> [as
  <name>]`, subject to the containing schema's binder requirement. Declared
  members instead use `<member-category> <shape-or-type> <name>` with no `as`,
  and parent-keyed entries use only their key. `cell` places its optional
  `at { ... }` after its instance binding.
- `private` and `public` are optional declaration modifiers, not declaration
  kinds. The grammar shows their token position; the authoring schema permits
  them only on declarations with a named public identity. `public` additionally
  requires a private structural ancestor and hoists to the nearest non-private
  named group. A `mark group` accepts `export` children and the optional opaque
  `component_kind:` provenance property. Definition headers accept the same
  exact `export` declaration, which expansion moves to the instance group.
- `group` is a language-owned `mark` kind. It uses the ordinary mark
  reference, export, visibility, document-symbol, and target categories while
  lowering recursively through `MarkGroup`; it is not dispatched through the
  native mark registry. A schema property with `MarkBlock` shape, currently
  `legend.overlay`, accepts a property-only block whose direct children are
  one or more `mark` declarations.
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
- `slot channel <name>` is the logical-channel input form. Its optional
  `default:` property names the default physical channel; without a default it
  is required at every instantiation.
- Every slot follows the declared-member law: the closed `slot_shape` precedes
  the public property name and there is no `as`. A slot is required exactly
  when it has no `default:`; a trailing `;` is merely an empty body. A body may
  declare `default:`; `enum` additionally requires `values:`,
  `ref` requires `kind:`, and `block` may declare `exposes:`. The authoring
  schema rejects properties not valid for the declared shape. `outputs` is
  transform-only, required, non-defaulted, unique within a definition, and
  contributes its supplied aliases to the instance output interface. Slot
  shapes are explicit and never inferred from body use.
- The schema-free parser selects a `value` production from local syntax and the
  globally reserved `sql`/`query`/`expressions` names; the authoring schema then
  validates that shape for the particular property. `sql_query` is reachable
  only from the first two reserved properties, while `expressions` selects a
  projection list. A top-level projection comma or explicit alias also
  self-identifies a projection supplied to an arbitrarily named `outputs`
  slot. Bare identifiers in enum-valued properties
  parse as `sql_expr` atoms that the resolver checks against the enum. A
  `typed_ref` records an explicit reference kind plus a lexical or
  qualified path; when a property schema already fixes the kind, its shorter
  bare `qual` form lowers to the same typed reference node. Scalar and store
  reads use `$qual` and lower to a typed scalar/table binding node. An
  imperative `set <qual>` target is resolved first, and its scalar, store, or
  selection category then selects the valid update algebra.
  An action's optional `at current|start` modifies that l-value's routed owner,
  never its RHS; the authoring schema permits `start` only under `between:`.
  `replacing scopes` is a second LHS modifier, valid only for scalar params and stores,
  that clears every concrete owner copy before writing the routed target.
  `set cursor` is the one write-only effect action: it has no target path, `at`,
  or `replacing scopes` modifier. The grammar lists the union of forms.
- The `ident , body` alternative has priority over the `sql_expr , terminator`
  alternative when the expression would be exactly one bare identifier and the
  terminator is a body. Thus `linear { ... }` is structurally a typed object.
  A configured expression may still begin with a quoted column, `$` binding,
  literal, qualified expression, parenthesized expression, or function call.
  The strict parser and editor grammar share fixtures for this boundary.
- `clause` remains a plain-body structural declaration. An `equality` or
  `interval` parent fixes its children's category and value shape, so each
  child is keyed directly by its dimension ID (`id { ... }`, `x { ... }`).
  The key is not a binding. Clause identity remains the ordinary
  `id: <sql_expr>;` property; scene-query field maps likewise use `id:` inside
  anonymous objects.
- A scalar param puts exactly one SQL scalar initializer before `as`; the
  initializer's DataFusion-planned Arrow type is authoritative. Its optional
  body accepts only `sharing:` and the empty body is canonically `;`. `type:`,
  `value:`, `default:`, and `kind:` are invalid. Stores and selections are peer
  declarations with their own body schemas. Sharing defaults to `shared` for
  scalar params and stores; chart/component scalar initializers may use the acyclic
  dependency rule, while catalog-table scalar initializers remain
  self-contained and row-free.
- Store fields are declared `field <arrow_type> <name> [nullable];`; nested
  struct members are `field(<arrow_type>, '<name>')`. Both are type-first, but
  nested Arrow names are strings so they can preserve names outside the DSL
  identifier grammar.
- `variable row|column|item <id>` is an ordered declared member, not an `as`
  binding. `adjust expr` is the explicit unaliased expression-adjustment form;
  all other adjustment kinds follow their schema's binder policy.
- `output <name>;` is the same-name shorthand. An explicit output is
  `output <sql_expr> as <public-name>;`; the top-level `as` is parsed after the
  complete expression and is not confused with SQL-internal `AS` such as a
  cast or subquery alias.
- A projection alias is likewise recognized only at the top level of one
  select item, but it belongs to the projection value rather than creating an
  `output` declaration. Named-projection schemas require explicit uppercase or
  lowercase `AS`; canonical formatting emits uppercase `AS`.
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
- Keywords are contextual. `encoded`, `direct`, `pattern`, `dim`, `env`, and
  `none` are recognized only in value-prefix position (immediately after
  `:`); `encoded` and `direct` are also property names inside conditional
  channel branch blocks;
  `level`, `part`, and the other declaration keywords only in
  declaration-head position; `group` is recognized as a mark kind after
  `mark`, and `overlay` is an ordinary schema-known property name. A property
  may therefore be named `value`, `encoded`, or `direct` outside those
  structural positions without turning the same words into SQL keywords.
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
   scalar/table/selection use, same-scope state-name collisions, and nearest-binding
   shadowing without kind-directed fallback. Param-schema fixtures require one
   row-free SQL initializer before `as`, reject the removed typed header and
   body `value:`/`kind:` forms, characterize DataFusion inference, reject bare
   untyped `NULL`, validate host values/table arguments/action results against
   the inferred type, preserve explicit typed nulls, reject unsupported Arrow
   results, and check `raw_domain:` use-site type constraints. Store-field
   fixtures continue to round-trip nested physical type spellings.
   Typed-boundary fixtures pin bare, parenthesized,
   and compound SQL sources; recursive list, fixed-list, struct, map, and typed
   `NULL` values; string and lossy numeric conversions under the pinned cast
   matrix; strict versus authored `TRY_CAST`; exact numeric sources; exact host
   `DataType`; and compile-time versus transactional runtime failures. Struct
   fixtures cover empty, flat, and
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
   output-interface resolution, acyclic and cyclic param initializers, order-free
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
   target warnings, and failed missing routes. Cross-owner fixtures pin that
   ordered visibility is keyed by concrete `(binding, owner)`, ordinary reads
   remain current-routed after an `at start` write, temporal reads remain frozen,
   RHS store scans remain current-routed, and an `at start` store primitive
   internally sees its target owner's pre-action working rows. They cover
   `replacing scopes`
   removal-before-write ordering, later-action visibility, rollback, param-
   initializer fallback, lazy store-row fallback, invalid selection use, and the
   redundant-shared warning. Sharing fixtures
   pin logical owner paths for `free`/`level(n)`/`shared`, root saturation,
   per-owner scalar-param initial values, lazy per-owner store initial rows and
   revisions, and atomic failure for unrouted non-shared store writes.
   Store-relation fixtures assert that
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
   relies on, including intrinsic-name exclusions and every registered
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
   param-initializer and table dependency DAGs, validation, alias fields,
   contextual-access/intrinsic rewriting, event targets,
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
   `define` with slots, channel slots, `match`, block slots,
   `export`/`exposes`, the first custom definition fixtures, and `avenger
   expand`. Gate: the expansion equivalence property for charts that use
   imported definitions — `compile(chart) == compile(expand(chart))` — over
   dedicated custom mark, tool, and SQL-transform fixtures. Native built-in
   fixtures remain native and are not rewritten as definitions, and widget
   declarations remain untouched because widgets have no definition form.

6. **Data catalogs.** Module-level catalog/schema/table bindings,
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
| `avenger render <module> [--chart name] -o out.png` | Compile and render one named or singleton chart entrypoint (`.png`/`.svg`/`.pdf`); `--param k=v` overrides; `--watch`. |
| `avenger watch <module> [--chart name]` | Open one native chart window with dependency-aware hot reload while editing in another editor; preserves compatible state and the physical-plan cache across reloads. `--chart` is required when the module has multiple charts. |
| `avenger editor [chart\|project]` | The full native playground: collapsible file browser + editor + live chart, one window — see below. |
| `avenger fmt [paths]` | Canonical formatter; `--check` for CI. Shares one printer with `expand` output and decompilation. |

Imports and distribution (the mutating verbs):

| Command | Purpose |
| --- | --- |
| `avenger add <url\|std:path> (--import export [--as local]\|--namespace alias) [module]` | Fetch, verify, and insert an explicit pinned named or namespace import. |
| `avenger pin [paths] [--print]` | Pin unpinned URL imports. |
| `avenger update [name\|--all]` | Re-fetch mutable-URL imports and refresh hashes — a deliberate re-pin, surfaced as a diff. |
| `avenger vendor <import>` | Copy a remote source module into the project and rewrite the import to a relative specifier. |

Flattening, introspection, and testing:

| Command | Purpose |
| --- | --- |
| `avenger bundle <module> [--chart name] [-o out]` | Link the selected module interface or chart-item closure into one ordinary import-free source module with deterministic private alpha-renaming. |
| `avenger expand <module> [-o out]` | Source-level inline-definition expansion; concrete definition instantiations become editable ordinary groups with visibility, exports, and component provenance, while native built-in kinds and semantic resource imports remain. |
| `avenger info [path]` | The doc-query surface over the entire language schema — native built-ins and imported definitions alike, `--format json` throughout. Bare `info` lists namespaces (marks, transforms, tools, scales, helpers, events); drill by path: `info marks --coord geo`, `info mark rect`, `info mark rect.x` (one channel's option suffixes), `info transform bin`, `info std:marks/error_bar` or any file/URL (slots, parts, outputs, doc comments — inspect before importing). |
| `avenger schema [--format json\|json-schema]` | The entire machine-readable language schema in one dump — `json` for tooling and big-context agents that load the reference once, `json-schema` to compile it into the generated full validator for the AST interchange form (the frozen core schema ships with the spec). |
| `avenger doc [-o dir] [--open] [--single-page]` | Generate the project's documentation site from three sources it already has — see below. |
| `avenger deps <module> [--chart name]` | The selected module or chart-item closure as a tree with pin status, private item dependencies, and origins. |
| `avenger table <module> [--chart name] --at <alias\|mark>` | Print the data context at a named point in the selected chart pipeline; anonymous transforms have opaque compiler identities and are inspected through a following named stage/mark or an editor source-position action. |
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
(saving an imported module, theme, or local data resource reloads every chart whose
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
   `lib/marks.avenger#error_bar` points at that exported definition's
   reference section, and a link to
   `charts/revenue_trend.avenger#summary` becomes that chart entrypoint's
   gallery entry, image included.
2. **The chart gallery**, built from blessed baselines and each chart's
   `-- |` blurb — no rendering at doc time, so output is deterministic and
   by construction in sync with what `avenger test` verified.
3. **The definition reference**: one section or page per mark, tool, and
   transform — schema tables (slots and defaults, channel slots,
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
  (`x: encoded "mpg"`, `title: 'MPG'`) — the inverse of every familiar language.
- Every expression-driven channel needs a mode (`fill: direct '#4682b4'`;
  `fill: encoded "category"`). Both modes accept arbitrary scalar SQL
  expressions; `direct` means bypass the channel scale, not “constant.”
- Bare identifiers are language-space names — kinds, properties, enums,
  aliases, slots — never columns.
- Helper arguments use the shape required by their signature. DSL-space names
  are bare (`channel.x`, `event.coord.x`); event-row fields use the quoted
  contextual relation (`datum."id"`).
- `avenger 1;` first; imports precede a non-empty ordered module-item list;
  a multi-chart module names every chart; `;` terminates a
  property unless a `{ }` config block follows; channel config attaches
  after the value (`x: encoded "hp" { axis: { title: 'HP'; } }`).
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
   authoring objects. A selected chart entrypoint
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

Step 1 of the lowering model names "a stable DSL AST". The source header laws
are intentionally contextual, but they normalize to the same declaration
record — keyword, optional kind, optional semantic name, properties, and
ordered children. Module items and import clauses wrap that generic
declaration tree; the AST still does not need one node type per language
feature:

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
    imports: Vec<Import>,
    items: Vec<ModuleItem>,          // non-empty, in authored source order
}

struct ModuleItem {
    exported: bool,                  // top-level module visibility
    declaration: Decl,               // chart | define | table | schema | catalog
}

struct Import {
    source: String,
    sha256: Option<String>,
    clause: ImportClause,
}

enum ImportClause {
    Named(Vec<ImportSpecifier>),
    Namespace(Name),
}

struct ImportSpecifier {
    imported: Name,
    local: Name,                     // equal when source uses shorthand
}

struct Decl {
    visibility: Visibility,              // default | private | public
    keyword: Keyword,                // chart | mark | transform | table | param | on | ...
    kind: Option<QualifiedName>,     // symbol, acme.hexbin, sql, parquet, ...
    name: Option<Name>,              // instance binder, declared-member name, or keyed-entry id
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
    Projection(SqlProjection),       // ordered SQL select items
    Query(SqlQuery),                 // sql:/query: query island
    Binding(BindingKind, Vec<Name>, BindingTime),
                                      // $name or $component.alias[@time]
    Ref(RefKind, Vec<Name>),         // selection hover.hovered, mark layers.points
    Channel(ChannelMode, Box<Value>),// encoded/direct <SQL expression>
    Dim(Vec<Name>),                  // dim pixels.x_dim; exactly two segments
    Pattern(Box<Value>),
    Env(String),
    None,
    Array(Vec<Value>),
    Block(Option<Box<Value>>, Body), // head value + body; Body = props + children
    Call(Name, Vec<Value>),          // span(...), polygon(...), list(float64)
}

struct NumericLiteral {
    canonical_decimal: String,       // exact digits; never converted through f64
}

enum BindingKind { Param, Store, Selection }
enum BindingTime { Current, Start, Previous } // Current is omitted in source/JSON
enum ChannelMode { Encoded, Direct }
```

`SqlProjection` contains one or more parsed SQL select items in source order
plus the same normalized bindings and source-independent canonicalization as
`SqlExpr` and `SqlQuery`. A named item retains its expression and exact DSL
`Name` separately; lowering never asks DataFusion to infer or normalize that
alias.

The parser initially classifies a scalar `$path` as `Param`; resolution
refines the resolved binding to `Selection` when the path names a selection
and the SQL island has a current row. `Selection` therefore does not introduce
a distinct lexical spelling.

Every feature in this document is an instance of `Decl` — `table sql` with
params, `catalog schemas` and `schema tables` containers, `match` arms,
effects, block-slot content.
Typed slots need no special node: `slot expr category;` is a `Decl` with
`keyword = slot`, `kind = expr`, and `name = category`; configured slot fields
such as `default`, `values`, `class`, `kind`, and `exposes` are ordinary
properties in its body.
Surface-header normalization likewise keeps the generic AST closed. A scalar
`param 640.0 as width;` is a `param` declaration whose
semantic `value` property came from the header expression; the parser forbids
authored body `type:` and `value:` properties and the canonical printer moves
the semantic value back into the header. Its Arrow type exists only in the
compiler-owned `ParamTypeIndex`, not the dependency-light AST. `store as rows`
and `selection as picked` map directly to `Decl { keyword: "store" }` and
`Decl { keyword: "selection" }`; there is no param-category normalization.
`mark group` remains `Decl { keyword: "mark", kind: "group" }` throughout
parsing, printing, expansion, and resolution. A `legend.overlay` mark block is
an ordinary object value whose `children` are fully resolved mark
declarations. Keyed selection entries normalize to the existing dimension
representation. These are source/AST mappings, not extra interchange node
variants.
Visibility is likewise generic declaration metadata rather than a new node:
the parser records a `private` or `public` prefix, the schema decides whether
that declaration may carry it, and the resolver constructs the public alias
graph. The default variant is omitted in text and interchange output.
`PropertyMap` has name-to-value equality and hashing: insertion or source order
is not semantic. The CST separately records authored order and comment anchors.
Canonical DSL printing uses RFC 8785's UTF-16 code-unit lexical name order for every key,
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
even when a schema-fixed property omits the surface prefix. `Expr`,
`Projection`, and `Query`
retain parsed semantic SQL only. Original island
spelling, whitespace, comments, and token ranges belong to the concrete syntax
tree and source map, not semantic AST identity.

`Block` is the one composite: an optional *head* value plus a body. All
three surface shapes lower to it — a bare block (`data: { ... }`, no
head), a typed object (`scale: linear { ... }`, an `Atom` head), and a
configured value (`x: encoded "amount" { scale: ... }`,
`title: 'Sales' { align: center; }`, `x: dim pixels.x_dim { axis: ... }` —
the value being configured is the head, whatever its variant). The concrete
parser deterministically treats a lone `ident` head as the typed-object form;
all other heads are configured values. Both lower to `Block`, after which the
schema decides whether the authored shape is legal for the property.

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
  `{"env": "ICEBERG_TOKEN"}`, `{"expr": "..."}`,
  `{"projection": "... AS name, ..."}`, `{"query": "..."}`. The
  inventory is closed — sixteen tags: `num`, `col`, `atom`, `binding`, `expr`,
  `projection`, `query`, `encoded`, `direct`, `dim`, `ref`, `pattern`, `env`,
  `none`, `block`, `call` — pinned by the core schema below. The `block` tag carries the
  optional head beside `props` and `children`
  (`{"block": {"head": {"col": "amount"}, "props": ...}}`); a typed
  object is simply a `block` whose head is an `atom`. Imports encode their
  `source`, optional `sha256`, and explicit named or namespace `clause`;
  module items encode `exported` plus their generic `declaration`.
  The prefix tags have deliberately distinct payload shapes: `encoded`,
  `direct`, and `pattern` contain another value, `dim` contains one two-segment dotted name,
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
  8785 UTF-16 code-unit object-key ordering recursively, and the canonical DSL
  printer deliberately uses the same order. Duplicate object members are rejected by the
  interchange decoder rather than accepted with first- or last-wins behavior;
  a duplicate textual DSL property is likewise a parse error.
- Children encode as one JSON array in semantic cross-kind order. Array order
  is preserved by ordinary JSON implementations and must not be regrouped by
  declaration keyword during encoding or decoding.

```avenger
table sql as borough_trips {
  param 'Manhattan' as borough;

  sql: SELECT * FROM trips WHERE "borough" = $borough;
}
```

```json
{
  "decl": "table", "kind": "sql", "name": "borough_trips",
  "children": [
    { "decl": "param", "name": "borough",
      "props": { "value": "Manhattan" } }
  ],
  "props": {
    "sql": { "query": "SELECT * FROM trips WHERE \"borough\" = $borough" }
  }
}
```

A configured channel shows the head in play — the same `block` tag at
both altitudes:

```avenger
x: encoded "amount" {
  scale: linear { nice: true; }
}
```

```json
{
  "x": {
    "block": {
      "head": { "encoded": { "col": "amount" } },
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
gate applies to both. Source origins, filenames, and module identities remain
external source-map metadata; they are never derived semantic fields in
interchange JSON.

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
    "imports": { "type": "array", "items": { "$ref": "#/$defs/import" } },
    "items": {
      "type": "array",
      "items": { "$ref": "#/$defs/moduleItem" },
      "minItems": 1
    }
  },
  "required": ["version", "items"],
  "additionalProperties": false,
  "$defs": {
    "name": {
      "type": "string",
      "pattern": "^(?:_|\\p{Alphabetic})(?:_|[0-9]|\\p{Alphabetic})*$"
    },
    "qualifiedName": {
      "type": "string",
      "pattern": "^(?:_|\\p{Alphabetic})(?:_|[0-9]|\\p{Alphabetic})*(?:\\.(?:_|\\p{Alphabetic})(?:_|[0-9]|\\p{Alphabetic})*)*$"
    },
    "moduleItem": {
      "type": "object",
      "properties": {
        "exported": { "type": "boolean" },
        "declaration": {
          "oneOf": [
            {
              "allOf": [
                { "$ref": "#/$defs/decl" },
                {
                  "properties": {
                    "decl": { "const": "chart" },
                    "kind": { "$ref": "#/$defs/qualifiedName" }
                  },
                  "required": ["decl", "kind"],
                  "not": { "required": ["visibility"] }
                }
              ]
            },
            {
              "allOf": [
                { "$ref": "#/$defs/decl" },
                {
                  "properties": {
                    "decl": { "const": "define" },
                    "kind": { "enum": ["mark", "tool", "transform"] }
                  },
                  "required": ["decl", "kind", "name"],
                  "not": { "required": ["visibility"] }
                }
              ]
            },
            {
              "allOf": [
                { "$ref": "#/$defs/decl" },
                {
                  "properties": {
                    "decl": { "enum": ["table", "schema", "catalog"] },
                    "kind": { "$ref": "#/$defs/qualifiedName" }
                  },
                  "required": ["decl", "kind", "name"],
                  "not": { "required": ["visibility"] }
                }
              ]
            }
          ]
        }
      },
      "required": ["exported", "declaration"],
      "additionalProperties": false
    },
    "import": {
      "type": "object",
      "properties": {
        "source": { "type": "string", "minLength": 1 },
        "sha256": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
        "clause": { "$ref": "#/$defs/importClause" }
      },
      "required": ["source", "clause"],
      "additionalProperties": false
    },
    "importClause": {
      "oneOf": [
        {
          "type": "object",
          "properties": {
            "named": {
              "type": "array",
              "items": { "$ref": "#/$defs/importSpecifier" },
              "minItems": 1
            }
          },
          "required": ["named"],
          "additionalProperties": false
        },
        {
          "type": "object",
          "properties": { "namespace": { "$ref": "#/$defs/name" } },
          "required": ["namespace"],
          "additionalProperties": false
        }
      ]
    },
    "importSpecifier": {
      "type": "object",
      "properties": {
        "imported": { "$ref": "#/$defs/name" },
        "local": { "$ref": "#/$defs/name" }
      },
      "required": ["imported", "local"],
      "additionalProperties": false
    },
    "decl": {
      "type": "object",
      "properties": {
        "decl": { "$ref": "#/$defs/name" },
        "kind": { "$ref": "#/$defs/qualifiedName" },
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
        "encoded": { "$ref": "#/$defs/value" },
        "direct": { "$ref": "#/$defs/value" },
        "dim": {
          "type": "string",
          "pattern": "^(?:_|\\p{Alphabetic})(?:_|[0-9]|\\p{Alphabetic})*\\.(?:_|\\p{Alphabetic})(?:_|[0-9]|\\p{Alphabetic})*$"
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
          "enum": ["mark", "selection", "tool", "widget", "resource"]
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

The core schema's `name` patterns deliberately use the Unicode
`\p{Alphabetic}` property escape. A validator used for Avenger interchange
must evaluate JSON Schema patterns with Unicode-aware ECMA-262-compatible
semantics; an ASCII-only regex fallback is non-conforming because source names
such as `café` and `Δvalue` are valid.

The instance corpus for this schema includes accept/reject and bidirectional
round-trip cases for every one of the sixteen tags, including exact `num`
spelling, two-segment `dim`, string `env`, boolean-true `none`, compact and
qualified binding paths, canonical SQL `projection`, and headed/headless
blocks. It also covers well-formed
single- and multi-chart modules, mixed data/definition modules, named and
namespace imports, and rejects
plain JSON numbers, two-key tagged objects, unknown tags, provenance fields,
invalid tag payload shapes, duplicate members, and malformed hashes. It belongs
to the same conformance corpus that pins the formatter.

### Round-Trip Laws

- **Printing is total.** Every constructible strict AST prints. Every `Expr`,
  `Projection`, and `Query`, whether parsed from source, decoded from JSON, or
  constructed by a host API, prints through the same pinned SQL unparser.
  Nothing a host API can build lacks a text spelling.
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
  query islands, comments, quoted identifiers, qualified scalar/table
  binding reads, and dependency-sensitive syntax. An `sqlparser` or DataFusion
  upgrade that changes canonical unparse output requires an explicit reviewed
  language-snapshot update; it cannot silently churn DSL, JSON, expansion, or
  semantic hashes.
- **Property order is neither identity nor semantics.** Equal property maps
  compare and hash equally regardless of insertion or source order. The CST
  remembers authored order for editor operations, while both `print(ast)` and
  `avenger fmt` emit all properties in RFC 8785 UTF-16 code-unit lexical order.
  Canonical JSON uses that same key order; decoding any input order
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
  `Parser::parse_statement()` followed by query-only validation for the
  full-query `sql:` and `query:` slots.
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
x: encoded amount { ... }
```

In this example, the outer parser consumes `encoded`, SQL consumes `amount`,
then the DSL parser sees `{` and parses
the channel configuration block. If a SQL expression itself contains braces,
such as a dictionary/map literal accepted by the chosen SQL dialect, SQL
consumes those braces before the DSL parser resumes. This avoids making `{`
ambiguous by convention.

Full-query SQL properties use the same idea:

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
pinned DataFusion adapter with the same expression/projection/query shape. DataFusion may
add planning semantics, but it cannot silently define a second source grammar;
any frontend/planner acceptance mismatch is a release-blocking conformance
failure.

The schema-free structural parser chooses the value production from local
syntax and the reserved property-name contract: `sql:`/`query:` enter a query
island, `expressions:` enters a projection island, a leading block/array/prefix enters
its structural value, and remaining positions enter an expression island
unless top-level projection punctuation makes a projection self-identifying.
Authoring schemas validate whether that parsed shape is legal for the
containing declaration afterward; they are not needed for parse/print round
trips. Channel values, filter predicates, sort keys, and visibility conditions
are SQL expression slots. Calculate outputs, aggregate measures, window
outputs, and select items are SQL projection slots. Selectors such as
`target: mark manual_box_plot.fence.outlier_layer.outliers;`, typed values such as
`scale: linear { ... }`, arrays of DSL names, and nested property objects are
DSL values.

The compiler's structural parser owns every DSL delimiter and delegates the
smallest complete SQL unit to the sqlparser entry point appropriate for that
context. The fixed boundary contexts are:

| Source context | Structural parser owns | Delegated SQL unit | Stops before |
| --- | --- | --- | --- |
| reserved `sql:` or `query:` property | property name, `:`, and terminating `;` | one query (`SELECT`, `FROM`-first `SELECT`, set operation, or `VALUES`) | top-level `;` |
| reserved `expressions:` property or structurally evident `slot outputs` argument | property name, `:`, terminating `;`, and semantic alias policy | one non-empty SQL projection list | top-level `;` |
| ordinary/configurable property, channel value, filter, or `value` payload | property/prefix and optional configuration body | one scalar expression | top-level `;` or the configuration `{` |
| scalar `param <expr> as <name>` initializer | `param`, top-level `as`, declared name, and `;` | one row-free scalar expression | top-level `as` |
| explicit `output <expr> as <name>` | `output`, top-level `as`, public name, and `;` | one scalar expression | top-level `as` |
| `set ... =` or another structurally terminated expression | declaration/action header and terminating `;` | one scalar expression | top-level `;` |
| structural array element | outer `[]`, commas, anonymous bodies, and `value`/`pattern`/`none` prefixes | one scalar expression for that element | top-level `,` or `]` |

The SQL parser recognizes SQL-owned parentheses, brackets, braces, subqueries,
strings, identifiers, and comments before returning control at an outer
delimiter. It therefore distinguishes top-level projection commas and aliases
from `CAST(... AS ...)`, nested commas, and subquery projections without a
second expression grammar. A SQL array, struct, function call, or subquery
remains one expression island, while a DSL array is never delegated wholesale. Each
ordinary element is parsed as its own expression, so the structural parser
retains its commas and closing bracket. The structural parser does not
reproduce an SQL expression subset.

Value bindings fit this model with a kind-neutral normalization. `sqlparser-rs`
tokenizes `$foo` as a placeholder. The DSL rejects positional placeholders such
as `$1`, `$2`, and `?`. For `$zoom.domain`, the
stock tokenizer produces a placeholder token for `$zoom`, followed by `.` and
the word `domain`. Under the narrowed Avenger dialect, `@` is never an
identifier character, so an adjacent temporal suffix tokenizes as `@` followed
by the word `start` or `previous`. Before an SQL island is parsed, Avenger's
token-normalization pass recognizes a named placeholder, zero or more
`. ident` pairs, and an optional adjacent exact `@ start` or `@ previous` token
pair with no source trivia between the two tokens. It records the full DSL path,
temporal version, and source span, then substitutes one unique opaque quoted
identifier token. A quoted identifier is
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
island. `@start` and `@previous` are temporal only as adjacent suffixes of a
complete `$path`; other supported `@` operator tokens retain their SQL meaning,
and `@` never enters an identifier. Trivia around `.` may be accepted, but the
canonical printer emits no spaces around `.` or `@`. Quoted identifiers and
numeric tokens are never path segments.
Synthetic identifier spellings live only in the normalized token buffer, are
distinguished by a side table rather than a reserved source prefix, and never
appear in diagnostics, serialized SQL, or printed DSL.

Contextual accesses such as `channel.x`, `event.coord.x`,
`item.channel.x`, and `datum."id"` parse as ordinary SQL qualified
identifiers; `event.facet[1]` additionally uses SQL's ordinary subscript AST.
The resolver recognizes them only in compatible scalar DSL expression islands
and rewrites the parsed AST before DataFusion planning. No lexer extension is
required.

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
indentation, bracket matching, outline, and runnable queries.

The Tree-sitter grammar is not the compiler parser. The compiler should still
use the `sqlparser-rs` tokenize-then-parse strategy described above. The
Tree-sitter grammar can be more tolerant so partially typed charts still
highlight well.

Initial Zed extension shape:

```text
tree-sitter-avenger/
  grammar.js                  # extends the pinned avenger_sql base
  SQL_BASE.md
  queries/
    highlights.scm            # composed structural + SQL captures
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
      brackets.scm
      indents.scm
      outline.scm
      runnables.scm
      tasks.json
```

The SQL grammar starts as an aggressively trimmed fork of the modular,
MIT-licensed `DerekStride/tree-sitter-sql` grammar rather than a from-scratch
grammar or a wholesale general-SQL import. The fork retains only the reusable
expression and read-query foundation needed for DataFusion: scalar expressions,
`SELECT`, `FROM`, joins, CTEs, set operations, windows, subqueries, and
`VALUES`. DDL, writes, transactions, procedures, administration, and unsupported
dialect syntax are removed together with their unused rules, tokens, scanner
branches, captures, and tests. Avenger then replaces the lexical/parameter
layer, adds bindings and `FROM`-first queries, and records the pinned upstream
revision and retain/remove/adapt inventory.

The result is maintained as the independently testable `avenger_sql` base
grammar. `tree-sitter-avenger` extends that base at parser generation time,
replaces its root with the Avenger source root, and references its query and
expression nonterminals directly. Zed registers only the self-contained
combined `avenger` parser; it does not use runtime SQL language injection.

The base should match the SQL accepted by DataFusion closely enough for
highlighting while accepting all Avenger SQL contexts:

- Full SQL statements after `sql:`, including standard `SELECT`,
  `FROM relation SELECT ...`, set operations, and `VALUES`. `FROM relation`
  without an explicit `SELECT` remains editor recovery syntax, not valid v1
  source.
- SQL expression fragments in channel values, filters, transform outputs,
  sort keys, visibility conditions, and event filters.
- Named bindings such as `$min_amount`, including the temporal param suffixes
  `$width@start` and `$width@previous`.
- Contextual property accesses such as `channel.x`, `event.coord.x`,
  `event.facet[1]`, and `datum."field"` remain ordinary qualified-identifier
  and subscript syntax in the grammar; the LSP supplies contextual semantic
  tokens.
- SQL comments using `-- ...` and `/* ... */`.

The base grammar is named `avenger_sql` rather than plain `sql`. A stock SQL
grammar may parse full `SELECT` statements but not expose a reusable top-level
expression rule for a fragment such as
`"amount" >= $min_amount and "region" = $selected_region`. The adapted grammar
can retain a tolerant standalone root for its own tests while exporting stable
query and expression nonterminals for the derived grammar:

```javascript
source_file: $ => repeat(choice(
  $.statement,
  $.expression,
  $.binding_ref,
))
```

Both generated Tree-sitter parsers are editor artifacts only. The compiler
remains the source of truth: sqlparser under `AvengerSqlDialect` parses the
frontend islands, and DataFusion receives them only at the planning boundary.

The SQL grammar must give `FROM`-first queries the same stable relation,
alias, projection, and clause nodes as standard ordering wherever the adapted
upstream permits. Highlight and recovery fixtures must include an incomplete
`FROM vega.movies AS m SELECT m."` because that is the authoring shape that
motivates the syntax guarantee.

The first combined grammar can parse declarations, property blocks, inherited
SQL expression/query nodes, comments, strings, params, and channel references.
The excerpt below is schematic: it names module/import and body rules omitted
for space. The peer Tree-sitter/Zed implementation plan owns the complete
stable node contract, base revision/synchronization contract, and corpus. A
sketch:

```javascript
const AvengerSql = require("tree-sitter-avenger-sql/grammar");

module.exports = grammar(AvengerSql, {
  name: "avenger",

  rules: {
    source_file: $ => seq(
      $.version_directive,
      repeat($.import_statement),
      repeat1($._module_item),
    ),

    version_directive: $ => seq("avenger", $.number, ";"),

    _module_item: $ => choice(
      $._module_declaration,
      seq("export", $._module_declaration),
    ),

    _module_declaration: $ => choice(
      $.chart_declaration,
      $.definition_declaration,
      $.catalog_declaration,
      $.schema_declaration,
      $.table_declaration,
    ),

    import_statement: $ => seq(
      "import",
      choice(
        seq(
          "{",
          $.import_specifier,
          repeat(seq(",", $.import_specifier)),
          optional(","),
          "}",
        ),
        seq("*", "as", field("local", $.identifier)),
      ),
      "from",
      field("source", $.single_quoted_string),
      optional(seq("sha256", field("hash", $.single_quoted_string))),
      ";",
    ),

    import_specifier: $ => seq(
      field("imported", $.identifier),
      optional(seq("as", field("local", $.identifier))),
    ),

    definition_declaration: $ => seq(
      "define",
      choice("mark", "tool", "transform"),
      field("name", $.identifier),
      $.definition_block,
    ),

    declaration: $ => seq(
      optional($.visibility_modifier),
      choice(
        $.chart_declaration,
        $.mark_declaration,
        $.transform_declaration,
        $.param_declaration,
        $.slot_declaration,
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
      choice(
        field("name", $.identifier),
        seq(
          field("source", $.sql_output_expression),
          "as",
          field("name", $.identifier),
        ),
      ),
      ";",
    ),

    slot_declaration: $ => seq(
      "slot",
      field("shape", choice(
        "expr", "expr_list", "literal", "number", "string", "boolean",
        "enum", "function", "ref", "block", "channel",
      )),
      field("name", $.identifier),
      choice($.property_block, ";"),
    ),

    chart_declaration: $ => seq(
      "chart",
      field("coordinate", $.qualified_name),
      optional($.as_clause),
      $.declaration_block,
    ),

    mark_declaration: $ => seq(
      "mark",
      field("kind", $.qualified_name),
      optional($.as_clause),
      $.mixed_block,
    ),

    transform_declaration: $ => seq(
      "transform",
      field("kind", $.qualified_name),
      optional($.as_clause),
      $.mixed_block,
    ),

    param_declaration: $ => seq(
      "param",
      field("type", choice($.arrow_type, "store", "selection")),
      $.as_clause,
      $.mixed_block,
    ),

    tool_declaration: $ => seq(
      "tool",
      field("kind", $.qualified_name),
      optional($.as_clause),
      choice($.mixed_block, ";"),
    ),

    widget_declaration: $ => seq(
      "widget",
      field("kind", $.qualified_name),
      $.as_clause,
      $.property_block,
    ),

    view_declaration: $ => seq(
      "view",
      field("kind", $.qualified_name),
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

    property: $ => choice(
      $.sql_query_property,
      $.sql_projection_property,
      $.ordinary_property,
    ),

    sql_query_property: $ => seq(
      field("name", choice("sql", "query")),
      ":",
      field("value", $.sql_query),
      ";",
    ),

    sql_projection_property: $ => seq(
      field("name", "expressions"),
      ":",
      field("value", $.sql_projection_list),
      ";",
    ),

    ordinary_property: $ => seq(
      field("name", $.identifier),
      ":",
      field("value", choice(
        $.object_value,
        $.array,
        $.typed_block_value,
        $.configured_expression,
      )),
    ),

    object_value: $ => $.property_block,

    typed_block_value: $ => seq(
      field("type", $.identifier),
      $.property_block,
    ),

    configured_expression: $ => seq(
      field("expression", $.sql_property_expression),
      choice($.property_block, ";"),
    ),

    sql_property_expression: $ => $.expression,
    sql_projection_list: $ => $.projection,
    sql_query: $ => $.query,
    sql_terminated_expression: $ => $.expression,
    sql_output_expression: $ => $.expression,
    sql_array_expression: $ => $.expression,

    array: $ => seq(
      "[",
      optional(seq($.array_value, repeat(seq(",", $.array_value)), optional(","))),
      "]",
    ),

    array_value: $ => choice(
      $.object_value,
      seq("value", $.sql_array_expression),
      seq("pattern", $.property_block),
      "none",
      $.sql_array_expression,
    ),

    // identifier, number, strings, columns, comments, SQL expressions,
    // projections, and queries are inherited from the pinned avenger_sql base.
    signed_number: $ => choice($.number, seq(choice("+", "-"), $.number)),
  },
});
```

These context wrappers reference inherited grammar rules rather than opaque
byte ranges. Their surrounding derived rules keep the DSL semicolon, comma,
closing bracket, or configuration body structural, while the SQL grammar owns
nested SQL delimiters. This removes the duplicate island-boundary scanner and
keeps binding/temporal nodes directly visible in the one syntax tree.

An external scanner may still be used by the SQL base for genuinely lexical
forms such as the exact whitespace-sensitive `line_comment`, nested
`block_comment`, or matching tagged dollar strings. If so, the combined grammar
inherits those external symbols and synchronizes the required scanner source
from the pinned base revision; it does not maintain another implementation.

`sql_query_property` is selected only by the two globally reserved property
names `sql` and `query`; both contain one query rather than an arbitrary SQL
statement. `sql_projection_property` is selected by `expressions`. A second
projection-valued property path recognizes the top-level alias/comma shape
needed by an arbitrarily named `slot outputs` argument. All other SQL-bearing
contexts expose an expression wrapper above. Authoring schemas still determine
whether that value shape is legal for a particular native or defined
declaration.

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
  "slot"
  "channel"
  "field"
  "variable"
  "adjust"
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

There is no Avenger SQL `injections.scm`. The SQL base's highlight query uses
ordinary SQL captures for keywords, operators, functions, identifiers,
strings, numbers, and comments, plus an Avenger capture for value-binding
references:

```scheme
(binding_ref) @variable.parameter
```

`tree-sitter-avenger` deterministically composes those base captures with its
structural captures into one `highlights.scm`, and `avenger-zed` synchronizes
that combined query.

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
  analysis distinguishes scalar params, stores, and current-row selection
  predicates and highlights the temporal suffix as a modifier.
- Contextual accesses (`channel.x`, `event.coord.x`, `datum."field"`, ...)
  as ordinary qualified identifiers. The context-free grammar does not assign
  them a language-owned meaning; the LSP refines roots, fixed members, and data
  fields contextually.
- DataFusion-oriented function names such as `approx_percentile_cont`, `date_bin`,
  `regexp_match`, and nested/struct functions as ordinary SQL functions.

The web editor uses CodeMirror 6, and the decisive fit is Lezer's
mixed-language parsing: a small Lezer grammar parses the DSL shell, and
`parseMixed` delegates expression, projection, and query islands to the Lezer SQL
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
parameter      $bindings (scalar params, table stores, and selection predicates)
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
modification   param/store/selection updates in event handlers
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

avenger-lsp
  Native protocol-adapter library used by `avenger-lang-cli`.
  stdio LSP transport exposed as `avenger lsp`.
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
highlighting, indentation, bracket matching, and a combined structural/SQL
syntax tree. The language
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

avenger-lsp/
  lib.rs          # native LSP protocol adapter and stdio service

avenger-lang-cli/
  main.rs         # exposes the adapter as `avenger lsp`

avenger-lsp-wasm/
  lib.rs          # Web Worker transport over dependency-light analysis
```

`avenger-lsp` is a library rather than a second installed executable. The
native distribution has one command/version surface, `avenger lsp`, owned by
`avenger-lang-cli`. LSP protocol types remain confined to that adapter;
`avenger-lang-analysis` exposes editor-neutral byte-offset request/result
types.

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

- In declaration blocks, sync at declaration starters such as `chart`,
  `mark`, `transform`, `param`, `tool`, `view`, `on`,
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
x: encoded amount + ;
```

For this example, the CST and analysis tree keep the surrounding mark and
property plus its `encoded` mode and an `SqlError` recovery node containing the
raw token range for `amount +`, and report the SQL diagnostic on that span. `SqlError` is not a
`Value` variant in the strict AST, cannot serialize to interchange JSON, and
cannot reach resolution or lowering.

SQL island recovery boundaries depend on the slot:

- Expression property: stop at a top-level `;`, top-level `{`, `}`, EOF, or a
  plausible next `identifier:`.
- Channel-mode payload: begin after `encoded` or `direct` and stop at a
  top-level `{` so `x: encoded amount { ... }` still separates the SQL
  expression from configuration.
- Explicit output expression: stop before the top-level `as` that introduces
  the required public output name. SQL-internal `AS` tokens inside a complete
  expression remain owned by the SQL parser.
- Structurally terminated action RHS: stop at the top-level `;`, `}`, or EOF;
  a top-level `{` is not a DSL terminator there.
- Array element: stop at the top-level `,` or `]`; the outer array owns both.
- Projection property: keep top-level commas as select-item separators and
  stop at the top-level `;`, `}`, EOF, or a plausible following property.
  Missing expressions, `AS`, aliases, or commas recover inside one projection
  node rather than consuming the next property.
- Full-query `sql:`/`query:` property: stop at the query semicolon, `}`, or EOF.

The analysis pipeline should be:

1. Tokenize with `sqlparser-rs`, preserving spans.
2. Parse the DSL token stream in tolerant or strict mode.
3. Normalize each qualified binding token sequence, including an adjacent
   `@start` or `@previous`, inside an SQL island to a unique kind-neutral quoted
   identifier, retaining a synthetic-token-to-path/version/source-span side
   table.
4. Parse SQL expression, projection, and query islands with sqlparser under
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
makes the common `FROM vega.movies AS m SELECT m."` case especially reliable
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
  contexts, param-initializer and table
  dependency cycles, later-slot default references, invalid block modes, group
  transform ordering violations, SQL parse errors, and invalid placeholders
  such as `$1` or `?`.
- Completion for declaration keywords, mark kinds, transform kinds, property
  names, channel names, scale and guide options, lexical and qualified
  `$binding` paths with scalar/table type information, typed state paths,
  contextual accesses and intrinsic operations, predeclared forward
  event/structural/state
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
  create or choose a definition module, add a definition with slots inferred
  from the selection's free names, replace the selection with an
  instantiation, and add a named import when crossing a module boundary), and
  **pin import** (fetch an unpinned URL import and insert its
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
the facade-side lowering registry owns constructors for marks, adjustments,
transforms, tools, widgets, coordinates, scales, guides, and other native
objects. Registry construction rejects duplicate entries, and CI checks schema
channel/property inventories against the Rust implementations. A DSL frontend
therefore cannot maintain a second native-kind switch whose behavior drifts
from schema, documentation, or Rust authoring.

The native registry is an injected, immutable host capability. The stock
language facade provides the canonical built-in registry; a third-party Rust
host may use the same public `NativeRegistryBuilder` to compose those entries
with explicit registration functions for its own marks, compound marks,
coordinates, adjustments, transforms, tools, and opaque widget kinds. Built-ins and
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
scale type, coordinate, contextual access, intrinsic operation, event type)
and each of its members — a channel per
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
- Each property's value shape — SQL expression, SQL projection list (including
  named-versus-select item policy), SQL query, literal, atom, scalar/table
  binding, typed reference, array, anonymous block, typed block, or configured value —
  plus requiredness, default, multiplicity, and nested block schema. A schema
  may include a non-semantic presentation rank for documentation tables and
  completion lists, but canonical DSL/JSON printing, AST equality, and hashing
  must ignore it and always use lexical property ordering.
- The closed slot-shape inventory and each shape's configuration contract:
  defaults, enum domains, reference kinds, block exposure, the transform-only
  dynamic output-list contract, and caller/block hygiene.
- Mark kinds, coordinate compatibility, supported channels, channel defaults,
  extra mark-level properties, and public part aliases. Native parts are
  registered aliases; defined parts are derived only from explicit mark
  exports. Every scale-bearing `ChannelSchema` composes the language-owned
  configured-channel properties `domain_contribution`, `domain_scope`, and
  `domain_group` (respectively the infer/exclude enum, `CoordinationScope`,
  and the constrained semantic-group string) with its profile-owned
  scale/axis/legend and position-specific properties; native entries cannot
  redefine their meanings.
- Transform kinds, properties, default values, required values, output
  handles, projection-alias output sources, and materialization behavior. The
  native `pipeline` entry additionally
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
  value-binding path, its param/store/selection kind, and its resulting scalar,
  table, or Boolean current-row-predicate type.
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
slot becomes a property with exactly its declared `ValueShape`; channel slots
become renameable channel positions; transform outputs and export
aliases become completion/validation surfaces; an `outputs` slot contributes a
call-site projection shape and per-instantiation alias handles; and enum slots supply the domains that `match`
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

## Post-v1 Considerations (Non-blocking)

None of the items below is an unresolved v1 requirement or an implementation
gate. V1 chooses the conservative form stated first; a future language version
may revisit it with usage evidence.

- V1 has no compact resource shorthands; params and data retain their canonical
  declaration/property forms.
- V1 chart dependencies are the importable definition and data resources
  specified above; it does not add named chart-local data declarations.
- V1 forbids nested `define` declarations. Private top-level definitions in
  the same module provide helpers without becoming importable.
- V1 requires explicit named or namespace imports
  (`import { error_bar } from 'std:marks';`) and has no automatic prelude.
- V1 has no `pkg:` scheme. A future naming/discovery layer may sit over the
  fetch-pin-cache mechanism — a community index of URLs first, a real
  registry later, or no registry at all.
- V1 pins only `sha256`; the grammar leaves room for later hash-algorithm
  agility.
- V1 does not theme structured pattern defaults through CSS. If demand appears,
  a future version may add pattern CSS syntax or a narrowly scoped theme form.
- V1 transform-definition outputs have fixed public names and never derive an
  output name from a slot.
- V1 block slots have exactly one splice point; repeated stamping is excluded.
- V1 `part` overrides are property-only. Configured channels and their
  `when`/`otherwise` branches are valid property values, but each supplied
  property replaces the definition's complete value rather than deep-merging
  it. A part cannot attach `adjust`, `derive`, or any other child declaration
  to an internal mark.
- V1 uses SQL-shaped contextual property access such as `event.coord.x`.
  Function-shaped read aliases such as `event_coord(x)` are intentionally not
  part of the language.
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
