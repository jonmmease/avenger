# Vega-Lite's names in avenger-chart

This page maps the names of Vega-Lite and Altair to the fields of the format and to the forms of avenger-sql. Altair takes Vega-Lite's names as its keyword arguments, such as `alt.Axis(labelAngle=-45)`, so a row covers both. The format's names are the fields of the `.proto` files in `proto/avenger/chart/v1/`, whose comments give their meaning. The avenger-sql forms follow [the guide to avenger-sql](../avenger-sql/SPEC.md): an argument takes a value, the constructor of its name, or one level down a constructor of one of that constructor's arguments.

The format takes Vega-Lite's name where the concept is the same. Where Avenger's concept differs, the row's note says how. "None" means that the other side has no counterpart, and "format only" means that the format has the field and avenger-sql has no form for it yet. The last section lists the concepts of Avenger that Vega-Lite doesn't have.

A `format` pattern in a chart of avenger-sql, on labels, a text mark, or a tooltip field, is GoogleSQL's, where Vega-Lite takes the patterns of d3. A pattern of numbers is a `FORMAT` specifier, such as `'%.1f'` or `"%'.2f"`, and a pattern of dates holds the format elements of `FORMAT_DATE`, such as `'%Y-%m'`. Each chart of avenger-sql sets `number_format_syntax` and `datetime_format_syntax` to `googlesql`, so the host that draws it registers the `googlesql` providers of avenger-format-number-googlesql and avenger-format-datetime-googlesql.

## Scales

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `scale.type`: `linear`, `log`, `pow`, `symlog`, `time`, `band`, `point`, `ordinal`, `quantile`, `quantize`, `threshold` | `Scale.type`, a choice of one message for each type | `scale => 'log'`, `scale => scale(type => log(base => 2))` | Each type takes its own options, such as `log(base => 2)` and `band(padding_inner => 0.2)` |
| `sqrt` | `Scale.type.pow` | `type => pow(exponent => 0.5)` | |
| `utc` | `Scale.type.time` | `type => time(timezone => 'UTC')` | A time scale takes an IANA time zone, or `UTC` when it is unset |
| `base`, `exponent`, `constant` | `LogScale.base`, `PowScale.exponent`, `SymlogScale.constant` | `log(base => 2)`, `pow(exponent => 2)`, `symlog(constant => 1)` | |
| `clamp`, `round` | `clamp` and `round` of the continuous types, `round` of the band and point types | `type => linear(clamp => TRUE, round => TRUE)` | A time scale has neither |
| `paddingInner`, `paddingOuter`, `align` | `BandScale.padding_inner`, `padding_outer`, `align` | `type => band(padding_inner => 0.2, padding_outer => 0.1)` | A band scale has no `padding` that sets both |
| `padding` of a point scale | `PointScale.padding` | `type => point(padding => 0.5)` | |
| `type` of a field: `nominal`, `ordinal` | `kind` of a band, point, nested band, or ordinal scale: `categorical`, `ordinal` | `type => ordinal(kind => 'ordinal')` | Whether the categories are ordered. When unset, it follows from the type of the values |
| `xOffset`, `yOffset` | `Scale.type.nested_band`, with `Encoding.band_level` | `type => nested_band(padding_inner => [0.4, 0.1])`, `x(..., band_level => 1)` | One scale of nested paths, with paddings for each level |
| `domain` as an array | `Scale.domain.explicit` | `scale => scale(domain => [0, 100])` | |
| `domain` as `{param, encoding}` | `Scale.domain.selection` | `domain => selection(selection => 'brush', channel => 'x')` | Without a `fallback`, a selection without values gives the data domain |
| `zero`, `nice`, `reverse` | `Scale.zero`, `Scale.nice`, `Scale.reverse` | `scale(zero => FALSE, nice => 5, reverse => TRUE)` | `nice` takes a Boolean or a tick count, not a time interval |
| `domainMin`, `domainMax` | `Scale.pin` at `low` or `high` | `scale => pin(0, position => 'low')` | A fit pins one value and finds the other end |
| `domainMid` | None | None | `pin(0, position => 'mid')` gives a domain symmetric about the value, which differs from a midpoint of a color ramp |
| `range` as an array | `Scale.range.values` | `scale(range => [2, 120])` | |
| `scheme` | `Scale.range.palette` | `scale(range => 'tableau10')` | A built-in palette, or one that a stylesheet defines with `@palette` |
| `sort` of a field: `ascending`, `descending`, an array | `Scale.sort` | `scale(sort => 'descending')`, `scale(sort => ['Gentoo', 'Adelie'])` | On the scale. The categories that an array leaves out follow it, ascending, and a listed category without data is not in the domain |
| `scale: null` | `direct_field` or `direct_value` of `Encoding` | `fill => value(css_color)` | `value(...)` draws a column or a constant as it is |
| `bins`, `interpolate`, `rangeMin`, `rangeMax`, `domainRaw`, `unionWith`, `sequential`, `bin-ordinal`, sorting by another channel | None | None | |

