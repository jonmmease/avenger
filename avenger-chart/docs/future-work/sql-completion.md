# SQL Completion

## Status

Active design note, researched 2026-07-13 and revised 2026-08-02 while the
completion implementation was underway. This document defines the completion
architecture and the boundary between the Avenger frontend, DataFusion-backed
project analysis, and language server. The phase-by-phase authority is
[`ide-quality-completion-implementation-plan.md`](../../../scratch/avenger-lang/ide-quality-completion-implementation-plan.md).

The normative language surface remains
[chart-dsl.md](chart-dsl.md). The language/compiler implementation plan is
[`dsl-language-compiler-implementation-plan.md`](../../../scratch/avenger-lang/dsl-language-compiler-implementation-plan.md);
it exposes the exact project schemas and lineage consumed here.

### Canonical completion contract

- Data/output columns are authored only as double-quoted SQL identifiers in
  every SQL island. Column candidates appear only after an opening `"`, and a
  quoted cursor contains only scoped columns or legal relation-path members.
  Strict rejection of manually authored bare columns is tracked separately;
  completion already assumes the normative rule.
- Scalar params enter SQL completion only through `$`; table-valued stores do
  the same in relation roles. No binding is offered for a bare word or empty
  expression cursor. Selection params are structural state, not SQL values.
- Relation aliases and relation paths remain bare where SQL requires them.
  Projection alias binders after `AS`, contextual properties, and Arrow struct
  members are not data-column references and retain their specified forms.
- Candidate providers run only for proven legal intents. Empty completion is
  preferred to a global recovery dump, and name-invention positions do not
  receive semantic name suggestions.

## Decision Summary

The editor-neutral completion engine lives in `avenger-lang-analysis`. Its
architecture is:

1. use the tolerant Avenger syntax tree to locate the SQL island and its
   dataset/pipeline context;
2. tokenize the complete SQL island, including text after the cursor;
3. apply a small, deterministic set of cursor-marker and local repair
   strategies to incomplete SQL;
4. derive the expected semantic role at the cursor;
5. build SQL query scopes for aliases, CTEs, subqueries, derived relations,
   and correlated parents;
6. obtain base and derived relation schemas from the compiler's immutable
   `ProjectAnalysis` and DataFusion logical planning;
7. merge SQL names with Avenger params, stores, datasets, helper functions,
   and temporal qualifiers;
8. rank and return editor-neutral completion items that `avenger-lsp`
   translates to protocol types without adding candidates or legality filters.

The pinned `AvengerSqlDialect` plus sqlparser frontend remains the authority for
accepted SQL source syntax; DataFusion remains the authority for logical schema
propagation, Arrow types, nullability, functions, and planning semantics. Neither
should be treated as the incomplete-input autocomplete engine. Avenger needs a
thin completion-specific recovery and scope layer in front of them.

Do not define a second complete SQL grammar for completion. Tree-sitter remains
the editor-highlighting parser, sqlparser under `AvengerSqlDialect` remains the
compiler frontend parser, and DataFusion enters at the planning boundary.
Completion should add bounded recovery around that frontend parser, not create
a third dialect implementation.

## Motivation

Avenger authors write SQL in two related forms:

- complete query islands that define datasets and SQL transforms;
- expression islands used by encodings, filters, transforms, handlers, and
  other schema-bearing slots.

Useful completion therefore requires more than keyword suggestions. At:

```sql
FROM vega.movies AS m
SELECT m."
```

the engine must know that `m` names `vega.movies`, obtain that table's Arrow
schema, and offer its columns. At:

```text
x: encoded "revenue" - coalesce($adjustment, 0)
```

it must know the schema at that exact pipeline stage, including columns
created or removed by preceding transforms. CTEs, subqueries, joins, SQL view
chains, stores, struct-valued columns, and custom catalog providers all create
the same requirement: completion needs query scope plus real schema
propagation.

The source is also commonly invalid at the moment completion is requested.
The engine must work after a trailing dot, comma, operator, partial keyword,
missing expression, or unmatched delimiter. Running the strict compiler on the
file is therefore necessary for stable project metadata but insufficient for
the active edit.

## Goals

- Complete SQL in both conventional `SELECT ... FROM ...` and DuckDB-style
  `FROM ... SELECT ...` order.
- Complete from the entire SQL island rather than only the prefix before the
  cursor, so relations written after the cursor are also visible.
- Offer catalogs, schemas, tables, datasets, stores, relations, CTEs, aliases,
  columns, Arrow struct fields, functions, types, keywords, and operators in
  the contexts where each is valid.
