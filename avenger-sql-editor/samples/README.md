# The sample project

The project that the avenger-sql editor embeds and opens when it starts. Its data script, `data/samples.sql`, loads nine tables into the schema `samples`: `samples.penguins`, `samples.cars`, `samples.movies`, `samples.iris`, `samples.seattle_weather`, `samples.sales`, `samples.earthquakes`, `samples.countries`, and `samples.plate_boundaries`. [`data/files/README.md`](data/files/README.md) gives the source and the terms of each file. A chart can also declare a table of its own over a file, as `charts/04-iris/02-sepals-from-a-table-of-the-chart.sql` does with `CREATE TEMP EXTERNAL TABLE`, which no other chart sees. Each chart script under `charts/` is a card of the editor's gallery, and a click on a card opens the chart in a tab and runs it. The editor's project is read-only: an edit runs in memory, and Save is off.

```text
samples/
  avenger.toml
  data/
    samples.sql           the data script of the sample tables
    files/                the CSV files, and the geographic files at files/geo/
  charts/
    01-penguins/          a section of the gallery
      13-donut-of-species.sql
    ...
```

The geographic files, `data/files/geo/countries.arrows`, `earthquakes.arrows`, and `plate_boundaries.arrows`, are the Arrow IPC streams of [`avenger-guides-geo/examples/data`](../../avenger-guides-geo/examples/data), which Git LFS stores, and `data/files/geo` is a link to that folder. The editor's `build.rs` embeds the files of this folder, with those of the link.

## Sections, titles, and descriptions

The gallery of any project names its cards by their paths and describes them by their text:

- The folder of a chart under `charts/` is its section, as `01-penguins`, and the sections are in the order of their folders' paths. A chart directly in `charts/` is in the section "Charts".
- A file stem reads as a title: each `-` becomes a space and the first letter is a capital, so `donut-of-species.sql` is "Donut of species". A leading number and `-`, as in `13-donut-of-species.sql`, orders the file among the others of its folder and is not part of its title. A folder's name reads the same way, so `01-penguins` is the section "Penguins". The cards of a section are in the order of their paths.
- The comment lines at the start of a chart script are its description, which a card shows on hover and the search reads. A line `--` alone separates two paragraphs.

The file stems of these charts are their numbers and their titles, with `-` for each run of spaces and punctuation, so a title holds no punctuation.

## The charts

Each chart in the first eight sections returns one row with one chart column, so the editor shows it as an interactive chart that fills the result pane. The charts in "Grouped charts" return a chart for each group, in a table. The charts in "Composition" lay out several charts as one chart: facets, layers, and concats. The charts in "Pan zoom and tooltips" declare tools.

Most charts end in a pipe `AGGREGATE` whose item is a mark call, which builds a chart of the rows before it:

- A channel that holds an aggregate, such as `y => count(*)`, is a measure. The other channels are keys, and the chart has one mark for each combination of the keys. A call without aggregates draws one mark for each row.
- The transforms are written inline. A window that reads only rows, such as `bin`, `timeunit`, `lump`, or `NTILE(4) OVER (ORDER BY x)`, computes before the aggregates, and a window that reads an aggregate, such as `stack(count(*))` or `sum(count(*)) OVER panel`, reads the aggregated rows. `panel`, `position`, and `series` name the windows of the call's groups, positions, and series.
- A `mark_line` or a `mark_area` connects its points along its key, `x` or `y`, or in the order of its `order` channel. A stacked `mark_area` or `mark_line` fills the keys that a series lacks with zero. A `|> CALL impute(...)` before the `AGGREGATE` adds the rows that the groups of a staged query lack.
- An axis title is the text of its expression, such as `count(*)`. A channel function's `title`, as `y(..., title => '...')`, sets another title.
- `GROUP BY` draws one chart for each group. The result has the group columns and a column for each chart, named by its alias, or without one by its mark function, such as `mark_bar`. Another item beside the mark call, such as `count(*) AS penguins`, is an aggregate of each group, and several mark calls draw a chart of each. An item can compose its mark calls, as in `layer(mark_bar(...), mark_rule(...))`.
- The Lifted SQL tab shows the query with each mark call that computes its statistics written as the stages that compute them, as a query that you could type, with the text that it adds highlighted.

The scalar compositions `layer`, `hconcat`, and `vconcat` compose the charts of a row, such as the mark calls of an item. The aggregates `layer_agg`, `hconcat_agg`, `vconcat_agg`, `facet_row`, `facet_col`, and `facet_wrap` compose the charts of a group, in a pipe `AGGREGATE`.

### Maps

A geo mark draws on a map: `mark_geo_symbol` at the `longitude` and `latitude` of each row, and `mark_geo_shape` of the GeoArrow `geometry` of each row, such as a country or a line. Its `coord => geo(...)` names its projection, `sphere` and `graticule` draw the globe behind the data and its lines of longitude and latitude, and `tiles` draws a street map under the data. With `navigate()` among its tools, a drag pans the map, or turns a globe, and the wheel zooms. `samples.earthquakes` holds the earthquakes of magnitude 6 and larger from 2000 through 2024, `samples.countries` the outlines of the countries, and `samples.plate_boundaries` the boundaries of the tectonic plates.

### Variables

A `DECLARE` before the query declares a script variable, which the query reads by its name. The plan holds a placeholder of each variable, and the pipeline computes the values of the variables and binds them before the query runs, so a variable can hold the result of a query. A variable stands in the options of a mark, such as `title`, and of a transform, such as the `step` of `bin`.

### Grouped charts

A mark call with `GROUP BY` builds a chart for each group, so these queries return a table with a row for each group. Drag a column edge or a row edge to resize the charts, check "Interactive charts" to pan and zoom them in place, and double-click a chart to expand it.

### Composition

A facet lays out the charts of a column by a key, with a label for each value. `facet_col` puts them in a row, `facet_row` in a column, and `facet_wrap` in rows that fill the width. An `AGGREGATE` of a mark call with `GROUP BY` gives the charts and their keys.

### Math

A title, a subtitle, the label markup of an axis or a legend, and a `mark_text` with `syntax => 'typst'` read Typst markup, so they can hold math. `display(...)` lays out a fraction, a root, a sum, or an integral at its full size inside a label, and `\` breaks the lines of a text mark. Each formula here is the subtitle of its chart, and each value that SQL computes is a `mark_text` at a direct position in the plot, which `layer` composes with the chart. Labels do not support block equations, matrices, vectors, `cases`, alignment, or statements such as `#let`.

### Pan zoom and tooltips

The `tools` option of a mark call declares its tools, and a `layer` declares the tools that its marks share: `navigate()` pans with a drag and zooms with the wheel, and `tooltip()` shows the row of the mark under the pointer. A single chart is interactive in the result pane. In a table, check "Interactive charts", or double-click a chart to expand it. A tooltip shows the columns of the chart's marks, so a column appears in it only when a channel encodes it, and each row takes the title of its channel, such as `count(*)`.