## Guides

### Axes

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `axis` of a position | `Encoding.axis` | `x => x(bill_length_mm, axis => axis(grid => FALSE))` | Encodings that share a scale share its axis |
| `axis: null` | `Axis.show` | `axis => FALSE` | |
| `title`, `titleFont`, `titleFontSize`, `titleFontWeight`, `titleColor` | `Axis.title`, a `Title` | `axis => title('Mass', font_size => 14)` | A title's text is Typst markup |
| `orient` | `Axis.orient` | `axis(orient => 'top')` | |
| `format`, `labelFont`, `labelFontSize`, `labelFontWeight`, `labelColor` | `Axis.labels`, a `Labels` | `axis => labels(format => '%.1f', color => 'gray')` | `format` is a GoogleSQL pattern, and `axis(labels => '%Y-%m')` sets it alone |
| `labels: false` | `Labels.show` | `axis(labels => FALSE)` | |
| `labelExpr` | `Labels.markup` | `axis => labels(markup => '#value mm')` | Typst markup in which `#value` is the formatted value |
| `labelAngle` | `Labels.angle` of `Axis.labels` | `axis(labels => labels(angle => -45))` | `labels(text_orient => 'radial')` or `'tangential'` turns the labels of a polar axis. A stylesheet writes `axis > label { angle: -45 }` |
| `labelAlign`, `labelBaseline` | `Labels.text_align`, `Labels.vertical_align` of `Axis.labels` | `axis(labels => labels(text_align => 'end'))` | `text_align` takes CSS's `start`, `center`, and `end` |
| `tickCount`, `values` | `Axis.value_placement`: `value_count` or `values` | `axis(value_placement => 5)`, `axis(value_placement => [35, 45, 55])` | The values that the ticks, the grid lines, and the labels stand at |
| `tickCount` as `{interval, step}` | `Axis.value_placement.value_step` | `axis(value_placement => value_step(3, interval => 'month'))` | A step of numbers or of months, days, or milliseconds, from a `start` |
| `ticks`, `tickSize`, `tickColor` | `Axis.ticks`, `tick_length`, `tick_stroke` | `axis(ticks => FALSE)`, `axis(tick_length => 3)` | `tick` names only the tick marks, as `axis > tick { length; stroke }` styles them |
| `domain`, `domainColor` | `Axis.rule`, `rule_stroke` | `axis(rule => FALSE)` | The axis line keeps the name of its CSS element, `rule` |
| `grid`, `gridColor`, `gridWidth` | `Axis.grid`, `grid_stroke`, `grid_stroke_width` | `axis(grid => TRUE, grid_stroke => '#eee')` | |
| `style` | `Axis.classes` | `axis(classes => ['minor'])` | Classes and ids of stylesheets |

