# View-Dependent Marks Implementation Plan

This is a future-work implementation plan for the raster/datashader/M4 design
work. Each section should be implementable independently.

## Section 1: Move Domain Inference Config To Channel Level

Goal: replace the current mark-level `exclude_from_scale_domains` behavior with
precise per-channel control. This section should not implement `View`, and
should not add arbitrary `infer_domain_from(expr)` behavior.

### Implementation Checklist

- [x] Add `ScaleDomainInference` to `avenger-chart-core/src/channel_value.rs`.
- [x] Add `scale_domain_inference` to `ChannelValue::Scaled`.
- [x] Add `scale_domain_inference` to `ChannelValue::Conditional`.
- [x] Add the same metadata to scaled and conditional `PatternChannelValue`.
- [x] Add `ChannelValue` helper methods for reading and setting domain inference.
- [x] Add `exclude_from_scale_domain()` to `ScaleChannelConfig`.
- [x] Preserve `scale_domain_inference` through channel-value rewrites.
- [x] Gate automatic domain-entry collection in the scale builder per channel.
- [x] Remove mark-level `exclude_from_scale_domains` state and generated API.
- [x] Update compile errors from changed `ChannelValue` enum fields.
- [x] Update tests that used the old mark-level API.
- [x] Add focused tests for channel-level exclusion.
- [x] Add DSL `domain_contribution: infer | exclude` lowering and editor support.
- [x] Reject inferred scales whose only matching channels are excluded.
- [x] Run focused validation commands.

### Public Semantics

Default behavior:

```text
Scaled channel -> participates in automatic scale-domain inference
Value/no_scale channel -> does not participate
```

Add channel-config API:

```rust
.x_with(col("x"), |x| x.exclude_from_scale_domain())
.fill_with(col("count"), |c| c.exclude_from_scale_domain())
```

This means:

```text
The channel still renders normally.
The channel can still configure/use a scale.
The channel's scaled input expression is not collected for automatic domain inference.
Explicit scale domains and raw domains are unaffected.
```

Remove the mark-level API entirely:

```rust
.exclude_from_scale_domains()
```

No compatibility shim is needed.

### Core Data Model

- [x] Implement this data model.

In `avenger-chart-core/src/channel_value.rs`, add:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleDomainInference {
    #[default]
    Infer,
    Exclude,
}
```

Add a `scale_domain_inference: ScaleDomainInference` field to:

```rust
ChannelValue::Scaled
ChannelValue::Conditional
PatternChannelValue::Scaled
PatternChannelValue::Conditional
```

The literal `Value` variants need no field because they never feed a scale
domain. Pattern-channel scalar surrogates preserve the pattern channel's
policy.

Add helpers on `ChannelValue`:

```rust
pub fn scale_domain_inference(&self) -> ScaleDomainInference
pub fn participates_in_scale_domain_inference(&self) -> bool
pub fn exclude_from_scale_domain(self) -> Self
pub fn with_scale_domain_inference(self, mode: ScaleDomainInference) -> Self
```

Important: do not make `scale_input_expr()` return `None` for excluded
channels. That method is also used for type inference and diagnostics. Gate
automatic domain collection explicitly in the scale builder instead.

### Channel Config API

- [x] Implement the channel config API.

In `avenger-chart-core/src/scale_channel_config.rs`, add this to
`ScaleChannelConfig`:

```rust
fn exclude_from_scale_domain(mut self) -> Self {
    let value = self.get_value().clone().exclude_from_scale_domain();
    self.set_value(value);
    self
}
```

Because `ScaleChannelConfig` has a blanket impl for `ChannelConfig`, this makes
the method available in existing `_with` closures for position, color, size, and
other scaled channel configs.

Do not add a public positive `include_in_scale_domain()` yet unless
implementation pressure appears. Default include is enough.

### Mechanical Updates

- [x] Update all `ChannelValue::Scaled` construction sites.
- [x] Update all `ChannelValue::Conditional` construction sites.
- [x] Update all pattern matches that rebuild `Scaled` or `Conditional`.
- [x] Ensure channel resolution preserves `scale_domain_inference`.
- [x] Ensure repeat resolution preserves `scale_domain_inference`.
- [x] Ensure conditional branch helpers preserve `scale_domain_inference`.
- [x] Ensure pattern-channel scalar surrogates preserve
      `scale_domain_inference`.
- [x] Ensure scale/channel config helpers preserve `scale_domain_inference`.
- [x] Ensure transform output channel handles preserve or default
      `scale_domain_inference` correctly.

Update all constructors and pattern matches for `ChannelValue::Scaled` and
`ChannelValue::Conditional`. Preserve `scale_domain_inference` whenever a channel
value is rebuilt.

Likely hot files:

```text
avenger-chart-core/src/channel_value.rs
avenger-chart-core/src/channel_config.rs
avenger-chart-core/src/scale_channel_config.rs
avenger-chart-core/src/channel_resolution.rs
avenger-chart-core/src/repeat.rs
avenger-chart/src/plot/compiled/mark_data_runtime.rs
avenger-chart-scales/src/mark_scale_builder.rs
avenger-chart-transforms/src/lib.rs
avenger-chart/src/lib.rs
```

When in doubt, the rule is:

```text
Operations that preserve the same authored channel should preserve scale_domain_inference.
Operations that create a new scaled channel from scratch should default to Infer.
Operations that convert to Value/no_scale drop the setting.
```

### Scale Builder Change

- [x] Remove the mark-level skip in scale-domain collection.
- [x] Add a per-channel skip before pushing automatic domain entries.
- [x] Confirm excluded channels can still create/configure scales.
- [x] Confirm explicit scale domains still work for excluded channels.
- [x] Confirm raw-domain params still work for excluded channels.

In `avenger-chart-scales/src/mark_scale_builder.rs`, remove the mark-level check:

```rust
if prepared.mark.state().exclude_from_scale_domains {
    continue;
}
```

Inside the channel loop that collects domain entries, add:

```rust
if !domain_channel_value.participates_in_scale_domain_inference() {
    continue;
}
```

This should only affect automatic data-entry collection for inferred domains.
Do not filter excluded channels when deciding:

```text
whether a scale exists
what scale type it has
what scale config applies
what axis/legend config exists
whether an explicit/raw domain exists
```

### Remove Mark-Level State

- [x] Remove `MarkState::exclude_from_scale_domains`.
- [x] Remove `CompiledMarkState::exclude_from_scale_domains`.
- [x] Remove default initialization of the field.
- [x] Remove serialization/deserialization references to the field.
- [x] Remove generated `.exclude_from_scale_domains()` from `impl_mark_base`.
- [x] Update manual `MarkState` constructors.
- [x] Remove or update docs/examples mentioning mark-level exclusion.

Remove `exclude_from_scale_domains` from:

```text
avenger-chart-core/src/mark_state.rs
avenger-chart-core/src/mark_macros.rs
CompiledMarkState
manual MarkState constructors such as avenger-chart-marks/src/subplot.rs
```

Delete the generated `.exclude_from_scale_domains()` method from
`impl_mark_base`.

Update all call sites/tests that use the mark-level method to use channel-level
exclusion, or remove them if they only tested the old coarse API.

### Tests

- [x] Add or update unit tests in `avenger-chart-core` for
      `ScaleDomainInference` serialization/defaults.
- [x] Add or update unit tests in `avenger-chart-core` proving
      `exclude_from_scale_domain()` survives channel-value transformations.
- [x] Add or update scale-builder tests proving per-channel exclusion affects
      automatic domain inference only.
- [x] Update any tests that currently call `.exclude_from_scale_domains()`.
- [x] Add an integration test where x is excluded but y still contributes.
- [x] Add an integration test where fill/color is excluded but x/y still
      contribute.
- [x] Add a rendering or mark-data test proving an excluded channel still
      renders normally.

Add focused tests proving:

1. Excluding only `x` prevents x-domain contribution but leaves `y`
   contributing.
2. Excluding `fill` prevents color-domain/legend-domain contribution without
   affecting x/y.
3. `exclude_from_scale_domain()` survives `.scale(...)`, `.share_domain()`,
   conditionals, and repeat resolution.
4. A channel can still render when excluded from domain inference.

Required test updates:

```text
Search for `.exclude_from_scale_domains()` and update every call site.