- Propagate schemas through chains of SQL views, transforms, aliases, CTEs,
  subqueries, joins, projections, aggregates, and set operations by using
  DataFusion logical analysis.
- Use the same catalog/schema/table hierarchy, dialect, function registry,
  Arrow types, and name-resolution rules as compilation.
- Complete Avenger scalar params and table-valued stores from their shared
  `$`-prefixed value-binding namespace without confusing their SQL roles.
- Preserve exact source and replacement ranges even when completion analyzes a
  repaired copy of an SQL island.
- Return useful syntactic completion when semantic analysis is unavailable,
  while clearly declining to invent columns or types.
- Remain responsive under continuous edits, catalog latency, and superseded
  requests.
- Be reusable by a native LSP and by other editor or inspector surfaces without
  depending on LSP protocol types.

## Non-goals

- A standalone, all-dialect SQL language server.
- A new SQL parser that competes with sqlparser-rs/DataFusion.
- Executing or collecting a query merely to discover completion candidates.
- Scanning remote table data for value completion.
- An SQL formatter, linter, rename engine, or semantic-token implementation.
- Natural-language or model-generated query completion.
- Requiring an active `avenger watch` session. Inspector data may enrich a
  future LSP, but static project completion must work independently.
- Making the Tree-sitter highlighting grammar the semantic source of truth.

## Completion Surface

### Query contexts

The engine should distinguish at least these expected roles:

| Role | Representative candidates |
| --- | --- |
| Statement or clause | `SELECT`, `FROM`, `WHERE`, `GROUP BY`, `ORDER BY`, `LIMIT`, joins, set operators |
| Catalog | configured DataFusion catalogs |
| Schema | schemas visible beneath the selected/default catalog |
| Relation | catalog tables, chart datasets, SQL views, CTEs, subqueries, and in-scope stores |
| Relation alias | aliases already introduced in the current query block |
| Projection/expression | double-quoted visible columns, `$` scalar params, scalar functions, operators, and literals |
| Aggregate expression | group keys, aggregate functions, scalar params, and legal scalar expressions |
| Function | scalar, aggregate, window, and table functions appropriate to the syntactic position |
| Function argument | columns and expressions, with signature detail when available |
| Type | DataFusion-supported SQL/Arrow types |
| Qualifier member | columns of a relation alias or fields of a struct-typed value |
| Temporal binding qualifier | `start` and `previous` after a legal `$binding@` in handler context |

The candidate kind alone is not enough. Every item should retain:

- display label and insertion text;
- exact source replacement range;
- semantic kind;
- detail such as qualification, Arrow type, nullability, and function
  signature;
- documentation when available;
- origin: lexical scope, project dataset, store, catalog provider, function
  registry, Avenger helper, or keyword;
- stable identity where the compiler exposes one;
- optional snippet placeholders;
- sort score and filter text;
- whether the result is incomplete because metadata is still loading.

### Avenger value bindings

Params and stores share one lexical namespace and both use a `$` prefix, but
they occupy different SQL roles:

- a param is a scalar expression and has one required physical Arrow type;
- a store is a table value and is offered in relation positions;
- a struct-typed param or struct-typed column exposes its fields after member
  access;
- `@start` and `@previous` are offered only for bindings and contexts where
  the DSL permits those temporal versions.

Completion must use the resolved binding kind instead of inferring it from the
spelling. A name collision is already a resolver error and should not produce
two ambiguous completion items.

### Catalog and schema hierarchy

Use DataFusion terminology consistently:

```text
catalog.schema.table
```

Completion after each dot should walk that hierarchy. A chart-local dataset,
CTE, relation alias, or store is not silently promoted to a DataFusion catalog.
The completion item should expose its actual origin even when SQL allows an
unqualified reference.

### Struct fields

Arrow struct support is a first-class requirement, not a later special case.
Member completion must distinguish:

- `m."title"`, where `m` is a relation alias and `title` is a column;
- `$row.address`, where `$row` is a struct-typed param and `address` is a
  field;
- deeper chains such as `$row.address.city`.

The scope/type layer resolves the left-hand side before candidates are
generated. If both interpretations remain plausible while the query is
invalid, completion may merge the candidates but must retain distinct kinds
and qualifications.

## Recommended Architecture