### Legends

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `legend` of a channel | `Encoding.legend` | `color => color(species, legend => legend(orient => 'bottom'))` | Encodings that share a scale share its legend |
| `legend: null` | `Legend.show` | `legend => FALSE` | |
| `type`: `symbol`, `gradient` | `Legend.type`: `symbol`, `colorbar` | `legend(type => 'colorbar')` | |
| `title`, `format`, `labelExpr`, `labelFont` and the other label fonts | `Legend.title`, `Legend.labels` | `legend => title('Species', font_size => 12)` | As on an axis, with GoogleSQL patterns |
| `orient` | `Legend.orient` | `legend(orient => 'bottom')` | One of the four sides, without corners or `none` |
| `columns` | `Legend.columns` | `legend(columns => 2)` | `wrap => 'grid'` or `'flow'` picks the layout of the entries |
| `fillColor`, `strokeColor`, `cornerRadius`, `padding` | `Legend.background`, `border_color`, `border_radius`, `padding` | `legend(background => 'white', border_color => '#ccc')` | CSS's names |
| `labelPadding`, `columnPadding`, `rowPadding` | `Labels.margin` of `Legend.labels`, `column_gap`, `row_gap` | `legend(labels => labels(margin => 4), column_gap => 8)` | |
| `tickCount`, `values` | `Legend.value_placement` | `legend(value_placement => 5)` | As on an axis, for a colorbar and a symbol legend alike |
| `gradientThickness`, `gradientLength` | `Legend.ramp_thickness`, `Legend.ramp_max_length` | `legend(ramp_thickness => 12, ramp_max_length => 200)` | `ramp_max_length` is a maximum, not a fixed length |
| `labelAlign`, `labelBaseline` | `Labels.text_align`, `vertical_align` of `Legend.labels` | `legend(labels => labels(text_align => 'start'))` | On colorbars |
| None | `Labels.angle` of `Legend.labels`, `swatches`, `ticks`, `rule` and their styles | `legend(labels => labels(angle => 30), swatches => FALSE)` | |

### Titles and facet headers

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `title`, `TitleParams.text`, `subtitle` | `Plot.title`, `Concat.title`, `Wrap.title`, a `Title` | `title => title('Penguins', subtitle => 'Palmer Archipelago')` | Typst markup, or plain text with `syntax => 'plain'` |
| `font`, `fontSize`, `fontWeight`, `color`, `subtitle`, `subtitleFont` and the other subtitle fonts | `Title.font_family`, `font_size`, `font_weight`, `color`, and `Title.subtitle`, a `Subtitle` with its text and fonts | `title('Penguins', subtitle => subtitle('By island', font_size => 11))` | CSS's `font_family` in place of `font`. The subtitle inherits the title's styles |
| `TitleParams.orient` | `Title.orient` | `title(..., orient => 'bottom')` | |
| `TitleParams.anchor`: `start`, `middle`, `end` | `Title.text_align`: `start`, `center`, `end` | `title(..., text_align => 'start')` | |
| `title` of a field | `Encoding.title` | `x(bill_length_mm, title => 'Bill length (mm)')` | Plain text. An axis or a legend without a title of its own takes the titles of its encodings |
| `header` of a facet | `Concat.facet`, `Wrap.facet`, a `Facet` | `facet_column(chart, island, facet => facet(labels => labels(angle => 0)))` | |
| `header: null` | `Facet.show` | `facet => FALSE` | |
| `Header.title`, `labelAngle`, `labelPadding`, `orient`, `format`, `labelExpr` | `Facet.title`, `orient`, and `Facet.labels` with its `angle` and `margin` | `facet => title('Island', font_size => 12)` | `labels` takes a GoogleSQL pattern, as on an axis. A facet also draws a rule with ticks, which `tick_length`, `tick_stroke`, `tick_stroke_width`, `rule_stroke`, and `rule_stroke_width` style |