If the old test intended "this whole generated mark should not affect any
scale", update every scaled channel on that mark with
`.exclude_from_scale_domain()`.

If the old test only needed x/y exclusion, update only x/y.

If the old test merely tested the old mark-level method, remove or replace it
with a channel-level API test.
```

### DSL And Tooling

- [x] Accept `domain_contribution: infer | exclude` in every configured
      ordinary channel block.
- [x] Default omitted `domain_contribution` to `infer`.
- [x] Lower the property to `ScaleDomainInference`.
- [x] Remove `exclude_from_scale_domains` from generic native and defined-mark
      schemas without a compatibility alias.
- [x] Complete the property in channel blocks and complete both enum values.
- [x] Update the semantic schema, generated native-schema references, compiler
      fixtures, and LSP acceptance tests.

The DSL uses author-facing `domain_contribution` because it describes the
channel's role. Rust uses `ScaleDomainInference` because the metadata is an
execution policy on automatic domain collection.

If exclusion leaves a scale with no automatic contributor and no explicit,
raw, or replacement domain, compilation/evaluation must report a targeted
error. It must not silently materialize the scale with a neutral fallback.

Good validation commands:

```bash
cargo test --release -p avenger-chart-core channel_value
cargo test --release -p avenger-chart-scales
cargo test --release -p avenger-chart --test test_mark_effects
cargo test --release -p avenger-chart
```

Validation checklist:

- [x] `cargo test --release -p avenger-chart-core channel_value`
- [x] `cargo test --release -p avenger-chart-scales`
- [x] `cargo test --release -p avenger-chart --test test_mark_effects`
- [x] `cargo test --release -p avenger-chart`

## Section 2: Add `View` Transform As A Standalone Feature

Goal: add a public pass-through `View` data transform that exposes the current
coordinate view to downstream marks/transforms and creates the coordinate-owned
domain fence needed by later M4/datashader/raster work. This section should not
implement M4, `Bin2D`, or a raster mark.

Important design goal: view transforms must be definable in external coordinate
crates. `View<Cartesian>` should be the first implementation, but the compiled
view machinery must not be a closed enum of built-in coordinate systems. Avoid
any core shape like `enum CompiledViewKind { Cartesian, WebMercator, ... }`.
External coordinate crates should register themselves through typetag in the
same spirit as `CompiledDataTransform` and `CoordinateSystemTransform`.

### Public API Sketch

The intended user-facing shape for the first implementation is:

```rust
Symbol::new()
    .transform(
        View::<Cartesian>::new()
            .x_domain(col("x"))
            .y_domain(col("y")),
        |symbol, view| {
            symbol
                .x(view.x().domain_mid())
                .y(view.y().domain_mid())
                .size(view.x().pixels() / lit(4.0))
        },
    )
    .fill(col("group"))
```

Provide `View::cartesian()` as sugar if that is ergonomic, but keep the
underlying public shape compatible with coordinate-specific/external views:

```rust
View::<Cartesian>::new()
View::<ExternalCoord>::new()
```

An external coordinate crate should be able to expose its own view constructor
and output handle without changing `avenger-chart` or
`avenger-chart-transforms`:

```rust
ExternalView::new()
    .u_domain(col("u"))
    .v_domain(col("v"))
```

Treat a generic `View<C>` builder as optional authoring sugar, not as the
extension boundary. The real extension boundary is the compiled
`CompiledViewTransform` trait described below. A coordinate crate may either:

```text
1. implement a shared authoring trait used by View<C>, if we add one, or
2. define its own authoring transform type, such as ExternalView, that compiles
   to CompiledViewDataTransform.
```

The second path must work first because it is the least coupled path for
external coordinate crates.

For this standalone section, expose a small set of symbolic handles:

```rust
view.x().domain_start()
view.x().domain_end()
view.x().domain_span()
view.x().domain_mid()

view.y().domain_start()
view.y().domain_end()
view.y().domain_span()
view.y().domain_mid()

view.x().range_start()
view.x().range_end()
view.x().range_span()
view.x().pixels()