```text
tolerant Avenger CST + cursor
              |
              v
SQL-island extraction and source map
              |
              v
tokenization + bounded cursor repair
              |
              v
expected semantic role
              |
              v
query scope graph and visible relations
              |
              v
ProjectAnalysis + DataFusion logical schemas/types
              |
              v
catalog/functions/params/stores/helpers
              |
              v
candidate generation, replacement, and ranking
              |
              v
editor-neutral result -> future LSP adapter
```

### 1. Locate the SQL island

The tolerant Avenger frontend supplies:

- the SQL island's source range;
- whether it is a query or expression island;
- its enclosing chart, dataset, transform, handler, or encoding;
- the input dataset or exact pipeline stage, if any;
- visible lexical bindings and their resolved kinds;
- a source map between normalized SQL tokens and authored `$binding` paths or
  `@start`/`@previous` qualifiers.

Completion should not rediscover these facts by scanning raw text. If the DSL
structure is damaged, the tolerant CST should return the smallest plausible
island and a partial context rather than pretending a strict AST exists.

### 2. Tokenize the complete island

Use the pinned sqlparser-rs tokenizer and Avenger's compiler dialect. Retain
comments, quoted identifiers, byte spans, and the tokens on both sides of the
cursor.

Prefix-only completion is specifically rejected. It fails for:

```sql
SELECT m. FROM vega.movies AS m
```

and it discards useful clause structure after the cursor. The completion model
always receives the entire statement plus an explicit cursor offset.

If tokenization fails on an unterminated string or comment, retain the tokens
before the failure, classify the cursor using the enclosing lexical construct,
and return the narrow candidates that are still safe. Do not fabricate a
strict SQL AST.

### 3. Apply bounded cursor recovery

Follow Calcite's general pattern: insert a unique cursor marker, simplify or
repair only the local incomplete construct, and then reuse the real parser and
validator. Avenger should implement a small repair matrix rather than arbitrary
error correction.

Candidate repairs include:

| Cursor situation | Representative analysis repair |
| --- | --- |
| After relation dot | append a unique quoted identifier |
| Empty expression | insert `NULL` or a unique expression identifier |
| Missing relation | insert a unique schema-only relation |
| After comma | insert a placeholder projection, argument, or relation |
| In function arguments | insert a placeholder expression and missing close delimiter |
| Partial clause keyword | preserve the prefix and parse the surrounding repaired clauses |
| In type position | insert a known placeholder type |

The actual replacement range always refers to the original source. Synthetic
tokens carry source-map entries that mark them as generated and prevent them
from leaking into diagnostics or suggestions.

Try a bounded number of repairs selected from the local token context. Cancel
the request or fall back to syntactic candidates when none parse. Completion
must never become an unbounded search for a valid statement.

### 4. Derive the expected semantic role

Syntax recovery answers what category can legally occur at the cursor:
relation, expression, qualifier member, type, function argument, clause, and so
on. It should return a set with confidence/priority rather than one brittle
enum, because malformed SQL may admit multiple interpretations.

This layer may use:

- token neighbors and delimiter nesting;
- a successfully repaired sqlparser-rs AST;
- the path from the cursor marker to the repaired AST root;
- expected-token information from a small completion grammar or parser state,
  if a prototype proves it necessary.

Do not initially fork the full SQL grammar merely to obtain expected tokens.
Start with the tokenizer, repair templates, and AST-path classification. Add a
generated or grammar-driven expectation layer only if the incomplete-SQL
corpus demonstrates durable gaps.

### 5. Build a query scope graph

A semantic scope layer, modeled after the strongest parts of DBeaver, is
required even though DataFusion will ultimately validate the query. It should
represent:

- one scope per query block;
- parent/correlated scope links;
- CTE declaration order and visibility;
- base relations and aliases;
- subquery and CTE output relations;
- projection aliases and the clauses in which they are visible;
- join inputs and ambiguity;
- set-operation output shape;
- wildcard inputs;
- lateral visibility if and when the pinned DataFusion dialect supports it.

This scope graph answers “what names are visible here?” DataFusion answers
“what schema and types does this valid logical expression produce?” Keeping
the responsibilities separate makes completion work while the active query is
still invalid.

Where the repaired statement plans successfully, reconcile the scope graph
with DataFusion's qualified `DFSchema`. Where it does not, retain all scope
facts that were recovered with high confidence and use schemas from the last
successful project analysis.

### 6. Reuse compiler project analysis

The language/compiler plan must first provide an immutable `ProjectAnalysis`
containing:

- stable IDs and source locations for tables, datasets, and pipeline stages;
- exact Arrow/`DFSchema` output schemas;
- qualification, nullability, and lineage;
- the project catalog/schema/table hierarchy;
- dependency fingerprints;
- schema-unavailable diagnostics for providers that cannot describe
  themselves without execution.