## Encodings and marks

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `field` | `Encoding.scaled_field`, or `direct_field` on a channel without a scale | `x => bill_length_mm` | An expression of the rows, or an aggregate or a window of them |
| `datum` | `Encoding.scaled_value` | `color => datum('Adelie')` | |
| `value` | `Encoding.direct_value`, or `direct_field` for a column | `fill => value('steelblue')`, `fill => value(css_color)` | A direct position is a fraction of the plot, as `x => value(0.5)` is the middle |
| `alt.X(...)`, `alt.Color(...)` and the other channel classes | `Encoding` | `x => x(bill_length_mm, ...)`, `color => color(species, ...)` | A channel function keeps the kind of its value: `fill => fill('red')` draws red |
| `bandPosition` | `Encoding.band_position` | `x(species, band_position => 0)` | Through a band or a nested band scale |
| `condition` | `Encoding.conditional` | Format only | The first matching predicate gives the value |
| `x2`, `y2`, `theta2`, `radius2` | The same channels | `x2 => end_column` | They follow the scale of the first position and take no options |
| `color`, `fill`, `stroke`, `opacity`, `fillOpacity`, `strokeOpacity`, `strokeWidth`, `strokeDash`, `size`, `shape`, `angle` | The same channels in snake case | `stroke_width => 2` | `shape` also takes SVG path data |
| `text`, `url` | `text`, `image` | `text => species` | An image is a URL or a struct of RGBA pixels |
| `detail` | `detail` | `detail => island` | One encoding |
| `order` | The `ORDER BY` of the mark call, and the `order` channel | `line(x => day, y => price, order => step ORDER BY symbol)` | `ORDER BY` gives the draw order of the rows, and `order` only the order of the rows along the path of a line or an area |
| `tooltip` | A tooltip tool | `tools => [tooltip(fields => [field(column => 'species')])]` | No fields show every column, as `content: 'data'` does |
| `longitude`, `latitude`, `longitude2`, `latitude2` | The same channels | `longitude => lon` | Without scales |
| `shape` of `geoshape` | `geometry` | `geo_shape(geometry => geom)` | GeoArrow geometries |
| `xError`, `yError`, `href`, `key`, `description` | None | None | |
| mark `point`, `circle`, `square` | `SymbolMark` | `symbol(...)` | Its `shape` channel picks the symbol |
| `line`, `trail`, `area`, `bar`, `rect`, `rule`, `text`, `image` | The marks of the same names | `line(...)` | |
| `arc` | The polar marks, such as `PolarRectMark` and `PolarBarMark`, on a polar plot | `polar_bar(theta => species, radius => COUNT(*))` | The polar marks pair with the Cartesian ones |
| `geoshape` | `GeoShapeMark` | `geo_shape(...)` | |
| `tick` | `RuleMark` or `BarMark` | `rule(...)` | The nearest marks |
| `filled`, `orient`, `cornerRadius` | `filled`, `orient`, `corner_radius` | `bar(..., corner_radius => 4)` | Properties of the mark |
| `font`, `align`, `baseline`, `ellipsis`, `format` of `text` | `TextMark.font_family`, `align`, `baseline`, `ellipsis`, `format` | `text(..., font_family => 'Lato')` | `ellipsis` is a Boolean, and `format` is a property of the mark, a GoogleSQL pattern as on an axis |
| `aspect`, `smooth` of `image` | `ImageMark.aspect`, `smooth` | `image(..., smooth => FALSE)` | |
| Mark constants, such as `color` and `strokeWidth` | Direct values of the channels | `stroke => 'black'` | |
| `innerRadius` | `Polar.inner_radius` | `coord => polar(inner_radius => 0.5)` | A fraction of the outer radius |
| `style` | `classes` of a layer | `classes => ['dots']` | |
| `interpolate`, `tension`, `strokeCap`, `strokeJoin`, `dx`, `dy`, `limit`, and the other mark properties of Vega-Lite | None | None | Stylesheets give many of them |

## Composition

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `layer`, Altair's `+` | The layers of one plot | `layer(a, b)`, `layer_agg(chart ORDER BY k)` | |
| `hconcat`, `vconcat`, Altair's `\|` and `&` | `Concat`, with its `direction` | `hconcat(a, b)`, `vconcat_agg(chart ORDER BY k)` | |
| `concat` with `columns` | `Wrap` | Format only | A wrap reflows its children as the chart resizes. avenger-sql builds a wrap only as a facet, with `facet_wrap` |
| `facet.row`, `facet.column` | `Concat` with a `facet` | `facet_row(chart, key)`, `facet_column(chart, key)` | The data's rows give the children. A `facet_row` of `facet_column` charts is a grid |
| `facet` with `columns` | `Wrap` with a `facet` | `facet_wrap(chart, key, max_columns => 3)` | `max_columns` is a maximum, which `min_column_width` lowers |
| `spacing` | `Concat.gap`, `Wrap.gap` | `gap => 16` | |
| `resolve.scale` | `Resolve`, one field for each channel with a scale, and `projection` | `resolve => resolve(y => 'independent')` | Also `pattern`, and the sharing of projections |
| `resolve.axis`, `resolve.legend` | `PartRules` | `parts => parts(axis_title => 'keep', legend => 'keep')` | A rule for each kind of part: `keep`, `collapse`, or `merge` |
| `width`, `height` of a view | `Plot.width`, `Plot.height` | `symbol(..., width => 300)` | They fix the length of the plot's track |
| `title` | `title` | `title => 'Penguins'` | |
| `name` | `id` | `id => 'points'` | Stylesheets select it with `#id` |
| `repeat` | None | Several mark calls, or unpivoted rows in a facet | |
| `align`, `bounds`, `center` | None | None | `horizontal_distribute` places the free width of the outermost concat |
| `autosize`, `padding`, `background`, `view`, `config` | `Chart.width`, `Chart.height`, `Chart.padding_top` and the other paddings, and stylesheets | Format only | A host that draws a chart at a size of its own uses that size |

