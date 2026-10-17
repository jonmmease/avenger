# avenger-layout

Compute rectangles for plots, axes, titles, and legends. Build a layout tree,
provide content sizes, and use the result to position your chart's drawing commands.
Nested rows and columns can align plot areas and share guides across panels.
Your application supplies the data, scales, text measurement, and rendering.

The default build has no runtime dependencies and supports WebAssembly. The
optional `diagnostics` feature adds Avenger dependencies for visual debugging.

## Quick start

This layout places two plots side by side and reserves space for their axes:

```rust
use avenger_layout::{Layout, Leaf, Rect, Row, Side, Size, Slab, SolveOptions};

let plot = |index| {
    Leaf::new()
        .id(("plot", index))
        .min_size(Size::new(200.0, 150.0))
        .slab(Side::Left, Slab::new().id(("y-axis", index)).min_width(40.0))
        .slab(Side::Bottom, Slab::new().id(("x-axis", index)).min_height(24.0))
};
let layout: Layout<(&str, usize)> = Row::new([plot(0), plot(1)]).into();
let solution = layout.solve(&SolveOptions::sized(600.0, 300.0)).unwrap();

let first_plot = solution.region(&("plot", 0)).unwrap().content;
let first_y_axis = solution.region(&("y-axis", 0)).unwrap().content;
assert_eq!(first_plot, Rect::new(40.0, 0.0, 260.0, 276.0));
assert_eq!(first_y_axis.width, 40.0);
```

Use `solve` when sizes are known, or `measure` when they depend on available space.
IDs are optional, but a node needs one for measurement or lookup with `region`.
IDs must be unique within a layout and implement `Clone + Eq + Hash`.

A region's `slot` is the space allocated by its parent. Its `content` is the
rectangle to draw into, after padding, contained guides, and placement. Coordinates
use `f32` layout units, with the origin at the canvas's top-left corner.
Set `SolveOptions::pixel_ratio` to the number of device pixels per layout unit to
snap the result to pixels.

## Choose a container

A `Leaf` represents one item whose internal layout your application handles.
Choose a container according to how its children should be arranged:

| Container | Use it for |
| --- | --- |
| `Row` | Children placed left to right, such as adjacent plots. |
| `Column` | Children placed top to bottom, such as a chart and a caption. |
| `Grid` | Explicit rows, columns, and spans, such as a dashboard. |
| `Wrap` | Equal-width panels that reflow into rows as the width changes. |
| `Flow` | Variable-width items that break into lines, such as legend entries. |

Nested `Row`s and `Column`s share tracks by default. A track is a row or column
of the resulting table. In a `Column` of `Row`s, corresponding columns line up.
In a `Row` of `Column`s, corresponding rows line up. A `Row` inside a `Row` adds
its children to the outer row's columns. A standalone child spans the table in
the cross direction: `Column[Row[a, b], c]` puts `c` beneath both `a` and `b`.

Use `share(false)` when child containers need independent layouts. For example,
`Column::new([row_a, row_b]).share(false)` lets the two rows choose their column
widths separately. A `Grid`, `Wrap`, or `Flow` also gives each child an independent
layout within its cell.

Only the outermost sequence in a shared table can set padding, a maximum size,
placement, `distribute`, or `Overflow::Contain`. Setting these on a sequence that
joins its parent's table returns `LayoutError::SharedSequenceSetting`. Disable
sharing when a nested container needs those settings.

A child of a `Row` or `Column` can use `span(n)` to occupy multiple tracks in the
parent's direction. For a `Grid`, declare column and row tracks with `Grid::new`
and place children with `GridSlot::new(column, row).span(columns, rows)`. Track
indices start at zero. Empty cells are allowed.

For responsive facets, set a column minimum and an optional column limit:

```rust
use avenger_layout::{Gap, Layout, Leaf, Size, SolveOptions, Wrap};

let panels = (0..7).map(|id| Leaf::new().id(id).min_size(Size::new(120.0, 90.0)));
let layout: Layout<i32> = Wrap::new(panels)
    .min_column(240.0)
    .max_columns(4)
    .gap(Gap::new(12.0))
    .into();
let solution = layout.solve(&SolveOptions::sized(800.0, 600.0)).unwrap();
```

`Wrap` fills rows from left to right. Each column is at least `min_column` or the
widest child's minimum, whichever is larger. Guides and gaps also affect how many
columns fit. `RegionKind::Wrap` reports the chosen column count and the empty cells
in the last row. A bottom guide above an empty cell can extend into that cell.