The completion engine consumes this model. It must not rebuild the project
catalog or independently infer transform schemas.

For a complete or successfully repaired query, use a fresh isolated DataFusion
analysis context backed by the immutable provider/schema snapshot and create
the logical plan without physical planning or `collect()`. Derived schemas for
CTEs, subqueries, and edited projections come from that plan.

For a query that remains invalid, combine:

1. the last successful `ProjectAnalysis` for stable project relations;
2. the current tolerant query scope for aliases and local derived relations;
3. any subquery or CTE whose repaired form can be planned independently;
4. syntactic candidates for the unresolved remainder.

Never run the watch session's mutable `SessionContext` in place. Static editor
analysis owns isolated generations and immutable snapshots.

### 7. Complete expression islands

An expression island is analyzed against the exact input schema of its
enclosing pipeline stage. Prefer the pinned DataFusion version's logical
expression-planning API when it can resolve an expression against an existing
`DFSchema` without constructing a full query. If that seam is not sufficiently
public or stable, register the input plan as a temporary schema-only relation
and plan:

```sql
SELECT <repaired-expression>
FROM <synthetic-input>
```

The wrapper and placeholder spans are synthetic; all returned ranges map back
to the original expression. Aggregate, window, and handler contexts still need
their real semantic restrictions, so the wrapper is an adapter rather than the
definition of legality.

This is how completion follows a transform chain: the compiler's schema index
selects the input stage, DataFusion supplies its exact columns and types, and
the cursor layer determines what expression member is being written.

### 8. Merge candidate providers

Candidate providers should be small and role-specific:

- SQL keywords and operators;
- visible query relations and aliases;
- columns and struct fields from scoped schemas;
- project catalogs, schemas, tables, views, and datasets;
- scalar params and table-valued stores;
- DataFusion scalar, aggregate, window, and table functions;
- SQL/Arrow types;
- Avenger reserved expression helpers;
- temporal binding qualifiers.

Each provider receives the expected roles and semantic context. There is no
global column or declaration recovery provider: when scope cannot establish a
column, completion omits it and reports `is_incomplete` only when newer
metadata could improve the answer.

### 9. Rank deterministically

Recommended ranking tiers:

1. exact prefix matches valid for the expected role;
2. members of an explicitly typed qualifier such as `m.`;
3. names in the innermost query/lexical scope;
4. columns from the exact pipeline input;
5. project datasets, stores, and default catalog/schema objects;
6. other reachable catalog objects;
7. legal role-specific keywords; explicit invocation may admit fuzzy
   subsequence matches only after legality is established.

Within a tier, prefer:

- unambiguous over ambiguous names;
- already-visible/qualified relations over implicit globals;
- exact case and prefix matches over subsequence matches;
- shorter necessary qualification;
- stable lexical ordering as the final tie-breaker.

Usage history may later refine ties, but deterministic semantic ranking must
work without storing editor history. Suggestions that would compile only after
adding a qualifier should insert that qualifier or be ranked below directly
valid alternatives.

### 10. Keep protocol boundaries clean

`avenger-lang-analysis` should expose editor-neutral request/result types,
roughly:

```rust
pub struct CompletionRequest {
    pub source_id: SourceId,
    pub byte_offset: u32,
    pub revision: SourceRevision,
}

pub struct CompletionItem {
    pub label: String,
    pub replacement: SourceSpan,
    pub insert_text: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub origin: CompletionOrigin,
    pub sort_key: CompletionSortKey,
}

pub struct CompletionResult {
    pub items: Vec<CompletionItem>,
    pub is_incomplete: bool,
    pub analysis_revision: AnalysisRevision,
}
```

Exact type names are provisional. The future native LSP maps these values to
`CompletionItem`, `textEdit`, snippets, and resolve requests. This keeps SQL
completion testable without launching an LSP server and permits a browser
analyzer to consume a serialized schema index.

## FROM-first SQL

FROM-first is a required first-class path, not syntax sugar handled only by the
strict compiler:

```sql
FROM vega.movies AS m
SELECT m.
```

The completion steps are:

1. the tokenizer reads the full island and records the cursor after the dot;
2. local recovery appends a sentinel member;
3. the query scope records `m -> vega.movies` from the preceding `FROM`;
4. `ProjectAnalysis` resolves schema `vega` and table `movies` under the
   default catalog;
