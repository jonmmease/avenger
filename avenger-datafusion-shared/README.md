# avenger-datafusion-shared

Shared inputs for Apache DataFusion: a plan that reads one input several times while the input runs once. DataFusion plans and runs each copy of a subplan separately, so a plan that joins an input with an aggregate of the same input computes the input twice, and a volatile input such as one that calls `random()` gives each reader different rows.

`shared(input)` wraps an input in a `SharedNode`, and every reference to that node in a plan reads one run of it. A query planner plans the node with `SharedExtensionPlanner`, which optimizes the input once and gives every reference a `SharedExec` of one state. The state buffers each output partition of the input under a memory reservation and frees a batch when every reader has read it. The buffer does not spill.

`InlineShared` is an optimizer rule that computes a cheap input again in place of sharing it: a `VALUES` list, an empty relation, or a scan of an in-memory table under projections, filters, and aliases, unless it computes something volatile. An inlined input keeps the pushdown of filters and projections into each reader.