## Interaction

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `bind: 'scales'`, Altair's `.interactive()` | `Navigate` | `tools => [navigate()]`, `navigate(channels => ['x'])` | A drag pans, the wheel zooms, and a double click returns to the fit |
| `params` | `Chart.selections`, and the tools that write them | Format only | A selection is the chart's, with one clause for each plot or layer that writes it |
| `select: {type: 'interval'}`, `encodings` | `Select` with `interval`, `IntervalSelect.channels` | `select('brush', type => interval(channels => ['x']))` | The tool names a selection that the chart declares |
| `select: {type: 'point'}`, `fields`, `toggle` | `Select` with `point`, `PointSelect.keys`, `toggle` | `type => point(keys => ['species'], toggle => TRUE)` | |
| None | `Select` with `lasso` | `type => lasso(keys => ['species'])` | |
| `select.resolve`: `global`, `union`, `intersect` | `Selection.resolution` | Format only | |
| `value` of a selection | `Selection.initial`, its clauses | Format only | |
| `nearest` | `Tooltip.nearest`, `max_radius`, `metric` | `tooltip(nearest => TRUE, max_radius => 20)` | For tooltips |
| `encoding.tooltip`, `StringFieldDef` | `Tooltip.fields`, `TooltipField` | `fields => [field(column => 'island', title => 'Island')]` | `format => '%.1f'` of a field takes a GoogleSQL pattern, as on an axis |
| `condition` with a `param` | `ConditionalValue` with `Predicate.selected` | Format only | |
| `empty: false` | `Selected.empty`: `none` | Format only | |
| `and`, `or`, `not` | `Predicate.all`, `any`, `negate` | Format only | |
| `bind` with `input: 'range'` | `SliderWidget` | Format only | |
| `bind` with `input: 'radio'` or `'select'` | `RadioWidget` | Format only | |
| `bind` with `input: 'checkbox'` | `CheckboxWidget` | Format only | |
| A variable parameter bound to an input | `Chart.modes` and `ModeWidget` | Format only | The nearest match. Modes turn tools on and off |

## Coordinates and geo

| Vega-Lite | Format | avenger-sql | Notes |
|---|---|---|---|
| `projection` | `Geo.projection` | `coord => geo(projection => 'mercator')` | The options of a geo plot |
| `projection.type` in camel case, such as `equalEarth` | `ProjectionType` in snake case, such as `equal_earth` | `projection => 'equal_earth'` | Also `robinson`, `mollweide`, `eckert4`, `winkel_tripel`, `aitoff`, `sinusoidal`, `miller`, `cylindrical_equal_area`, and `hammer`. There is no `albers` or `albers_usa` |
| `identity`, `reflectX`, `reflectY` | `identity`, `Projection.reflect_x`, `reflect_y` | `projection(type => 'identity', reflect_y => TRUE)` | Planar coordinates |
| `parallels` | `Projection.parallels` | `projection('conic_conformal', parallels => [33, 45])` | When unset, 30 and 30 for `conic_conformal`, and 0 and 60 for the other conics |
| `parallel`, `coefficient` | `Projection.parallel`, `coefficient` | `projection('hammer', coefficient => 2)` | |
| `rotate`, `center`, `precision` | `Projection.rotate`, `center`, `precision` | `projection('orthographic', rotate => [96, 0], center => [-96, 38])` | |
| `clipAngle` | `Projection.clip.circle_angle` | `projection(..., clip => 80)` | `clip => 'antimeridian'` cuts at the antimeridian |
| `fit` | `Projection.fit` | `fit => [-125, 24, -66, 50]`, `fit => 'whole_sphere'` | A box of west, south, east, and north. When unset, the view fits the data |
| `scale`, `translate`, `clipExtent` | None | None | The fit and the navigation give the scale and the translation, and the plot clips to its rectangle |
| `lobes`, `ratio`, `tilt`, and the parameters of other projections | `CustomProjection.options` | `projection(type => custom('healpix', options => STRUCT(4 AS lobes)))` | A projection that an app adds |
| The `graticule` generator | `Geo.graticule` | `geo(..., graticule => TRUE)` | |
| `stepMinor`, `stepMajor`, `extentMinor`, `extentMajor`, `precision` | `Graticule.step`, `step_major`, `extent`, `extent_major`, `precision` | `graticule => graticule(step => [30, 30])` | A graticule also labels its lines, as `labels => labels(format => '%.0f', frame => 'outline', sides => ...)` sets, and `grid_stroke` and `grid_stroke_width` style its lines |
| The `sphere` generator | `Geo.sphere` | `geo(..., sphere => sphere(fill => '#def'))` | |
| `arc` with `theta` and `radius` | A polar plot, `Polar` | `coord => polar(start_angle => -90, end_angle => 90)` | Angles in degrees, clockwise from the top |