5. DataFusion supplies the table schema;
6. the member provider returns columns for `m`, ranked as exact qualified
   members.

Conventional ordering works through the same algorithm:

```sql
SELECT m.
FROM vega.movies AS m
```

Because completion reads text after the cursor, `m` is still resolvable. This is
the critical distinction from shell completers that look only at the SQL
prefix.

No separate compiler lowering is required for FROM-first beyond whatever the
pinned sqlparser-rs/DataFusion dialect already uses. Completion does need
explicit fixtures for incomplete FROM-first statements because parser support
for valid statements does not guarantee good cursor recovery.

## Caching, Concurrency, and Latency

Maintain immutable analysis generations. A completion request captures:

- source revision;
- project-analysis generation;
- catalog/provider metadata generation;
- SQL island fingerprint;
- cursor offset.

Safe reusable caches include:

- token streams and delimiter indexes by island fingerprint;
- tolerant SQL scope graphs;
- catalog/schema/table metadata by provider fingerprint;
- project and pipeline schema indexes;
- logical plans and derived schemas for complete subqueries;
- function/type inventories.

Do not cache repaired source as if it were authored source. Repair results are
keyed by island fingerprint, cursor position, and repair kind and must never
enter compilation.

Recommended operational targets:

- return warm, metadata-backed completion in about 100 ms at p95 on the local
  fixture corpus;
- never wait for a table scan, physical plan, or query execution;
- debounce background project analysis, but do not unnecessarily debounce an
  explicit completion request;
- cancel work for superseded source revisions;
- use cached provider metadata when a remote provider is slow and set
  `is_incomplete`;
- reject results whose source revision no longer matches the request.

The exact latency budget should be baselined during the first prototype rather
than enforced as an unmeasured release gate.

## Failure and Fallback Policy

Completion is layered so one failure does not erase all help:

| Available information | Behavior |
| --- | --- |
| Valid/repaired SQL + current project analysis | Full scoped semantic completion |
| Recovered scope + last successful project analysis | Scoped names and cached schemas, marked stale where relevant |
| Token context + catalog snapshot | Role-filtered keywords, relations, functions, params, and stores |
| Token prefix with proven role only | Narrow role-specific keyword completion; otherwise no items |
| Unterminated SQL string/comment | No items; preserve the correct replacement range and bounded synthetic closer metadata |

When a provider has no planning-time schema, offer the relation itself but do
not invent its columns. Surface the existing schema-unavailable diagnostic and
mark the completion result incomplete if metadata may arrive later.

Ambiguous unqualified columns should be returned as qualified candidates.
Unknown types should remain unknown; do not coerce them to UTF-8 merely to
produce field suggestions.

## Test and Baseline Strategy

Create a shared corpus whose source contains a harness-only `⟦cursor⟧` marker.
The harness removes the marker, computes the byte offset, and compares an
editor-neutral result baseline. Each fixture should record:

- expected semantic roles;
- ordered top candidates and candidate kinds;
- candidates that must be absent;
- insertion text and source replacement range;
- type/nullability/qualification details;
- whether the result is incomplete;
- which repair strategy, if any, was used.

Required fixture groups:

### Syntax and recovery

- empty statements and empty expressions;
- partial keywords and identifiers;
- trailing dot, comma, operator, and open parenthesis;
- missing projection, relation, join condition, or function argument;
- unterminated quote and comment;
- nested delimiters and CASE expressions;
- comments and whitespace at the cursor;
- quoted, mixed-case, Unicode, and reserved-word identifiers;
- multiple statements with completion isolated to the active one;
- valid SQL parity: inserting a candidate yields SQL accepted by the strict
  compiler.

### Query scope

- SELECT-first and FROM-first forms of the same query;
- relation written before and after the cursor;
- catalog/schema/table qualification;
- aliases, self-joins, ambiguous columns, and qualification;
- CTE declaration order, recursive-CTE policy, and CTE output aliases;
- nested and correlated subqueries;
- derived-table output columns;
- projection aliases in every clause where DataFusion permits or rejects them;
- joins, USING/NATURAL behavior if supported, aggregates, windows, and set
  operations;
- wildcard and qualified-wildcard suggestions/expansion.

### Avenger schemas and bindings

- a three-query dataset chain that adds, renames, and removes columns;
- completion at every transform pipeline stage;
- catalog tables, chart datasets, SQL views, and table-valued stores;
- scalar params of every supported Arrow family;
- struct params, struct columns, and nested struct members;
- `$binding@start` and `$binding@previous` eligibility;
- lexical scope, component-qualified paths, private visibility, shadowing, and
  param/store collision diagnostics;
