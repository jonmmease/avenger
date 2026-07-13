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
| Used-scale discovery | N/A | Required | Pending: discover non-positional channel scales from part marks and namespace each widget-local scale by widget id. |
| Default range selection | N/A | Required | Pending: use part-mark defaults for namespaced non-positional scales under PixelFrame. |
| Scale-domain discovery and revision caching | Required when a part scale reads items | Required | Pending: prepare each widget once, build independent namespaced domains, and include widget identity/data dependencies in the domain cache key. |
| Coordinate domains and automatic guides/legends | Excluded | Excluded | PixelFrame positions are logical pixels. Widget scales must not contribute coordinate domains or synthesize axes/legends. |
| Mark-data preparation | Required | Required | Implemented: one `WidgetPreparedBaseData` is shared by measurement and every part render; explicit part data still follows ordinary mark precedence. |
| Store and selection data | Required when referenced | Required when referenced | Dependency discovery is implemented. Runtime equivalence and revision gates remain pending. |
| Dependency placeholders and session invalidation | Required | Required | Implemented for item plans, item validation params, part marks, and persistent presentation expressions. Cache-behavior gates remain pending. |
| Materialization scheduling and prefetch planners | Required | Required when declared by a part | Item plans follow ordinary evaluation/baking. View/materialization and asynchronous prefetch/resource wake-up gates remain pending. |
| Image/resource requests | N/A | Required | Part rendering uses the owning evaluation context, so requests are retained; fulfillment/redraw tests remain pending. |
| Rendering and style/measurement caches | Required | Required | Implemented without a second item evaluation or CSS query. Complete theme/environment cache-key gates remain pending. |
| Event-datum schema inference | Required | Required | Implemented for item schemas, part `EventDatumFieldSpec`s, explicit part data, and stores. |
| Rendered event-datum rows | Required | Required | Implemented: source/generated rows are retained at the exact final widget-container scene path; widget insertion also rebases later guide/legend rows. |
| Bake assembly and residual retargeting | Required | Required through their data contexts | Implemented for root and nested widget item plans with stable bake context ids and source manifests. Part-data-context audit remains pending. |
| Runtime-unfoldable/source-manifest discovery | Required | Required through their data contexts | Item plans are implemented. The full part-mark census remains pending. |
| Serialization and direct/bincode parity | Required | Required | Implemented for composed attachments, item plans, marks, measurement, placement, relative targets, and presentation bindings. Native payload registry parity is W5. |
| Child-plot recursion | Required for one-shot children | Required for one-shot children | Implemented for concat WidgetCells. Facet/repeat multiplication rejects attachments until a repeated-widget identity/state contract exists. |

The matrix is complete only when no `Pending` row remains and the W1.4 gates
exercise each `Required` branch. When a new compiled-content consumer is
introduced, it must add a row here in the same change.