`Flow` breaks lines using each child's max-content width, or its minimum width
when no max-content size is provided. Columns do not align between lines, and its
children's guides do not collapse or merge. Use
`distribute(Direction::Horizontal, ...)` to place unused space on each line.
Children of `Wrap` and `Flow` cannot span tracks. Both use `Track::fr(1.0)` for
their rows by default. Use `rows(track)` to change row sizing.

## Control sizes and spacing

`min_size` and `max_size` bound a node's content. For a container, that content
includes its tracks and gaps. Padding and contained guides add to those bounds.
Nodes stretch to their slots by default, subject to their maximum sizes.
`Placement::fit(position)` uses the node's max-content size within the available
space and size bounds, or its minimum if no max-content size is provided.

Max-content is the size content would take without a space constraint, such as
the width of unwrapped text. Declare it with `Leaf::max_content`, or return it from
`Measure::max_content`.

Use `Row::track` for a column and `Column::track` for a row. `tracks` sets the
default for all their tracks. These indices count tracks, including spans, rather
than children. Undeclared tracks use `Track::fr(1.0)`. Declarations for the same
track in a shared table must agree.

| Track | Behavior |
| --- | --- |
| `Track::fr(weight)` | Share remaining space by weight, respecting content minimums. |
| `Track::auto()` | Grow from the content minimum toward its max-content size before fractional tracks take the rest. |
| `Track::fit_content(limit)` | Like `auto`, but cap growth beyond the minimum at `limit`. |
| `Track::px(length)` | Use a fixed length. Larger content can overflow it. |
| `Track::aspect(column, ratio)` | Set a `Grid` row's height to the named column's width divided by `ratio`. |

`Track::fr_max` caps a fractional track's growth. `Track::fr_basis` adds a fixed
basis to its weighted share. For example, `fr_basis(band_count, padding)` can give
panels equal band steps while reserving fixed padding in each panel.
`Distribute` controls space left after track sizing. Fractional tracks consume
that space first, so changing distribution may have no visible effect.

`Gap::new(min)` sets a minimum distance between tracks. Guides can make the gap
larger. Add `clearance` to reserve space beyond the guides themselves:
`Gap::new(12.0).clearance(4.0)`. Use `gap` on sequences, `Wrap`, and `Flow`, or
`column_gap` and `row_gap` on a `Grid`.

A `Leaf::aspect(ratio)` fits its content to a width-to-height ratio inside its
allocated space. This differs from `Track::aspect`, which sizes a whole row.

Leaving a canvas dimension as `None` uses max-content sizing in that direction.
For responsive wrapping, supply a width. A canvas smaller than the required
content can produce overflow. Check `Solution::warnings()` when deciding whether
to enlarge, scroll, or clip a chart.

## Attach axes, titles, and legends

A `Slab` is a strip attached to one side of a node. Its thickness is its width on
the left or right, or its height on the top or bottom. It normally runs along the
host's content edge. Repeated calls to `slab` on the same side add strips outward,
so an axis title can follow tick marks and tick labels.

Give a slab a fixed minimum thickness or an ID for measurement. Its minimum
length along the side can also enlarge its host. `max_length`, `max_content`, and
`place` control its content along that side. For example,
`Placement::fit(Position::Center)` centers a title at its content length.
`full()` extends a slab across neighboring corners. Full top and bottom slabs
take the corners ahead of full left and right slabs.

A node's overflow policy determines where space for its slabs and child overflow
is reserved:

| Policy | Effect |
| --- | --- |
| `Overflow::Propagate` (default) | Reserve space outside the node, in its parent's gaps or margins. |
| `Overflow::Contain` | Reserve space inside the node's slot. Useful for a padded dashboard card. |
| `Overflow::Flush` | Reserve no space for overflow. Content may overlap neighboring regions. |

The root contains overflow that it would otherwise propagate. Node `padding`
reserves space inside the node regardless of its overflow policy.
`SolveOptions::padding` reserves an outer margin within the requested canvas.
These policies allocate rectangles. Your renderer controls clipping.

For an inset or annotation that should not affect surrounding sizes, use an
`Overlay`. Its rectangle is specified as fractions of its host's content.

## Share guides across panels