view.y().range_start()
view.y().range_end()
view.y().range_span()
view.y().pixels()
```

Defer `pixel_buckets()` until a later section. The methods above are enough to
prove runtime view-state plumbing, serialization, and domain-fence semantics.

### Rules Decided So Far

- [ ] `View::cartesian()` is a pass-through transform: it does not add, remove,
      reorder, or aggregate dataframe rows.
- [ ] `View::cartesian().x_domain(expr)` declares the automatic domain input for
      Cartesian x position scales, evaluated at the input to the `View`
      transform.
- [ ] `View::cartesian().y_domain(expr)` declares the automatic domain input for
      Cartesian y position scales, evaluated at the input to the `View`
      transform.
- [ ] Cartesian x/y channels authored inside the `View` closure are render-only
      and do not participate in automatic domain inference.
- [ ] Non-position channels authored inside the `View` closure still participate
      in automatic domain inference normally.
- [ ] Channels authored after the `.transform(View::cartesian(), ...)` call are
      ordinary channels and participate in automatic domain inference normally.
- [ ] `x`, `x2`, and other x-family Cartesian position channels count as x
      channels for this rule.
- [ ] `y`, `y2`, and other y-family Cartesian position channels count as y
      channels for this rule.
- [ ] If x/y channels are authored inside a `View` closure and the corresponding
      `x_domain`/`y_domain` is omitted, return a clear validation error unless
      that scale has an explicit/raw domain that makes automatic inference
      unnecessary.
- [ ] For v1, reject custom scale names on x/y channels inside a Cartesian
      `View` closure unless the implementation also adds explicit
      `x_scale(...)`/`y_scale(...)` support.
- [ ] The core view-transform implementation must be external-crate extensible.
      Do not pattern-match on concrete built-in view transform types in
      `avenger-chart`.

Important user-facing simplification:

```text
Users should not configure where in the transform pipeline domain inference
happens. The View transform owns that boundary. Domain expressions on View are
always evaluated at View input.
```

### Crate Placement And Dependency Contract

- [ ] Put engine-facing view traits and structs in `avenger-chart-core`.
- [ ] Keep `CompiledViewTransform`, `CompiledViewDataTransform`,
      `ViewDomainFenceSpec`, `ViewRequirements`, and
      `DataTransformViewContext` free of dependencies on `avenger-chart`.
- [ ] Allow coordinate crates to implement `CompiledViewTransform` with only
      `avenger-chart-core`, their coordinate crate dependencies, and normal
      transform dependencies.
- [ ] Let `avenger-chart-transforms` provide the first Cartesian authoring
      wrapper because it already depends on `avenger-chart-cartesian`.
- [ ] Do not make external coordinate crates edit a central registration list.

This mirrors the current crate split:

```text
avenger-chart-core
  owns serializable trait-object contracts

avenger-chart-cartesian / external-coordinate-crate
  owns coordinate-specific transform behavior

avenger-chart-transforms
  may own shared authoring helpers and built-in transform wrappers

avenger-chart
  owns high-level plot compilation, scale building, and runtime view-context
  construction
```

If we later want the Cartesian view to live beside the Cartesian coordinate
instead of in `avenger-chart-transforms`, this trait design still allows that.
The important constraint is that the engine sees only
`CompiledViewDataTransform`.

### Core Trait And Typetag Design

Use a trait-based design for compiled view transforms. The preferred shape is a
core adapter that implements ordinary `CompiledDataTransform` plus a nested
typetagged view trait:

```rust
#[typetag::serde(tag = "type")]
pub trait CompiledViewTransform: Send + Sync {
    fn clone_box(&self) -> Box<dyn CompiledViewTransform>;

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledViewTransform>, AvengerChartError>;

    fn validate_coordinate(
        &self,
        coord: &dyn CoordinateSystemTransform,
    ) -> Result<(), AvengerChartError>;

    fn domain_fence(&self) -> ViewDomainFenceSpec;

    fn view_requirements(&self) -> ViewRequirements;

    fn materialize_view_scalars(
        &self,
        context: &DataTransformViewContext<'_>,
    ) -> Result<DerivedScalarMap, AvengerChartError>;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledViewDataTransform {
    view: Box<dyn CompiledViewTransform>,
}
```

Use `CoordinateSystemTransform`, not only `CoordinateSystemTransformCore`, for
coordinate validation. The existing coordinate transform trait already exposes
`as_any()`, so external compiled views can downcast to their matching coordinate
type without core knowing the concrete type:

```rust
fn validate_coordinate(
    &self,
    coord: &dyn CoordinateSystemTransform,
) -> Result<(), AvengerChartError> {
    if coord.as_any().is::<ExternalCoordTransform>() {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(
            "ExternalView can only be used with ExternalCoord".to_string(),
        ))
    }
}
```

`CompiledViewDataTransform` implements `CompiledDataTransform` once in core.
Its `apply` implementation should:

```text
1. require render-time view context
2. call inner.materialize_view_scalars(...)
3. return the input dataframe unchanged
4. return the derived scalars from the inner view transform
```

This keeps `DataTransformStage` unchanged because it still stores
`Box<dyn CompiledDataTransform>`, while external coordinate crates can implement
their own typetagged `CompiledViewTransform` and wrap it in
`CompiledViewDataTransform`.

Add a default hook to `CompiledDataTransform` so the mark-transform closure
plumbing can discover view fences without downcasting:

```rust
fn domain_fence(&self) -> Option<ViewDomainFenceSpec> {
    None
}
```

`CompiledViewDataTransform` should override this hook and delegate to
`self.view.domain_fence()`. This keeps the hook open to future transform-owned
domain behavior while avoiding concrete-type checks in `avenger-chart`.

Checklist:

- [ ] Add an object-safe `CompiledViewTransform` trait in `avenger-chart-core`.
- [ ] Mark `CompiledViewTransform` with `#[typetag::serde(tag = "type")]`.
- [ ] Add `clone_box` and `Clone for Box<dyn CompiledViewTransform>`.
- [ ] Add `CompiledViewDataTransform` adapter in `avenger-chart-core`.
- [ ] Add `CompiledViewDataTransform::new(view: Box<dyn CompiledViewTransform>)`.
- [ ] Implement `CompiledDataTransform` for `CompiledViewDataTransform`.
- [ ] Mark the adapter impl as `#[typetag::serde(name = "view")]`.
- [ ] Implement `map_exprs` on the adapter by delegating to
      `view.map_exprs(...)`.