- custom providers with known and unavailable planning-time schemas.

### Functions and types

- scalar, aggregate, window, and table-function positions;
- signature detail and active argument;
- registered custom functions;
- casts and physical Arrow types, including nested struct/list types;
- Avenger reserved helpers and their context restrictions.

### Performance and invalidation

- warm and cold completion timings over representative project sizes;
- cancellation under rapid edits;
- project file, data schema, catalog metadata, and function-registry
  invalidation;
- no physical plan or execution/`collect()` during completion;
- stale analysis results rejected by revision;
- deterministic ordering across runs.

Baselines should be reviewed semantically. Exact ordered identities and edits
are authoritative for focused structural and SQL intent cases; generated
registry/function inventory tests separately prove that newly registered
entries flow through without hand-maintained switches.

## Suggested Implementation Sequence

This is sequencing guidance for the later implementation plan, not a progress
checklist.

### Phase A — Corpus and API spike

- Define editor-neutral request/result, role, scope, and candidate types.
- Freeze representative valid and invalid SQL fixtures, including FROM-first.
- Audit the pinned sqlparser-rs tokenizer/parser and DataFusion planner APIs
  against the corpus.
- Measure how many cases cursor-marker plus local repairs can classify before
  adding any completion grammar.

### Phase B — Syntax completion

- Locate SQL islands from the tolerant Avenger CST.
- Implement full-island tokenization, cursor offsets, replacement ranges, and
  bounded repairs.
- Produce keywords, clauses, operators, and expected semantic roles without
  catalog access.

### Phase C — Base semantic completion

- Consume `ProjectAnalysis`.
- Complete catalogs, schemas, tables, chart datasets, stores, aliases, and base
  columns.
- Support SELECT-first and FROM-first equivalently.

### Phase D — Derived query scopes

- Add CTE, subquery, join, projection-alias, correlation, set-operation, and
  wildcard scope semantics.
- Plan valid/repaired fragments through DataFusion and attach exact derived
  schemas and types.

### Phase E — Expression and Avenger integration

- Complete encoding and transform expressions against exact pipeline stages.
- Add scalar params, table stores, temporal qualifiers, nested Arrow struct
  fields, and Avenger helper functions.
- Preserve normalized-binding source maps through repaired SQL.

### Phase F — Product quality

- Add signatures, snippets, documentation, deterministic ranking, cancellation,
  incremental caches, and latency baselines.
- Expose the engine through the future native LSP.
- Keep inspector/runtime values as a separate optional enrichment layer.

## Research Survey

The survey below records useful existing work and the lesson Avenger should
take from it. Links point to the primary project source or documentation
reviewed on 2026-07-13.

### DataFusion and Rust ecosystem

