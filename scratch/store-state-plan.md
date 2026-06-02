# Store State Plan

## Summary

Add `Store` as a first-class chart concept alongside `Param` and ordinary
data. A store is a session-owned, mutable, table-shaped value with an Arrow
schema. Event bindings can insert, update, delete, toggle, clear, or replace
rows, and marks can use store rows as ordinary data.

This deliberately separates three roles:

- `Param`: scalar/list/struct values used in expressions.
- `Data`: immutable or externally supplied tabular data.
- `Store`: mutable session-owned tabular data updated by interaction.
- `Selection`: semantic predicate composition, which can later derive clauses
  from store rows instead of owning drawable geometry itself.

The immediate motivation is box-selection chrome. Brush rectangles should be
ordinary `Rect` marks over a store of box rows. Selection predicates can keep
using the current selection state during the migration, then move to deriving
predicate clauses from store rows once store data sources are in place.

## Prior Art

Vega supports dynamic data set updates through triggers with `insert`,
`remove`, `toggle`, and `modify` operations. Those triggers mutate data
objects, and marks can render from the updated data set.

Mosaic models selections as dynamic predicate clauses rather than a fixed
coordinate-specific object. That suggests Avenger selections should become
neutral predicate-combining objects, while stores hold the mutable rows used
for editable visual state such as brush boxes, lasso paths, annotations, and
tool handles.

## Goals

- Define stores with a name and Arrow schema, optionally seeded from an initial
  `RecordBatch`.
- Add stores to plots through `Plot::add_store(...)`.
- Let stores use the same `Sharing` model as params and selections.
- Add event-binding store updates for the common CRUD operations.
- Let marks consume store data through the normal data/channel pipeline.
- Move box-selection overlay rectangles off `SelectionClauseDataset` and onto
  `StoreData`.
- Leave a clear migration path to neutral selections whose predicates are
  generated from store rows.
- Keep the store API serializable in `CompiledPlot`.

## Non-Goals For The First Implementation

- Large, database-backed mutable tables.
- Arbitrary SQL mutation statements.
- Joins between stores and source data during mutation.
- Persistent stores across app launches.
- Selection-aware query optimization or Mosaic-style pixel pre-aggregation.
- Automatic conflict resolution across simultaneous app clients.

## Core Public Model

### Store Spec

```rust
let brush_boxes = Store::new("brush_boxes", schema)
    .primary_key(["id"])
    .sharing(Sharing::Free);

Plot::<Cartesian>::new()
    .add_store(brush_boxes)
```

Construction options:

```rust
Store::new("brush_boxes", schema)
Store::from_record_batch("brush_boxes", batch)
Store::empty("brush_boxes").field("id", DataType::Utf8, false)
```

Compiled representation:

```rust
pub struct Store {
    pub name: String,
    pub schema: Arc<Schema>,
    pub initial: Option<RecordBatch>,
    pub primary_key: Vec<String>,
    pub sharing: Sharing,
}

pub struct CompiledStoreSpec {
    pub name: String,
    pub schema: SerializableSchema,
    pub initial: Option<SerializableRecordBatch>,
    pub primary_key: Vec<String>,
    pub sharing: Sharing,
}
```

`CompiledPlot` stores `store_specs: IndexMap<String, CompiledStoreSpec>`.
`PlotSession` instantiates these into scoped runtime store state.

### Primary Key Policy

Stores should support both keyed and unkeyed modes.

Unkeyed stores are valid for append-only logs, transient single-row stores,
and full replacement:

```rust
StoreUpdate::clear()
StoreUpdate::replace_rows(rows)
StoreUpdate::insert_rows(rows)
StoreUpdate::delete_where(predicate)
StoreUpdate::update_where(predicate, fields)
```

Keyed stores are required for operations that target row identity:

```rust
StoreUpdate::upsert_row(row)
StoreUpdate::update_by_key(key, fields)
StoreUpdate::delete_by_key(key)
StoreUpdate::toggle_row(row)
```