- [ ] Add a default `domain_fence()` hook to `CompiledDataTransform`.
- [ ] Override `domain_fence()` on `CompiledViewDataTransform`.
- [ ] Ensure nested serialization works for
      `CompiledViewDataTransform { view: Box<dyn CompiledViewTransform> }`.
- [ ] Ensure external crates can implement `CompiledViewTransform` without
      depending on `avenger-chart`.
- [ ] Validate the compiled view against the active
      `CoordinateSystemTransform` before applying the fence or executing the
      transform.
- [ ] Do not require external view transforms to be registered in a central enum.

Serialization note:

```text
The serialized transform stage should look like an ordinary data transform of
type "view", with a nested typetagged view payload. The nested payload's type
name is owned by the coordinate/view crate, e.g. "cartesian" or
"external_coord_view".
```

### Implementation Checklist

- [ ] Add a `view` module to `avenger-chart-transforms`.
- [ ] Export `View`, `CartesianView`, `CartesianViewOutput`, and
      `CartesianViewAxis` from `avenger-chart-transforms/src/lib.rs`.
- [ ] Re-export the user-facing types from `avenger-chart/src/prelude.rs`.
- [ ] Implement `View::cartesian()`.
- [ ] If implementing generic `View<C>`, make it delegate through an open trait
      that external coordinate crates can implement for their local coordinate
      type.
- [ ] Ensure a coordinate crate can skip generic `View<C>` entirely and expose
      its own `ExternalView` authoring transform.
- [ ] Implement `.x_domain(expr)` and `.y_domain(expr)`.
- [ ] Implement `DataTransform` for the Cartesian view transform.
- [ ] Implement a serializable `CompiledCartesianViewTransform` that implements
      `CompiledViewTransform`, not directly `CompiledDataTransform`.
- [ ] Have the Cartesian authoring transform return
      `CompiledViewDataTransform::new(Box::new(CompiledCartesianViewTransform))`.
- [ ] Make the view adapter pass input data through unchanged.
- [ ] Produce derived scalar placeholders from the output handle methods.
- [ ] Produce matching derived scalar values during render-time transform
      execution.
- [ ] Add internal metadata through `CompiledViewTransform::domain_fence()`.
- [ ] Update mark transform plumbing so a domain fence can inspect channels
      changed inside its closure.
- [ ] Mark x/y-family channels changed inside the closure as excluded from
      automatic domain inference.
- [ ] Register x/y domain replacement contributions from the `View` transform.
- [ ] Ensure channels chained after the View transform are not marked as
      render-only by the View fence.
- [ ] Add validation for missing x/y replacement domains.
- [ ] Add validation/error behavior for unsupported custom scale names.
- [ ] Add a short internal example or test fixture showing the external view
      implementation pattern.

### Runtime View State

The current `DataTransformExecutionContext` does not expose resolved scales,
ranges, or plot dimensions. `View` needs render-time view state.

- [ ] Add an optional view-state field to `DataTransformExecutionContext`.
- [ ] Keep the field generic enough for core to own without depending on
      `avenger-chart-scales` concrete configured-scale types.
- [ ] Include the active coordinate transform in the view context as
      `&dyn CoordinateSystemTransform`, not just
      `&dyn CoordinateSystemTransformCore`, so external view transforms can
      validate/downcast through `as_any()`.
- [ ] Suggested core shape:

```rust
pub struct DataTransformViewContext<'a> {
    pub coord_transform: &'a dyn CoordinateSystemTransform,
    pub plot_area_width: f64,
    pub plot_area_height: f64,
    pub scales: &'a IndexMap<String, DataTransformViewScale>,
    pub scale_to_coord_channel: &'a IndexMap<String, String>,
    pub params: &'a IndexMap<String, ScalarValue>,
}

pub struct DataTransformViewScale {
    pub scale_name: String,
    pub coord_channel: Option<String>,
    pub domain_start: ScalarValue,
    pub domain_end: ScalarValue,
    pub range_start: f64,
    pub range_end: f64,
}
```

- [ ] During render mark-data preparation, build this view context from the
      resolved configured scales, scale-to-coordinate-channel mapping, plot
      width, plot height, params, and active coordinate transform.
- [ ] During scale-domain inference, do not require resolved view state. Domain
      inference should use the View domain-source expressions and should not
      execute downstream view-dependent x/y render channels.
- [ ] If a `View` transform is executed at render time without the required
      view-state entry, return a clear error.

`View` output handles should use the existing derived-scalar pattern used by
`BinOutput`: public methods return `Expr`/`ChannelExpr` placeholders, and the
compiled transform returns matching runtime-derived scalars.

External coordinates should not need new core runtime fields for every custom
view. Prefer a generic view context containing resolved scale states, plot
dimensions, params, and coordinate metadata; then let each compiled view
transform decide how to materialize its own scalars. If a coordinate needs more
than domain/range/plot-size information, first look for an existing
coordinate-owned provider hook before adding a core field.

### Domain-Fence Plumbing

This section likely needs a small internal generalization of mark transform
plumbing. The generic `transform_with_scope` method currently:

```text
1. compiles the transform
2. appends it to the mark's data transforms
3. runs the user closure
```

For `View`, add a default no-op hook to compiled transforms or data-transform
stages, for example:

```rust
fn domain_fence(&self) -> Option<ViewDomainFenceSpec> {
    None
}
```

Then update `transform_with_scope` to:

```text
1. snapshot mark channels before running the closure
2. compile and append the transform
3. run the user closure
4. if the transform exposes a domain fence, compare before/after channels
5. apply the fence rules to channels changed by the closure
```

For `View<Cartesian>`, applying the fence means:

```text
changed x-family channels -> exclude_from_scale_domain()
changed y-family channels -> exclude_from_scale_domain()
domain contribution for x -> View.x_domain expression at View input
domain contribution for y -> View.y_domain expression at View input
all other changed channels -> leave unchanged
```

The domain contribution should be stored separately from render channels. Do
not encode it as a normal visible mark channel. The scale builder should see
the replacement x/y domain expressions while mark rendering should use the
actual render channels authored inside the closure.