A **tag** identifies a role, such as `x-labels` or `y-title`. A **content key**
identifies what is drawn. Build every panel with its guides, then apply rules on
a containing `Row`, `Column`, `Grid`, or `Wrap`:

- `match_tag(tag)` equalizes tagged track sizes, gap sizes, or slab thicknesses
  within its scope. Each kind and direction is matched separately. It does not
  remove guides or require equal content keys.
- `collapse_tag(tag)` keeps the outermost guide in each aligned run of panels
  with the same content key. For bottom axes, runs follow columns. For left axes,
  they follow rows. Runs do not cross separate tables or incompatible spans.
- `merge_tag(tag)` first collapses guides, then replaces the remaining guides on
  a side with one on the container if they have the same key and collectively
  cover that side's tracks. Use it for a common title or legend.

This pair of vertically stacked plots shares one bottom axis:

```rust
use avenger_layout::{Column, Layout, Leaf, Side, Size, Slab, SolveOptions};

let panel = |index| {
    Leaf::new()
        .id(("plot", index))
        .min_size(Size::new(120.0, 90.0))
        .slab(
            Side::Bottom,
            Slab::new()
                .id(("x-axis", index))
                .min_height(24.0)
                .tag("x-labels")
                .content_key("month-domain/month-format/12px-font"),
        )
};
let layout: Layout<(&str, usize)> = Column::new([panel(0), panel(1)])
    .collapse_tag("x-labels")
    .into();
let solution = layout.solve(&SolveOptions::sized(400.0, 300.0)).unwrap();
let axis = solution.representative(&("x-axis", 0)).unwrap();
assert_eq!(axis.id, Some(("x-axis", 1)));
```

Use `Solution::representative(&id)` to find the guide that draws a slab's content.
Deduplicate returned regions by `path` before drawing them. A collapsed slab still
resolves by ID but takes no space. A merged slab uses the first member's ID.

Equal content keys must describe the same drawing at the same available length.
For axes, include the domain, scale type, tick settings, format, font, and rotation
rules. For titles, include text, font, and wrapping settings. For legends, include
entries, fonts, and layout options. Leave the available length out of the key:
measurement receives it separately. The same key also shares cached measurements,
even without a tag.

Slabs that collapse or merge must have compatible size limits, placement, and
extent. A host can have only one slab of a collapsed or merged tag on each side.
For `Wrap`, left and right guides collapse only when every child has the same key.

## Measure content that depends on available space

Implement `Measure<Id>` and call `Layout::measure`. A measurement specifies a
minimum size in one direction at the supplied size in the other direction.
Return `None` for IDs or directions that your measurer does not handle.

| Content | Method |
| --- | --- |
| Bottom or top axis, wrapped text, horizontal legend | `height(id, width)` |
| Left or right axis, legend that adds columns as height decreases | `width(id, height)` |
| Unwrapped text or another unconstrained content size | `max_content(id, direction)` |

For slabs, `height` and `width` measure thickness only. The input is the slab's
content length along its side. Use declared sizes or `max_content` for that
length. For ordinary leaves, the input is the content size in the other direction.

This example models an axis whose labels use two lines below a width of 200:

```rust
use avenger_layout::{Layout, Leaf, Measure, Measurement, Side, Size, Slab, SolveOptions};

struct AxisMeasure;

impl Measure<&str> for AxisMeasure {
    fn height(&mut self, id: &&str, width: f32) -> Option<Measurement> {
        if *id != "x-axis" {
            return None;
        }
        Some(if width >= 200.0 {
            Measurement::over(24.0, 200.0..=f32::INFINITY)
        } else {
            Measurement::over(48.0, 0.0..=200.0f32.next_down())
        })
    }
}

let layout: Layout<&str> = Leaf::new()
    .id("plot")
    .min_size(Size::new(100.0, 80.0))
    .slab(Side::Bottom, Slab::new().id("x-axis"))
    .into();
let mut measured = layout
    .measure(&SolveOptions::sized(400.0, 200.0), &mut AxisMeasure)
    .unwrap();
assert_eq!(measured.solution().region(&"x-axis").unwrap().content.height, 24.0);

let narrow = measured.remeasure(Size::new(150.0, 200.0), &mut AxisMeasure).unwrap();
assert_eq!(narrow.region(&"x-axis").unwrap().content.height, 48.0);
```