Recommendation: built-in tools should always define an explicit key column,
usually `id`, because moving/resizing/deleting an existing box or handle needs
stable identity. The key should not be hidden or invented by default; explicit
schema keeps store rows inspectable and makes DataFusion expressions over store
data predictable.

Validation rules:

- Store names use the same identifier rules as params/tools.
- Primary-key fields must exist in the schema.
- Primary-key fields must be non-nullable, or compile should reject the store.
- Composite keys are allowed.
- Key uniqueness is enforced per scoped store instance.
- Insert with duplicate key errors unless the operation is explicitly `upsert`
  or `toggle`.

## Event Binding API

Add store assignments next to param and selection assignments:

```rust
ChartEventBinding::on(ChartEventType::MouseUp)
    .set_store_at_start_scope(
        "brush_boxes",
        StoreUpdate::replace_rows([
            StoreRow::new()
                .field("id", lit("active"))
                .field("x_min", ev::interval_start(ev::interval_ordered(...)))
                .field("x_max", ev::interval_end(ev::interval_ordered(...)))
                .field("y_min", ev::interval_start(ev::interval_ordered(...)))
                .field("y_max", ev::interval_end(ev::interval_ordered(...)))
        ])
    )
```

Add these public update builders:

```rust
StoreUpdate::clear()
StoreUpdate::replace_rows(rows)
StoreUpdate::insert_rows(rows)
StoreUpdate::upsert_rows(rows)
StoreUpdate::delete_by_key(key)
StoreUpdate::delete_where(predicate)
StoreUpdate::update_by_key(key, fields)
StoreUpdate::update_where(predicate, fields)
StoreUpdate::toggle_rows(rows)
```

`StoreRow` and `StoreFieldPatch` hold serializable DataFusion expressions:

```rust
StoreRow::new()
    .field("id", expr)
    .field("x_min", expr)

StoreFieldPatch::new()
    .field("x_min", expr)
    .field("x_max", expr)
```

Compile-time validation:

- Every assigned field must exist in the store schema.
- Values must be castable to the target field type.
- Keyed operations require a declared primary key.
- `upsert` and `toggle` rows must provide every key field.
- `replace_rows` and `insert_rows` may omit nullable non-key fields, which
  become null.

Runtime evaluation:

- Store row expressions are evaluated against the same one-row event batch used
  for param/selection assignments.
- `update_where` and `delete_where` predicates are evaluated against existing
  store rows augmented with event/param derived columns.
- Mutations return whether the store actually changed so chart reevaluation can
  be skipped for no-op updates.

## Scoping

Stores use `Sharing` exactly like params:

- `Sharing::Shared`: one root store instance.
- `Sharing::Free` / `Level(0)`: one store instance per leaf facet cell.
- `Sharing::Level(N)`: one store instance per logical ancestor at level `N`.

Event binding writes use `ChartEventAssignmentScope`:

```rust
.set_store("brush_boxes", update)                // current routed scope
.set_store_at_start_scope("brush_boxes", update) // gesture-start scope
```

The runtime resolves the store owner path with the same
`owner_path_for_sharing(...)` helper used by params. For a free faceted brush,
dragging in a cell writes only that cell's scoped store. For a shared brush,
the same binding writes the root store.

Initial values:

- The initial `RecordBatch` seeds the root/shared store.
- Non-root scoped stores start empty by default.
- If a non-root store needs authored initial rows later, add an explicit
  scoped-initial-data feature rather than overloading the V1 constructor.

## Store Data Sources

Replace the selection-specific data source escape hatch with a general store
data source.

```rust
Rect::new()
    .data(StoreData::new("brush_boxes").current_scope())
    .x(col("x_min"))
    .x2(col("x_max"))
    .y(col("y_min"))
    .y2(col("y_max"))
```

Core data source shape:

```rust
pub enum DataSource {
    DataFrame(DataFrame),
    Store(StoreData),
}

pub struct StoreData {
    pub store_name: String,
    pub scope: StoreDataScope,
}

pub enum StoreDataScope {
    CurrentOwner,
    AllOwners,
}
```