Because View domain expressions are evaluated at View input, the runtime data
preparation path must capture domain-source dataframes while it walks the
transform pipeline:

```text
1. before applying each transform stage, check stage.transform.domain_fence()
2. if the fence has replacement domain sources, snapshot the current dataframe
   and current accumulated derived scalars
3. apply the transform as usual
4. keep rendering channels pointed at the final dataframe after all transforms
```

This matters even though v1 View is pass-through: a later transform in the same
mark pipeline should not accidentally change the View replacement domain
source. The captured dataframe should already reflect upstream transforms and
facet filtering that occur before the View stage.

Important: a single `domain_dataframe/domain_channels` pair is probably not
precise enough once View is present. A mark may need x/y domain contributions
from View input while fill/color domain contributions still come from render
data after View. Prefer an explicit list of prepared domain contributions:

```rust
pub struct PreparedDomainContribution {
    pub scale_name: String,
    pub dataframe: Option<DataFrame>,
    pub channel_value: ChannelValue,
    pub derived_scalars: DerivedScalarMap,
}
```

The scale builder should collect from these explicit contributions when present
and continue using normal channel collection for ordinary marks.

Checklist for this runtime change:

- [ ] Extend `apply_mark_data_transforms` to return captured domain
      contributions in addition to final dataframe and derived scalars.
- [ ] Extend `PreparedLogicalMarkData` and `PreparedScaleMark` with
      `domain_contributions: Vec<PreparedDomainContribution>`.
- [ ] Populate View replacement contributions from the dataframe immediately
      before the View stage.
- [ ] Preserve normal post-transform domain collection for channels that are
      not replaced by the View fence.
- [ ] Teach the scale builder to prefer explicit `domain_contributions` when
      present and fall back to the legacy domain dataframe/channel pair for
      ordinary marks during the transition.

External view transforms should return a `ViewDomainFenceSpec` that describes:

```text
which authored channels/families become render-only
which scale names receive replacement domain expressions
which coordinate compatibility validation should run
```

Do not hard-code this logic to Cartesian in the generic plumbing. Cartesian is
only the first provider of the trait.

Suggested generic shape:

```rust
pub struct ViewDomainFenceSpec {
    pub coordinate_label: String,
    pub render_only_channel_families: Vec<ViewChannelFamily>,
    pub replacement_domain_sources: Vec<ViewDomainSource>,
}

pub struct ViewChannelFamily {
    pub default_scale_name: String,
    pub channel_family: String,
}

pub struct ViewDomainSource {
    pub scale_name: String,
    pub channel_name: String,
    pub value: ChannelValue,
}
```

For Cartesian, `render_only_channel_families` contains x and y families, and
`replacement_domain_sources` contains the authored `x_domain` and `y_domain`
expressions. Other coordinates can use different families/scale names without
engine changes. The exact struct names can change during implementation, but
the contract should stay declarative and serializable/debuggable.

### Tests That Do Not Require Raster/M4/Bin2D

- [ ] Unit test: compiled `View::cartesian()` serializes/deserializes.
- [ ] Unit test: `CompiledViewDataTransform` serializes/deserializes with a
      nested typetagged `CompiledViewTransform`.
- [ ] Unit test: `CompiledViewDataTransform::map_exprs()` rewrites nested view
      domain expressions.
- [ ] Unit test: `CompiledViewDataTransform::domain_fence()` delegates to the
      nested view transform without downcasting.
- [ ] Unit test: `View::cartesian()` passes dataframe rows and columns through
      unchanged.
- [ ] Unit test: `CartesianViewOutput` methods reference stable derived scalar
      ids.
- [ ] Unit test: executing a `View` transform with a stub view context produces
      domain/range/pixel derived scalars.
- [ ] Unit test: executing a `View` transform without required view context
      errors clearly.
- [ ] Integration test: x/y channels authored inside a `View` closure do not
      contribute to x/y automatic domains.
- [ ] Integration test: `x_domain`/`y_domain` expressions on `View` replace the
      x/y automatic domain inputs.
- [ ] Integration test: a fill/color channel authored inside the `View` closure
      still contributes to color domain/legend inference.
- [ ] Integration test: a fill/color channel authored after the `View` closure
      still contributes to color domain/legend inference.
- [ ] Integration test: an x or y channel authored after the `View` closure is
      ordinary and participates in domain inference.
- [ ] Integration test: omitting `x_domain` while authoring x inside the View
      closure produces a clear error.
- [ ] Integration test: omitting `y_domain` while authoring y inside the View
      closure produces a clear error.
- [ ] Integration test: using `view.x().domain_mid()` and
      `view.y().domain_mid()` renders a symbol at the center of the inferred
      data domain.
- [ ] External-crate test: define a tiny test-only external coordinate/view
      transform that implements `CompiledViewTransform`, round-trips through
      serialization, and executes through `DataTransformStage`.
- [ ] External-crate test: the tiny external view validates against its matching
      coordinate and rejects a mismatched coordinate with a clear error.
- [ ] External-crate test: the tiny external view contributes a non-Cartesian
      domain fence, proving the engine does not special-case x/y.

Concrete domain-fence test idea:

```rust
let plot = Plot::<Cartesian>::new()
    .data(df_with_x_y_0_to_10)
    .mark(
        Symbol::new()
            .transform(
                View::cartesian()
                    .x_domain(col("x"))
                    .y_domain(col("y")),
                |symbol, _view| {
                    symbol
                        .x_with(lit(1_000_000.0), |x| {
                            x.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        })
                        .y_with(lit(1_000_000.0), |y| {
                            y.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        })
                },
            ),
    );
```

Build scales and assert the x/y domains come from `col("x")` and `col("y")`,
not from the render-only `1_000_000` values.

Concrete "outside channels are ordinary" test idea:

```rust
let mark = Symbol::new()
    .transform(
        View::cartesian()
            .x_domain(col("x"))
            .y_domain(col("y")),
        |symbol, view| {
            symbol
                .x(view.x().domain_mid())
                .y(view.y().domain_mid())
        },
    )
    .fill(col("group"));
```

Assert the fill legend/color domain includes the `group` values.

### Visual Baselines That Do Not Require Later Features

Add visual tests under a new category such as `view`.

