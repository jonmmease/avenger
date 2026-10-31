# avenger-chart

A serializable description of a chart: its data as Arrow record batches, its marks and their
encodings, its scales and guides, and how it combines other charts in layers, rows, columns, and
wraps. A chart is one protobuf message, `Chart` of package `avenger.chart.v1`, so it can travel
between processes and languages and live in an Arrow column. The crate depends on Arrow but not
on DataFusion, and it builds for `wasm32-unknown-unknown`.

`avenger-chart-eval` evaluates a chart to a scene graph at any size.

The `.proto` files in `proto/avenger/chart/v1/` are the reference for the model: their comments
document every message, field, and rule. This README gives the shape of it.

## The model

A chart is a tree of nodes over a list of data tables:

- A `Unit` draws one mark over one table, with at most one encoding for each channel. It is one
  plot.
- An `Overlay` draws its units and overlays into one plot, such as bars with a rule of their
  mean.
- A `Concat` places its children in a row or a column, and a `Wrap` places them in rows of as
  many equal columns as fit its width, so a wrapped facet grid reflows as the chart resizes.
- An `Empty` keeps a cell of a row, a column, or a wrap and draws nothing.

An encoding reads a column, a constant, or a constant through the channel's scale. A channel with
a scale stores a scale spec: its type, where its domain comes from, its range, its fit, and its
guide. The evaluator builds the configured scale from the spec and fits it at the size of each
plot, so one chart draws at any size.

## Sharing and families

Units share a scale of a channel as the nodes above them say:

- An overlay shares every channel.
- A concat keeps `x`, `y`, `theta`, and `radius` independent and shares every other channel, so a
  row of charts has one legend for a shared color.
- A concat with a facet shares every channel.
- A resolve rule of an overlay or a concat changes the sharing of one channel, and an inner
  independent rule wins over an outer shared rule.
- A share family joins the scales of units whose specs name the family for a channel, wherever
  they are in the tree. A facet grid with a free `x` in each column makes each row's `x`
  independent and names a family for each column.

The specs of the units that share a scale must agree, as `ScaleSpec` describes: a value that one
spec sets and another does not takes the set value, and values that both set must be equal.
Plots that share `x` or `y` get equal lengths along it, and polar plots that share `radius` take
one outer radius, so a value lies at one distance in each plot.

## Facets and headers

A facet is the partition column that the children of a concat or a wrap split. Each child
declares its value of the column, with `partition`, and the facet's header labels each child
with its value. A facet grid is a column of rows, each a facet, and the concats of a table share
one table of tracks, so the plots of a grid line up. A facet without data for a cell holds an
`Empty` that declares the cell's values, so the header still labels it.

Faceting happens before the chart: the producer splits its data, builds one chart for each value,
and combines them. `examples/chart-facets` splits a table with Arrow compute and builds a grid.

## Guide rules

A concat applies a rule to each part of the guides of the scales that it owns, which are the
scales that it shares and the families whose units are all below it. By default, it collapses
the ticks and the labels of an axis to the outer plots, merges legends, colorbars, and
attributions into one, and collapses the labels and the title of a facet header. A guide follows
the sharing of its scale, so an independent scale draws a guide of its own. The title of an axis
draws only its text, so a concat applies its rule to the axis titles of every plot below it, and
equal titles merge into one whatever their scales. The `guide_rules` of a concat's layout keep,
collapse, or merge a part instead.

## Coordinate systems

A plot is Cartesian unless its coordinate system says otherwise:

- A map plot places longitudes and latitudes through a projection, with a graticule, tiles, and
  geometry columns. Its children share the projection of the plot, and the plots below a concat
  share one when the concat says so or has a facet. The evaluator draws map plots with its `geo`
  feature.
- A polar plot places angles and radii about a center, for pies, donuts, sunbursts, rose charts,
  radial bar charts, and radar charts. A text in a polar plot runs horizontally, along its spoke,
  or along its circle, and lies outward or inward of its point, and clockwise or
  counterclockwise, as `radius_side` and `theta_side` say.

## Tools

A unit or an overlay with an id declares tools with `tool`: a `Navigate` that pans and zooms its
scales, a `Tooltip` that describes the instance under the pointer, or a `Select` whose gesture
writes a selection. An app handles the events of each kind, and reads the declarations of the
current chart.

## Routes

A line, a trail, an area, a rule, a rect, or an axis's lines take a route between points. With
`ROUTE_COORDINATE`, the coordinate system decides: a great circle on a map of the sphere, a steady
change of angle and radius in a polar plot, and a straight line otherwise. With `ROUTE_CANVAS`,
each segment is straight on the canvas, as the edges of a radar chart are.

## Building a chart

`Chart::unit` starts a unit, whose builder sets the encodings, scales, partition values, and other
options, and `Chart::overlay`, `Chart::row`, `Chart::column`, and `Chart::wrap` combine charts.
Each `build` checks every rule of the format.

```rust
use arrow::array::RecordBatch;
use avenger_chart::{proto, Chart, ChartError, Encode, Scale};

/// A row of a facet for each genre, with a free `x` for each genre down the grid.
fn genre_row(movies_of: impl Fn(&str) -> RecordBatch, era: &str) -> Result<Chart, ChartError> {
    let panels = ["Drama", "Comedy"].map(|genre| {
        Chart::unit(movies_of(genre), proto::SymbolMark::default())
            .x(Encode::column("rotten_tomatoes_rating").scale(Scale::new().family(genre)))
            .y("imdb_rating")
            .partition("genre", genre)
            .partition("era", era)
            .build()
    });
    let [drama, comedy] = panels;
    Chart::row([drama?, comedy?])
        .facet("genre")
        .independent(proto::Channel::X)
        .build()
}
```

## The format

A chart encodes to one `Chart` message, with `Chart::to_bytes` and `Chart::from_bytes`:

- Each data table is an Arrow IPC stream inside the message, and the other Arrow values, such as
  constants and explicit domains, are one more IPC stream that the message refers to by index, so
  each value keeps its exact Arrow type.
- Each stream starts at a multiple of 64 bytes from the start of the chart, so `from_bytes`
  decodes every column in place.
- Equal charts encode to equal bytes, and units whose tables encode to equal bytes share one
  table.
- The first field is the format version, 1, and a reader rejects every other version.

In an Arrow table, a column of charts is a `BinaryView` column whose field has the extension name
`avenger.chart` (`EXTENSION_NAME`). `chart_field` makes the field, `charts_to_arrow` writes the
column with each chart aligned, and `charts_from_arrow` reads it.