Default `StoreData::new(name)` should use `CurrentOwner`, because overlay marks
inside facets usually want only the rows owned by the current cell. Selection
predicate generation and diagnostics can use `AllOwners`.

When materialized, store data should include Avenger-owned metadata columns in
addition to user schema columns:

- `__store_name`
- `__store_owner_path`
- `__store_scope_id` when available
- `__store_revision`

These metadata columns are reserved and cannot appear in authored store schemas.

## Runtime State

Add a scoped store runtime next to scoped params and scoped selections:

```rust
pub struct ScopedStoreAssignment {
    pub store_name: String,
    pub owner_path: Vec<ScalarValue>,
    pub update: StoreStateUpdate,
}

pub(crate) struct ScopedStoreState {
    specs: IndexMap<String, CompiledStoreSpec>,
    instances: IndexMap<ScopedStoreKey, MutableStoreTable>,
    revisions: IndexMap<String, u64>,
}
```

`MutableStoreTable` can start as a small-row implementation:

```rust
struct MutableStoreTable {
    schema: Arc<Schema>,
    rows: Vec<IndexMap<String, ScalarValue>>,
    revision: u64,
}
```

The first implementation should optimize for simplicity and correctness. Store
sizes used by interactions will normally be tiny. Materialize to `RecordBatch`
when a mark requests store data.

Evaluation context:

- `PlotSession` owns `ScopedStoreState`.
- `EvaluationContext` carries an optional `Arc<ScopedStoreState>` like scoped
  params/selections.
- Mark data preparation includes store revision fingerprints in cache keys.

## Relationship To Selections

Phase 1 should not require rewriting selection predicates. The safe migration is:

1. Build `Store` independently.
2. Move brush overlay rectangles to `StoreData`.
3. Keep current `SelectionState` for selected-point predicates while the
   prototype is stabilizing.
4. Add `Selection::new("brush")` as a neutral predicate-combining spec.
5. Add selection predicate derivation from store rows.
6. Remove `SelectionClauseDataset`, `SelectionGeometrySchema`, and
   `SelectionGeometryValue` once no public examples or tools need them.

In the final model, selections are not configured with coordinate systems or
geometry schemas. A Cartesian box tool creates a keyed store of boxes and a
neutral selection whose predicate is derived from those store rows.

## Box Selection Shape After Store Migration

Manual form:

```rust
let brush_boxes = Store::empty("brush_boxes")
    .field("id", DataType::Utf8, false)
    .field("x_min", DataType::Float64, false)
    .field("x_max", DataType::Float64, false)
    .field("y_min", DataType::Float64, false)
    .field("y_max", DataType::Float64, false)
    .primary_key(["id"])
    .sharing(Sharing::Free);

let brush = Selection::new("brush")
    .from_store("brush_boxes")
    .predicate(SelectionPredicate::cartesian_interval(
        (":x", "x_min", "x_max"),
        (":y", "y_min", "y_max"),
    ))
    .combine(SelectionCombine::Union)
    .empty_selects_nothing();

Plot::<Cartesian>::new()
    .add_store(brush_boxes)
    .add_selection(brush)
    .event_binding(draw_or_update_store_rows)
    .mark(points_with_fill_condition(brush.predicate()))
    .mark(
        Rect::new()
            .data(StoreData::new("brush_boxes").current_scope())
            .exclude_from_scale_domains()
            .x(col("x_min"))
            .x2(col("x_max"))
            .y(col("y_min"))
            .y2(col("y_max"))
    )
```

Tool form:

```rust
Plot::<Cartesian>::new()
    .tool(BoxSelection::cartesian().id("brush"))
```

The tool expands into the same public pieces: store, neutral selection, event
bindings, and overlay marks.

## Phases

### Phase 1: Store Specs And Session State

- Add `Store`, `CompiledStoreSpec`, `StoreData`, `StoreDataScope`, `StoreRow`,
  `StoreFieldPatch`, and `StoreUpdate` to `avenger-chart-core`.