- [ ] `view_domain_center_symbol`: use `view.x().domain_mid()` and
      `view.y().domain_mid()` to draw one symbol in the plot center.
- [ ] `view_domain_frame`: use `Rect` or four `Rule` marks inside a
      `View::cartesian()` closure to draw the current inferred data-domain
      boundary.
- [ ] `view_fill_inside_closure_legend`: set x/y from view handles and set
      `fill(col("group"))` inside the View closure; verify the legend appears.
- [ ] `view_fill_after_closure_legend`: set x/y from view handles inside the
      View closure and chain `fill(col("group"))` afterward; verify the legend
      appears.
- [ ] `view_pixel_size_symbol`: with fixed plot size, set symbol size from
      `view.x().pixels()` or `view.y().pixels()` to prove plot-size view
      scalars reach render channels.

Keep these baselines visually simple and deterministic. Use fixed plot size and
linear x/y scales with `nice(false)` and `zero(false)` where exact placement
matters.

Suggested validation commands:

```bash
cargo test --release -p avenger-chart-transforms view
cargo test --release -p avenger-chart --test core_integration view
cargo test --release -p avenger-chart visual_regression view -- --nocapture
```

Validation checklist:

- [ ] `cargo test --release -p avenger-chart-transforms view`
- [ ] `cargo test --release -p avenger-chart --test core_integration view`
- [ ] `cargo test --release -p avenger-chart visual_regression view -- --nocapture`

## Section 3: Retained View Results And `preview_cached`

Goal: add a small public flag on `View` that permits low-latency reuse of a
retained View-dependent result during Preview, while designing the runtime
concept broadly enough to later support async raster/datashader/M4
materialization.

This section should build on Section 2. It should not replace the existing
Preview layout/profile system. Instead, it adds a more local retained-result
boundary that Preview can use when the whole-plot data-mark reuse gate is too
coarse.

### Current System Observations

- [ ] Treat these observations as implementation constraints.

Current Preview already supports a coarse version of this idea:

```text
1. PlotSession stores the last exact LayoutProfileSnapshot.
2. Preview clones/retargets that measurement.
3. Preview refreshes scales/domains for current params.
4. Preview may reuse cached PlotComponents data marks.
5. Reused scene marks are retargeted through old-scale -> new-scale affine
   adjustments.
```

Useful existing hooks:

```text
PlotSession
  owns durable evaluation caches and last exact layout profile.

LayoutProfileSnapshot.rendered_components
  stores whole PlotComponents for coarse Preview data-mark reuse.

ConfiguredScale::adjust(...)
  proves whether old-scale -> new-scale can be represented by a
  LinearScaleAdjustment. Today, linear scales support this path; unsupported
  scales fail gracefully.

retarget_cached_data_marks_for_plot_area(...)
  retargets cached scene marks using x/y scale adjustments and returns None
  when a mark cannot be retargeted safely.

SceneImageResource { key, fallback_key }
  already models desired image plus fallback image for renderer-side resource
  substitution.

RenderInvalidationHub
  already lets async resource completion request another render. Producers call
  RenderInvalidationSink::request_render(RenderInvalidationRequest), and hosts
  subscribe through RenderInvalidationCallback so they can request a repaint or
  redraw.
```

Important limitations:

```text
Current Preview data-mark reuse is all-or-nothing at PlotComponents level.
Current Preview does not compute missing mark results in the background.
Current image resource fallback is image-only.
M4/vector payloads need either materialized-data access during reevaluation or
  a more general resource-like scene payload.
```

### Public API

Add a boolean flag to View authoring:

```rust
View::cartesian()
    .id("density")
    .x_domain(col("x"))
    .y_domain(col("y"))
    .preview_cached(true)
```

This is intentionally a permission flag, not a guarantee:

```text
During EvaluationMode::Preview, this View boundary may render the last ready
View-dependent result, retargeted into the current view, while the desired
result is unavailable or expensive to recompute.

If no retained result exists, or if retargeting cannot be proven correct, the
runtime falls back to normal synchronous evaluation.
```

Do not expose throttle/debounce/pending-policy API in this section. Those can
grow later from the same retained-boundary model. The v1 user-facing API is
only:

```rust
.preview_cached(bool)
```

Suggested defaults:

```text
preview_cached(false) for ordinary View.
Component marks such as future datashader/M4 wrappers may set it internally or
expose a domain-specific preview option.
```

### Unified Runtime Concept

The internal abstraction is a retained View result boundary:

```text
desired result
  the result for the current exact View key

last ready result
  the most recent completed result for the same boundary and compatible
  non-view dependencies

retarget
  display last ready result through the current view/scales
```

One decision procedure should cover both Preview reuse and later async pending
results:

```text
if desired result is ready:
    render desired result
else if retained display is permitted and last ready result can be retargeted:
    render retargeted last ready result
    ensure desired result is queued/running when async materialization exists
else:
    fall back to normal synchronous evaluation, clear, or placeholder depending
    on the future policy
```

For v1 `preview_cached(true)`, the fallback is normal synchronous evaluation.
That keeps correctness first and makes the flag safe to enable incrementally.

### Keys And Fingerprints

A retained boundary needs two identities, not one:

```text
ViewResultKey
  Exact identity of the desired result. Includes current view state such as
  x/y domains, scale ranges, plot pixel size, View transform spec, downstream
  transform specs, and relevant params.

ViewRetargetFamilyKey
  Identity of everything that must match for a last-ready result to be reused
  as a retargeted fallback. This excludes retargetable x/y view changes but
  includes data revisions, source logical-plan identity, upstream transforms,
  downstream transform specs, non-position encodings, style params, selections,
  stores, and time context.
```

This mirrors the existing Preview profile-key split where raw-domain-only
params are excluded from the profile dependency key because current scale
objects are compared directly.

Suggested runtime structures:

```rust
pub struct ViewBoundaryId {
    pub explicit_id: Option<String>,
    pub mark_path: Vec<usize>,
    pub transform_index: usize,
    pub facet_path: Vec<ScalarValue>,
}

pub struct ViewResultKey {
    pub boundary_id: ViewBoundaryId,
    pub exact_fingerprint: String,
}

pub struct ViewRetargetFamilyKey {
    pub boundary_id: ViewBoundaryId,
    pub non_view_fingerprint: String,
}

pub struct ViewRetargetState {
    pub coordinate_type: String,
    pub plot_area_width: f32,
    pub plot_area_height: f32,
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,
}

pub struct RetainedViewResult {
    pub result_key: ViewResultKey,
    pub family_key: ViewRetargetFamilyKey,
    pub source_view: ViewRetargetState,
    pub payload: RetainedViewPayload,
}
```