## Concepts that Vega-Lite doesn't have

- **Fits.** A fit finds a domain from the data with room for the reach of the marks, such as the radius of a symbol, so no mark is cut at the edge of the plot. `zero`, `nice`, and `pin` shape the fit, `domain_empty` gives the domain without data, and a summary domain, `domain => domain(summary => ...)`, fits a summary that a query computed, which can cover more than the rows of the layer, such as all of the data of a facet grid.
- **Pins.** A pin holds one value of the domain exactly, at its low end, its high end, or its middle, and the fit finds the rest: `scale => pin(0, position => 'low')`.
- **Navigation limits.** `scale(navigation => navigation('data', min_span => 1))` keeps a pan or a zoom inside the data, and a geo plot's `navigation` sets its pan mode and its zooms.
- **Share families.** `scale(family => 'lengths')` shares one scale among the layers that name the family, beside the resolve rules.
- **Selection domains.** A scale's domain can be a selection's extent, so a plot zooms to the box that another plot draws.
- **The layout of tracks and slabs.** A concat lays out its children on the tracks of a table, which line up across nested concats, so the plots of a facet grid keep equal widths. A track takes a length or a share of the free length, as in `tracks => [track(200), track(share => 2, max => 600)]`. Axes, legends, and titles stand in slabs along the sides of a plot's area, so a plot's `width` fixes its area whatever its guides take.
- **Part rules.** `parts` keeps a part for each plot, collapses it to one for each lane of plots that line up, or merges it into one for the concat. By default, a concat collapses the ticks and the labels of axes, and merges axis titles and legends.
- **Tools and modes.** Tools in place of parameters act on a plot or a layer: navigations, tooltips, and selection tools. Modes turn tools on and off, as a group of modes picks whether a drag pans or draws a box, and mode widgets show them.
- **Widgets.** Sliders, range sliders, radio groups, check box lists, and check boxes write clauses of selections, as a legend draws a scale. Layers declare them under keys, and the resolve rules share them as they share scales.
- **Symbol marks with pattern fills.** `symbol` draws any shape, SVG paths included, and the `pattern` channel fills marks with patterns, such as presets and lattices of symbols, which a scale can map from categories.
- **`order` and `ORDER BY`.** The `ORDER BY` of a mark call is the draw order of its rows. The `order` channel orders only the rows along the path of a line or an area.
- **Direct positions.** A direct `x` or `y` is a fraction of the plot from its left or bottom edge, and a direct `radius` a fraction of the outer radius, so a mark keeps its place at any size.
- **Typst markup.** Titles, label markup, and the credits of an attribution are Typst markup, and text from the data is plain.
- **Stylesheets.** CSS stylesheets style every part of a chart, and `classes` and `id` name parts for them. The style fields of the format, such as `font_size`, are inline styles.
- **Attributions, tiles, and routes.** A plot's attribution credits its sources. A geo plot draws raster tiles under its marks. A route draws a line of a polar or a geo plot straight on the canvas or along the coordinates.
