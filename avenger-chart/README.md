# avenger-chart

Prepare an `avenger-chart-definition`, query its dataflow, and produce immutable rendered frames. The runtime uses `avenger-scales`, `avenger-guides`, `avenger-layout`, and `avenger-panels` to turn materialized outputs into scenegraph geometry.

## Lifecycle

```rust,ignore
let formatting = avenger_scales::formatter::ScaleFormatting::d3(
    Default::default(), Default::default(),
);
let chart = Chart::prepare(
    definition, ChartOptions::default().with_formatting(formatting),
).await?;
let frame = chart.render(RenderOptions::default()).await?;
std::fs::write("chart.svg", frame.to_svg()?)?;
std::fs::write("chart.pdf", frame.to_pdf()?)?;
std::fs::write("chart.png", frame.to_png(2.0).await?)?;
```

Preparation validates descriptors and prepares the dataflow. Rendering queries the required outputs, expands observed facet instances, resolves shared domains, measures guides, solves layout, and constructs a scene. Export reuses that frame and its text engine without querying again. The `svg`, `pdf`, and `png` Cargo features enable the respective exports and are enabled by default.

`ChartOptions` accepts a shared text engine, explicit scale formatting, and an existing dataflow `Runtime`. `with_formatting` configures text markup and scale labels from the same settings while preserving the supplied engine's font and layout settings. The default runtime supports the built-in transform codec and versions. Supply a configured runtime for other application functions or execution limits.

## Inputs and parameters

Chart parameter initial values seed `chart.inputs()`. It returns the native dataflow input builder, so table inputs, expression inputs, scope defaults, and per-instance overrides remain available.

```rust,ignore
let inputs = chart.inputs()?
    .table(&sales, sales_snapshot)?
    .at(&west_2025, |bindings| {
        bindings.scalar(&fraction, ScalarValue::Float64(Some(0.5)))
    })?
    .finish()?;
let frame = chart.render(RenderOptions::default()
    .inputs(inputs)
    .parameter("multiplier", ScalarValue::Float64(Some(2.0))))
    .await?;
```

A supplied `Inputs` snapshot is complete. Named render overrides apply to root chart parameters. Scoped values use native handles and addresses. The rendered frame retains the effective input snapshot, actual dataflow diagnostics, and resolved plot rectangles and scales.

## App integration

```rust,ignore
let mut app = chart.into_app(RenderOptions::default()).await?;
app.register_handler(config, handler);
// Pass app to an Avenger host, such as avenger-winit-wgpu.
```

Handlers receive `ChartAppState`. `set_parameter`, `set_parameters`, and `set_inputs` submit background renders. Parameter batches validate before replacing pending work. Equal parameter requests are ignored unless the last render failed. The last successful scene remains visible until the latest requested frame completes. Completion installs the axes, marks, scales, and effective inputs together. Read `last_error()` to report background failures.

`state.parameter(name)` reads the latest requested root value. `state.rendered()` reads the displayed frame. This distinction lets wheel events accumulate while an earlier request is still running. Background cancellation drops the dataflow consumer, allowing the dataflow's interest tracking to retain work needed by other consumers.

## Retained symbols

When both symbol positions bind fields through unclamped linear scales, the runtime retains their base position arrays and configured scales. Subsequent domain changes compute adjustments from those base scales. The original `ScalarOrArray` wrappers remain shared, including their cached hashes. Source snapshot changes rebuild positions. Ineligible encodings use full scale application.

Points outside the viewport remain in the arrays and are clipped by the renderer. Symbol size remains in screen space. The wgpu instanced-symbol path can reuse instance buffers when origin, canvas dimensions, clip, and style also remain unchanged. A real resize or style change can invalidate that renderer cache. Adjustment uniforms still require a small upload, and axes and text can require their own updates.

`frame.geometry_report()` records actual CPU position builds and reuses. `frame.report()` contains actual dataflow execution and caching diagnostics. Optional tracing in `avenger_wgpu::retained_symbols` reports the GPU-side operations. No cache behavior is inferred from physical-plan counts.

## Initial limits

The runtime supports the descriptor set documented by `avenger-chart-definition`. Position channels must evaluate to finite, non-null numbers. Filter invalid rows in the dataflow. Band scales currently accept the categorical types supported by the underlying scale kernels. Facets show observed combinations and do not reserve a Cartesian grid for missing keys.

Fixed guide reservations keep plot bounds stable during pan and zoom. Other plots measure guides and use bounded layout refinement. The runtime does not compile Vega-Lite, infer encodings, implement legends, or supply a chart interaction grammar.

See the `bars`, `facets`, and `pan_zoom` examples in `avenger-chart-definition` for complete programs.

Chart definitions also support point scales, null categorical values, category-step dimensions, centered rectangles, span spacing/minimums, scale-edge baselines, axis angles, and optional backgrounds. Unclipped marks contribute to the outer frame bounds. Clipped retained-symbol plots keep their existing geometry path. See [avenger-vegalite-compiler](../avenger-vegalite-compiler/README.md) for a Vega-Lite frontend using these descriptors.