`ViewBoundaryId::explicit_id` should be encouraged for long-lived async or
component-mark usage. If no id is provided, use the compiled mark path plus
transform index. Faceted results must include the facet path or equivalent
sharing-owner path so independent cells do not reuse each other's retained
payloads.

### Payload Model

The retained boundary should be payload-neutral. Do not bake raster-specific
behavior into View.

Candidate payloads:

```rust
pub enum RetainedViewPayload {
    SceneMarks {
        marks: Vec<SceneMark>,
        event_datums: Vec<EvaluatedEventDatumRows>,
    },
    ImageResource {
        desired_key: ResourceKey,
        fallback_key: Option<ResourceKey>,
    },
    RecordBatch {
        key: MaterializationKey,
    },
}
```

For v1 synchronous retained Preview reuse, `SceneMarks` is the most direct
payload because it can reuse the existing scene-mark retargeting code.

For future datashader/raster, `ImageResource` can use the existing scenegraph
and WGPU image fallback path:

```text
desired_key = current viewport raster
fallback_key = last ready raster for this View boundary
```

For future M4/vector materialization, `RecordBatch` should be a typed
materialized payload in a shared materialization cache. Rendering then either:

```text
1. reevaluates the mark when the desired batch becomes ready, or
2. introduces a resource-like scene primitive for vector data.
```

### Retarget Capability

Retargeting is allowed only when the runtime can prove it.

For Cartesian v1:

```text
1. The coordinate type is the same.
2. The retained result and current evaluation use compatible x/y scale names.
3. Each needed scale pair supports ConfiguredScale::adjust(...).
4. The retained payload can apply affine x/y adjustments.
5. The ViewRetargetFamilyKey matches the current non-view fingerprint.
```

If any check fails, return `None` and fall back to synchronous evaluation.

Do not silently draw an unretargeted stale result. That is worse than blocking
because it looks like a correctness bug.

Implementation work:

- [ ] Factor the current scene-mark retargeting helpers so they can work from a
      smaller `ViewRetargetState`, not only full `ComponentsMeasurement`.
- [ ] Keep unsupported mark types as graceful misses.
- [ ] Keep unsupported scales as graceful misses.
- [ ] Record diagnostic metrics for retarget miss reasons.

### Runtime Ownership

`PlotSession` should own or reference the retained View result cache because:

```text
PlotSession is already the stateful runtime instance for repeated evaluation.
PlotSession already owns Preview/profile caches.
One-shot evaluation should not retain View results across calls.
```

Add to `PlotSession`:

```rust
retained_view_cache: RetainedViewCacheHandle
```

Add to `EvaluationContext` or `RenderContext`:

```rust
retained_view_cache: Option<RetainedViewCacheHandle>
evaluation_mode: EvaluationMode
```

The retained cache is runtime state only. It must not become part of
`CompiledPlot` serialization.

External coordinate/view crates should not depend on `avenger-chart`. They
should define serializable View/result/materialization specs in core-facing
terms, while the chart runtime provides the cache and executors.

### Phase 1: Synchronous Retained Preview Reuse

This is the first implementable slice. It does not add background computation.

Behavior:

```text
Exact evaluation:
  render normally.
  after a preview-cached View-dependent mark/region renders successfully,
  store its RetainedViewResult in PlotSession.

Preview evaluation with preview_cached(false):
  render normally.

Preview evaluation with preview_cached(true):
  build the current ViewResultKey and ViewRetargetFamilyKey.
  if an exact-key result is ready, use it.
  else if a family-key result is ready and retargetable, render the retargeted
  retained payload.
  else render normally and store the result when complete.
```

The first slice can cache rendered scene marks for a whole mark whose data
pipeline contains a `preview_cached` View. That is less general than arbitrary
sub-mark closure caching, but it proves the retained-boundary model without
inventing a whole new renderer.

Important implementation detail:

```text
The reuse check must happen before expensive View-dependent transforms run.
Otherwise preview_cached(true) would still pay the cost before discovering the
retained result.
```

This likely requires render preparation to identify the View boundary in the
mark's transform chain, compute the current View keys, and split the data
pipeline into:

```text
upstream transforms before View
View boundary
View-dependent transforms/channels after View
```

If a mark cannot expose such a split, it should not opt into retained Preview
reuse yet.

Checklist:

- [ ] Add `ViewStalePolicy::RetargetCached` to View authoring and compiled
      View metadata. `.preview_cached(true)` may remain builder sugar, but
      should not be stored separately.
- [ ] Serialize/deserialize the View stale policy.
- [ ] Add stable View boundary ids.
- [ ] Add `RetainedViewCacheHandle` to `PlotSession`.
- [ ] Thread retained cache and `EvaluationMode` through render/evaluation
      context.
- [ ] Add key/fingerprint builders for View boundaries.
- [ ] Add a retained scene-mark payload for v1.
- [ ] Factor scene-mark retargeting to operate from `ViewRetargetState`.
- [ ] Add a mark-render fast path that can return a retained retargeted payload
      before running expensive View-dependent work.
- [ ] Store successful exact/current rendered payloads back into the retained
      cache.
- [ ] Fall back to synchronous rendering when no retained result is available
      or retargeting is unsupported.

### Phase 2: Async Materialization

Once Phase 1 proves the retained boundary, add background materialization.

This should unify datashader/raster and M4:

```rust
pub struct MaterializationRequest {
    pub key: MaterializationKey,
    pub kind: String,
    pub spec: SerializedMaterializationSpec,
    pub output: MaterializationOutputKind,
    pub priority: f32,
}

pub enum MaterializationResult {
    Image(RgbaImage),
    RecordBatch(RecordBatch),
    Error(String),
}

pub trait MaterializationExecutor: Send + Sync {
    fn kind(&self) -> &'static str;

    async fn run(
        &self,
        request: MaterializationRequest,
        ctx: MaterializationExecutionContext<'_>,
    ) -> Result<MaterializationResult, MaterializationError>;
}
```