| Project | Useful work | Lesson for Avenger |
| --- | --- | --- |
| [Apache DataFusion CLI helper](https://github.com/apache/datafusion/blob/main/datafusion-cli/src/helper.rs) | The current helper completes quoted file paths in `LOCATION` contexts. | DataFusion itself does not supply the semantic SQL completion engine Avenger needs. |
| [datafusion-dft editor](https://github.com/datafusion-contrib/datafusion-dft/blob/main/src/tui/state/tabs/sql.rs) and [completion issue](https://github.com/datafusion-contrib/datafusion-dft/issues/90) | A DataFusion TUI with an open completion work item at the research date. | Do not wait for a reusable upstream engine; keep the Avenger API modular enough to adopt improvements later. |
| [DBCrust completion](https://github.com/clement-tourriere/dbcrust/blob/main/src/completion.rs) and [DataFusion metadata adapter](https://github.com/clement-tourriere/dbcrust/blob/main/src/database_datafusion.rs) | Schema/table/column caches, aliases, forward-looking relation discovery, and nested Arrow field suggestions. | Best nearby proof that useful DataFusion-aware completion is practical; reuse its ergonomic ideas, but add real query scopes and planner-derived schemas. |
| [Spice REPL completer](https://github.com/spiceai/spiceai/blob/trunk/crates/repl/src/completer.rs) | Background metadata caching over FlightSQL. | Provider metadata must be cached and completion must tolerate asynchronous refresh. |
| [datafusion-ui `sql-ide`](https://github.com/wyatt-herkamp/datafusion-ui/blob/main/crates/sql-ide/src/complete.rs) | A GUI-independent Rust completion module with lightweight token heuristics. | Keep Avenger's completion core independent of LSP/UI types, but do not stop at token heuristics. |
| [Tabiew SQL completion](https://github.com/shshemi/tabiew/tree/main/src/sql_completion) | A terminal completion implementation; its current SQL backend is Polars rather than DataFusion. | Useful UI reference, not a semantic foundation for Avenger. |
| [sqlparser-rs incomplete parsing issue](https://github.com/apache/datafusion-sqlparser-rs/issues/224) and [partial-parser discussion](https://github.com/apache/datafusion-sqlparser-rs/issues/1392) | Longstanding demand for parsing incomplete SQL, without a complete completion API to rely on. | Plan for an Avenger-owned bounded recovery layer around the pinned parser. |

### Broader open-source completion engines

| Project | Strengths worth studying | Boundary or caution |
| --- | --- | --- |
| [DBeaver SQL completion](https://dbeaver.com/docs/dbeaver/SQL-Assist-and-Auto-Complete/), [semantic completion context](https://github.com/dbeaver/dbeaver/blob/devel/plugins/org.jkiss.dbeaver.model.sql/src/org/jkiss/dbeaver/model/sql/semantics/completion/SQLQueryCompletionContext.java), and [tests](https://github.com/dbeaver/dbeaver/blob/devel/test/org.jkiss.dbeaver.test.platform/src/org/jkiss/dbeaver/model/sql/analyzer/SQLQueryCompletionAnalyzerTest.java) | The most complete product reference surveyed: lexical scopes, aliases, CTEs/subqueries, derived columns, joins/set operations, catalogs, star expansion, and semantic/legacy/combined modes. | Java/product architecture is not reusable directly; copy the semantic scope model and test categories. |
| [Apache Calcite `SqlAdvisor`](https://github.com/apache/calcite/blob/main/core/src/main/java/org/apache/calcite/sql/advise/SqlAdvisor.java), [`SqlSimpleParser`](https://github.com/apache/calcite/blob/main/core/src/main/java/org/apache/calcite/sql/advise/SqlSimpleParser.java), [advisor validator](https://github.com/apache/calcite/blob/main/core/src/main/java/org/apache/calcite/sql/advise/SqlAdvisorValidator.java), and [tests](https://github.com/apache/calcite/blob/main/core/src/test/java/org/apache/calcite/sql/test/SqlAdvisorTest.java) | The best core algorithmic reference: inject a hint token, simplify/repair incomplete SQL, then use tolerant semantic validation to obtain scope-aware hints. | Calcite's parser/validator cannot be Avenger's dialect authority; reproduce the architecture around sqlparser-rs/DataFusion. |
| [Hue autocomplete parser documentation](https://docs.gethue.com/developer/components/parsers/) and [generic Jison grammar](https://github.com/cloudera/hue/tree/master/desktop/core/src/desktop/js/parse/sql/generic/jison) | Cursor-native grammar outputs expected keywords, entity categories, and source locations across dialect families. | Strong syntactic reference; metadata and exact derived-schema propagation still need a separate semantic layer. |
| [DTStack dt-sql-parser completion API](https://github.com/DTStack/dt-sql-parser#code-completion), [semantic context collector](https://github.com/DTStack/dt-sql-parser/blob/main/src/parser/common/semanticContextCollector.ts), and [PostgreSQL suggestion tests](https://github.com/DTStack/dt-sql-parser/tree/main/test/parser/postgresql/suggestion) | Packaged ANTLR/antlr4-c3 expected-token completion, semantic context categories, nested-query accessibility, and tests after syntax errors. | Valuable if token/repair classification proves insufficient; a second generated grammar carries dialect-sync cost. |
| [DuckDB autocomplete core](https://github.com/duckdb/duckdb/blob/main/src/parser/peg/autocomplete_core.cpp), [catalog-provider interface](https://github.com/duckdb/duckdb/blob/main/src/include/duckdb/parser/peg/autocomplete_catalog_provider.hpp), and [SELECT tests](https://github.com/duckdb/duckdb/blob/main/test/sql/function/autocomplete/select.test) | Highly relevant dialect and UX reference: FROM-first, catalogs/schemas/tables/columns/types/functions/files/settings, quoting, replacement, and ranking. | Study its FROM-first fixtures and candidate presentation; Avenger still needs its own DataFusion query-scope/schema layer. |
| [pgcli context analysis](https://github.com/dbcli/pgcli/blob/main/pgcli/packages/sqlcompletion.py), [candidate generation](https://github.com/dbcli/pgcli/blob/main/pgcli/pgcompleter.py), and [CTE parsing](https://github.com/dbcli/pgcli/blob/main/pgcli/packages/parseutils/ctes.py) | Mature shell UX: relations on either side of the cursor, aliases, CTE-local tables/output names, search paths, function signatures, star expansion, join snippets, and ranking. | Its heuristic architecture is an excellent feature checklist, not the type/schema authority. |
| [sqls features](https://github.com/sqls-server/sqls#features), [completer](https://github.com/sqls-server/sqls/blob/master/internal/completer/completer.go), and [candidates](https://github.com/sqls-server/sqls/blob/master/internal/completer/candidates.go) | A self-contained LSP reference with metadata caching, schemas/tables/columns, aliases, subqueries, subquery output columns, and join snippets. | Study protocol boundaries and cache lifecycle; use the stronger Calcite/DBeaver model for incomplete syntax and query semantics. |

## Recommendations From the Survey

### Adopt

- Calcite's “repair incomplete SQL, then reuse real semantic analysis” pattern.
- DBeaver's explicit lexical query scopes and derived-relation model.
- Hue and dt-sql-parser's treatment of the cursor as a grammar position with
  expected semantic categories.
- DuckDB's FROM-first coverage, quoting/replacement behavior, and candidate
  ranking tests.
- pgcli's full-statement/forward-looking relation discovery, star expansion,
  signatures, and join-snippet UX.
- sqls and Spice's editor-neutral metadata caches and invalidation discipline.
- DBCrust's practical DataFusion catalog integration and nested Arrow-field
  completion.

### Avoid

- Prefix-only SQL analysis.
- A global unscoped column list as the primary completion source.
- Reimplementing DataFusion schema propagation in completion code.
- Depending on strict whole-file compilation for every keystroke.
- Executing queries to learn a schema.
- Coupling candidate generation directly to LSP structs or one editor.
- Maintaining a full hand-written or generated SQL grammar before the recovery
  corpus proves it is necessary.
- Treating the last runtime-observed schema as the static language truth.

### Reference reading order for implementation

1. Calcite `SqlSimpleParser`, `SqlAdvisor`, and advisor tests.
2. DBeaver's semantic completion context and analyzer tests.
3. DuckDB's autocomplete SELECT tests, especially FROM-first cases.
4. DBCrust's DataFusion metadata and nested-field completion.
5. pgcli's context/candidate split and CTE handling.
6. dt-sql-parser/Hue only for incomplete cases that remain unsolved.
7. sqls/Spice for transport-independent caching and refresh behavior.

## Detailed-Plan Resolutions

The native LSP implementation plan is now tracked in
`scratch/avenger-lang/lsp-implementation-plan.md`. Its 2026-07-21 research pass
resolved the planning questions as follows:

1. Freeze sqlparser-rs 0.62.0 and DataFusion 54.0.0 for the first completion
   corpus, using the same `AvengerSqlDialect` and DataFusion parser/session
   options as strict compilation.
2. DataFusion 54 exposes
   `SessionState::create_logical_expr(sql, &DFSchema)`. Use that public seam for
   expression islands against the exact pipeline-stage schema; keep a
   synthetic `SELECT <expr> FROM <input>` only as a tested fallback for a
   context the direct API cannot represent.
3. Begin with full-island tokens, bounded cursor repairs, repaired sqlparser AST
   paths, and an Avenger-owned query scope graph. Do not add a second
   completion grammar until measured corpus failures justify its dialect-sync
   cost.
4. The serializable browser/Wasm subset is not a prerequisite for the native
   LSP. Keep editor-neutral APIs and stable schema identities so a later plan
   can define that wire format without leaking LSP types into analysis.
5. Explicit completion never waits for a new remote metadata fetch. It consumes
   the newest published immutable provider/schema snapshot, may trigger a
   cancellable background refresh, and returns `is_incomplete` when metadata is
   stale or unavailable. Exact provider timeouts are baselined before becoming
   release defaults.
6. The first semantic milestone covers SELECT-first/FROM-first parity,
   catalogs/schemas/tables, aliases, ordinary CTEs and subqueries, joins,
   wildcards, set operations, functions/types, nested struct fields, params,
   stores, and exact pipeline stages. Advanced forms such as lateral or
   recursive scopes, wildcard modifiers, named windows, and table functions
   enter the corpus only with strict compiler conformance fixtures and are
   completed before being advertised as supported.
7. Defer join-condition snippets until Avenger has standardized relationship or
   foreign-key metadata. Plain join-scope column completion remains in the
   first semantic milestone.
