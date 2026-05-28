# Facet System

Facet row and facet column are built-in layout containers in `avenger-chart`.
They are implemented as coordinate systems, but they are not external
layout-container extension points.

## Runtime Pipeline

```mermaid
flowchart TD
    Compile["Subplot<FacetRow/FacetColumn>\nCompiledFacetRowSubplot / CompiledFacetColumnSubplot"]
    Tree["EvaluatedFacetTree\nPartitionNode hierarchy"]
    Band["FacetBandMeasurePipeline\nper-band measurement"]
    Domains["scale_precompute and domain_coordination"]
    Coordination["coordinate_facet_measurement_tree\nrequirements, retarget, final propagation"]
    Placement["FacetBandPlacement\nfacet_child_frame_placement_from_band"]
    Render["render_facet_band_common\nchild plot groups"]

    Compile --> Tree
    Tree --> Band
    Domains --> Band
    Band --> Coordination
    Coordination --> Placement
    Placement --> Render
```

## Compile Path

Facet authoring uses the neutral `Subplot<C>` mark from `avenger-chart-marks`.
`FacetRow` and `FacetColumn` implement `SubplotContainerCoordinateSystem` in
`facet/marks/facet.rs`. Their compile hooks build `CompiledSubplotPayload`
values and return `CompiledFacetRowSubplot` or `CompiledFacetColumnSubplot`.

Facet child plots inherit parent data through filtered per-cell data overrides.
Partitioned facet children do not use explicit child plot data as an
independent data source.

## Partition Tree

`EvaluatedFacetTree::from_compiled_plot` builds the facet hierarchy before plot
measurement. It discovers compiled facet subplot marks with `facet_subplot_ref`,
extracts distinct partition values with `PartitionKeyExtractor`, and builds a
tree of `PartitionNode` values.

```mermaid
flowchart TD
    Data["Plot or mark DataFrame"]
    Dimensions["PartitionDimensionSpec\nFacetDirection, sharing, field_expr"]
    Extractor["PartitionKeyExtractor"]
    Root["PartitionNode"]
    Content["PartitionContent\nLeaf or Branch"]
    Tree["EvaluatedFacetTree"]
    Queries["path predicates, slot membership,\naxis visibility, value enumeration"]

    Data --> Extractor
    Dimensions --> Extractor
    Extractor --> Root
    Root --> Content
    Content --> Tree
    Tree --> Queries
```

`PartitionDimensionSpec` describes one facet dimension while the tree is being
built. `PartitionNode` stores the facet direction, sharing level, field name,
field expression, observed values, and `PartitionContent`. `PartitionCellPlan`
is the per-cell metadata used by measurement and rendering.

`EvaluatedFacetTree` caches path metadata, predicates, slot membership,
enumerations, jagged-axis checks, and channel-domain sharing levels. Guide,
legend, domain, and render code query the tree instead of recomputing
partition relationships.

## Measurement

`FacetColumn` and `FacetRow` both use the shared per-band measurement pipeline
in `facet/coord.rs`. The pipeline resolves the active facet node, precomputes
scale/domain artifacts, builds cell semantics, prepares child plots, probes
overflow, computes local layout, and assembles `FacetBandCoordMeasurement`.

`FacetBandCoordMeasurement` is the central measured facet value. It contains
the measured cells, band scale, local layout, measured overflow, coordination
state, placement model, empty-cell policy, child-frame path prefix, and
compiled child plot.

## Coordination And Placement

`coordinate_facet_measurement_tree` runs the cross-band coordination pipeline.
The driver is in `facet/coordination.rs`, immutable plans live in
`facet/coordination_plans.rs`, bounded mutation walkers live in
`facet/coordination_apply.rs`, and sizing decisions live in
`FacetCoordinationPolicy`.

The coordination stages are:

- initial requirement collection and distribution,
- retarget planning and execution,
- retargeted requirement collection,
- final propagation.

Rendering resolves `FacetBandPlacement` through `facet/placement.rs`, converts
that placement to child-frame render placements, and calls
`CompiledPlot::build_plot_components` for each renderable cell.

## Sharing

Facet slot sharing and scale-domain sharing both use `Sharing` at the API
boundary and `SharingLevel` internally. `facet/sharing_policy.rs` combines
sharing primitives with facet path metadata to decide domain grouping, axis
label ownership, axis title ownership, and legend ownership.

Facet measurements also feed the generic child-frame path with
`ContainerPathSegment::FacetValue`, so nested child-frame guide/domain/legend
sharing can cross facet and non-facet containers. See
[layout-and-child-frames.md](layout-and-child-frames.md).