Evaluation should be able to:

```text
1. compute the desired materialization key for the current View.
2. register a request if the desired key is missing.
3. choose a last-ready fallback from the same ViewRetargetFamilyKey.
4. return immediately with a retained/retargeted display if permitted.
5. update cache state when the background result completes.
6. request redraw or lightweight reevaluation through RenderInvalidationHub.
```

Completion invalidation is required, not optional. When an async View
materialization result is accepted into the cache, the materialization runtime
must call the configured render invalidation sink:

```rust
render_invalidation_sink.request_render(RenderInvalidationRequest::now(
    RenderInvalidationReason::ResourceChanged {
        kind: "view-materialization",
    },
));
```

That call is what invokes host `RenderInvalidationCallback` subscriptions. The
host can then schedule its normal repaint/redraw path and evaluate the plot
again so the now-ready View result replaces the retained fallback.

If a completion is discarded because its generation/epoch is stale, it should
not request a rerender. If the result is accepted but still needs renderer-side
resource upload, the accepted cache update and the renderer resource update may
each request invalidation; coalescing should happen in `RenderInvalidationHub`
or the host event loop, not by silently skipping the View completion
notification.

Exact mode should eventually mean:

```text
layout, scales, guides, legends, params, and desired materialization keys are
canonical for the current request.
```

It should not necessarily mean:

```text
every async View-dependent payload is already ready.
```

That invariant is already captured in the async raster/M4 future-work docs and
should be preserved here.

Checklist:

- [ ] Add materialization request/result types that are not image-only.
- [ ] Add a materialization executor registry.
- [ ] Add cache states: Missing, Queued, Running, Ready, Error.
- [ ] Remember last-ready key per ViewRetargetFamilyKey.
- [ ] Add generation/epoch checks so late results do not overwrite newer state.
- [ ] Connect accepted materialization completion to
      `RenderInvalidationSink::request_render(...)` so subscribed
      `RenderInvalidationCallback`s fire.
- [ ] Ensure stale/discarded materialization completions do not request a
      rerender.
- [ ] For image outputs, map materialization keys to `SceneImageResource`
      `key`/`fallback_key`.
- [ ] For RecordBatch outputs, add mark-runtime access to materialized data by
      key.
- [ ] Decide whether vector completion triggers lightweight reevaluation or a
      renderer-level resource swap.

### Interaction With Existing Preview

Do not remove the current Preview profile reuse path.

The layers should be:

```text
Existing global Preview:
  reuses layout/profile/chrome and sometimes all top-level data marks.

Retained View boundary:
  reuses one View-dependent mark/region when the global data-mark reuse gate is
  too coarse or when async materialization is pending.
```

If the existing global Preview path reuses all data marks, the View cache may
not be consulted for that evaluation. That is fine. The View cache is most
valuable when:

```text
the chart can reuse layout/profile but must rebuild some data marks, and one
expensive View-dependent mark can be safely retained and retargeted.
```

### Interaction With Domain Inference

Retained display affects rendering only.

Domain inference remains governed by Sections 1 and 2:

```text
View domain expressions contribute domains from View input.
Cartesian x/y channels authored inside View do not infer x/y domains.
Non-position channels infer according to normal channel rules.
```

Current scales, axes, legends, and layout should be based on the current
evaluation request, even if a retained View payload is displayed temporarily.

### Metrics And Diagnostics

Add metrics so tests do not depend on wall-clock timing:

- [ ] retained View cache hits
- [ ] retained View cache misses
- [ ] retained View retarget successes
- [ ] retained View retarget misses
- [ ] retained View synchronous fallbacks
- [ ] retained View exact-key hits
- [ ] retained View family-key stale hits
- [ ] materialization requests queued
- [ ] materialization ready hits
- [ ] materialization pending fallback draws
- [ ] materialization errors

Miss reasons should include at least:

```text
no retained result
family key mismatch
unsupported coordinate
unsupported scale adjustment
unsupported payload retarget
selection/store/data revision changed
```

### Tests

Phase 1 tests:

- [ ] Unit test: the View stale policy serializes/deserializes on compiled
      View.
- [ ] Unit test: `ViewStalePolicy::HideUntilReady` does not consult retained
      cache.
- [ ] Unit test: retained View boundary id uses explicit `.id(...)` when set.
- [ ] Unit test: retained View boundary id falls back to mark path and transform
      index.
- [ ] Unit test: family key changes when style/non-view params change.
- [ ] Unit test: family key does not change for retargetable x/y domain-only
      changes.
- [ ] Unit test: unsupported scale adjustment returns a retarget miss.
- [ ] Unit test: unsupported scene mark returns a retarget miss.
- [ ] Integration test: Preview with `preview_cached(true)` reuses a retained
      mark result and applies x/y scale adjustments.
- [ ] Integration test: Preview with `preview_cached(true)` falls back to sync
      when retarget is impossible.
- [ ] Integration test: style param changes rebuild the View-dependent mark
      instead of reusing stale styling.
- [ ] Integration test: selection/store changes rebuild the View-dependent mark
      instead of reusing stale filtered data.
- [ ] Integration test: Exact mode renders current data normally in Phase 1.
- [ ] Integration test: domain inference uses current View domain sources even
      when rendering a retained Preview payload.

Phase 2 tests:

- [ ] Unit test: materialization request key includes current View state.
- [ ] Unit test: missing desired key queues one request.
- [ ] Unit test: pending desired key does not queue duplicate work.
- [ ] Unit test: last-ready family-key result is chosen as fallback.
- [ ] Unit test: late generation results are discarded.
- [ ] Unit test: accepted materialization completion calls
      `RenderInvalidationSink::request_render(...)`.
- [ ] Unit test: stale/discarded materialization completion does not call
      `RenderInvalidationSink::request_render(...)`.
- [ ] Integration test: raster image emits desired resource key plus fallback
      key.
- [ ] Integration test: async View completion fires the subscribed
      `RenderInvalidationCallback`, allowing the host to schedule a rerender.
- [ ] Integration test: M4/RecordBatch completion triggers the chosen vector
      refresh path.
