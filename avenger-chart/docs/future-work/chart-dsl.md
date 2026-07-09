# Avenger Chart DSL Syntax

## Status

Draft syntax proposal. This document sketches a dedicated scripting language for
authoring `avenger-chart` charts. The goal is not to encode the Rust builder API
one-to-one, and not to start from JSON or YAML. The goal is a small declarative
DSL that can express all current chart features while remaining pleasant for
chart authors.

The first draft favors a regular canonical form. Shorthands can be added later
once the core grammar is proven.

Adopted decisions (2026-07-07): a required `avenger 1;` version pragma; SQL
string semantics everywhere with mandatory double-quoted data columns and
bare identifiers reserved for DSL names; `value` as the only unscaled
spelling; reserved helper functions (`channel(x)`, `datum('id')`, ...)
instead of sigil forms, with bare arguments for DSL-space names and strings
for data-space names; `sql:` restricted to query statements; cross-file
reuse via `import` and parameterized `define` with `channel` parameters;
**no Rust-defined compound marks or tools — they ship as a standard library
written in the language** (`import 'std:marks/box_plot';`);
custom transforms via the primitive `transform sql` (over the reserved
`input` relation) and importable `define transform` pipelines with `output`
handle declarations; `match` over enum slots as the only branching construct
in definitions (compile-time, closed arms); block slots (caller-provided
declarations at a marked splice point, with declared `exposes` handles) and
function-name slot values as the two remaining extension constructs;
two-tier publicity (named-is-public within a chart; across definition
boundaries, own-level names plus explicit `export` of nested internals);
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
catalog files configure named tables — iceberg/delta/object-store sources
and inline `source tables` namespace mounts — ambient by default and
importable as pinned single-mount dataset packs by catalogs and charts
(never definitions), with credentials via capability-gated `env`
values and `.env`, and catalog-level SQL views spelled
`table sql` — logical by default, materialized per session by opt-in, and
parameterized by defaulted `param` declarations, `$name` placeholders in a
once-planned query rebound per use via data-block properties or named
table-function arguments, chaining over any catalog relation with
placeholder forwarding), `expand`
as the only flattening (instantiations lowered to pure primitives with no
imports at all), and duplicate vendored definitions are harmless because
expansion is instance-namespaced; with the rejected shapes recorded in
[What Definitions Deliberately Cannot Do](#what-definitions-deliberately-cannot-do);
lowercase snake_case kind names throughout (`mark rect`, `mark box_plot`);
a six-node generic AST whose serde encoding is the JSON interchange form —
single-key tagged values, SQL islands as canonical text, closed node set —
under round-trip laws (printing is total and canonical: it *is*
`avenger fmt`; `parse(print(ast)) == ast`), so specs are produced and
consumed without the Rust library, validated structurally by a frozen
hand-written core JSON Schema (closed thirteen-tag value inventory) and
semantically by a full JSON Schema generated from the authoring schema;
and the EBNF grammar below.

This is the single reference for the language. The earlier companion
documents — the API/baseline audit and the missing-syntax proposals — have
been folded in: the feature-surface sections below carry their still-valid
content updated to the adopted rules, and the
[Coverage And Validation](#coverage-and-validation) section carries the
audit's inventory method and fixture plan. Every one of the 96 baseline
categories (891 deduped scenarios) has a syntax home in this document.

Added 2026-07-09: a draft [Dashboards](#dashboards) section — the third
file kind (`dashboard`), chart imports/instantiation, state bindings,
layout declarations, widgets/tabs/callbacks — normative for syntax, with
semantics owned by `dashboard-layer.md`, `dashboard-layout.md`, and
`widgets.md`. It is newer than the rest of this reference: its EBNF,
interchange-schema, and fixture integration are open questions, and the
96-category coverage claim above predates it.

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
`data: { table: 'sales'; }`, `data: { sql: ...; }` — always the block
form (no bare-string shorthand), with the same reserved source
properties as before (`table`, `sql`, `url`, `values`, plus table-param
bindings). Anonymous means private: chart-local relations are never
referenced by name; named shared relations are catalog (or dashboard)
`table` declarations, which is also the graduation path.

## Design Principles

- Every file begins with a version pragma: `avenger 1;`.
- Use block-structured declarations for charts, groups, marks, transforms,
  interactions, tools, and other chart objects.
- Use `as` whenever a typed declaration binds a structural or dataflow name:
  `group as manual_box_plot`, `mark rule as median`, and
  `transform aggregate as stats`. Declaration headers are uniformly
  `<keyword> <kind> [as <name>]`.
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
- Params are named `$param` placeholders inside SQL expressions; positional
  placeholders are rejected. All other DSL-injected references are reserved
  helper functions (`channel(x)`, `datum('id')`, `event_coord(x)`, ...),
  which tokenize as ordinary SQL and need no lexer support.
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
  declarations (marks, tools, themes). Compound marks and interaction tools
  are not Rust built-ins: they ship as a standard library written in the
  language (`import 'std:marks/box_plot';`) over a primitive
  Rust core — primitive marks, transforms, coordinates, scales, guides, and
  the event system. The conditional budget is explicit and closed:
  `channel` parameters rename, `match` over an enum slot selects among
  declared variants at expansion time, SQL `CASE` handles expression logic —
  and nothing else branches. No loops, no string templating: data-driven
  multiplicity belongs to `repeat` and `facet`, and programmatic generation
  belongs to the host-language APIs, which emit DSL.
- Preserve a path to a canonical DSL form generated from a compiled or
  lowered chart, even if exact source round-tripping is not a v1 requirement.

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
file: a compound mark, tool, or transform), or one `dashboard`
declaration (a dashboard file — the document format that imports and
composes charts; see [Dashboards](#dashboards), draft). A project is a
collection of such files; other projects import its definition files,
never its chart files — with one amendment: *dashboard files* import
chart files (definitions still cannot, and charts cannot import charts).
The definition kind is normative from the file's content; the
conventional extensions mirror it: `.avenger` for charts,
`.mark.avenger`, `.tool.avenger`, `.transform.avenger` for definitions,
and `.dashboard.avenger` for dashboards.

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
single *declared* mount name, whose in-file presence is load-bearing for
the pack's internal chains. **One public item per file is the law**; if
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
| `$name` | chart param placeholder | `"mpg" >= $min_mpg` |

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

## Core Declaration Form

The common declaration shapes are:

```avenger
chart <coord-kind> [as <name>] {
  ...
}

group [as <name>] {
  ...
}

mark <mark-kind> [as <name>] {
  ...
}

transform <transform-kind> [as <alias>] {
  ...
}
```

The body mode depends on the declaration. `chart` and `group` bodies are
ordered declaration blocks. `mark`, `transform`, `param`, `scale`,
`axis`, `legend`, `tool`, `selection`, and `view` bodies are property
blocks, and the `data:` property takes an anonymous source block.

Examples:

```avenger
chart cartesian as SalesByCategory {
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
and groups, the name is the structural id used by event targets and scene
queries. For transforms, the name is a dataflow alias used to address outputs.

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
unordered configuration; repeated child declarations are ordered. Container
bodies (`chart`, `cell`, `group`, and channel configuration blocks) mix both
modes:

```avenger
chart cartesian as Example {
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

Child-declaration order matters wherever a variable number of the same kind
of child can appear: transform order defines dataflow in a group, `cell`
order defines placement in concat containers, and `when` order defines
conditional-branch priority. A formatter emits properties first (`data:` among
them), then transforms, groups, marks, and cells; the parser allows
interleaving as long as lowering preserves declaration order.

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

Params are referenced with named `$param` placeholders:

```avenger
param as min_amount {
  default: 0;
}

transform filter {
  predicate: "amount" >= $min_amount;
}
```

Only named placeholders are valid. Positional placeholders such as `$1`, `$2`,
and `?` are rejected. A `$name` reference must match a param declared in the
enclosing scope — the chart in chart files, the declaring table inside a
catalog `table sql` body ([Data Catalogs](#data-catalogs)) — and lowers to
the same DataFusion placeholder id used by `Param::expr()`.

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
channels, enum values, declared ids — are bare identifiers; arguments naming
data-space things — datum fields, store columns — are strings. The reserved
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
interval(lo, hi)             construct a domain interval value
interval_ordered(a, b)       domain interval with endpoints sorted
polygon(event_path())        scene-query geometry from a drag path
```

Reserved namespaces (`repeat.row`, `repeat.column_id`, ...) resolve the same
way as transform aliases. No sigil forms exist beyond `$param`. The bare
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

Single-line comments require whitespace after `--`, so `amount--1`
continues to parse as a SQL expression rather than silently starting a
comment.

Doc comments use `---` and attach to the next declaration — a `define`, a
`slot`, `channel`, `output`, or `export`, a named internal mark (part
documentation), a `match` arm (so completing a mode value shows what it
means), or a `chart` (gallery blurbs). Content is CommonMark: the first
paragraph is the summary shown in completion; the rest is detail shown on
hover and in generated documentation. Fenced `avenger` code blocks inside
doc comments are runnable examples — `avenger doc` renders them into the
generated pages as images, and `avenger test` compiles them, so a published
definition's documentation carries proven examples:

````avenger
--- One rule per category showing the min-max span of a measure.
---
--- ```avenger
--- avenger 1;
--- import 'error_bar.mark.avenger';
--- chart cartesian {
---   data: { sql: SELECT * FROM 'examples/sales.csv'; }
---   mark error_bar { category: "region"; measure: "amount"; }
--- }
--- ```
define mark error_bar {
  --- Grouping expression; one bar per distinct value.
  slot category;
  --- Measure whose min and max span the bar.
  slot measure;
  ...
}
````

The lexical carve-out: line comments are `--` followed by whitespace, doc
comments are `---` — so `a---b` is reserved rather than an expression
(write `a - -b`; the formatter spaces operators anyway).

Block comments should support nesting:

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
form — set at chart, group, or mark level. A scope inherits its parent's
data context unless it sets its own `data:`; anonymous means private,
per the hygiene law, so a chart-local relation is never referenced by
name. Named, shared relations belong one level up: the catalog's
`table <kind> as` declarations (and a dashboard's), which is also the
graduation path when an inline block outgrows its chart.

```avenger
chart cartesian as sales_by_region {
  data: { table: 'sales'; }            -- reference a catalog table

  mark rect { x: "region"; y: "amount"; }
}
```

The block's reserved properties select the source — always the block
form, never a bare value:

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
  default: 'all';
}
```

Marks may also select interaction state as their source
(`data: store brush;` — see
[Tools, Selections, Stores, And Views](#tools-selections-stores-and-views)),
the same property in its reference form.

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

Statement policy: only query statements are accepted — `SELECT` (including
set operations such as `UNION`) and `VALUES`. DDL and DML (`CREATE`, `DROP`,
`INSERT`, `UPDATE`, `DELETE`, `COPY`, ...) are rejected at parse time, and
multi-statement payloads are rejected. Whether a query may touch the
filesystem or network (`FROM 'file.csv'`, URLs) is not a language question:
the system interpreting the file grants or denies those capabilities.

Resource declarations (`param`, `store`, `selection`, ...) bind names in
the chart scope; `data:` deliberately binds none. A future shorthand may
allow compact forms such as `param selected_region: "all";`, but the
canonical grammar should start with object bodies.

## Data Catalogs

A data file (`.data.avenger`) configures the named tables available to the
project's charts. By default it is **ambient host configuration**: charts
reference tables by name and stay environment-independent, so swapping the
catalog file retargets every chart against a different environment without
touching one. Definitions can never declare or import data — a library can
never smuggle a connection.

```avenger
avenger 1;

source iceberg as warehouse {
  catalog: rest;
  uri: 'https://catalog.example.com';
  warehouse: 's3://analytics-warehouse';
  namespace: 'analytics';
  token: env 'ICEBERG_TOKEN';
}

source delta as events {
  uri: 's3://analytics/events';
}

table parquet as orders {
  path: 'data/orders.parquet';
}

table csv as regions {
  path: 'https://example.com/regions.csv';
}
```

- `source <kind> as <name>` mounts a namespace of tables; charts reach them
  as `table: 'warehouse.orders'` (and full `sql:` statements use ordinary
  qualified names: `FROM warehouse.orders`). `table <kind> as <name>` binds
  one table into the flat namespace.
- Kinds lower to DataFusion catalog and `TableProvider` registrations:
  `iceberg` (REST, Glue, and SQL catalogs via iceberg-rust), `delta`
  (delta-rs), `parquet`/`csv`/`json` over `object_store` URIs (`s3://`,
  `gs://`, `az://`, and project-relative paths), and `sql` — a query over
  the catalog ([Views](#views)). For the file kinds, `path:` accepts a
  single file, a directory, or a glob; directory binds lower to DataFusion
  listing tables, with hive-style partition directories (`year=2024/`)
  exposed as columns.
- **Credentials never live in the file.** Providers use their standard
  credential chains by default; the `env '<NAME>'` value form wires a
  property to an environment variable explicitly. Reading the environment
  is a capability (`--allow-env[=VAR]`), and the CLI loads a project-root
  `.env` file (gitignored by `avenger new`) before resolving them. A lint
  warns when a secret-shaped property carries a string literal.
- All `.data.avenger` files in the project load and merge (name collisions
  are errors); `--catalog <file>` restricts the session to specific files —
  the dev/prod switch.
- Inline `data:` blocks inside charts remain for chart-owned files (the
  chart-package pattern); the catalog is for shared, named, and remote
  tables.
- Catalogs give tooling real schemas: `avenger tables` lists resolved names
  and columns, and the language server grounds column completion inside
  expression slots on them.

A namespace can also be declared rather than mounted from an external
system — `source tables` collects `table` binds into what consumers see
as a catalog:

```avenger
source tables as vega {
  table json as cars   { path: 'data/cars.json'; }
  table csv  as stocks { path: 'data/stocks.csv'; }

  table sql as cars_clean {
    sql: SELECT * FROM cars WHERE "Horsepower" IS NOT NULL;
  }
}
```

This sits exactly on the two catalog axes: `source` because it mounts a
namespace (`vega.cars`, like `warehouse.orders`), `tables` because the
namespace's origin is inline declaration rather than an external catalog.
Inside the mount, siblings resolve bare (`FROM cars`); outside, names are
qualified. Everything a top-level `table` can do is unchanged inside a
mount — file kinds, `table sql`, params, `materialize:`, chains. Mounts do
not nest: the lowering is DataFusion's catalog/schema/table, and one mount
is one schema.

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
  any relation in the merged catalog — file-backed tables, source-mounted
  namespaces, other `table sql` entries, imported packs — regardless of
  declaration order. Query-over-query forms a DAG; cycles
  are errors. `$name` placeholders resolve only to the table's own
  declared params (below) — a chart's params never reach the catalog.
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
  (`warehouse.orders`, `vega.cars` — mounts are schemas); environment
  retargeting is the catalog swap, which is `ref()`'s other job.

Params parameterize a `table sql` — the same `param` construct charts
declare, referenced the same way, lowering to the same DataFusion
placeholder. A parameterized table plans **once**, placeholders included;
bindings are applied at execution (`with_param_values`), so changing a
binding — interactively or between uses — rebinds the plan rather than
re-planning it:

```avenger
table sql as borough_trips {
  param as borough { default: 'Manhattan'; }
  param as min_fare { default: 0; }

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

- **Every param carries a default.** A bare `FROM borough_trips` is always
  valid — it binds the defaults. A query with genuinely required inputs is
  a `define transform`, which also differs in kind: it rewrites an
  upstream `input` relation mid-pipeline rather than acting as a source.
  The default doubles as the param's type witness for schema derivation.
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

Chains compose freely — a `table sql` builds on file-backed tables, source
mounts, and other `table sql` entries alike, and a parameterized link
forwards its params to the links it calls:

```avenger
table parquet as zones { path: 'data/zones.parquet'; }

table sql as zoned_trips {
  param as borough { default: 'Manhattan'; }

  sql:
    SELECT t.*, z."zone_name"
    FROM borough_trips(borough => $borough) AS t
    JOIN zones AS z USING ("zone_id");
}
```

- **A table-function argument accepts exactly what a binding accepts**: a
  scalar literal or a `$param` visible in the calling scope — the calling
  table's own params here, the chart's params in chart files. Forwarding
  composes placeholders: the inlined plan carries the outer placeholder,
  so an entire chain still plans once and rebinds at execution —
  `zoned_trips(borough => 'Queens')` reaches through to the
  `borough_trips` filter.
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

- **Data files import data files** (catalog composition): the project's
  catalog pulls in a published pack — one line binding its mount.
- **Chart files may import data files** (the portable mode): a tutorial or
  example chart carries its data reference and runs anywhere. Deployed
  charts should prefer ambient tables; portability is a choice, not the
  default.

A pack is a data file organized as one `source tables` mount:

```avenger
-- vega-datasets@2.11.data.avenger, published on a CDN
avenger 1;

source tables as vega {
  table json as cars   { path: 'data/cars.json'; }
  table csv  as stocks { path: 'data/stocks.csv'; }
}
```

```avenger
avenger 1;

import 'https://cdn.example.com/vega-datasets@2.11.data.avenger'
  sha256 '4c1e...';                 -- binds the pack's mount: vega

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
  single `source tables` mount, so a data-file import binds exactly one
  name — the same rule as every other import — and `as` optionally
  renames the mount (`vega` → `v`). Multi-name data files are not
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
declarations in the same group. If order-sensitive interleaving becomes useful,
the compiler can later lower it by inserting anonymous groups.

## Transforms

All transforms use:

```avenger
transform <kind> [as <alias>] {
  property: value;
}
```

The alias is optional when the transform writes explicitly named fields into
the current data scope. The alias is strongly recommended, and may be required,
for transforms with conventional output handles such as `bin`, `stack`, `kde`,
`time_unit`, and `rasterize_2d`.

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
transform list or a `define transform` body — each stage's `input` is the
previous stage's result, and the first stage's `input` is the data context
at that point (the group's inherited data, or the definition's
instantiation site).

```avenger
transform sql as ranked {
  query:
    SELECT *,
           row_number() OVER (ORDER BY "amount" DESC) AS rank
    FROM input;
}
```

The `query:` statement follows the same policy as data `sql:` — `SELECT`
(including set operations) or `VALUES` only — and may join registered tables
(`FROM input JOIN dims ON ...`), subject to the host's data capabilities.
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

`domain`, `raw_domain`, `range`, `order_by`, and `order` map to
`ScaleConfigSpec`; the remaining per-type options come from the generated
language schema.

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
  target: mark manual_box_plot.outliers;
  filter: datum('value') > $threshold;
  consume: true;

  set param selected_group = datum('group');
  set param selected_value = datum('value');
}
```

The target selector is a value. It should support at least:

```avenger
target: plot;
target: legend fill;
target: mark manual_box_plot.outliers;
target: marks [manual_box_plot.box, manual_box_plot.median];
```

Between-event bindings can be block-valued properties:

```avenger
on pointermove as drag_box {
  target: plot;
  between: {
    start: pointerdown {
      target: mark manual_box_plot.box;
    }
    end: pointerup;
  }

  set param drag_x = event_coord(x);
}
```

Event type names use the Rust event names in snake case: `mouse_down`,
`mouse_up`, `click`, `double_click`, `mouse_wheel`, `key_press`,
`key_release`, `cursor_moved`, `mark_mouse_enter`, `mark_mouse_leave`,
`window_resize`, `window_resize_settled`, `canvas_resize`,
`canvas_resize_settled`, `window_moved`, `window_focused`, and
`window_close_requested`. Binding properties include `target:`, `surface:`,
`filter:`, `throttle_ms:`, `consume:`, `mode: preview | exact`, and
`settle_exact:`.

Store and selection updates use the same action form with typed update
payloads:

```avenger
set store hover = clear;
set store hover = insert_rows  { row { id: datum('id'); } }
set store hover = replace_rows { row { id: datum('id'); } }
set store hover = upsert_rows  { row { id: datum('id'); x: event_coord(x); } }
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
`toggle_clauses`. Clause predicates support `equality` and `interval`
dimensions (`interval x { from: start_coord(x); to: event_coord(x); }`), and
geometry-driven selection uses the scene-query update kind — the primitive
that makes lasso and box selection definable in the language:

```avenger
set selection picked = from_scene_query {
  geometry: polygon(event_path());
  policy: intersects;
  marks: [points];
}
```

## Tools, Selections, Stores, And Views

The same object syntax covers interaction state. Params declare a `kind` for
the special runtime param families, and stores declare ordered `field` and
`row` children:

```avenger
param as x_domain {
  kind: raw_domain;
  default: null;
  sharing: shared;
}

param as hover_cursor {
  kind: cursor;
  default: 'default';
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

Tool kinds are standard-library definitions, not Rust built-ins — the
language's primitive interaction surface is the event system itself (event
bindings, helpers, scale edits, and store/selection updates), and every tool
is a definition over it (see
[Imports And Definitions](#imports-and-definitions)). Their slots complete
from the same schema machinery as built-in properties:

```avenger
import 'std:tools/pan_scroll_zoom';

tool pan_scroll_zoom as zoom {
  x_domain_param: x_domain;
  y_domain_param: y_domain;
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

`lasso_selection`, `box_selection`, and coordinate-specific tools such as
`geo_pan_zoom` follow the same declaration shape — all of them `std:tools`
definitions built from event-system primitives (lasso, for example, is a
between-binding accumulating `event_path()` plus a scene-query selection
update).

Views declare reactive viewport state:

```avenger
view cartesian as viewport {
  x_domain: $x_domain;
  y_domain: $y_domain;
}
```

Views used by groups should be declaration statements in the group block:

```avenger
group as viewed_points {
  view viewport;

  transform filter {
    predicate: "amount" > 0;
  }

  mark symbol as points {
    x: "x";
    y: "y";
  }
}
```

## Low-Level Box Plot Example

This example mirrors the current compound box plot lowering: a root group, a
fence branch that feeds inliers and outliers, and a summary branch for the box
and median.

```avenger
avenger 1;

chart cartesian as ManualBoxPlot {
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

The public event paths are rooted at the named group and named marks:

```avenger
on click as select_outlier {
  target: mark manual_box_plot.outliers;
  set param selected_group = datum('group');
}
```

Intermediate group names such as `fence`, `inliers`, and `summary` are useful
for readability. A later design pass should decide whether all named groups are
public event-path nodes, or whether there is a separate `public: true;`
property for structural ids that should survive lowering.

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
headers are `cell <coord> [as <id>] [at { ... }]`:

```avenger
chart hconcat as overview {
  spacing: 12;
  widths: [fr(2), px(280)];
  axis_guide_visibility: outer_edges;

  cell cartesian as left {
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
chart grid_concat as dashboard {
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
`shared`, or `level(n)`; `empty_cells` accepts `holes`, `empty_subplot`, and
`auto`:

```avenger
chart facet as by_region_segment {
  row: "region" {
    title: 'Region';
    slots: shared;
    empty_cells: holes;
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

## Compound Statistical Marks

Compound statistical marks are standard-library definitions
(see [Imports And Definitions](#imports-and-definitions)), not Rust-native
kinds — the language's built-in mark vocabulary is primitives only. They are
instantiated like any mark, styled through `part` blocks, and oriented
through channel-parameter bindings rather than an `orientation` property:

```avenger
avenger 1;

import 'std:marks/box_plot';
import 'std:marks/violin';

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

`std:marks`' `box_plot` declares `channel band_axis: x;` and
`channel value_axis: y;` plus `slot category;`, `slot values;`, and
`slot extent: 1.5;` — so vertical is the default binding and horizontal is a
rebinding, with scales, axes, and band boundaries following the channels
automatically. Part names are the definition's internal mark names and lower
to stable event-path suffixes (`mpg_box.box`, `mpg_box.median`,
`mpg_box.whiskers`, `mpg_box.lower_cap`, `mpg_box.upper_cap`,
`mpg_box.outliers`). The `group` lowering shown in
[Low-Level Box Plot Example](#low-level-box-plot-example) is what such a
definition expands to.

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

The targetable surface is exactly the definition's public surface — its
own-level named marks plus `export`s — so events, caller `part` overrides,
and theme selectors share one declared surface, and nesting privacy applies
uniformly (an internal `error_bar`'s `bar` is `ranged_dots::part(bar)` only
if exported, and `error_bar::part(bar)` never matches it). Cascade
layering makes part theming effective: a definition's literal styling on
its public parts sits at the default layer, below theme part rules, while
caller `part` overrides sit above them — chart-author explicit values >
caller part overrides > theme part rules > definition part defaults >
general theme rules.

Part selectors are a theme-engine (Rust) capability, but a generic one: the
engine parses `<kind>::part(<name>)`, compiled marks carry an opaque
`(kind, part)` provenance pair populated by lowering for public parts (the
expanded-source `theme_part:` property is that field's serialization), and
the matcher compares strings. Rust never learns specific kinds — a
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

  opacity_by_total: pixels.total {
    scale: linear { range: [0.15, 1.0]; }
  }

  smooth: true;
}
```

The `rasterize_2d` output handle exposes `pixels.raster`, `pixels.x_dim`,
`pixels.y_dim`, `pixels.by_dim`, and `pixels.total`.

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

import 'std:tools/geo_pan_zoom';

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
      mark box_plot { y: "mpg"; orientation: vertical; }
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

Definitions are load-bearing: **the language has no Rust-defined compound
marks or tools.** Box plots, violins, pan/zoom, and selection tools ship as
a standard library written in the language itself and imported like any
other library. The Rust boundary sits at primitives: primitive marks,
transforms, coordinates, scales, guides, and the event system (bindings,
helpers, event paths, scene queries, scale edits, store/selection updates).
Everything compound is composition, and definitions are structural
templates over those primitives — parameterized by slots and channel
parameters, with `match` over enum slots as the only branching form.

### Defining A Compound Mark

```avenger
-- lib/error_bar.mark.avenger
avenger 1;

define mark error_bar {
  slot category;
  slot measure;
  slot cap_width: 0.3;

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

`slot` declarations are the definition's property schema. Slot references
are bare identifiers inside the body — they live in the DSL namespace like
every other bare name, which is why mandatory column quoting matters:
`group_by: category;` is unambiguously the slot, `group_by: "category";`
would be the literal column. Slot values are ordinary property values
(expressions, `value` literals, numbers, typed blocks), plus function names
and declaration blocks (see below), substituted structurally at expansion
time, before name resolution. A slot default may reference earlier slots
(`slot cap_width: band_width / 2;`); default references must be acyclic.

### Channel Parameters

A `channel` parameter binds a *logical* channel to a physical one at the
instantiation site. This is how one definition serves both orientations
without conditionals — orientation is just a channel binding:

```avenger
define mark error_bar {
  channel band_axis: x;      -- logical channel, default binding x
  channel value_axis: y;
  slot category;
  slot measure;

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

Expansion renames logical channels wherever channel identity appears:

- channel property names on primitive marks, including the interval family —
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
context at that point — the mechanism that makes standard-library compounds
extensible without forking them. The default content is the slot's default;
a bare `name;` statement marks the splice point:

```avenger
define mark box_plot {
  channel band_axis: x;
  channel value_axis: y;
  slot category;
  slot values;
  slot outlier_marks: {
    mark symbol as outliers {
      band_axis: category { band: 0.5; }
      value_axis: values;
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
mark box_plot as mpg_box {
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
  slot time_col;
  slot measure;
  slot annotations: { }

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
reorients the annotations untouched. `rev.target` is a valid event path:
spliced content lands inside the expansion like anything else.

Rules:

- Block-slot content resolves names in the caller's scope, except that the
  definition's channel parameters apply and the data context is the splice
  point's.
- By default, block content reaches only the data context's *columns*. A
  slot may hand over specific internal handles with an `exposes` clause —
  the definition declares exactly what crosses the boundary, and nothing
  else leaks (Vue's scoped-slot props are the precedent):

  ```avenger
  slot annotations exposes [fence, stats]: { }
  ```

  Content passed to that slot may reference `fence.lo` or `stats.median`;
  content passed to a slot without `exposes` may not reference any internal
  alias.
- A block slot has exactly one splice point. Passing an empty block
  (`outlier_marks: { }`) removes the default structure; an empty default
  (`slot annotations: { }`) makes the slot purely additive.
- Guidance: `match` is for closed structural modes the definition owns;
  a block slot is for an open extension point the caller owns. The two
  compose.

### The Caller Surface: Slots Plus Parts

Instantiating a defined mark looks exactly like using a built-in compound
mark: properties bind slots, and `part` blocks style the internal named
marks — the same surface native compound marks expose:

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
- `part <name> { ... }` targets the internal mark named `<name>`; its
  properties merge over the definition's, use site winning. Parts cover
  open-ended styling so definitions do not need a slot per styleable
  property. The same public-part surface serves theme part selectors
  (`box_plot::part(median)`; see [Themes And CSS](#themes-and-css)) and
  event targets — one declared surface, three consumers.
- Nesting is private by default: when a definition instantiates another
  definition internally, the inner instance's parts and state are *not*
  reachable from outside — an internal `error_bar` is an implementation
  detail, free to change. The outer definition re-publishes deliberately
  with `export`, optionally renaming (the Web Components `exportparts` rule):

  ```avenger
  define mark ranged_dots {
    slot category;
    slot measure;

    mark error_bar as inner { category: category; measure: measure; }

    export inner.bar as bar;
  }
  ```

  Callers then style `part bar { ... }` and target `<instance>.bar` in
  events; `inner.caps` remains private. `export` covers nested parts and
  nested instance state (params, stores, selections) alike.
- Internal `as` names expand hygienically under the instance id: `errs.bar`,
  `errs.caps`, and `errs.center` are the public event paths, exactly like
  built-in compound part paths.
- The expansion inherits the instantiation site's data context and
  participates in the chart's scales like any inline group; definitions may
  not set `data:`.

### Defining Tools And Behaviors

A tool is a definition over the event system, and needs up to four kinds of
expansion content — the same set Rust's `ToolExpansion` produced when tools
were native. Tool definitions have a spelling for each, all instance-scoped:

- `param as` / `store as` / `selection as` declarations (generated state);
- `on` event bindings;
- `scale_edit { channel: ...; ... }` declarations, which apply scale
  configuration in the instantiating chart's scope — this is what lets a
  zoom tool bind its own raw-domain params without the caller wiring
  `raw_domain:` by hand;
- ordinary marks, typically store-backed (`data: store brush;`) for
  drag-rectangle and lasso chrome.

A from-scratch tool built only from these primitives:

```avenger
define tool drag_pan {
  channel axis: x;
  slot button: left;

  param as domain { kind: raw_domain; default: null; }

  scale_edit {
    channel: axis;
    raw_domain: $domain;
  }

  on cursor_moved {
    target: plot;
    between: {
      start: mouse_down { target: plot; button: button; }
      end: mouse_up;
    }

    set param domain = interval(
      event_domain_start(axis) - (event_coord(axis) - start_coord(axis)),
      event_domain_end(axis) - (event_coord(axis) - start_coord(axis))
    );
  }
}
```

`tool drag_pan;` pans x; `tool drag_pan { axis: y; }` pans y — the channel
parameter renames through the scale edit and the bare helper arguments
alike. Geometry-driven tools are compositions too: the event system's scene
queries are a primitive, so a lasso is a between-binding accumulating
`event_path()` plus a selection update from a scene query
(`set selection picked = from_scene_query { ... }`) — which is exactly how
the standard library defines `lasso_selection`. Definitions may also wrap
other definitions, preconfiguring them through slots:

```avenger
-- lib/wheel_zoom.tool.avenger
avenger 1;

import 'std:tools/pan_scroll_zoom';

define tool wheel_zoom {
  slot base: 1.05;

  param as x_domain { kind: raw_domain; default: null; }
  param as y_domain { kind: raw_domain; default: null; }

  tool pan_scroll_zoom {
    x_domain_param: x_domain;
    y_domain_param: y_domain;
    scroll_zoom: true;
    zoom_base: base;
  }
}
```

```avenger
-- lib/hover_highlight.tool.avenger
avenger 1;

define tool hover_highlight {
  slot target;

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

  tool wheel_zoom;
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
instance namespace (`hover.hovered` above), so two instances of the same
tool never collide. A slot may receive a mark name (`target: points;`), a
param reference (the caller passes `$zoom_enabled` as the slot value), a
column, or any other property value.

### Defining Transforms

A transform definition is a named, slotted pipeline of transform stages —
built-ins, other imported transform definitions, and `transform sql` stages.
`output` declarations are its public handle schema: the fields an
instantiation alias exposes, each mapping to a column of the pipeline's
result (bare when the column has the same name, explicit when re-exporting
an internal stage's field).

```avenger
-- lib/share_within.transform.avenger
avenger 1;

define transform share_within {
  slot measure;
  slot by;                       -- array slot: grouping expressions
  output share;

  transform sql {
    query:
      SELECT *, measure / sum(measure) OVER (PARTITION BY by) AS share
      FROM input;
  }
}
```

```avenger
-- lib/binned_counts.transform.avenger
avenger 1;

define transform binned_counts {
  slot field;
  slot maxbins: 30;
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
      by: ["region", "year"];
    }

    mark rect {
      x: "region";
      y: s.share;
      fill: "year";
    }
  }
}
```

Slot substitution inside `query:` statements: the declared slot names are
spliced as expressions into the statement's AST before planning (everything
else in a statement follows ordinary SQL rules), and an array-valued slot in
a list position — `PARTITION BY`, `GROUP BY`, a `SELECT` list, `IN (...)` —
expands to a comma-separated list. The resolver warns when a slot name
shadows a column of the incoming context; rename the slot or quote the
column.

An instantiation-level `scope:` property sets the coordination scope for
every stage that does not declare its own. Output declarations are validated
against the pipeline's actual result schema at compile time, and they are
what the alias namespace and editor completion expose (`s.share` above).

Slot names must parse as identifiers inside statements, so SQL reserved
words (`order`, `end`, `group`) cannot name slots; the resolver rejects them
with a rename suggestion.

A slot may also be bound to a *function name* from the function registry,
which splices in call position — the construct that keeps open sets of
aggregations from becoming one `match` arm per function:

```avenger
define transform rolling {
  slot measure;
  slot order_key;
  slot agg: avg;             -- any aggregate function name
  slot preceding: 6;
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
validates `median` against the registry at instantiation; binding a
non-function to call position (or vice versa) is an error.

### Modes: `match` Over An Enum Slot

Some transforms need different logic per mode. When the modes differ only in
*expressions*, no construct is needed: a mode slot splices as a literal, so a
SQL `CASE WHEN mode = 'center' ...` constant-folds at planning time. When
the modes differ in *stages*, `match` selects among closed variants at
expansion time. The arms are the slot's entire domain: instantiating with
any other value is an error listing the arms, there is no default arm, and
editors complete mode values from the arm names.

The stack transform is the canonical case — one shared windowing stage, then
a per-mode finishing stage:

```avenger
define transform stack {
  slot measure;
  slot by;                       -- partition expressions
  slot order_key;                -- sort within each partition
  slot mode: zero;
  output start;
  output end;

  transform sql {
    query:
      SELECT *,
        sum(measure) OVER (PARTITION BY by ORDER BY order_key) AS __stack_end,
        sum(measure) OVER (PARTITION BY by) AS __stack_total
      FROM input;
  }

  match mode {
    zero {
      transform sql {
        query:
          SELECT *,
            __stack_end - measure AS start,
            __stack_end AS "end"
          FROM input;
      }
    }
    center {
      transform sql {
        query:
          SELECT *,
            __stack_end - measure - __stack_total / 2 AS start,
            __stack_end - __stack_total / 2 AS "end"
          FROM input;
      }
    }
    normalize {
      transform sql {
        query:
          SELECT *,
            (__stack_end - measure) / __stack_total AS start,
            __stack_end / __stack_total AS "end"
          FROM input;
      }
    }
  }
}
```

```avenger
transform stack as s {
  measure: "amount";
  by: ["category"];
  order_key: "segment";
  mode: center;
}

mark rect { x: "category"; y: s.start; y2: s.end; fill: "segment"; }
```

The arms use the shared base through ordinary pipeline chaining: `match`
splices the selected arm's stages in place, so the expanded pipeline is
exactly *base stage → arm stage*, and the arm's `FROM input` reads the base
stage's result — `__stack_end` and `__stack_total` are ordinary columns of
its input relation. A stage written after the `match` block would consume
the arm's output the same way. Instantiating with `mode: center;` and
`measure: "amount"` expands to nothing more than:

```avenger
transform sql {
  query:
    SELECT *,
      sum("amount") OVER (PARTITION BY "category" ORDER BY "segment") AS __stack_end,
      sum("amount") OVER (PARTITION BY "category") AS __stack_total
    FROM input;
}

transform sql {
  query:
    SELECT *,
      __stack_end - "amount" - __stack_total / 2 AS start,
      __stack_end - __stack_total / 2 AS "end"
    FROM input;
}
```

`match` is compile-time selection, distinct from runtime `when` branches on
channels: exactly one arm's declarations splice into the body during
expansion. Intermediate columns use a `__<define>_` prefix by convention to
avoid colliding with input columns, and expansion qualifies them by
*instance* id (`__s1_stack_end`), so instantiating the same definition twice
in one pipeline cannot collide; a projection option for dropping
intermediates is an open question.

`match` works in every definition kind and at any depth within one — inside
a group, a mark body, or an event body — with each arm contributing whatever
items are valid at that position: transform stages, marks, event bindings,
actions, scale edits, or properties. An empty arm splices nothing, which is
how optional structure is spelled. A mark definition selecting structure
(sketch):

```avenger
define mark box_plot {
  channel band_axis: x;
  channel value_axis: y;
  slot category;
  slot values;
  slot outliers: show;

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
define tool point_select {
  slot target;
  slot sel;
  slot mode: toggle;

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
`match` composes with channel parameters — the box-plot sketch above is
orientation-generic and outlier-optional at once.

This keeps the conditional budget of the language explicit: `channel`
parameters rename, `match` selects among declared closed variants, SQL
`CASE` handles per-row and constant-foldable expression logic — and nothing
else branches.

### The Standard Library

Compound marks and interaction tools ship as definition files bundled with
the language — one definition per file, like everything else — imported
individually through the `std:` scheme, with no privileged status beyond
distribution:

```avenger
avenger 1;

import 'std:marks/box_plot';
import 'std:marks/violin';
import 'std:tools/pan_scroll_zoom';
import 'std:tools/lasso_selection';
import 'std:transforms/stack';
```

- `std:marks/` holds `box_plot`, `violin`, `error_bar`, ...; `std:tools/`
  holds `pan_scroll_zoom`, `drag_pan`, `box_zoom`, `point_selection`,
  `lasso_selection`, `box_selection`, `unit_aspect_box`, ...;
  `std:transforms/` holds pipelines over `transform sql` such as `stack`
  and `share_within` (algorithmic kernels — kde, bin nice-ing,
  rasterize_2d — stay primitive); `std:themes/` holds plain CSS files
  (`light.css`, `dark.css`, ...) referenced with
  `theme css from 'std:themes/dark.css';` rather than imported.
- Standard library imports are explicit and per-definition; there is no
  implicit prelude.
  Decompilation emits the imports it needs.
- `std:` resolution is host-provided (bundled resources, no filesystem
  capability required) and versioned with the language: `avenger 1` pins
  stdlib 1.
- Standard definitions use snake_case kind names (`box_plot`, not
  `BoxPlot`), matching every other kind family.
- Because stdlib definitions expand to primitive groups, the low-level
  form shown in [Low-Level Box Plot Example](#low-level-box-plot-example)
  is, in essence, the body of `std:marks/box_plot` — decompilation of a
  stdlib instantiation can emit either the instantiation or its expansion.

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
  (channel parameters and function-name slots). Output column names, mark
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
    exec_dashboard.avenger
    exec_dashboard.png
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
| Standard library | `import 'std:marks/box_plot';` | bundled with the language, versioned by the pragma |
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

`avenger expand` is **macro expansion**: every instantiation is replaced by
its expansion — slots substituted, `match` arms resolved, channel parameters
renamed, block slots spliced, part overrides merged. Each defined-mark
instance is emitted as a group named by the instance id, so `mpg_box.box`
event paths survive through ordinary group nesting; instance-scoped state
mangles to `<instance>__<name>` with all references rewritten (the `__`
separator is reserved for this). The output contains no imports — `std:`
included — and no `define`, `slot`, `channel`, `match`, `exposes`, or
`export` constructs: pure primitives, exactly the elaborated forms shown in
the `match` and block-slot sections. One reserved property carries part
provenance through the lowering: expansion stamps each public part mark with
`theme_part: box_plot.median;`-style metadata so theme part selectors
(`box_plot::part(median)`) keep matching after the kind names are gone —
and the equivalence property below forces this to be right, since a themed
chart whose expansion dropped its stamps would compile differently.
Expansion has no name-reconciliation problem at all: the definition
namespace — where any collection operation's conflicts would live — is
deleted, and every remaining name is instance-scoped by construction
(groups nest under instance ids, state and intermediate columns qualify by
instance id), so two versions of the same library expand side by side
without contact. This is the archival form (it freezes
rendering semantics against library and stdlib evolution), the debugging
form (what a chart actually lowers to), and the minimal-runtime form (a
host can execute it with the definition machinery entirely absent).

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

### Hygiene And Resolution Rules

- Expansion is purely structural: no loops, recursion, or string
  templating, and the only branching is `match` over an enum slot, resolved
  at expansion time. Definitions may not define other definitions. A
  definition body may instantiate other imported definitions; acyclic
  imports keep expansion finite.
- **Full param hygiene**: `$name` inside a definition body may reference
  only params declared within that same definition. External state arrives
  through slots — the caller passes `$zoom_enabled` as a slot value. A
  library file can never silently depend on chart state.
- **No styling side effects**: `theme css` declarations are valid only in
  chart bodies. A definition styles itself through its own mark properties
  and part defaults and can never inject chart-global CSS.
- **Two-tier publicity**: within a chart, anything named is public — the
  chart author owns everything. Across a definition boundary, the public
  surface is exactly what the definition names at its own level plus its
  `export` declarations; nested instances' internals stay private, and
  block-slot content sees only data-context columns plus declared `exposes`
  handles. Anonymous declarations are private everywhere.
- `import 'path';` binds exactly one name — **the file stem** (filename-
  as-name, adopted 2026-07-09; a matching in-file binder is optional and
  validated); `import 'path' as eb2;` renames it, which is also how two
  same-named items from different sources coexist, and is *required* when
  the stem is not a valid bare identifier. (A data file is importable only
  when it declares exactly one name — in practice a `source tables`
  mount; catalogs keep declared names; see
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
internal mark names complete inside `part` blocks and event targets, and a
defined kind is indistinguishable from a native one in the editor.

## Dashboards

**Maturity: draft (2026-07-09)** — newer than the rest of this reference.
This section is the normative home of the dashboard *syntax*; the
architecture and semantics live in dedicated documents that win on their
own turf: `dashboard-layer.md` (component/binding model, engine
representation, runtime), `dashboard-layout.md` (the layout model),
`widgets.md` (the widget paradigm). EBNF productions, interchange-schema
entries, and fixture coverage for this section are pending (see
[Open Questions](#open-questions)); implementation is sequenced after the
widget system.

A dashboard is the third file kind: a document that imports charts,
composes them with widgets and text panels in a document-flow layout, and
coordinates them through shared state. Its governing law: **concat
composes aligned plots** (one chart — one data context, coordinated
scales/guides/layout); **a dashboard composes independent panels** (many
charts — independent scales, coordinated *state*). Alignment lives below
that line, state above it; a panel needing aligned plot areas holds a
concat chart.

```avenger
avenger 1;

import 'charts/revenue_trend.avenger';        -- chart files: importable by dashboards
import 'charts/category_detail.avenger';
import 'std:widgets/range_slider';
import 'std:widgets/select';

dashboard as exec_overview {
  title: 'Revenue Overview';
  theme css from 'themes/corporate.css';

  -- dashboard-scope state: the same constructs, one level above chart-shared
  param as region   { default: 'all'; }
  param as date_lo  { default: DATE '2026-01-01'; }
  param as date_hi  { default: DATE '2026-12-31'; }
  selection as picked_categories { empty: all; }

  -- shared derived data: the catalog construct, scoped to the dashboard
  table sql as filtered_orders {
    materialize: session;
    sql:
      SELECT * FROM orders
      WHERE ("region" = $region OR $region = 'all')
        AND "date" BETWEEN $date_lo AND $date_hi;
  }

  -- document-flow layout: fixed width, height grows, one document scroll
  width: fill { max: 1200; }
  spacing: 12;

  sidebar left {
    width: 280;

    widget select as region_w {
      param: region;
      options: SELECT DISTINCT "region" FROM orders ORDER BY 1;
      all_value: 'all';
      label: 'Region';
    }

    widget range_slider as dates_w {
      lo_param: date_lo;
      hi_param: date_hi;
      extent: (SELECT min("date"), max("date") FROM orders);
      label: 'Dates';
    }

    text as kpi {
      syntax: typst;
      content: 'Total: #currency(' || (SELECT sum("amount") FROM filtered_orders) || ')';
    }
  }

  row {
    height: px(220);

    chart revenue_trend as trend {
      date_lo: $date_lo;                       -- bare $ = alias (two-way)
      date_hi: $date_hi;
      highlight: $picked_categories;           -- selection aliasing: cross-filter
    }
  }

  row {
    height: aspect(21, 9);

    chart category_detail as detail {
      picked: $picked_categories;              -- writes here filter `trend` above
    }
  }
}
```

### Chart Imports And Instantiation

- A dashboard imports chart files with the ordinary import machinery
  (binds the chart's declared name, `as` renames, hash pinning and
  closure fetching unchanged). This is the answer to the former open
  question "should whole charts become importable" — importable **into
  dashboards only**; definitions still may not import charts, and charts
  may not import charts.
- Instantiation is `chart <name> as <instance> { <bindings> }`, mirroring
  defined-mark instantiation, with the identical kind-slot rule:
  coordinate kinds (`cartesian`, `polar`, ...) are reserved words —
  `chart cartesian as inline_scatter { ... }` declares an inline chart in
  place — while imported names are user names. Instantiating one chart
  twice with different bindings is ordinary; that is what makes it a
  component.
- **A chart's public interface is its declared state**: params (name +
  default), named selections and stores, and named marks (event paths).
  Nothing is added to a chart file to make it embeddable, and unbound
  params keep their defaults, so every chart remains
  standalone-renderable. `avenger info <chart file>` prints the contract;
  the language server completes binding names at instantiation sites from
  the imported file's declarations.
- **Naming: the file stem is the chart's name** (filename-as-name; see
  [Source Header And Versioning](#source-header-and-versioning)). An
  in-file `as` binder is optional and must match the stem; every chart
  file is importable. Imports share the single flat namespace with
  definitions; the same-name-collision and `as`-rename rules apply
  unchanged, and `as` is required where the stem is not a valid bare
  identifier.
- **What travels: the code closure, not the data environment.** The
  chart's own imports (stdlib marks/tools, themes, dataset packs) come
  along, transitively hash-pinned; ambient catalog references do not —
  the dashboard's project supplies the catalog, which is what retargets
  one chart across environments.
- **The instance name is the panel key**, and it is load-bearing: event
  paths surface instance-prefixed (`trend.points`, the compound-mark
  rule), chart-internal state is panel-scoped under it (the
  `generated_tool_name` convention — two instances never cross-link
  accidentally), hot-reload state survival matches by it, and
  baselines/introspection address by it. Anonymous instantiation is legal
  (the mark precedent) but keyed structurally: no addressable event
  paths, and state survival breaks if panels reorder — name your panels.

### State Bindings

Instantiation-body properties bind the chart's state to dashboard scope,
uniformly across all three families (params, selections, stores):

- **Bare `$name` — aliasing (two-way).** The chart's param and the
  dashboard's share one cell; chart-internal `set param` writes propagate
  up. Two charts aliasing their `x_domain` params to one dashboard param
  are link-zoomed with no further syntax.
- **A SQL expression — derived (one-way).** The chart param follows the
  expression over dashboard params. Because `set param` actions are
  declared syntax, a chart tool writing to a derived-bound param is a
  **compile-time error** (this is deliberate: the equivalent in
  property-binding UI toolkits — an imperative write silently discarding
  a binding — is a documented footgun).

Dashboard-scope `param` / `selection` / `store` declarations use the
chart constructs unchanged; their scope sits one level above a chart's
`shared`. Chart-internal state that is not bound at the instantiation
site stays panel-scoped (instance-namespaced), so two instances of one
chart never cross-link accidentally — linking is always explicit.

### Data

Charts consume the ambient catalog as always. A dashboard body may
declare `table sql` views — the catalog construct scoped to the document,
with `materialize: session` for compute-once-feed-many-panels — and may
import dataset packs. Widget item relations, slider extents, and KPI text
are ordinary SQL (scalar subqueries are ordinary expressions); their
reactive execution is a runtime concern, not new syntax.

### Layout Declarations

Syntax only; the model — document flow vs `fill`, the closed-form
invariant, pinning behavior — is normative in `dashboard-layout.md`:

- Document policies: `width: fill { max: <px>; } | px(<n>);`,
  `height: flow | fill [{ min_height: px(<n>); }];` (flow default),
  `spacing:`.
- Shell slots (closed set, chrome outside the document scroll): `header`,
  `footer` (`document` | `pinned`), `sidebar left` / `sidebar right` —
  each hosting the same row grammar as content.
- Content: ordered `row { ... }` declarations; per-row
  `height: px(<n>) | aspect(<w>, <h>) | content;` and
  `widths: [ ... ]` with the concat track tokens (`px(n)`, `fr(n)`,
  content-sized); a track cell may hold a nested `column { row ... }`.
  There are deliberately **no scroll declarations** (one document scroll
  is the model, not a construct) and **no flexbox vocabulary** (no
  grow/shrink/basis/justify — the absence is the anti-content-negotiation
  law).
- Panel leaves: `chart` instantiations, `widget` instantiations, and
  `text` panels (`syntax: plain | typst`).

### Widgets, Tabs, Modals, Callbacks

- Widgets instantiate like tools (`widget <kind> as <name> { ... }`),
  imported from `std:widgets/`; wiring properties name dashboard state
  (`param: region;`), item properties are SQL. The paradigm — composed vs
  native tiers, data encoding, sizing — is `widgets.md`.
- `tabs { tab as <id> { <rows> } ... }` declares an implicit active-tab
  param and renders driver chrome; a `modal` is
  `visible:`-driven-by-param structure. Both are sugar over params —
  layout stays closed-form given params, hidden content evaluates lazily
  with state retained.
- `callback <name>(<args>);` declares a host-implemented action — the
  imperative escape hatch that keeps the language query-only. Invocation
  form from event actions is an open question.
- `theme css` in a dashboard cascades to charts that do not declare their
  own; precedence across the boundary is an open question.

### What Dashboards Deliberately Cannot Do

- **No loops.** Item multiplicity is data: data-encoded widgets
  (`widgets.md`) at the item level, and data-encoded panel lists (the
  `mark subplot` precedent) as the recorded future for panel-level
  multiplicity.
- **No imperative logic.** Side effects are declared `callback`s handled
  by the host; the dashboard file remains a pure function of (files,
  catalog, params).
- **Not importable by charts or definitions** (whether dashboards can
  import dashboards — nesting — is open).
- **No per-panel scrolling, no flexbox words** — see Layout Declarations.

### AST Notes

`Root` gains a `Dashboard(Decl)` variant; every construct above is an
ordinary `Decl` (`dashboard`, `sidebar`, `header`, `footer`, `row`,
`column`, `widget`, `text`, `tabs`/`tab`, `modal`, `callback`, and
`chart`-as-instantiation), so the six-node generic AST absorbs the file
kind with no new node types — validity lives in the authoring schema, as
everywhere else.

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
param     $ident                 chart param placeholder
punct     { } [ ] ( ) : ; , = .
```

Comments (`--` followed by whitespace; nested `/* ... */`) and whitespace are
trivia. `sql_expr` and `sql_query` are islands parsed by the DataFusion
expression/statement parser from the same token stream; the resolver then
rewrites `$param` placeholders, bare qualified names, and reserved helper
functions.

```ebnf
file          = version , { import } , ( chart | define | data_file ) ;
data_file     = ( source | table_bind ) , { source | table_bind } ;
                     (* .data.avenger: catalog config. Data files import
                        only data files; chart files may import data files
                        (portable mode); definition files may not. *)
source        = "source" , ident , bind , body ;
                     (* source iceberg as wh; table children are
                        schema-valid on source tables only *)
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
slot          = "slot" , ident , [ "exposes" , "[" , ident ,
                { "," , ident } , "]" ] , [ ":" , value ] , ";" ;
channel_param = "channel" , ident , [ ":" , ident ] , ";" ;
output        = "output" , ident , [ ":" , sql_expr ] , ";" ;
export        = "export" , qual , [ "as" , ident ] , ";" ;
match_block   = "match" , ident , "{" , { match_arm } , "}" ;
                     (* any depth within a define; compile-time, closed;
                        arms may be empty and may contribute any items
                        valid at the match's position *)
match_arm     = ident , "{" , { item } , "}" ;
splice        = ident , ";" ;
                     (* define bodies only: splice point of a block slot *)

resource      = param | store | selection | res | theme ;
                     (* data is a property: `data: { ... }` *)
param         = "param" , bind , body ;
store         = "store" , bind , body ;
selection     = "selection" , [ kind ] , bind , body ;
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
child         = param | table_bind | resource | group | mark
              | transform | tool | view | view_use | event | cell | plot
              | variable | part | level | adjust | derive | overlay
              | layer | when | field | row | action | scale_edit
              | match_block | splice ;

group         = "group" , [ bind ] , body ;
mark          = "mark" , kind , [ bind ] , body ;
transform     = "transform" , kind , [ bind ] , body ;
tool          = "tool" , kind , [ bind ] , ( body | ";" ) ;
view          = "view" , kind , [ bind ] , body ;
view_use      = "view" , ident , ";" ;
event         = "on" , ident , [ bind ] , body ;
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
field         = "field" , ident , ":" , ident , [ "nullable" ] , ";" ;
row           = "row" , body ;
scale_edit    = "scale_edit" , body ;      (* tool defines: edit a chart scale *)
action        = "set" , ( "param" | "store" | "selection" ) , ident , "=" ,
                ( sql_expr , ";" | ident , ( body | ";" ) ) ;

property      = ident , ":" , value ;
value         = body                                 (* anonymous object *)
              | ident , body                         (* typed object: linear { ... } *)
              | array , ";"
              | "value" , sql_expr , terminator      (* unscaled literal value *)
              | "dim" , qual , [ body ]              (* raster dimension handle *)
              | "pattern" , body
              | "store" , ident , ";"                (* store-backed mark data *)
              | "env" , string , ";"                 (* environment variable, capability-gated *)
              | "none" , ";"
              | sql_query , ";"                      (* the `sql:` property only *)
              | sql_expr , terminator ;              (* default expression slot *)
terminator    = body | ";" ;                         (* config block or semicolon *)
array         = "[" , [ elem , { "," , elem } , [ "," ] ] , "]" ;
elem          = sql_expr | "value" , sql_expr | "pattern" , body | "none" ;
qual          = ident , { "." , ident } ;

sql_expr      = ? one DataFusion SQL expression ? ;
sql_query     = ? one SELECT (including set operations) or VALUES statement ? ;
```

Grammar notes:

- Declaration headers are uniformly `<keyword> <kind> [as <name>]`; `cell`
  places its optional `at { ... }` after the binding.
- Which `value` production a property uses is selected by the property's
  schema — an expression slot never parses as a typed object, `sql_query` is
  reachable only from the `sql` property, and enum-valued properties accept
  bare identifiers as `sql_expr` atoms that the resolver checks against the
  enum. The grammar lists the union of forms.
- Keywords are contextual. `value`, `pattern`, `dim`, `store`, `env`, and
  `none` are recognized only in value-prefix position (immediately after
  `:`);
  `group`, `level`, `part`, and the other declaration keywords only in
  declaration-head position. A property may therefore be named `value`
  (conditional branch payloads are) without colliding with the `value`
  prefix.
- Ordered semantics: child declarations preserve source order (`transform`
  dataflow, `cell` placement, `when` branch priority, `level` index order,
  `set` action order); properties are unordered within their body.

## Coverage And Validation

The coverage unit is the deduped visual-baseline inventory:

```sh
find avenger-chart/tests/baselines avenger-chart/tests/baselines_svg \
  -type f \( -name '*.png' -o -name '*.svg' \) |
  sed 's#^avenger-chart/tests/##; s#^baselines_svg/##; s#^baselines/##; s#\.png$##; s#\.svg$##' |
  sort -u
```

That yields 891 logical scenarios across 96 baseline categories, and every
category has a syntax home in this document: most need only generated
property schemas over the core block shapes; the feature-surface sections
above cover the families that needed dedicated syntax; raster, view, tile,
and CSS-cardinality families additionally depend on runtime materialization
and resource semantics that the syntax merely names.

To prove coverage, a DSL fixture suite runs parallel to the visual tests:

1. For each baseline category, at least one canonical `.avenger` fixture that
   lowers to the same public API feature family.
2. For families with many variants, parameterized fixtures generated from the
   same schema tables the LSP uses.
3. Parse each fixture, lower to `CompiledPlot`, render through the existing
   visual helpers, and compare against the same baseline image.
4. Once canonical generation exists, a `CompiledPlot -> DSL` decompilation
   smoke test per Rust visual test, plus a parse → lower → decompile → parse
   round-trip property test over the corpus.
5. Parser tolerance and LSP tests stay separate from rendering tests so
   syntax diagnostics can evolve without re-blessing images.
6. An expansion equivalence property over the corpus: for every fixture,
   `compile(chart)` and `compile(expand(chart))` produce identical results,
   which pins `avenger expand` (and with it the entire definition system's
   lowering) to the compiler's own semantics.

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
| `avenger serve [chart\|project]` | Pop open a native chart window with hot reload: edit files in your own editor, the window recompiles on save. Project mode opens the gallery with a chart switcher. |
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
| `avenger expand <file> [-o out]` | Macro expansion; a definition file expands to a define over pure primitives. |
| `avenger info [path]` | The doc-query surface over the entire language schema — primitives and definitions alike, `--format json` throughout. Bare `info` lists namespaces (marks, transforms, tools, scales, helpers, events); drill by path: `info marks --coord geo`, `info mark rect`, `info mark rect.x` (one channel's option suffixes), `info transform bin`, `info std:marks/box_plot` or any file/URL (slots, parts, outputs, doc comments — inspect before importing). |
| `avenger schema [--format json\|json-schema]` | The entire machine-readable language schema in one dump — `json` for tooling and big-context agents that load the reference once, `json-schema` to compile it into the generated full validator for the AST interchange form (the frozen core schema ships with the spec). |
| `avenger doc [-o dir] [--open] [--single-page]` | Generate the project's documentation site from three sources it already has — see below. |
| `avenger deps <chart>` | The import closure as a tree with pin status and origins; the supply-chain review. |
| `avenger table <chart> --at <alias\|mark>` | Print the data context at a point in the pipeline; the transform-debugging tool. |
| `avenger tables [--format json]` | List the catalog's resolved table names and schemas (`table sql` views included) — the source for column completion and the first thing an agent should read. |
| `avenger ast <file>` | Convert between the two encodings: `.avenger` text to interchange JSON, or interchange JSON back to canonical text (byte-identical to `avenger fmt`). |
| `avenger test [--bless]` | Every chart is an example: compile, render, and fuzzy-perceptually compare each chart against its sibling `.png` baseline (exact matching is brittle across GPUs; threshold configurable). Failures emit actual and diff images; a missing baseline fails with a hint; `--bless` (re)generates baselines for all or changed charts. Also compiles doc-comment examples (visual doctests). |

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
the project gallery. `avenger serve` is the same machinery minus the
editing panes — the preview window with your own editor filling the editing
role. Both share two hot-reload semantics: reloads are import-graph-aware
(saving a definition file, theme, or data file reloads every chart whose
closure includes it), and session state survives recompiles — params,
stores, selections, and view domains carry over where names still match, so
a zoomed viewport stays put while a color is tweaked. Scope is deliberately
a playground, not an IDE — chart-sized files, no multi-cursor ambitions;
serious project work pairs a real editor with `avenger lsp` and
`avenger serve`.

`avenger doc` composes a static site from three sources, zero configuration:

1. **README.md** at the project root becomes the front-page prose
   (CommonMark; `--readme <path>` overrides). Links to project files rewrite
   to their documentation targets: a link to
   `marks/error_bar.mark.avenger` points at that definition's reference
   section, and a link to `charts/revenue_trend.avenger` becomes its gallery
   entry, image included.
2. **The chart gallery**, built from blessed baselines and each chart's
   `---` blurb — no rendering at doc time, so output is deterministic and
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
   targets.
3. Lower the AST into the Rust authoring model: `Plot`, `MarkGroup`, primitive
   marks, transform builders, channel values, scales, guides, params, stores,
   selections, tools, and event bindings.
4. Compile using the existing chart compiler.

For decompilation, a `CompiledPlot` should be able to produce a canonical,
fully elaborated DSL form if enough compiled metadata is retained. Exact
authoring-source recovery should instead preserve the original DSL AST or a
source map.

## AST And Interchange Form

Step 1 of the lowering model names "a stable DSL AST". Its shape is the
payoff of the grammar's uniformity — every declaration is
`keyword kind? as name? { props; children }` — so the tree needs six
generic node types, not a node type per language feature:

```rust
struct File {
    version: u32,                    // the `avenger 1;` pragma
    name: Option<Name>,              // filename-derived canonical name (loader-populated;
                                     // None for plural data catalogs)
    imports: Vec<Import>,            // source, sha256 pin, rename
    root: Root,                      // Chart(Decl) | Define(Decl) | Data(Vec<Decl>) | Dashboard(Decl)
}

struct Decl {
    keyword: Keyword,                // chart | mark | transform | table | param | on | ...
    kind: Option<Name>,              // symbol, sql, parquet, cartesian, ...
    name: Option<Name>,              // the `as` binder
    props: Vec<(Name, Value)>,       // semantically unordered; source-ordered for fmt
    children: Vec<Decl>,             // ordered (transform dataflow, cells, levels)
    doc: Option<String>,             // attached `---` doc comment
    origin: Origin,                  // Span(file, range) | Host(frame)
}

enum Value {
    Str(String), Num(f64), Bool(bool), Null,
    Column(String),                  // "Horsepower"
    Atom(Name),                      // lone bare identifier: retarget_cached, median
    Expr(SqlExpr),                   // expression island: parsed form + raw text
    Query(SqlQuery),                 // sql: statement island
    Param(Name),                     // $name
    Prefixed(Prefix, Box<Value>),    // value | dim | pattern | store | env | none
    Array(Vec<Value>),
    Block(Option<Name>, Body),       // typed or bare block; Body = props + children
    Helper(Name, Vec<Value>),        // channel(x), datum('id'), view_x(...)
}
```

Every feature in this document is an instance of `Decl` — `table sql` with
params, `source tables` mounts, `match` arms, effects, block-slot content.
**Validity lives in the authoring schema, not the node types**: the schema
checks keyword/kind/property combinations over a generic tree, which is
the inversion that keeps the AST closed while the language grows. A lone
bare identifier parses as `Atom`; whether it names an enum member, a slot,
or a function is the resolver's schema-directed decision (the bare-name
law). `Expr` retains both the parsed DataFusion form and the raw island
text.

`origin` is why one AST serves two front-ends: parsed nodes carry source
spans, nodes constructed by a host API (the Python bindings) carry the
host frame that created them, and diagnostics render either. The Python
specification's construction-site breadcrumbs are this field.

### The JSON Encoding

Serde over these nodes defines the interchange form. Three rules:

- JSON scalars encode literals directly: `"Manhattan"`, `12`, `true`.
- Every other value is a single-key tagged object: `{"col": "Horsepower"}`,
  `{"atom": "retarget_cached"}`, `{"param": "borough"}`,
  `{"env": "ICEBERG_TOKEN"}`, `{"expr": "..."}`, `{"query": "..."}`. The
  inventory is closed — thirteen tags: `col`, `atom`, `param`, `expr`,
  `query`, `value`, `dim`, `store`, `pattern`, `env`, `none`, `block`,
  `call` — pinned by the core schema below. Imports encode as
  `{"import": "<specifier>", "sha256": "...", "as": "..."}`.
- SQL islands serialize as canonical SQL text (the unparser's output),
  never as DataFusion ASTs — compact, readable, re-parsed on load under
  the shared-tokenizer contract, and independent of DataFusion's internal
  types.

```avenger
table sql as borough_trips {
  param as borough { default: 'Manhattan'; }

  sql: SELECT * FROM trips WHERE "borough" = $borough;
}
```

```json
{
  "decl": "table", "kind": "sql", "name": "borough_trips",
  "children": [
    { "decl": "param", "name": "borough", "props": { "default": "Manhattan" } }
  ],
  "props": {
    "sql": { "query": "SELECT * FROM trips WHERE \"borough\" = $borough" }
  }
}
```

Interchange JSON carries content and doc comments; provenance and trivia
stay out (exact-source recovery preserves the original text or a source
map, as the lowering model notes). The `version` field carries the pragma,
so the JSON is exactly as versioned as the text — the same major-version
gate applies to both.

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
declared params), SQL island validity (islands are opaque strings here),
DAG acyclicity, sibling-name uniqueness, import closures. `avenger check`
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
        "kind": { "$ref": "#/$defs/name" },
        "props": { "$ref": "#/$defs/props" },
        "children": { "type": "array", "items": { "$ref": "#/$defs/decl" } }
      },
      "additionalProperties": false
    },
    "value": {
      "oneOf": [
        { "type": ["string", "number", "boolean", "null"] },
        { "type": "array", "items": { "$ref": "#/$defs/value" } },
        { "$ref": "#/$defs/tagged" }
      ]
    },
    "tagged": {
      "type": "object",
      "minProperties": 1,
      "maxProperties": 1,
      "properties": {
        "col": { "type": "string", "minLength": 1 },
        "atom": { "$ref": "#/$defs/name" },
        "param": { "$ref": "#/$defs/name" },
        "expr": { "type": "string", "minLength": 1 },
        "query": { "type": "string", "minLength": 1 },
        "value": { "$ref": "#/$defs/value" },
        "dim": {
          "type": "string",
          "pattern": "^[A-Za-z_][A-Za-z0-9_]*\\.[A-Za-z_][A-Za-z0-9_]*$"
        },
        "store": { "$ref": "#/$defs/name" },
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
    }
  }
}
```

The instance corpus for this schema — well-formed chart, data, and define
files on the accept side; two-key tagged objects, unknown tags, provenance
fields in interchange, malformed hashes on the reject side — belongs to
the same conformance corpus that pins the formatter.

### Round-Trip Laws

- **Printing is total.** Every constructible AST prints: constructed
  expression trees print through the DataFusion unparser, parsed islands
  through their retained text. Nothing a host API can build lacks a text
  spelling.
- **Printing is canonical.** The printer *is* `avenger fmt`: equal trees
  produce byte-identical files, so generated artifacts diff cleanly
  against hand-written ones. The formatter is thereby load-bearing, not
  cosmetic — its output is pinned by the conformance corpus.
- **`parse(print(ast)) == ast`**, modulo trivia. Text, host-language
  nodes, and JSON are three encodings of one tree with one lowering;
  surface drift is unrepresentable.

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

The workspace already depends on DataFusion 54, whose SQL parser stack includes
`sqlparser` 0.62. The useful APIs are:

- `Tokenizer::new(&dialect, source).tokenize_with_location()` to produce
  `Vec<TokenWithSpan>`.
- `DFParserBuilder::new(tokens).build()` to parse from an existing token
  buffer instead of a raw SQL string.
- `DFParser::parse_expr()` for expression slots and
  `DFParser::parse_statement()` for full-statement `sql:` slots.
- `DFParser.parser.index()` to recover the index of the first unconsumed token
  after a SQL parse.

The outer parser can then keep a cursor into the shared token buffer:

```text
tokens = sqlparser_tokenize(source)
parse_declaration_block(tokens, cursor)
```

This means every DSL surface feature must first be valid input to the
DataFusion SQL tokenizer, using either DataFusion's default SQL dialect or an
Avenger dialect built on the same tokenizer contract. The Avenger parser can
reinterpret token sequences such as `chart cartesian as SalesByCategory { ... }`
as DSL declarations, but it should not require a second lexer or syntax that
`sqlparser-rs` cannot tokenize. New punctuation, string forms, comments, and
sigils should be accepted only after checking how they tokenize as DataFusion
SQL.

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

Property schemas decide which parser is used for a value. Channel values,
filter predicates, calculate outputs, aggregate expressions, sort keys, and
visibility conditions are SQL expression slots. Selectors such as
`target: mark manual_box_plot.outliers;`, typed values such as
`scale: linear { ... }`, arrays of DSL names, and nested property objects are
DSL values.

Params fit this model well. `sqlparser-rs` tokenizes `$foo` as a placeholder,
and DataFusion lowers named placeholders into `Expr::Placeholder`. The DSL
resolver should validate that placeholders are named params and reject
positional placeholders such as `$1`, `$2`, and `?`.

Every other DSL-injected reference — `channel(x)`, `datum('id')`,
`event_coord(x)`, `item_channel(x)`, and the rest of the reserved helper
namespace — parses as an ordinary SQL function call and is rewritten by the
resolver before DataFusion planning. No token-level extensions are required
beyond what the SQL tokenizer already produces, which is also why the
mandatory column-quoting rule costs nothing at this layer: quoted identifiers
and bare compound identifiers are both native SQL, and the resolver simply
assigns them to the data and DSL namespaces respectively.

Because the surface language is defined over a third-party tokenizer, the
token classes the DSL relies on (words, quoted identifiers, strings, numbers,
placeholders, punctuation, comments) must be specified normatively and pinned
by a conformance corpus of golden token streams. DataFusion and `sqlparser`
upgrades run against that corpus, so an engine upgrade can never silently
change the language.

`TokenWithSpan` carries line and column spans, which is enough for diagnostics.
If exact source text reconstruction or byte-range source maps become important,
the DSL parser should maintain a separate line/column-to-byte-offset table
alongside the token buffer.

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

zed-avenger/
  extension.toml
  languages/
    avenger/
      config.toml
      highlights.scm
      injections.scm
      brackets.scm
      indents.scm
    avenger-sql/
      highlights.scm
```

The SQL grammar can start as an adapted Tree-sitter SQL grammar rather than a
from-scratch grammar. The adaptation should match the SQL accepted by
DataFusion closely enough for highlighting, while also accepting Avenger's SQL
island shapes:

- Full SQL statements after `sql:`.
- SQL expression fragments in channel values, filters, transform outputs,
  sort keys, visibility conditions, and event filters.
- Named params such as `$min_amount`.
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
  $.param_ref,
))
```

The adapted SQL grammar is for editor highlighting only. The compiler remains
the source of truth and still parses SQL islands through `sqlparser-rs` and
DataFusion.

The first grammar can parse declarations, property blocks, SQL expression
islands, full-statement `sql:` properties, comments, strings, params, and
channel references. A sketch:

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
    source_file: $ => repeat($.declaration),

    declaration: $ => choice(
      $.chart_declaration,
      $.group_declaration,
      $.mark_declaration,
      $.transform_declaration,
      $.param_declaration,
      $.selection_declaration,
      $.tool_declaration,
      $.view_declaration,
      $.view_use_declaration,
      $.event_declaration,
      $.facet_declaration,
      $.cell_declaration,
      $.config_declaration,
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
      $.declaration_block,
    ),

    mark_declaration: $ => seq(
      "mark",
      field("kind", $.identifier),
      optional($.as_clause),
      $.property_block,
    ),

    transform_declaration: $ => seq(
      "transform",
      field("kind", $.identifier),
      optional($.as_clause),
      $.property_block,
    ),

    param_declaration: $ => seq("param", $.as_clause, $.property_block),

    selection_declaration: $ => seq(
      "selection",
      field("kind", $.identifier),
      optional($.as_clause),
      $.property_block,
    ),

    tool_declaration: $ => seq(
      "tool",
      field("kind", $.identifier),
      optional($.as_clause),
      $.property_block,
    ),

    view_declaration: $ => seq(
      "view",
      field("kind", $.identifier),
      optional($.as_clause),
      $.property_block,
    ),

    view_use_declaration: $ => seq("view", field("name", $.identifier), ";"),

    event_declaration: $ => seq(
      "on",
      field("event", $.identifier),
      optional($.as_clause),
      $.property_block,
    ),

    facet_declaration: $ => seq("facet", $.property_block),

    cell_declaration: $ => seq(
      "cell",
      field("coordinate", $.identifier),
      $.declaration_block,
    ),

    config_declaration: $ => seq(
      field("name", $.identifier),
      $.property_block,
    ),

    as_clause: $ => seq("as", field("name", $.identifier)),

    declaration_block: $ => seq("{", repeat($.declaration), "}"),

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
      $.param_ref,
      $.identifier,
    ),

    param_ref: $ => token(/\$[A-Za-z_][A-Za-z0-9_]*/),
    identifier: $ => /[A-Za-z_][A-Za-z0-9_]*/,
    number: $ => /-?([0-9]+(\.[0-9]*)?|\.[0-9]+)/,
    string: $ => /'([^'\\]|\\.)*'/,
    column: $ => /"([^"\\]|\\.)*"/,
    boolean: $ => choice("true", "false"),
    comment: $ => choice($.line_comment, $.block_comment),
    line_comment: $ => token(seq("--", /[ \t][^\n\r]*/)),
  },
});
```

The sketch uses external tokens for `sql_expression` and `sql_statement`
because those boundaries are the hard part. The external scanner should mirror
the compiler rule: consume one DataFusion SQL expression or statement and stop
before the DSL semicolon or channel/config block. If this is too much for the
first pass, the external scanner can start as a tolerant delimiter-aware scanner
that balances parentheses and brackets, then improve toward the compiler's
`sqlparser-rs` behavior.

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
  "data"
  "param"
  "selection"
  "tool"
  "view"
  "on"
  "facet"
  "cell"
  "as"
] @keyword

(comment) @comment
(string) @string
(number) @number
(boolean) @boolean

(param_ref) @variable.parameter

(property name: (identifier) @property)
(as_clause name: (identifier) @label)

(chart_declaration coordinate: (identifier) @type)
(cell_declaration coordinate: (identifier) @type)
(mark_declaration kind: (identifier) @type)
(transform_declaration kind: (identifier) @function)
(selection_declaration kind: (identifier) @type)
(tool_declaration kind: (identifier) @type)
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
operators, functions, identifiers, strings, numbers, and comments, plus Avenger
captures for the two special reference forms:

```scheme
(param_ref) @variable.parameter
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

The first extension should associate a file extension such as `.avenger` or
`.avc` with the `avenger` language, and should configure comment toggling for
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

- `avenger` for `.avenger` or `.avc` files.
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
  `param`, `selection`, `tool`, `view`, `on`, `facet`, `cell`, `as`.
- Highlight declaration kinds: coordinate kinds, mark kinds, transform kinds,
  selection/tool/view kinds.
- Highlight property names before `:`.
- Highlight comments, strings, numbers, booleans, braces, brackets,
  semicolons, and commas.
- Highlight `$param` references wherever they appear.
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
          "match": "\\b(chart|group|mark|transform|data|param|selection|tool|view|on|facet|cell|as)\\b"
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
- `$param` as a parameter token.
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

Editor intelligence wires to `avenger-lang.wasm` through CodeMirror's native
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
parameter      $params
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
  avenger-lang.wasm providers (lint/complete/hover/semantic decorations)

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
  avenger-lang.wasm
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

The crate split should support both native and browser deployments:

```text
avenger-lang
  Pure Rust library.
  Native + wasm target.
  No direct filesystem, stdio, socket, or process assumptions.
  Exposes parse/analyze/complete/hover/symbols/semantic-token APIs.

avenger-lsp-native
  Desktop/server adapter.
  stdio or socket LSP transport.
  Uses avenger-lang.

avenger-lsp-wasm
  Browser adapter.
  Web Worker message transport.
  Calls avenger-lang.wasm.
  Can expose real LSP JSON-RPC or a smaller editor-specific RPC.
```

The editor integration has two viable levels:

- Full LSP-style integration using a browser language client and
  worker-hosted server. This maximizes reuse with VS Code/Zed-style LSP
  clients.
- Direct CodeMirror sources backed by `avenger-lang.wasm`: lint,
  autocomplete, hover tooltip, and semantic-token decorations registered as
  ordinary extensions. This is simpler for the first online editor if
  multi-editor LSP reuse is not yet needed.

The full editor is a **two-bundle architecture** with opposite constraints,
and the repository's existing wasm-pack browser examples have already
de-risked the heavy half (the complete chart stack — DataFusion, typst
labels, wgpu rendering — runs under Wasm today):

```text
avenger-lang.wasm        small, instant-loading (worker #1)
  tokenizer + tolerant parser + resolver + generated schema
  embedded std: definition sources (versioned text)
  diagnostics, completions, hover, semantic tokens, formatter
  source-level expansion/refactorings, sha256 verification
  -- no DataFusion execution, no GPU

avenger-runtime.wasm     heavy, lazy-loaded (worker #2 + OffscreenCanvas)
  chart compile (lowering -> CompiledPlot) + DataFusion execution
  scales, layout, scenegraph, PlotSession (events, tools, views,
  preview evaluation and retarget_cached previews)
  avenger-wgpu (WebGPU/WebGL2), cosmic-text, typst labels
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

Schema metadata should cross the worker boundary as plain data:

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

A shared crate layout could look like:

```text
avenger-lang/
  lexer.rs        # sqlparser-rs tokenization and source maps
  parser.rs       # Avenger token cursor parser
  ast.rs          # loss-aware DSL AST
  sql.rs          # SQL island parsing via DataFusion/sqlparser-rs
  resolver.rs     # names, scopes, params, aliases, event paths
  diagnostics.rs
  completion.rs
  hover.rs
  symbols.rs
  lowering.rs     # strict AST -> avenger-chart authoring model

avenger-lsp-native/
  main.rs         # stdio/socket LSP transport

avenger-lsp-wasm/
  lib.rs          # Web Worker/LSP-ish transport and wasm bindings
```

The parser should support two modes:

- Strict mode for compiling, tests, and save/build validation.
- Tolerant mode for live LSP analysis while the user is mid-edit.

Tolerant mode is not a different language. It is the same token stream and AST
shape with recovery nodes, synthesized missing delimiters, and diagnostics. The
goal is to keep enough structure to power editor features even when the file is
temporarily incomplete.

Tolerant mode can still start with `sqlparser-rs` tokenization. The language
server should try normal tokenization first. If tokenization fails because of an
unterminated string, unterminated block comment, or other lexical error, it can
keep the tokens emitted before the error, attach a lexical diagnostic, and
resume from a recovery point if one can be found.

The Avenger parser should recover at DSL sync points:

- In declaration blocks, sync at declaration starters such as `chart`, `group`,
  `mark`, `transform`, `param`, `selection`, `tool`, `view`, `on`,
  `facet`, `cell`, or at `}`.
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

For this example, the LSP should keep the surrounding mark and property, store
an invalid SQL expression island for `amount +`, and report the SQL diagnostic
on the expression span. It should not discard the whole mark block.

SQL island recovery boundaries depend on the slot:

- Expression property: stop at a top-level `;`, top-level `{`, `}`, EOF, or a
  plausible next `identifier:`.
- Channel value: stop at a top-level `{` so `x: amount { ... }` still separates
  the SQL value from channel configuration.
- Full-statement `sql:` property: stop at the statement semicolon, `}`, or EOF.

The analysis pipeline should be:

1. Tokenize with `sqlparser-rs`, preserving spans.
2. Parse the DSL token stream in tolerant or strict mode.
3. Parse SQL expression and statement islands with DataFusion/sqlparser-rs where
   possible.
4. Resolve names, scopes, params, transform aliases, event targets, views,
   tools, selections, stores, and channel references.
5. Lower only in strict mode, or in tolerant mode only when the AST has no
   blocking errors.

The LSP can offer:

- Diagnostics for unknown declaration kinds, mark kinds, transform kinds,
  properties, channels, params, transform aliases, event targets, duplicate
  names, invalid block modes, group transform ordering violations, SQL parse
  errors, and invalid placeholders such as `$1` or `?`.
- Completion for declaration keywords, mark kinds, transform kinds, property
  names, channel names, scale and guide options, `$param` references,
  reserved helper functions, transform aliases, event targets, and data columns when
  schema is known.
- Hover for marks, transforms, properties, params, data columns, SQL expression
  types, aliases, selections, tools, and event paths — with doc comments as
  the content for definitions (slots, parts, outputs, `match` arms; mode-value
  completion shows the arm's doc) and schema docs for primitives at the same
  granularity, down to channel option suffixes and enum values.
- Go to definition and references for `as` bindings, `$params`, transform alias
  fields such as `stats.median`, views, tools, selections, and mark paths such
  as `manual_box_plot.outliers`.
- Document symbols for charts, groups, marks, transforms, params,
  views, tools, selections, and events.
- Semantic tokens for precise highlighting beyond Tree-sitter, especially for
  resolved params, aliases, generated fields, and unknown or deprecated names.
- Formatting for the DSL block/property structure, with SQL formatting either
  deferred or delegated to a SQL formatter later.
- Code actions such as add missing param declaration, add missing `as` binding,
  qualify an ambiguous alias field, or convert an unknown property into a
  suggested known property.
- Flattening as refactorings, at finer granularity than the CLI command:
  **expand instantiation** (replace one `mark box_plot as ...` with its
  expansion — slots substituted, `match` resolved, channels renamed — the
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

The language server needs a source of truth for valid mark properties,
channels, transform properties, transform output handles, enum values, required
properties, and docs.

Documentation is a schema requirement, not an afterthought, at every
granularity: the entity (mark kind, transform kind, scale type, coordinate,
reserved helper, event type) and each of its members — a channel per
(coordinate, mark) pair, each channel option suffix, each mark base
property, each transform property and output field, each scale option, and
**each enum value** (`empty_cells: holes` hover-explains what `holes`
does). For definitions, docs come from `---` doc comments; for built-in
primitives, from Rust `///` doc comments captured by the schema-emitting
macros, with doc attributes in the macro invocations where Rust structure
does not map one-to-one (channel suffixes, enum members). Both use the same
CommonMark format with the same summary/detail split, so hover,
`avenger info`, and generated documentation render primitives and
definitions identically. A completeness lint runs in CI: a public schema
node without a non-empty doc fails the build — `missing_docs`, extended to
the language surface. The language's own reference site is generated from
the same schema, so hover text, CLI output, and published reference can
never disagree. This schema should describe the authoring DSL, not the
compiled serialization shape. Compiled structs are useful for execution, but
they often contain generated implementation fields or omit authoring concepts
such as repeated aggregate measures, channel configuration blocks, and transform
output handles.

The preferred long-term approach is a small authoring-schema model shared by
the chart crates and `avenger-lang`:

```text
avenger-chart-schema
  MarkSchema
  TransformSchema
  PropertySchema
  ChannelSchema
  TransformOutputSchema
  ValueShape
```

Minimum schema contents:

- Declaration kinds and their allowed body mode: ordered declarations,
  unordered properties, raw payload, or mixed body.
- Mark kinds, coordinate compatibility, supported channels, channel defaults,
  and extra mark-level properties.
- Transform kinds, properties, default values, required values, output
  handles, and materialization behavior.
- Scale, axis, legend, guide, theme, pattern, layout, selection, store,
  event, tool, resource, and view property schemas.
- Enum values and aliases used by the human-facing DSL.
- CommonMark docs on every node above, enum values included, enforced by a
  completeness lint.

The ordinary `avenger-chart` rendering path should not depend on the LSP. If a
separate schema crate is introduced, it should be tiny, dependency-light, and
Wasm-friendly. Existing chart crates can depend on it behind a feature such as
`language-schema`; normal rendering builds can leave that feature disabled.

Macros can reduce duplication by emitting schema metadata next to the API they
already generate. For example, `define_common_mark_channels!` and
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
`Aggregate` should advertise repeated grouping and measure forms rather than
only the compiled `group_by` and `measures` fields. This transform schema can
start as hand-authored declarations near each transform implementation, then be
macro-assisted once the shape settles.

A manual schema registry is also a viable first implementation:

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

The manual approach is simpler to bootstrap and keeps the LSP bundle small, but
it introduces drift risk. A validation test should compare the manual schema
against the public chart APIs that already expose partial metadata:

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
- For docs/examples, generate a schema snapshot used by the LSP and fail CI on
  unexpected changes.

This gives two migration paths. The first release can use manual schema tables
with validation tests. Over time, channel and transform macros can absorb the
schema declarations so the builder API, compiler lowering, docs, and LSP all
read from the same authoring metadata.

Schema-aware analysis should be incremental. The first LSP can validate syntax
and names without executing data. Later versions can accept schema information
from configured data sources, examples, an application session, or a running
DataFusion context so completions and type diagnostics know real column names.

## Open Questions

- Should resource declarations such as params and data get compact shorthands
  in v1, or remain object declarations until the grammar settles?
- Should transform aliases be mandatory for all transforms, or only for
  transforms with conventional output handles?
- Should anything beyond `define` declarations become importable (named data
  declarations)? *(Partially answered 2026-07-09: whole charts are
  importable into `dashboard` files — see [Dashboards](#dashboards) — and
  remain non-importable everywhere else.)*
- Dashboards (draft section): EBNF productions, interchange-schema
  entries, and fixture coverage; the `callback` invocation form from
  event actions; theme precedence across the dashboard/chart boundary;
  dashboard-in-dashboard nesting. (The architecture-side questions live
  in `dashboard-layer.md`.)
- Private nested defines — the designated pressure valve if one-item
  file sprawl ever hurts: a helper `define` visible only to the file's
  single export (one *public* item per file stays the law; imports,
  naming, expansion, and the gallery untouched). Currently forbidden by
  the no-nested-definitions hygiene rule; open until real usage shows
  the need.
- Shadowing between chart-local relation names and ambient catalog names
  in full `sql:` statements. With `data:` anonymous (adopted 2026-07-09)
  the chart side of this question mostly dissolves; what remains is
  store names vs catalog names (and the reserved `input`, which already
  shadows by rule). Precedents point at shadow-with-editor-warning, but
  the rule is unpinned.
- How do the host-language APIs share the standard library as a single
  source of truth — do Rust/Python `box_plot` builders lower through the
  stdlib definitions (parsed at build time), or remain parallel natives held
  to parity by fixture tests?
- Should a future shorthand auto-import the standard library (a prelude), or
  do explicit per-definition imports (`import 'std:marks/box_plot';`) stay mandatory?
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
- Which built-in transforms migrate to `std:transforms` (stack is the first
  candidate) versus staying primitive? Working criterion: pipelines over
  `transform sql` migrate; algorithmic kernels (kde, bin nice-ing,
  rasterize_2d) stay Rust.
- Should `transform sql` stages gain an `except:` projection option so
  definitions can drop their `__`-prefixed intermediate columns?
- Should a block slot ever allow multiple splice points (the same caller
  content stamped at several positions), or does single-splice stay the
  rule?
- Should `part` overrides at instantiation sites be allowed to attach mark
  effects (`adjust`, `derive`) to a definition's internal marks, or only to
  restyle properties?
- Should reserved helper functions gain dotted sugar (`event.coord.x` for
  `event_coord(x)`) once the parser can keep it unambiguous?
- Should decompilation emit native compound marks (`mark box_plot`) whenever
  compiled metadata allows, or always the provable low-level group form?
- Should the Tree-sitter grammar use a real external scanner for SQL expression
  boundaries in v1, or start with a tolerant delimiter scanner for editor
  highlighting only?
- How much schema information should the first language server require for data
  column completion and SQL expression type diagnostics?
- How much of this grammar should be extensible by external mark, transform,
  coordinate, and tool crates?
