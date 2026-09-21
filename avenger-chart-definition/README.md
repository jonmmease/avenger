# avenger-chart-definition

Build portable chart definitions by binding an existing `Dataflow` to visual templates. The library owns descriptors, validation, and protobuf serialization. `avenger-chart` owns preparation, evaluation, layout, rendering, and app state.

## Construct a definition

Build the dataflow first. Publish the tables and scalars that the chart needs, then bind their typed output handles:

```rust,ignore
let mut chart = ChartDefinition::builder(dataflow);
chart.parameter("multiplier", &multiplier, ScalarValue::Float64(Some(1.0)))?;
chart.plot("sales", |plot| {
    plot.content_size(400.0, 240.0);
    let x = plot.scale("x", Scale::band(
        Domain::column(&bars, "category"), Range::PlotWidth,
    ).padding_inner(0.15))?;
    let y = plot.scale("y", Scale::linear(
        Domain::extent(&total_extent), Range::PlotHeightReversed,
    ).zero(true))?;
    plot.rect("bars", &bars, RectEncoding::new()
        .x(x.field("category")).width(x.bandwidth())
        .y(y.field("total")).y2(y.constant(0.0)))?;
    plot.axis(Axis::bottom(&x).title("Category"))?;
    plot.axis(Axis::left(&y).title("Sales"))?;
    Ok(())
})?;
let definition = chart.finish()?;
```

`finish` checks handle ownership, scope visibility, columns, binding types, local scale ownership, and layout declarations. It does not evaluate the dataflow or initialize a text engine. Invalid data values can still fail during rendering.

The initial descriptor set includes rectangles, circle symbols, linear and band scales, axes, group titles, and fixed or faceted groups. Numeric domains come from literal endpoints, two scalar outputs, or an extent struct with `min` and `max` fields. Band domains preserve the order of distinct non-null values from a table column or literal list. Sort that table in the dataflow when order matters.

## Compose plots and facets

`group` constructs fixed children. `facet` repeats its template for the observed instances of an immediate child dataflow scope. Callbacks execute once during construction.

```rust,ignore
chart.facet("regions", &regions, Arrangement::column(), |region| {
    region.title(Text::key("region"));
    region.facet("years", &years, Arrangement::row(), |year| {
        year.title(Text::key("year"));
        year.plot("sales", |plot| add_bars(plot, &bars, &extent))
    })
})?;
```

Bindings can read local or ancestor outputs. A facet needs a local or descendant output to request discovery. Use `discover_with` with a local scalar output when its visuals otherwise read only ancestor data. Nested scopes use the full typed key path. Missing combinations do not create empty cells. `KeyOrder` controls display order independently of identity.

`Arrangement` supports row, column, wrap, and explicit fixed grids. Track sharing uses `avenger-layout` policies. `share_domain` unions domains within the specified `PanelScope`. Axis label visibility and shared titles use `avenger-panels`. Ancestor depths count the expanded panel tree, including facet collection groups and facet instance groups. `PanelScope::Root` is often the simplest sharing policy.

## Serialization

```rust,ignore
let bytes = definition.to_bytes()?;
let decoded = ChartDefinition::from_bytes(&bytes, &dataflow_runtime)?;
```

The versioned protobuf artifact imports the dataflow and DataFusion messages directly. It stores the native dataflow, descriptors, parameter initial values, and references by scope path and name. Decoding creates a normal `ChartDefinition` with fresh handles. Retrieve handles from `decoded.dataflow().interface()` when supplying new bindings.

The artifact excludes evaluated query caches, text engines, expanded panels, configured scales, retained geometry, and GPU resources. Fixed snapshot sources use the dataflow's Arrow IPC assets. External sources follow the dataflow serializer's provider rules. Encoding does not bake external sources automatically.

For application UDFs, use `to_bytes_with_codec` and a runtime configured with the matching codec and function versions. For transforms, use `avenger_transform::TransformExtensionCodec` and `function_versions`. Compatibility follows the pinned DataFusion version and the dataflow artifact checks.

## Examples

Run from the repository root:

```sh
cargo run --release -p avenger-chart-definition --example bars
cargo run --release -p avenger-chart-definition --example facets
cargo run --release -p avenger-chart-definition --example facets -- --export
cargo run --release -p avenger-chart-definition --example facets -- --ragged
cargo run --release -p avenger-chart-definition --example pan_zoom
```

`bars` exports PNG, SVG, and PDF, round-trips the definition through protobuf, and renders a second parameter value. `facets` opens a nine-panel figure. Use `--export` to write PNG and SVG instead. `--ragged` removes one observed region/year combination. `--reverse` reverses source rows while preserving panel order. `pan_zoom` opens a fixed-size window with one million points. Use `--points 10000` for a smaller run.

The pan/zoom example spreads small, translucent points across a spiral and uses single-sample rendering. Retained buffers avoid position uploads, but the GPU still draws every point each frame. Larger symbols and multisampling increase that cost even when every buffer is reused.

The facet source has `region: Utf8`, `year: Int32`, `category: Utf8`, and `amount: Float64` columns. The dataflow partitions first by region, then by year, and sums each category locally. A root output supplies the ordered category domain. Local extent outputs supply the shared numeric domain. The sparse variant compacts the remaining years within that region rather than reserving a cell for the missing year.

For actual retained-array, renderer-cache, buffer-allocation, and uniform-copy diagnostics:

```sh
RUST_LOG=avenger_chart=debug,avenger_wgpu::retained_symbols=debug \
  cargo run --release -p avenger-chart-definition --example pan_zoom
```

The definition library has no chart-runtime or renderer dependency. Those dependencies are used only by its examples and tests.
