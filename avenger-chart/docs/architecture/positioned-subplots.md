# Coordinate-Positioned Subplots

Coordinate-positioned subplots let a coordinate system place child plots inside
its parent plot area. Coordinate crates own the authoring methods and placement
channel names, while the top-level `avenger-chart` crate owns child-frame
measurement, sharing, guide/legend coordination, and rendering.

Facet and concat layout remain built-in layout containers. Coordinate-positioned
subplots are the extension point for coordinate systems that can produce point
anchors for child plot frames.

## Authoring Surface

The neutral `Subplot<C>` mark lives in `avenger-chart-marks`. Coordinate crates
add coordinate-specific extension traits for placement channels.

Cartesian subplots use `CartesianSubplotPositionChannels` from
`avenger-chart-cartesian`:

```rust
Subplot::<Cartesian>::new(child_plot)
    .partition_by(col("species"))
    .subplot_x(avg(col("sepal_length")))
    .subplot_y(avg(col("petal_length")))
    .plot_size(90.0, 70.0)
```

Polar subplots use `PolarSubplotPositionChannels` from `avenger-chart-polar`:

```rust
Subplot::<Polar>::new(child_plot)
    .partition_by(col("species"))
    .theta(avg(col("angle")))
    .r(avg(col("radius")))
    .plot_size(90.0, 70.0)
```

External coordinate crates can define their own placement methods, such as
`subplot_u` and `subplot_v`, by adding an extension trait for `Subplot<Foo>`.

## Compile Contract

Coordinate crates opt into positioned subplots by implementing
`SubplotContainerCoordinateSystem`.

The implementation calls `compile_positioned_subplot_mark(...)` from
`avenger-chart-core` with a `PositionedSubplotSpec`. The spec declares:

- `outer_label`: the coordinate label used in author-facing errors,
- `group_name_prefix`: the scene group prefix used for rendered child frames,
- `placement_channels`: the source channel names on the `Subplot` mark and the
  transform-channel names passed to the coordinate transform,
- `partition_channel`: the optional channel that creates one child frame per
  partition value,
- default child plot-area width and height.

The shared compile helper produces `CompiledPositionedSubplot`, which implements
`PositionedSubplotMarkCore`. Cartesian and Polar expose
`CompiledCartesianSubplot` and `CompiledPolarSubplot` as aliases for that
compiled mark type.

## Runtime Discovery

`CompiledMarkCore::as_positioned_subplot()` is the runtime discovery hook. The
top-level coordinate measurement dispatcher asks each compiled mark whether it
is a positioned subplot. If at least one positioned subplot mark is present, the
generic runtime in `avenger-chart/src/positioned_subplot.rs` measures those
child frames as part of coordinate measurement.

The runtime does not know about Cartesian, Polar, or any external coordinate
type directly. It only reads `PositionedSubplotMarkCore` metadata and calls the
parent coordinate transform.

## Placement Transform

For each positioned subplot mark, the runtime evaluates the declared placement
channels from the prepared mark data. It maps each source channel to its
`transform_channel` from `PositionedSubplotSpec`, then calls the parent
coordinate transform.

The transform result must downcast to `PointGeometry`. Each point is used as the
anchor for one child frame. Coordinates that cannot return point anchors do not
support coordinate-positioned subplots.

Cartesian declares:

```text
subplot_x -> x
subplot_y -> y
```

Polar declares:

```text
r -> r
theta -> theta
```

External coordinates choose their own channel names and transform-channel
mappings.

## Non-Partitioned Mode

Without a partition channel, the runtime creates one child frame per evaluated
placement row.

Each child frame:

- uses a stable `ChildFrameKey::PositionedSubplot`,
- receives a `ContainerPathSegment::PositionedSubplot` in the child-frame
  container path,
- uses `ChildFrameSharingLevel::positioned_subplot(...)` for nested sharing,
- uses the child plot's explicit data if present,
- otherwise inherits the current parent data.

## Partitioned Mode

With a partition channel, the runtime creates one child frame per partition
value.

Partitioned mode requires:

- no plot-level `.data(...)` on the child plot,
- all required placement channels present,
- placement expressions that are aggregates, literals/constants, or the
  partition expression itself,
- a parent data source to partition.

The runtime keeps two data views:

```text
parent data
  |
  |-- placement summary:
  |     one row per partition, with evaluated placement channels
  |
  |-- child data overrides:
        one filtered dataframe per partition value
```

The placement summary provides child anchors and parent placement-scale
domains. The raw parent data is filtered by partition value and passed to the
child plot as inherited data.

Each partitioned child frame:

- uses `ChildFrameKey::PositionedPartition`,
- receives a `ContainerPathSegment::PositionedPartition` in the child-frame
  container path,
- uses `ChildFrameSharingLevel::positioned_partition(...)` for nested sharing,
- includes the partition value in the rendered scene group name when no user
  key is provided.

## Child-Frame Runtime Integration

Positioned subplots use the same child-frame machinery as concat and facet:

- `ChildFrameRuntime` prepares child plots,
- `PreparedChildFramePlot` stores child plot preparation results,
- `ChildFrameDomainSharingInput` feeds nested domain sharing,
- `ChildFrameScopeKey` provides stable child identity,
- `ContainerPathSegment` records nested container ownership,
- `ChildFrameSharingLevel` and `ChildFrameSharingPath` drive guide/axis
  ownership,
- `ChildFrameContainerView` exposes measured child frames to guide, legend,
  debug, and render helpers,
- `ChildFrameRenderPlacement` stores final child-frame origins and sizes.

Rendering reads the saved `PositionedCoordMeasurement` and
`PositionedChildMeasurement` values. It does not recompute partitions or child
plot measurements.

## Scale Domains, Axes, And Legends

Placement channels are ordinary scale channels owned by the parent coordinate
system. Cartesian placement channels are named `subplot_x` and `subplot_y`, so
their domains, sharing levels, and axes are independent from the inner child
plot's `x` and `y` channels.

If an axis is configured on a placement channel, guide sharing treats that
channel like other positional axes:

- `subplot_x` uses bottom-axis defaults in Cartesian,
- `subplot_y` uses left-axis defaults in Cartesian,
- Polar placement axes use Polar guide behavior for `r` and `theta`.

Nested child plot domains and legends follow child-frame sharing levels:

- `ScaleSharing::Free` / `Level(0)` stays local to each child frame,
- `ScaleSharing::Level(n)` hoists to the ancestor `n` child-frame/facet levels
  up,
- `ScaleSharing::Shared` hoists to the root shared ancestor.

Visual legends are owned by the visual channel that creates the legend. Sharing
`subplot_x` or `subplot_y` does not promote a `fill`, `stroke`, `size`, or
`shape` legend by itself.

## Visual Coverage

Positioned subplot visual coverage lives in:

- `avenger-chart/tests/visual_tests/test_cartesian_subplot.rs`,
- `avenger-chart/tests/visual_tests/test_positioned_subplot_scale_sharing.rs`,
- `avenger-chart/tests/visual_tests/test_positioned_subplot_legend_sharing.rs`.

The baseline set covers:

- Cartesian parent with Cartesian child plots,
- Cartesian parent with Polar child plots,
- Polar parent with Cartesian child plots,
- Polar parent with Polar child plots,
- partitioned positioned subplot filtering,
- independent `subplot_x` / `subplot_y` sharing and axis ownership,
- fill legend hoisting through nested positioned child frames,
- Cartesian and Polar parent legend hoisting.