In a renderer integration, return sizes from the actual text or guide measurement.
avenger-guides gives these answers for axes, colorbars and legends: `Axis::extent`,
`Colorbar::extent` and `Legend::extent` return a part's thickness, its overhang, and the lengths
over which both hold, which become `Measurement::over(thickness, holds).with_overhang(overhang)`.
A legend's range is exact, so the solver asks again only when its entries reflow.
The solver caches answers according to their validity:

- `Measurement::from(size)` is valid only at the input size that was asked.
- `Measurement::over(size, range)` is valid throughout the inclusive range, which
  must contain the input size.
- `Measurement::always(size)` is valid at every input size. Use it only when the
  result is independent of the other dimension.

A slab can add `with_overhang([start, end])` for labels extending past the ends of
its slot. Overhang follows the host's overflow policy, just like its slabs.
With the default policy, the solver reserves room in adjacent gaps or outer margins.
The declared validity range applies to both the size and the overhang.

Answers must depend only on the ID and input size, or on the content key and input
size when a key is set. An overly broad validity range can silently reuse an
incorrect size. Start with exact-size answers unless your measurement code can
establish a valid range.

For wrapping content, measure the dimension that changes: a side legend that adds
columns should answer `width` and declare a minimum height. Answering `height`
with its full one-column height would prevent it from wrapping. Width measurements
and overhangs only grow within one solve operation, so start from the minimum you
actually require.

The solver can repeat measurement when widths, overhangs, or aspect fitting change
the available space. `SolveOptions::max_solves` defaults to 4. If it reaches that
limit while sizes are still changing, the result includes `Warning::StillGrowing`.
Inspect that warning before treating a measured layout as settled.

## Resize and update content

`Layout::measure` returns a `Measured` that stores its solution and measurement
cache. Choose the next operation according to what changed:

| Operation | Use it when |
| --- | --- |
| `Measured::remeasure(size, measure)` | The canvas changed and measurement is allowed. It reuses valid answers and stores the new solution. |
| `Measured::resize(size)` | Measurement must be deferred. It returns a solution using cached answers, falling back to the last measurements where necessary. |
| `Layout::measure_reusing(previous, options, measure)` | The layout tree changed and measurements for unchanged content can be reused. |
| `Measured::min_size()` | You need the narrowest layout and its required height, based on stored measurements. |

A drag over a chart with axes calls `remeasure` on each frame, because an axis's labels answer
only at the length that was asked. After `resize`, check `solution.stale()`. It identifies content that used a
fallback measurement. Remeasure before relying on those sizes. `resize` leaves
the stored solution unchanged. Cached answers remain valid only while the content
and measurement settings they describe remain unchanged.

When content changes, call `forget(&id)` for nodes cached by ID before remeasuring
or reusing the cache. For nodes with content keys, rebuild the layout with a new
key instead. `forget(&id)` does not invalidate answers stored under a content key.

## Inspect and render the result

Use `region(&id)` for a named rectangle or `regions()` to visit regions in drawing
order. `Region::cell` identifies a child's tracks within its table.
`Solution::warnings()` reports slab overlaps, content beyond the canvas, aspect
adjustments, unsettled measurements, and stale answers. Invalid inputs such as
duplicate IDs or conflicting track declarations return `LayoutError` instead.

Set `SolveOptions::explain` to record track and gap sizing decisions. Read them
with `explanation()`, print them with `report()`, or query a point with
`explain_at([x, y])`.

With the `diagnostics` feature, `diagnostics::diagnostic_scene` creates an Avenger
scene graph for inspection. `diagnostics::diagnostic_layer` creates an overlay to
add to your chart. Both take an ID-naming function and a `LabelEngine`. Enable
`explain` to include tracks and gaps. See the
[diagnostic gallery](../avenger-layout-gallery/tests/baselines/) for examples.

The [chart_grid example](../avenger-layout-gallery/examples/chart_grid/) measures
each axis's ticks, labels, and title in their own slabs, and its legend in one, with the
prepared axes and legend of `avenger-guides`, then renders through `avenger-svg`.
Run it from the workspace root:

```sh
cargo run --release -p avenger-layout-gallery --example chart_grid
```

It writes `target/layout-gallery/chart-grid.png`. The example and image tests live
in `avenger-layout-gallery`, so the core crate's tests build without a renderer.

![Three plots with aligned edges and a spanning bottom row](docs/images/chart-grid.png)

To run the core tests and compile the README examples:

```sh
cargo test -p avenger-layout
```