- Add `Plot::add_store(...)` and compile stores into `CompiledPlot`.
- Add `ScopedStoreState` to `PlotSession`.
- Add direct diagnostic APIs for reading scoped store contents.
- Validate schema, name, reserved fields, and primary-key constraints.

Validation:

- Store specs serialize/deserialize.
- Initial `RecordBatch` seeds the root store.
- Shared/free/level store owner paths match param owner paths.
- Primary-key uniqueness is enforced per scoped store.

### Phase 2: Event Binding Store Mutations

- Add `ChartEventStoreAssignment`.
- Add `.set_store(...)` and `.set_store_at_start_scope(...)`.
- Compile store update expressions in `avenger-chart-app`.
- Apply scoped store patches in `PlotSession`.
- Add mutation metrics and diagnostics.

Validation:

- Insert, replace, clear, upsert, update-by-key, delete-by-key work.
- `delete_where` and `update_where` work over existing rows plus event columns.
- Free facet store mutations update only the start/current cell.
- Shared store mutations update root.
- No-op mutations skip reevaluation.

### Phase 3: Store Data For Marks

- Replace `DataContext`'s selection-specific variant with a generic store data
  source.
- Add `CompiledDataContext` support for `StoreData`.
- Materialize store rows to `RecordBatch` during mark data preparation.
- Include store revision fingerprints in mark data/profile cache keys.
- Add `.data(StoreData::new(...))` or an equivalent mark builder method.

Validation:

- A `Rect` mark renders one row per store row.
- Store-backed marks update when store rows mutate.
- Current-owner scope filters rows correctly inside facets.
- All-owner scope can materialize rows across scoped store instances.

### Phase 4: Box Selection Overlay Migration

- Update manual box-selection examples so the overlay `Rect` reads from a
  `brush_boxes` store.
- Keep current `SelectionState` writes for point predicates during this phase.
- Remove or deprecate example usage of `SelectionClauseDataset`.

Validation:

- Existing single-panel and faceted manual box selection behavior is preserved.
- Shift+drag inserts multiple box rows.
- Replacing a selection replaces the store rows for the appropriate scope.
- Box movement/resizing updates keyed rows.

### Phase 5: Neutral Selection From Stores

- Add `Selection::new(id)` as a neutral selection spec.
- Move selection dimensions and geometry schema out of `Selection`.
- Add a selection predicate-source model that can read store rows.
- Generate dynamic predicates from store rows at evaluation time.
- Preserve selection combine/empty/sharing semantics.

Validation:

- `brush.predicate()` works from store rows.
- Union/intersection over multiple box rows works.
- Facet context is captured through store owner metadata and/or explicit row
  columns.
- Sibling concat views can use the same predicate.

### Phase 6: Remove Selection Geometry Dataset Plumbing

- Remove `SelectionClauseDataset`.
- Remove `SelectionGeometrySchema` / `SelectionGeometryValue` if no longer
  needed.
- Remove `DataContext::selection_clause_dataset(...)` and
  `CompiledDataContext::new_selection_clause_dataset(...)`.
- Update docs/examples/tools to describe stores as the way to render/edit
  variable interaction geometry.

Validation:

- Full focused selection and tool tests pass.
- No remaining public API uses selection geometry datasets.
- Architecture docs describe stores and neutral selections in final-state terms.

## Open Questions

- Should store data expose owner metadata as columns by default, or only through
  opt-in helpers? Recommendation: include reserved metadata columns by default
  for diagnostics and filtering, but keep user schemas from declaring them.
- Should `StoreData::new(name)` default to `CurrentOwner` or `AllOwners`?
  Recommendation: `CurrentOwner`; it is safer for overlay marks and matches the
  common faceted-chrome case.
- Should `update_where` and `delete_where` ship in V1? Recommendation: yes if
  expression evaluation over existing rows is straightforward; otherwise ship
  keyed update/delete first and add predicate updates in Phase 2b.
- Should stores be allowed at subplot roots? Recommendation: yes, same as
  params/tools, with root compilation collecting specs into the compiled plot.

