# avenger-layout

Grid layout for charts. A layout is a tree of leaves and containers. Leaves are the parts that
the layout does not look inside, such as plots, legends and blocks of text. A `Row` or `Column`
places its children in order, and nested rows and columns share one table of tracks, so the
panels of a faceted chart line up at every level. A `Grid` places its children in declared
slots, as a dashboard does. Axes, headers and titles attach to the sides of a node as slabs, and
the solver finds every rectangle so that plot edges line up across rows, columns and nested
tables.

The crate has no runtime dependencies and builds for WebAssembly. The optional `diagnostics`
feature draws an image of a solution that shows why each track and gap has its size.

```rust
use avenger_layout::{Layout, Leaf, Row, Side, Size, Slab, SolveOptions};

let plot = |id| {
    Leaf::new()
        .min_size(Size::new(200.0, 150.0))
        .id(id)
        .slab(Side::Left, Slab::new().min_width(40.0))
        .slab(Side::Bottom, Slab::new().min_height(24.0))
};
let page: Layout<&str> = Row::new([plot("a"), plot("b")]).into();
let solution = page.solve(&SolveOptions::sized(600.0, 300.0)).unwrap();
let a = solution.region(&"a").unwrap().content;
```

## The model

The layout has six rules.

1. **Nodes.** A leaf has a minimum size, an optional maximum and an optional aspect ratio. A
   `Row` places its children left to right and a `Column` top to bottom, each child on one or
   more tracks. A `Grid` has declared column and row tracks, and places each child in a slot of
   one or more tracks. A `Wrap` places its children row by row in as many columns as fit, and a
   `Flow` breaks them into lines by their widths. A slot can stay empty, and a child can be
   another container. Containers take a minimum and maximum
   size too, as described in [Padding and sizes](#padding-and-sizes).
2. **Attachment.** A node is in a slot of its parent container, on a side of another node as a
   slab, or over another node as an overlay. A slab is a leaf whose thickness is its size across
   its side. By default it runs along its host's content edge, and with `Extent::Full` it covers
   the corners too. Top and bottom slabs take the corners: a full left or right slab stops at the
   full top and bottom slabs. A slab's minimum length along its side raises its host's minimum
   in that direction, so a legend taller than its plot makes the plot's row taller. Its
   `max_length` and `max_content` are lengths along its side too. A slab's content can reach
   past the ends of its slot, as an axis's end labels reach past its plot, and its host counts
   that overhang as overflow on the side that it reaches past. An overlay is placed at
   fractions of its host's content, and never changes any size.
3. **Overflow.** On each side, a node chooses what happens to its slabs, to the overhang of its
   other slabs, and to its children's overflow. `Propagate`, the default, hangs them outside the node, into the parent's gaps.
   `Contain` keeps them inside the node's slot. `Flush` draws them but reserves no space, which
   suits a facet's axis that hangs into an empty cell below it. The root contains what it would
   propagate. A node's `padding` is reserved inside its slot whatever the policy, and
   `SolveOptions::padding` reserves space between the canvas edges and everything the layout
   draws.
4. **Tracks and tables.** A track is `Auto`, `FitContent`, `Px`, `Fr` or `Aspect`, as the CSS
   track sizing algorithm sizes them. Each track has a base size, the largest minimum of the
   items that span only it, and a growth limit, the largest max-content size of those items.
   Auto tracks grow from their base sizes toward their growth limits when there is room, before
   Fr tracks share the rest by weight up to an optional maximum, so `[Track::auto(),
   Track::fr(1.0)]` gives a text column its unwrapped width when it fits, as CSS `auto 1fr`
   does. An Fr track can also have a basis, a fixed length that it adds to its share, as a CSS
   `flex-basis` does. So band panels whose scales keep a clip padding in pixels have equal band
   steps when each column is `Track::fr_basis(bands, padding)`. Without a canvas size, Fr
   tracks take the share that every Fr track's max-content items need, as CSS sizes them under
   a max-content constraint, so each `Wrap` in a `Row` of `Wrap`s puts its children on one row.
   Fr tracks whose items have no max-content sizes do not set that share.
   `Track::fit_content(limit)` caps an Auto track's growth at `limit`. An Aspect row takes its
   height from a column's width.

   Each gap between tracks is `max(min, trailing + leading + clearance)`, where `trailing` and
   `leading` are the overflow of the items on either side in the same row or column. So a wide
   axis in one row does not widen the gaps of the other rows. A gap uses the `Gap` of the
   innermost container around it: `gap` on a `Row` or `Column`, and `column_gap` and `row_gap`
   on a `Grid`, as CSS `column-gap` and `row-gap` set.

   Rows and columns share their tracks by default, so nested sequences form one table, as
   described in [Tables](#tables). A `Grid`'s children are separate cells.
5. **Tags and content keys.** A tag names the role of a track, gap or slab, such as `x-labels`
   or `panel-gap`. A content key names what a leaf or slab draws, such as a scale domain or a
   title's text. A tag takes effect only where a container declares a rule for it over its subtree:
   - `match_tag` makes everything with the tag take the largest size in its direction: the
     tracks that the container spans, and the gaps and slabs anywhere inside it.
   - `collapse_tag` lets a slab with the tag stand in for the slabs inward of it along its lane
     that show the same content key, so only the outermost of each run is drawn.
   - `merge_tag` collapses, and then replaces the slabs left on a side with one slab on the
     container when they all show one key and cover every track of the container along that
     side.

   [Sharing guides](#sharing-guides) describes the last two rules.
6. **Placement.** In each direction, a node takes its fit-content size with `Sizing::Fit` or
   stretches to its slot up to its maximum with `Sizing::Stretch`, at the start, center or end
   of the slot. A fit-content size is the node's max-content size, as `max_content` declares or
   `Measure::max_content` answers it, within its minimum, its maximum and the slot. A node
   without one keeps its minimum. Space that Fr tracks leave in a table goes where `Distribute`
   says: to the Auto tracks, to one end, or between and around the tracks, as CSS
   `justify-content` places it.

Collapse and merge follow from the tree's structure and each `Wrap`'s count. The widths are
solved first, where each `Wrap` picks its count and each `Flow` breaks its lines, and then the
heights. Each direction solves one problem per table: a `Grid`, or the outermost sequence of a
table. Each direction measures up from the leaves and then allocates down from the root, in
these steps:

1. Measure each slab's thickness, and give each tag that a container matches its thickest slab.
2. Find, from the leaves up, the overflow that each node puts into its parent's gaps.
3. Size each gap per lane, then give each matched gap tag its largest gap.
4. Give each track its base size and growth limit from the items that span only it. Then spread
   what items that span several tracks still need over Auto tracks, or Fr tracks when they span
   no Auto tracks.
5. Size the root from the canvas, or take the layout's max-content size.
6. Allocate the space from the root down, place each node in its slot, and place the slabs.
7. Snap to device pixels, when `SolveOptions::pixel_ratio` is set.

## Tables

`Row` and `Column` share tracks by default. Nested sequences form one table, which ends at a
cell of a `Grid`, `Wrap` or `Flow`, at a sequence with `share(false)`, or at the root.

- In a `Column` of `Row`s, the rows' columns line up. In a `Row` of `Column`s, the columns'
  rows line up. A `Row` inside a `Row`, or a `Column` inside a `Column`, adds its children to the
  outer sequence's tracks.
- A leaf, a `Grid`, a `Wrap`, a `Flow`, or a sequence that does not share is one cell. It spans all of its parent's
  tracks across the parent's direction, so `Column[Row[a, b], c]` puts `c` under both `a` and
  `b`.
- A sequence with fewer children than its neighbors leaves its last cells empty, as a ragged
  facet wrap does. Its slot covers only its own tracks.
- `span(n)` makes a child of a `Row` cover `n` columns, or a child of a `Column` cover `n` rows.
- `share(false)` makes each child of a sequence a separate table in one cell. Tags that the
  sequence matches still reach the children, but their tracks do not line up.

Each sequence declares the tracks in its own direction: a `Row` declares column widths with
`track(i, track)` or `tracks(track)`, and a `Column` declares row heights. `i` counts tracks, not
children. A track that a sequence leaves undeclared takes the size that another sequence in the
table declares for it, and a track that no sequence declares is `Fr(1)`, so plot areas share
space equally. Two sequences that declare different sizes for one track fail validation. Only a
`Grid` declares `Aspect` tracks, since they name a column by index.

The outermost sequence of a table sizes the whole table. So a sequence that joins its parent's
table cannot set `distribute`, padding, a maximum or a placement, or contain a side. Set
`share(false)` on the parent to give it its own table.

```rust
use avenger_layout::{Column, Gap, Grid, GridSlot, Layout, Leaf, Row, Size, Track};

let panel = |id| Leaf::new().min_size(Size::new(120.0, 90.0)).id(id);
let region = |ids: [&'static str; 3]| Row::new(ids.map(panel)).gap(Gap::new(10.0));
// Two regions of three panels, with the second region twice as tall as the first.
let facet = Column::new([region(["n0", "n1", "n2"]), region(["s0", "s1", "s2"])])
    .track(1, Track::fr(2.0))
    .gap(Gap::new(24.0));
let dashboard: Layout<&str> = Grid::new(vec![Track::fr(1.0); 2], [Track::fr(1.0)])
    .child(GridSlot::new(0, 0), facet)
    .child(GridSlot::new(1, 0), panel("table"))
    .into();
```

Each child's `Region::cell` gives its tracks in its table, so a caller can tell which panel is
first in its row or lowest in its column.

## Padding and sizes

`padding` on any node is space inside its slot on each side, between the slot's edge and what
the node keeps inside it: its content, and on a contained side its slabs and its children's
overflow. Padding is reserved on every side whatever the overflow policy, and slabs that a side
propagates or flushes hang outside it. A dashboard card is a container with `Contain` on every
side, a title slab on top, and padding:

```rust
use avenger_layout::{Column, Direction, Edges, Leaf, Overflow, Side, Size, Slab};

let card = Column::new([Leaf::new().min_size(Size::new(100.0, 50.0)).id("chart")])
    .padding(Edges::all(10.0))
    .overflow_direction(Direction::Horizontal, Overflow::Contain)
    .overflow_direction(Direction::Vertical, Overflow::Contain)
    .slab(Side::Top, Slab::new().min_height(20.0).id("title"));
```

A `Grid`, `Row` or `Column` takes `min_size`, `max_size` and their width and height forms, as a
leaf does. They bound the container's content, its tracks and gaps, so padding and contained
slabs add to them.

- A minimum larger than the tracks need grows them, as a larger canvas would. In a sequence
  that joins its parent's table, the minimum spreads over the tracks that the sequence spans, as
  a spanning child's does.
- A container larger than its maximum keeps its maximum and is placed in its slot by its
  placement. Only the outermost sequence of a table can take a maximum or padding, as
  [Tables](#tables) describes.

## Wrap

`Wrap` places its children row by row in as many equal columns as fit its width, so a facet wrap
or a card gallery reflows as the canvas resizes:

```rust
use avenger_layout::{Gap, Leaf, Size, Wrap};

let panels = (0..7).map(|p| Leaf::new().min_size(Size::new(120.0, 90.0)).id(p));
let facet = Wrap::new(panels)
    .min_column(240.0)
    .max_columns(4)
    .gap(Gap::new(12.0))
    .collapse_tag("x-labels")
    .merge_tag("x-title");
```

- A `Wrap` that chooses `k` columns solves as the `Column` of `Row`s with `k` children per row
  would, with equal `Fr(1)` columns, as CSS `repeat(auto-fit, minmax(min, 1fr))` gives. Its
  children are the cells of its table, so lanes, collapse and merge work on its columns. A
  `Wrap` always starts its own table, like a `Grid`, and its children cannot span.
- The count is the largest that fits, at least 1, and at most the number of children and
  `max_columns`. Each column is at least the larger of `min_column` and the widest child's
  minimum. `rows(track)` sizes every row, and `distribute` places the rows' leftover space.
- The widths do not depend on the count, so the width pass picks it: each column gap takes the
  largest gap that any two children could need, each edge takes the largest overflow that any
  child could put there, and a contained left or right slab counts as drawn even when it
  collapses. A solve whose count differs from the tree it starts on solves again
  at the new count. A `Measured` keeps the trees of its last 8 counts and starts each solve on
  the most recent one, so a resize that keeps the count solves once.
- A short last row leaves its last cells empty, and `RegionKind::Wrap` reports their
  rectangles, so a caller can draw a note or a legend into one. A kept slab above an empty cell
  hangs into it: the `Wrap` flushes the bottom of the child above, so the row gap does not grow.
  `Warning::Overlap` reports a slab that reaches past the empty cell.
- Left and right slabs collapse only when every child shows the same content key, because only
  then is the first child of each row the same at every count. A facet wrap with a shared y
  scale draws y labels on the first panel of each row, and one with independent y scales keeps
  every panel's labels.
- Every child's slabs are measured, not only the kept ones, because any child can become the
  first in its row or the lowest in its column at some width. Children with equal content keys
  share answers, so `Measured::resize` can change the count without asking `Measure`.
- A solve without a canvas size puts every child on one row, and `Measured::min_size` reports
  one column.

`Wrap` is row-major only. A wrap that fills columns first would take its count from its height,
which makes a container's width depend on its height. A side legend that wraps its entries into
columns stays a measured leaf.

## Flow

`Flow` breaks its children into lines by their max-content widths, or their minimums when they
have none, for widget rows, filter chips and horizontal legend entries. Each line is a separate
row with its own columns, so nothing aligns across lines, and the children's slabs never
collapse or merge. `distribute(Direction::Horizontal, ..)` places each line's leftover space, at
the start unless set otherwise, and `distribute(Direction::Vertical, ..)` places the rows'. A
solve without a canvas size puts every child on one line.

## Sharing guides

A faceted chart draws one axis per scale, not one per panel. The caller builds the layout as
if every panel shows all of its guides, gives each guide slab a tag and a content key, and
declares which tags collapse or merge.

- **Lanes.** Two slab hosts are in the same lane for a side when they occupy the same tracks of
  one table across the side, such as one column for bottom slabs. Hosts in a lane are ordered
  outward toward the side. Separate tables never share lanes.
- **Collapse.** Along each lane, a slab stands in for the next slab inward when both show the
  same content key. Empty cells, and hosts without a slab of the tag, are passed over. A slab
  that shows another key or none ends the run, and so does a host that overlaps the lane without
  matching it, such as a plot that spans more columns. The outermost slab of each run is kept,
  and the others take no space and are never measured. When containers nest, the outermost one
  that declares a rule for a tag sets the runs.
- **Merge.** From the innermost declaring container out, a container that merges a tag replaces
  the slabs left on a side with one slab on itself, when they all show one key and their hosts
  cover every track of the container along the side, in any row. The merged slab is innermost
  on the container's side and runs along its content edge, the plot area, so `Position::Center`
  centers a title on the plots and `Position::Start` puts a side legend level with the top plot.
  It takes the id of the first slab in tree order that it stands for, and its path ends in
  `PathStep::Merged`.
  A slab merged onto an inner container counts as one of the slabs left for the next container
  out.

In this facet of two regions, the x labels collapse to the south row, each region keeps its own
y labels because the regions have different y domains, and one x title, one y title and one
legend serve the whole figure:

```rust
use avenger_layout::{Column, Leaf, Placement, Position, Row, Side, Size, Slab};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Plot(usize),
    XLabels(usize),
    XTitle(usize),
    YLabels(usize),
    YTitle(usize),
    Legend(usize),
    Header(usize),
}

let (x_title, y_title, legend) = (Size::new(60.0, 14.0), Size::new(14.0, 60.0), Size::new(80.0, 60.0));
let panel = |p: usize, y: &'static str| {
    Leaf::new()
        .min_size(Size::new(120.0, 90.0))
        .id(Id::Plot(p))
        .slab(Side::Bottom, Slab::new().min_height(6.0))
        .slab(Side::Bottom, Slab::new().id(Id::XLabels(p)).tag("x-labels").content_key("x-labels/x"))
        .slab(
            Side::Bottom,
            Slab::new().min_size(x_title).id(Id::XTitle(p)).tag("x-title")
                .content_key("x-title/Month").place(Placement::fit(Position::Center)),
        )
        .slab(Side::Left, Slab::new().min_width(6.0))
        .slab(Side::Left, Slab::new().id(Id::YLabels(p)).tag("y-labels").content_key(y))
        .slab(
            Side::Left,
            Slab::new().min_size(y_title).id(Id::YTitle(p)).tag("y-title")
                .content_key("y-title/Sales").place(Placement::fit(Position::Center)),
        )
        .slab(
            Side::Right,
            Slab::new().min_size(legend).id(Id::Legend(p)).tag("legend")
                .content_key("legend/channel").place(Placement::fit(Position::Start)),
        )
};
let region = |r: usize, y: &'static str, panels: [usize; 3]| {
    Row::new(panels.map(|p| panel(p, y))).slab(Side::Top, Slab::new().id(Id::Header(r)))
};
let figure = Column::new([region(0, "y-labels/north", [0, 1, 2]), region(1, "y-labels/south", [3, 4, 5])])
    .collapse_tag("x-labels")
    .collapse_tag("y-labels")
    .merge_tag("x-title")
    .merge_tag("y-title")
    .merge_tag("legend");
```

`Measure` is asked about the 3 kept x label slabs, the 2 kept y label slabs, the 2 headers and
the 3 merged slabs. A slab's `min_size` is its minimum thickness across its side and its length
along it. The label slabs leave it at zero because `Measure` sets their thickness. `Measure`
never answers a slab's length, so the titles and the legend declare their measured sizes.

Every slab keeps its region, so every id resolves. A slab that another slab draws has zero
thickness, and its `RegionKind::Slab::represented_by` names the slab drawn in its place. The
first slab that a merge stands for has no region of its own, because its id moves to the
merged slab.
`Solution::representative` returns the region that draws an id, so a chart draws each guide
once:

```rust,ignore
let mut drawn = std::collections::HashSet::new();
for p in 0..6 {
    for id in [Id::XLabels(p), Id::XTitle(p), Id::YLabels(p), Id::YTitle(p), Id::Legend(p)] {
        if let Some(region) = solution.representative(&id)
            && drawn.insert(region.path.clone())
        {
            draw_guide(id, region.content);
        }
    }
}
```

`Explanation::representations` lists each run and merge, and `report()` prints one line per
entry. Validation rejects two slabs with one tag and content key on one side that differ in
their size limits, placement or extent, and a host with two slabs of a collapsed or merged tag
on one side.

## Measuring leaves

A leaf whose size depends on the space that it gets, such as an axis, a wrapped header or a
legend, is measured through the caller's `Measure` implementation. The solver asks only about
leaves and slabs that have an id. Widths are solved first, so the two methods differ:

- `height(id, width)` is asked in the height pass, once the leaf's width is final, and its
  answer is used as given. A height that depends on a width is exact in one solve.
- `width(id, height)` is asked after each solve, at the leaf's solved height. A larger answer
  raises the leaf's minimum width and starts another solve, up to `SolveOptions::max_solves`.
  Widths only grow within one measure, so the caller writes no loop.

For a slab, the value passed in is the length that its content takes along its side: an x axis
is measured at its plot's width, and a y axis at its plot's height, unless the slab's
`max_length` or placement makes its content shorter. For any other leaf, it is the space that
its slot leaves for its content. A slab that another slab draws is never asked about, except in a
`Wrap`, where another count can draw it.

A slab's answer can also give its overhang with `Measurement::with_overhang`: how far its content
reaches past the start and the end of its slot along its side, such as an axis's first and last
labels. The overhang holds where the size holds. The host counts it as its overflow on the side
that it reaches past, in the host's row or column only, so an x axis's last label widens the gap
or the canvas padding to the right of its plot, and a y axis's top label the gap or padding above
it. On each side, the larger of the host's slabs and the overhang counts, and a full slab's
overhang starts past the corners that it spans. The overhang changes the pass that ran before
the slab was asked, so it follows the rule for widths: a larger overhang starts another solve,
and overhangs only grow within one measure. A page with overhangs solves at least twice per
measure.

A leaf or slab with a content key stores its answers under the key instead of its id, so nodes
with equal keys share answers: the six x label slabs of a facet are asked once for each range instead
of six times. `Measure` is asked with the id of the first node in tree order that needs the
answer, so an answer for a node with a key may depend only on the key and the value passed in.
The key includes everything that changes how the node draws at a given length, and leaves out
the length:

| Node | In the key | Left out |
| --- | --- | --- |
| Tick labels | The resolved domain, scale type, tick settings, number format, locale, label font and rotation rule | The plot's width or height |
| Axis title or facet header | The text, its font and its wrap settings | The panel that it belongs to |
| Legend | Its title, entries, symbols, colors and layout options, hashed when they are long | The space that it gets |

Equal keys must mean the same drawing, so two charts that draw one domain in different fonts
need different keys. A slab with a key and no tag shares answers but never collapses.

Each kind of leaf answers one method:

| Leaf | Method | Why |
| --- | --- | --- |
| x axis | `height` | Tick density and label rotation follow the plot width. |
| Wrapped title, header or caption | `height` | Text wraps at the width it gets. |
| Top or bottom legend that wraps into rows | `height` | Its row count follows the width. |
| y axis | `width` | Its tick labels follow the plot height. |
| Side legend that wraps into columns | `width` | Its column count follows the height. |
| Legend or label that never wraps | either, with `Holds::Always` | Its size never depends on the other direction. |
| Text that sets its own column width | `max_content`, with `height` | Its unwrapped width is its max-content width, and it wraps at the width it gets. |

An answer says where it holds: only at the value asked, over a range of values, or always. The
solver stores each answer and reuses it wherever it holds, so a range saves measuring later.
Here is one `Measure` implementation for a faceted chart, where `Charts` stands for the caller's
own chart data:

```rust,ignore
use avenger_layout::{Measure, Measurement};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Id {
    Plot(usize),
    XAxis(usize),
    YAxis(usize),
    Header(usize),
    Legend,
}

struct ChartMeasure<'a> {
    charts: &'a Charts,
}

impl Measure<Id> for ChartMeasure<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        match *id {
            // The tick logic knows the widths at which the tick count or rotation changes, and
            // how far the end labels reach past the plot.
            Id::XAxis(panel) => {
                let (height, overhang, holds) = self.charts.x_axis_height(panel, width);
                Some(Measurement::over(height, holds).with_overhang(overhang))
            }
            // Labels break lines first-fit, so the breaks hold from the widest line up to the
            // width that the label was measured at, or without limit for one line.
            Id::Header(panel) => {
                let metrics = self.charts.header_metrics(panel, width);
                let widest = metrics
                    .lines
                    .iter()
                    .map(|line| line.right - line.left)
                    .fold(0.0, f32::max);
                let high = if metrics.lines.len() == 1 { f32::INFINITY } else { width };
                Some(Measurement::over(metrics.height, widest..=high))
            }
            _ => None,
        }
    }

    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        match *id {
            Id::YAxis(panel) => {
                let (width, overhang, holds) = self.charts.y_axis_width(panel, height);
                Some(Measurement::over(width, holds).with_overhang(overhang))
            }
            // Ten 20 px entries in 50 px columns: as many columns as the height needs.
            Id::Legend => {
                let per_column = (height / 20.0).floor().max(1.0);
                let columns = (10.0 / per_column).ceil();
                let holds = per_column * 20.0..=(per_column + 1.0) * 20.0 - 0.01;
                Some(Measurement::over(columns * 50.0, holds))
            }
            _ => None,
        }
    }
}
```

Follow these rules when measuring:

- Answer one method per leaf. A side legend that adds columns when it is short answers only
  `width`, and declares a minimum height of about one entry, such as
  `Leaf::new().min_height(20.0)`. If it also answered `height` with its one-column height,
  its row would grow to that height, and the legend would never get short enough to wrap.
- Start a width-measured leaf low, at 0 or a cheap estimate. Its declared minimum is where each
  measure starts, and widths never shrink within a measure.
- Give text that sets its own column width its unwrapped width as its max-content size, with
  `Leaf::max_content` or `Measure::max_content`, and put it in an Auto column. The column then
  takes the unwrapped width when there is room, and the text wraps at the column's base size
  when there is not. A max-content size does not depend on the other direction, so it is asked
  once per leaf and direction.
- Make each answer depend only on the id and the value passed in. A range that is too wide, or
  `Holds::Always` for a size that does change, gives wrong sizes without an error. Test a
  `Measure` implementation by asking again at both ends of each range that it returns.

## Resizing

`Layout::measure` returns a `Measured`, which keeps every answer that the solve used.

- `Measured::remeasure` solves again at another canvas size and asks `Measure` only when no
  stored answer holds. Called on each frame of a drag, it gives an exact solution every frame
  and measures only the leaves whose answers stop holding, such as a header that rewraps or an
  axis that gains a tick. It reuses the prepared layout, so a frame skips the lowering,
  validation and collapse and merge runs that each `measure` does.
- `Measured::resize` solves again at another canvas size without asking `Measure`. It suits
  each frame of a drag that must never measure. Where no stored answer holds, it takes the
  answer from the last measure, and its solution's `stale()` lists those leaves. When the list
  is empty, the resized solution is the one that `remeasure` gives.
- `Layout::measure_reusing` measures a new layout, such as the same chart with new data, and
  asks only when no stored answer holds. It returns the same solution as `measure` would, and
  `remeasure` returns the same solution as `measure_reusing` with an unchanged layout.
- `Measured::min_size` is the smallest canvas that keeps every leaf at its minimum, solved
  from stored answers as `resize` solves. A solve without a canvas size takes the layout's
  max-content size instead.
- `Measured::forget` drops a leaf's stored answers, for when its content changes. A leaf or
  slab with a content key gets a new key when its content changes instead.

Each measured leaf keeps up to 8 answers per direction, so a drag that widens and then narrows
again finds the earlier answers. The answers of leaves that a new tree does not contain are kept
too, up to as many leaves as the tree measures, so a shape change and back reuses them.

`Layout`, `Measured` and `Solution` are `Send` and `Sync` when the id type is. A caller can keep
resizing one `Measured` on the UI thread while another thread measures the next one, such as
the layout with new data, through `measure_reusing`.

## Reading a solution

A `Solution` holds one `Region` per node, in drawing order. Each region has its id, its path from
the root, its kind, its slot and its content, and a child's `cell` in its table. Find regions
with `region(&id)` or `at_path`, and the region that draws a slab with `representative(&id)`.

- `report()` prints the canvas, the runs and merges, the warnings, and, when the solve was asked
  to explain, each owner's tracks and gaps with what decided them. Use it in test failures and
  for debugging.
- `warnings()` lists overlapping slabs, rectangles past the canvas, aspect fits, widths that were
  still growing, and stale measurements.
- `explain_at(point)` returns the regions, tracks and gaps under a point. Apps can show it as
  hover text over the diagnostic layer.

Set `SolveOptions::explain` to record why each track and gap has its size, in
`Explanation::sizes`.

## Diagnostic images

With the `diagnostics` feature, `diagnostics::diagnostic_scene` draws a solution as a scene graph,
with rulers, a key and the solution's warnings. Each region's label is inside the region, on a
light, rounded background, at a slightly smaller font size when that makes it fit, and shortened
with an ellipsis when a line fits across the region but the text does not fit along it. A label that
fits only when turned reads up on the left half of the canvas and down on the right half. A region
too small for a line of text gets its label beside it, in space that no other label takes, with an
arrow to it. Tracks and gaps have colors but no labels, and the key explains the colors.
`diagnostics::diagnostic_layer` draws the same layers as a light overlay for a chart built from the
solution. Push it into the group that holds the chart: its z-index puts it above every other mark.
Both take a function that names each id, and the caller's label engine. Tracks and gaps appear when
the solve explained. The gallery in
[`avenger-layout-gallery/tests/baselines/`](../avenger-layout-gallery/tests/baselines/) has one image
per concept of the model.

## The example

The `chart_grid` example lays out three plots, measures their axes and legend with
`avenger-guides`, and renders the page through `avenger-svg`. It and the gallery are in the
`avenger-layout-gallery` crate, so the tests of this crate build without a renderer.

```sh
cargo run --release -p avenger-layout-gallery --example chart_grid
cargo test --release -p avenger-layout
cargo test --release -p avenger-layout-gallery
cargo check --release --target wasm32-unknown-unknown -p avenger-layout --lib
```

![Three plots with aligned edges and a spanning bottom row](docs/images/chart-grid.png)

The crate does not group data, find facet domains, or build content keys. Those decisions
belong to the code that builds the layout.
