# Compiled widget traversal matrix

This is the implementation checklist for every consumer of compiled plot
content. A composed widget owns two compiled-content branches: its optional
item relation and its PixelFrame part marks. A native widget owns declarative
state only until the native runtime registry is implemented.

`Required` means that the widget branch must behave like ordinary plot
content. `Excluded` means that omission is intentional and covered by the
widget contract, rather than an accidental marks-only traversal. `Pending`
keeps W1.4 open.

| Consumer | Item relation | Part marks | Contract and implementation state |
| --- | --- | --- | --- |
| Compile validation and target registration | Required | Required | Implemented: structural ids, duplicate ids/parts, reserved columns/params, decorative targets, stable relative numeric paths, and multiplied-child rejection. |
| Used-scale discovery | N/A | Required | Implemented: non-positional part channels populate an isolated, serialized scale namespace keyed by widget id. |
| Default range selection | N/A | Required | Implemented: each widget scale builder resolves ranges through its own part-mark defaults under PixelFrame. |
| Scale-domain discovery and revision caching | Required when a part scale reads items | Required | Implemented: the shared prepared item relation feeds one builder, its cached domains serve provisional and final ranges, and param-driven item revisions build fresh domains without coupling host/peer scales. Evaluation metrics prove one item collect across measurement and multiple parts; the session cache reuses that prepared relation for a CSS-only repaint and invalidates it for a query-param revision. |
| Coordinate domains and automatic guides/legends | Excluded | Excluded | PixelFrame positions are logical pixels. Widget scales must not contribute coordinate domains or synthesize axes/legends. |
| Mark-data preparation | Required | Required | Implemented: one `WidgetPreparedBaseData` is shared by measurement and every part render; explicit part data still follows ordinary mark precedence. |
| Store and selection data | Required when referenced | Required when referenced | Dependency discovery is implemented. Runtime equivalence and revision gates remain pending. |
| Dependency placeholders and session invalidation | Required | Required | Implemented for item plans, item validation params, part marks, and persistent presentation expressions. The durable item cache fingerprints only item-plan/validation parameter dependencies and conservatively includes selection/store revisions. |
| Materialization scheduling and prefetch planners | Required | Required when declared by a part | Item plans follow ordinary evaluation/baking. View/materialization and asynchronous prefetch/resource wake-up gates remain pending. |
| Image/resource requests | N/A | Excluded from the initial Rect/Symbol/Rule/Text matrix | Text parts use the shared text/font service. A resource wake/redraw gate belongs to the first resource-bearing PixelFrame or native mark in W5; W1 must not invent an Image part that the declared mark matrix does not support. |
| Rendering and style/measurement caches | Required | Required | Implemented without a second item evaluation or CSS query. The durable prepared-item key contains only item-plan/validation params plus selection/store revisions: style-only repaints are cache hits with zero item collects, while a query-param revision is one miss/collect. The resolved-style digest hashes CSS-source identity and typed evaluated values, so `rem` base font, color scheme, referenced custom properties, and media boundaries invalidate it while an unreferenced custom property does not. Integration coverage proves those environment changes rebuild painted geometry/color while retaining the prepared item relation. |
| Event-datum schema inference | Required | Required | Implemented for item schemas, part `EventDatumFieldSpec`s, explicit part data, and stores. |
| Rendered event-datum rows | Required | Required | Implemented: source/generated rows are retained at the exact final widget-container scene path; widget insertion also rebases later guide/legend rows. |
| Bake assembly and residual retargeting | Required | Required through their data contexts | Implemented for root and nested widget item plans and explicit/transform-bearing part data, with stable item/part context ids and source manifests. Pure inherited parts deliberately reuse the one item target rather than duplicating it per part. A bincode-round-tripped explicit part evaluates self-contained with its source table absent. |
| Runtime-unfoldable/source-manifest discovery | Required | Required through their data contexts | Implemented by the same root/nested item and explicit-part census. Live transform-stage tables remain reported when a target cannot bake. |
| Serialization and direct/bincode parity | Required | Required | Implemented for composed attachments, item plans, marks, measurement, placement, relative targets, and presentation bindings. Native payload registry parity is W5. |
| Child-plot recursion | Required for one-shot children | Required for one-shot children | Implemented for widgets attached inside one-shot concat child plots. `WidgetCell` itself remains W4. Facet/repeat multiplication rejects attachments until a repeated-widget identity/state contract exists. |

The matrix is complete only when no `Pending` row remains and the W1.4 gates
exercise each `Required` branch. When a new compiled-content consumer is
introduced, it must add a row here in the same change.
