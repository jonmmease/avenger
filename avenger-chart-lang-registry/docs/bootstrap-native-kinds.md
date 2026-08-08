# Avenger native schema: bootstrap-vertical-slice

Language schema 1.0.

## `Coordinate.cartesian`

A two-dimensional Cartesian coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |
| `unit_aspect` | property | false | Optional positive ratio between x and y data units. |

## `Coordinate.facet`

A row facet, optionally containing a nested column facet for a two-dimensional grid.

| Name | Role | Required | Description |
|---|---|---:|---|
| `column` | property | false | Optional nested column facet expression and configuration. |
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `row` | property | true | Required outer row facet expression and configuration. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.facet_column`

A one-dimensional column facet container.

| Name | Role | Required | Description |
|---|---|---:|---|
| `column` | property | true | Column facet expression and configuration. |
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.facet_wrap`

A wrapped one-dimensional facet whose physical columns may be fixed or responsive.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `facet` | property | true | Wrapped facet expression and configuration. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.geo`

A geographic map projection with an optional authored viewport.

| Name | Role | Required | Description |
|---|---|---:|---|
| `center_lon_lat` | property | false | Viewport center as `[longitude, latitude]` in degrees. |
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `precision` | property | false | Adaptive projection resampling precision in pixels; zero disables it. |
| `projection` | property | false | Map projection; defaults to Equal Earth. |
| `rotate` | property | false | Three-axis spherical rotation in degrees. |
| `subtitle` | property | false | Chart subtitle expression. |
| `tiles` | property | false | Raster tile resource and use-site layer configuration. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |
| `viewport_id` | property | false | Runtime viewport state id prefix. |
| `zoom` | property | false | Initial slippy-style zoom level. |

## `Coordinate.grid_concat`

An explicitly placed two-dimensional grid of child plot cells.

| Name | Role | Required | Description |
|---|---|---:|---|
| `axis_guide_visibility` | property | false | Axis label and title compaction policy across grid cells. |
| `column_widths` | property | false | Per-column plot-area track sizing. |
| `columns` | property | true | Positive number of grid columns. |
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `row_heights` | property | false | Per-row plot-area track sizing. |
| `rows` | property | true | Positive number of grid rows. |
| `spacing` | property | false | Minimum gap between adjacent plot areas. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.hconcat`

A horizontal ordered concatenation of child plot cells.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `spacing` | property | false | Minimum gap between adjacent plot areas. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |
| `widths` | property | false | Per-cell horizontal plot-area track sizing. |

## `Coordinate.parallel`

A wide-form parallel-coordinate frame with user-named dimensions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `dimensions` | property | false | Sparse frame configuration keyed by mark-owned logical dimension id. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `order` | property | false | Static left-to-right dimension order; undeclared dimensions follow declaration order. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.polar`

A radial and angular two-dimensional coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.repeat_columns`

A repeat_columns container instantiated from ordered repeat variables.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `domain_coordination` | property | false | Domain coordination policy for repeated cells. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.repeat_grid`

A repeat_grid container instantiated from ordered repeat variables.

| Name | Role | Required | Description |
|---|---|---:|---|
| `axis_guide_visibility` | property | false | Axis label and title compaction across repeat-grid cells. |
| `data` | property | false | Chart-level data source. |
| `domain_coordination` | property | false | Domain coordination policy for repeated cells. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.repeat_rows`

A repeat_rows container instantiated from ordered repeat variables.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `domain_coordination` | property | false | Domain coordination policy for repeated cells. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.repeat_wrap`

A repeat_wrap container instantiated from ordered repeat variables.

| Name | Role | Required | Description |
|---|---|---:|---|
| `columns` | property | false | Expression yielding the fixed number of physical columns. |
| `data` | property | false | Chart-level data source. |
| `domain_coordination` | property | false | Domain coordination policy for repeated cells. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `responsive_columns` | property | false | Expression yielding the target minimum repeated-cell width in pixels. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.treemap`

A hierarchical treemap layout coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `display_levels` | property | false | Maximum number of hierarchy levels displayed below the current root. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `path` | property | true | Ordered hierarchy-level expressions from root to leaf. |
| `root_path_id` | property | false | Initial visible hierarchy root path id. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |
| `value` | property | true | Non-negative leaf weight expression used for area allocation. |

## `Coordinate.vconcat`

A vertical ordered concatenation of child plot cells.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `heights` | property | false | Per-cell vertical plot-area track sizing. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `spacing` | property | false | Minimum gap between adjacent plot areas. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.wrap_concat`

A row-major wrapping concatenation of child plot cells.

| Name | Role | Required | Description |
|---|---|---:|---|
| `axis_guide_visibility` | property | false | Axis label and title compaction policy across wrapped cells. |
| `columns` | property | false | Expression yielding the fixed number of columns. |
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `responsive_columns` | property | false | Expression yielding the target minimum cell width in pixels. |
| `spacing` | property | false | Minimum gap between adjacent plot areas. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Coordinate.zerod`

A zero-dimensional coordinate system that places marks at the plot center.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | false | Chart-level data source. |
| `format` | property | false | Chart formatting defaults. |
| `guide` | property | false | Coordinate-independent guide styling. |
| `layout` | property | false | Chart canvas, plot-area, and margin layout. |
| `subtitle` | property | false | Chart subtitle expression. |
| `time` | property | false | Chart temporal defaults. |
| `title` | property | false | Chart title expression. |

## `Adjust.dodge`

Separate items into pixel-spaced lanes according to a data field.

| Name | Role | Required | Description |
|---|---|---:|---|
| `apply` | property | true | Target mark channels mapped to outputs of this bound adjustment. |
| `axis` | property | false | Displacement axis; defaults to x. |
| `by` | property | true | Data field whose values define dodge lanes. |
| `step_px` | property | false | Pixel distance between adjacent lanes; defaults to 1. |

## `Adjust.jitter`

Apply deterministic random displacement along one item-frame axis.

| Name | Role | Required | Description |
|---|---|---:|---|
| `apply` | property | true | Target mark channels mapped to outputs of this bound adjustment. |
| `axis` | property | false | Displacement axis; defaults to x. |
| `seed` | property | false | Optional non-negative deterministic random seed. |
| `width_px` | property | false | Full displacement width in pixels; defaults to 1. |

## `Adjust.nudge`

Offset item-frame positions by fixed horizontal and vertical pixel distances.

| Name | Role | Required | Description |
|---|---|---:|---|
| `apply` | property | true | Target mark channels mapped to outputs of this bound adjustment. |
| `dx` | property | false | Horizontal pixel offset; defaults to 0. |
| `dy` | property | false | Vertical pixel offset; defaults to 0. |

## `Mark.cartesian.area`

A filled Cartesian area mark.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `defined` | channel | false | The `defined` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `fill_pattern` | channel | false | The `fill_pattern` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `order` | channel | false | The `order` encoding channel. |
| `orientation` | channel | false | The `orientation` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_dash` | channel | false | The `stroke_dash` encoding channel. |
| `stroke_join` | channel | false | The `stroke_join` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `x2` | channel | false | The `x2` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |
| `y2` | channel | false | The `y2` encoding channel. |

## `Mark.cartesian.box_plot`

A compound box-and-whisker plot generated from Cartesian primitives and transforms.

| Name | Role | Required | Description |
|---|---|---:|---|
| `extent` | property | false | Non-negative interquartile-range multiplier used for whisker fences; defaults to 1.5. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `orientation` | property | false | Explicit value-axis orientation; otherwise inferred from the position channels. |
| `fill` | channel | false | Shared categorical fill encoding for generated summary parts. |
| `x` | channel | true | Horizontal position or grouping encoding. |
| `y` | channel | true | Vertical position or grouping encoding. |

## `Mark.geo.geo_shape`

A GeoJSON or WKB geometry projected through the geo coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `fill` | channel | false | The `fill` encoding channel. |
| `geometry` | channel | false | The `geometry` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `x2` | channel | false | The `x2` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |
| `y2` | channel | false | The `y2` encoding channel. |

## `Mark.cartesian.image`

An image positioned in Cartesian coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `align` | channel | false | The `align` encoding channel. |
| `aspect` | channel | false | The `aspect` encoding channel. |
| `baseline` | channel | false | The `baseline` encoding channel. |
| `height` | channel | false | The `height` encoding channel. |
| `image` | channel | false | The `image` encoding channel. |
| `smooth` | channel | false | The `smooth` encoding channel. |
| `width` | channel | false | The `width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.cartesian.line`

A Cartesian line mark.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `defined` | channel | false | The `defined` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `order` | channel | false | The `order` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_dash` | channel | false | The `stroke_dash` encoding channel. |
| `stroke_join` | channel | false | The `stroke_join` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.geo.line`

A projected line with planar or longitude/latitude positions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `defined` | channel | false | The `defined` encoding channel. |
| `lat` | channel | false | The `lat` encoding channel. |
| `lon` | channel | false | The `lon` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `order` | channel | false | The `order` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_dash` | channel | false | The `stroke_dash` encoding channel. |
| `stroke_join` | channel | false | The `stroke_join` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.polar.line`

A line in radial and angular coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `defined` | channel | false | The `defined` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `order` | channel | false | The `order` encoding channel. |
| `r` | channel | false | The `r` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_dash` | channel | false | The `stroke_dash` encoding channel. |
| `stroke_join` | channel | false | The `stroke_join` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `theta` | channel | false | The `theta` encoding channel. |

## `Mark.parallel.parallel_line`

A wide-form polyline spanning the declared parallel dimensions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `dimensions` | property | true | User-named dimension ids mapped to configured encoding channels. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `defined` | channel | false | The `defined` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_dash` | channel | false | The `stroke_dash` encoding channel. |
| `stroke_join` | channel | false | The `stroke_join` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |

## `Mark.parallel.parallel_symbol`

A symbol at every row and parallel-dimension intersection.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `dimensions` | property | true | User-named dimension ids mapped to configured encoding channels. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `angle` | channel | false | The `angle` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `shape` | channel | false | The `shape` encoding channel. |
| `size` | channel | false | The `size` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |

## `Mark.cartesian.path`

An arbitrary Cartesian path mark.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `fill` | channel | false | The `fill` encoding channel. |
| `fill_pattern` | channel | false | The `fill_pattern` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `path` | channel | false | The `path` encoding channel. |
| `path_transform` | channel | false | The `path_transform` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_join` | channel | false | The `stroke_join` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.cartesian.rect`

A Cartesian rectangle mark.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `corner_radius` | channel | false | The `corner_radius` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `fill_pattern` | channel | false | The `fill_pattern` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `x2` | channel | false | The `x2` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |
| `y2` | channel | false | The `y2` encoding channel. |

## `Mark.geo.rect`

A rectangle in projected geo plot coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `corner_radius` | channel | false | The `corner_radius` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `fill_pattern` | channel | false | The `fill_pattern` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `x2` | channel | false | The `x2` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |
| `y2` | channel | false | The `y2` encoding channel. |

## `Mark.cartesian.rule`

A Cartesian rule mark.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_dash` | channel | false | The `stroke_dash` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `x2` | channel | false | The `x2` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |
| `y2` | channel | false | The `y2` encoding channel. |

## `Mark.cartesian.subplot`

A data-driven child plot positioned in Cartesian coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `height` | property | false | Child plot-area height. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `width` | property | false | Child plot-area width. |
| `zindex` | property | false | Integer rendering order. |
| `key` | channel | false | The `key` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.polar.subplot`

A data-driven child plot positioned in polar coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `height` | property | false | Child plot-area height. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `width` | property | false | Child plot-area width. |
| `zindex` | property | false | Integer rendering order. |
| `key` | channel | false | The `key` encoding channel. |
| `r` | channel | false | The `r` encoding channel. |
| `theta` | channel | false | The `theta` encoding channel. |

## `Mark.cartesian.symbol`

A point symbol in Cartesian coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `angle` | channel | false | The `angle` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `fill_pattern` | channel | false | The `fill_pattern` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `shape` | channel | false | The `shape` encoding channel. |
| `size` | channel | false | The `size` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.geo.symbol`

A symbol positioned by projected x/y or geographic lon/lat channels.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `lon_lat` | property | false | Convenience pair `[longitude, latitude]`; do not also author `lon` or `lat`. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `angle` | channel | false | The `angle` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `fill_pattern` | channel | false | The `fill_pattern` encoding channel. |
| `lat` | channel | false | The `lat` encoding channel. |
| `lon` | channel | false | The `lon` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `shape` | channel | false | The `shape` encoding channel. |
| `size` | channel | false | The `size` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.polar.symbol`

A point symbol in radial and angular coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `angle` | channel | false | The `angle` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `r` | channel | false | The `r` encoding channel. |
| `shape` | channel | false | The `shape` encoding channel. |
| `size` | channel | false | The `size` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `theta` | channel | false | The `theta` encoding channel. |

## `Mark.zerod.symbol`

A non-spatial symbol placed at the plot center.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `angle` | channel | false | The `angle` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `shape` | channel | false | The `shape` encoding channel. |
| `size` | channel | false | The `size` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |

## `Mark.cartesian.text`

Text positioned in Cartesian coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `syntax` | property | false | Text syntax mode; defaults to `plain`. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `align` | channel | false | The `align` encoding channel. |
| `angle` | channel | false | The `angle` encoding channel. |
| `baseline` | channel | false | The `baseline` encoding channel. |
| `color` | channel | false | The `color` encoding channel. |
| `defined` | channel | false | The `defined` encoding channel. |
| `font` | channel | false | The `font` encoding channel. |
| `font_size` | channel | false | The `font_size` encoding channel. |
| `font_style` | channel | false | The `font_style` encoding channel. |
| `font_weight` | channel | false | The `font_weight` encoding channel. |
| `leader` | channel | false | The `leader` encoding channel. |
| `leader_arrow` | channel | false | The `leader_arrow` encoding channel. |
| `leader_arrow_length` | channel | false | The `leader_arrow_length` encoding channel. |
| `leader_arrow_width` | channel | false | The `leader_arrow_width` encoding channel. |
| `leader_label_padding` | channel | false | The `leader_label_padding` encoding channel. |
| `leader_min_length` | channel | false | The `leader_min_length` encoding channel. |
| `leader_offset_x` | channel | false | The `leader_offset_x` encoding channel. |
| `leader_offset_y` | channel | false | The `leader_offset_y` encoding channel. |
| `leader_shape` | channel | false | The `leader_shape` encoding channel. |
| `leader_stroke` | channel | false | The `leader_stroke` encoding channel. |
| `leader_stroke_cap` | channel | false | The `leader_stroke_cap` encoding channel. |
| `leader_stroke_dash` | channel | false | The `leader_stroke_dash` encoding channel. |
| `leader_stroke_join` | channel | false | The `leader_stroke_join` encoding channel. |
| `leader_stroke_width` | channel | false | The `leader_stroke_width` encoding channel. |
| `leader_target_radius` | channel | false | The `leader_target_radius` encoding channel. |
| `limit` | channel | false | The `limit` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `text` | channel | false | The `text` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.polar.text`

Text positioned in radial and angular coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `syntax` | property | false | Text syntax mode; defaults to `plain`. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `align` | channel | false | The `align` encoding channel. |
| `angle` | channel | false | The `angle` encoding channel. |
| `baseline` | channel | false | The `baseline` encoding channel. |
| `color` | channel | false | The `color` encoding channel. |
| `defined` | channel | false | The `defined` encoding channel. |
| `font` | channel | false | The `font` encoding channel. |
| `font_size` | channel | false | The `font_size` encoding channel. |
| `font_style` | channel | false | The `font_style` encoding channel. |
| `font_weight` | channel | false | The `font_weight` encoding channel. |
| `leader` | channel | false | The `leader` encoding channel. |
| `leader_arrow` | channel | false | The `leader_arrow` encoding channel. |
| `leader_arrow_length` | channel | false | The `leader_arrow_length` encoding channel. |
| `leader_arrow_width` | channel | false | The `leader_arrow_width` encoding channel. |
| `leader_label_padding` | channel | false | The `leader_label_padding` encoding channel. |
| `leader_min_length` | channel | false | The `leader_min_length` encoding channel. |
| `leader_offset_x` | channel | false | The `leader_offset_x` encoding channel. |
| `leader_offset_y` | channel | false | The `leader_offset_y` encoding channel. |
| `leader_shape` | channel | false | The `leader_shape` encoding channel. |
| `leader_stroke` | channel | false | The `leader_stroke` encoding channel. |
| `leader_stroke_cap` | channel | false | The `leader_stroke_cap` encoding channel. |
| `leader_stroke_dash` | channel | false | The `leader_stroke_dash` encoding channel. |
| `leader_stroke_join` | channel | false | The `leader_stroke_join` encoding channel. |
| `leader_stroke_width` | channel | false | The `leader_stroke_width` encoding channel. |
| `leader_target_radius` | channel | false | The `leader_target_radius` encoding channel. |
| `limit` | channel | false | The `limit` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `r` | channel | false | The `r` encoding channel. |
| `text` | channel | false | The `text` encoding channel. |
| `theta` | channel | false | The `theta` encoding channel. |

## `Mark.zerod.text`

Text placed at the plot center.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `syntax` | property | false | Text syntax mode; defaults to `plain`. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `align` | channel | false | The `align` encoding channel. |
| `angle` | channel | false | The `angle` encoding channel. |
| `baseline` | channel | false | The `baseline` encoding channel. |
| `color` | channel | false | The `color` encoding channel. |
| `defined` | channel | false | The `defined` encoding channel. |
| `font` | channel | false | The `font` encoding channel. |
| `font_size` | channel | false | The `font_size` encoding channel. |
| `font_style` | channel | false | The `font_style` encoding channel. |
| `font_weight` | channel | false | The `font_weight` encoding channel. |
| `limit` | channel | false | The `limit` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `text` | channel | false | The `text` encoding channel. |

## `Mark.cartesian.trail`

A variable-width Cartesian trail mark.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `defined` | channel | false | The `defined` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `order` | channel | false | The `order` encoding channel. |
| `size` | channel | false | The `size` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |

## `Mark.treemap.tree_header`

Header bars for visible non-leaf treemap nodes.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `max_depth` | property | false | Maximum relative hierarchy depth. |
| `min_depth` | property | false | Minimum relative hierarchy depth. |
| `padding_px` | property | false | Header text padding in pixels. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `fill` | channel | false | The `fill` encoding channel. |
| `font` | channel | false | The `font` encoding channel. |
| `font_size` | channel | false | The `font_size` encoding channel. |
| `font_style` | channel | false | The `font_style` encoding channel. |
| `font_weight` | channel | false | The `font_weight` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `text` | channel | false | The `text` encoding channel. |
| `text_color` | channel | false | The `text_color` encoding channel. |

## `Mark.treemap.tree_label`

Labels fitted inside visible treemap nodes.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `fit` | property | false | Overflow behavior for labels. |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `min_height_px` | property | false | Minimum node height for a label. |
| `min_width_px` | property | false | Minimum node width for a label. |
| `node_mode` | property | false | Node set to render; an integer selects one relative depth. |
| `padding_px` | property | false | Inner label padding in pixels. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `align` | channel | false | The `align` encoding channel. |
| `baseline` | channel | false | The `baseline` encoding channel. |
| `color` | channel | false | The `color` encoding channel. |
| `font` | channel | false | The `font` encoding channel. |
| `font_size` | channel | false | The `font_size` encoding channel. |
| `font_style` | channel | false | The `font_style` encoding channel. |
| `font_weight` | channel | false | The `font_weight` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `text` | channel | false | The `text` encoding channel. |

## `Mark.treemap.tree_rect`

Rectangles for visible treemap nodes.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `node_mode` | property | false | Node set to render; an integer selects one relative depth. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `zindex` | property | false | Integer rendering order. |
| `corner_radius` | channel | false | The `corner_radius` encoding channel. |
| `fill` | channel | false | The `fill` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `u` | channel | false | The `u` encoding channel. |
| `u2` | channel | false | The `u2` encoding channel. |
| `v` | channel | false | The `v` encoding channel. |
| `v2` | channel | false | The `v2` encoding channel. |

## `Mark.cartesian.uniform_raster_2d`

A uniformly binned two-dimensional raster image.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `fill_by` | property | false | Categorical raster plane dimension that drives the fill scale. |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `raster` | property | true | Raster struct expression, usually a rasterize_2d output handle. |
| `smooth` | property | false | Enable smooth image sampling. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `x` | property | false | Configured raster x-dimension handle. |
| `y` | property | false | Configured raster y-dimension handle. |
| `zindex` | property | false | Integer rendering order. |
| `fill` | channel | false | The `fill` encoding channel. |
| `non_finite_color` | channel | false | The `non_finite_color` encoding channel. |
| `null_color` | channel | false | The `null_color` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `opacity_by_total` | channel | false | Configuration for the internal per-pixel total-to-opacity channel. |

## `Mark.geo.uniform_raster_2d`

A uniformly binned two-dimensional raster image.

| Name | Role | Required | Description |
|---|---|---:|---|
| `details` | property | false | Data field names retained for interaction details and path partitioning. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `fill_by` | property | false | Categorical raster plane dimension that drives the fill scale. |
| `geometry_space` | property | false | Space in which the mark constructs geometry. |
| `raster` | property | true | Raster struct expression, usually a rasterize_2d output handle. |
| `smooth` | property | false | Enable smooth image sampling. |
| `visible` | property | false | Scalar boolean expression controlling whether the mark is rendered. |
| `x` | property | false | Configured raster x-dimension handle. |
| `y` | property | false | Configured raster y-dimension handle. |
| `zindex` | property | false | Integer rendering order. |
| `fill` | channel | false | The `fill` encoding channel. |
| `non_finite_color` | channel | false | The `non_finite_color` encoding channel. |
| `null_color` | channel | false | The `null_color` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `opacity_by_total` | channel | false | Configuration for the internal per-pixel total-to-opacity channel. |

## `Mark.cartesian.violin`

A compound kernel-density violin generated from Cartesian primitives and transforms.

| Name | Role | Required | Description |
|---|---|---:|---|
| `bandwidth` | property | false | Kernel bandwidth expression; zero requests automatic bandwidth selection. |
| `counts` | property | false | Scale density values by the number of samples in each group. |
| `density_data_scope` | property | false | Facet coordination scope used to compute density values. |
| `density_extent` | property | false | Two scalar SQL expressions defining the density sample interval. |
| `density_extent_resolve` | property | false | Coordination strategy for inferred KDE extents. |
| `facet_data_scope` | property | false | Facet visibility scope: filtered, broadcast, or level(n). |
| `orientation` | property | false | Explicit value-axis orientation; otherwise inferred from the position channels. |
| `steps` | property | false | Number of density samples as a scalar SQL expression; defaults to 200. |
| `width` | property | false | Fraction of the group band occupied by the violin, in (0, 1]. |
| `width_normalization` | property | false | Density-to-width normalization strategy. |
| `fill` | channel | false | Violin body fill encoding. |
| `opacity` | channel | false | Violin body opacity encoding. |
| `stroke` | channel | false | Violin body stroke encoding. |
| `stroke_dash` | channel | false | Violin body stroke-dash encoding. |
| `stroke_width` | channel | false | Violin body stroke-width encoding. |
| `x` | channel | true | Horizontal position or grouping encoding. |
| `y` | channel | true | Vertical position or grouping encoding. |

## `Transform.aggregate`

Group rows and compute named aggregate measures.

| Name | Role | Required | Description |
|---|---|---:|---|
| `expressions` | property | false | Named aggregate expressions written as `expression AS output`. |
| `group_by` | property | false | One grouping expression or an array of grouping expressions. |
| `scope` | property | false | Coordination scope for this transform stage. |

Dynamic transform outputs:

- ProjectionAliases { property: "expressions" }: Each projection alias exposes a same-named field handle.

## `Transform.bin`

Discretize a quantitative field into stable interval columns.

| Name | Role | Required | Description |
|---|---|---:|---|
| `anchor` | property | false | Optional boundary anchor. |
| `base` | property | false | Radix used to choose candidate steps. |
| `divide` | property | false | Positive divisors used to refine candidate steps. |
| `extent` | property | false | Two expressions defining the input extent. |
| `field` | property | true | The quantitative input expression. |
| `maxbins` | property | false | Requested maximum bin count. |
| `minstep` | property | false | Minimum allowed step. |
| `name` | property | false | Base name for generated columns and state. |
| `nice` | property | false | Whether to choose pleasant boundaries. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `span` | property | false | Optional extent span override. |
| `step` | property | false | Exact requested step. |
| `steps` | property | false | Explicit positive candidate steps. |

## `Transform.calculate`

Append user-named columns computed from row expressions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `expressions` | property | true | Named row expressions written as `expression AS output`. |
| `scope` | property | false | Coordination scope for this transform stage. |

Dynamic transform outputs:

- ProjectionAliases { property: "expressions" }: Each expression exposes a same-named output field handle.

## `Transform.filter`

Retain rows for which a predicate is true.

| Name | Role | Required | Description |
|---|---|---:|---|
| `predicate` | property | true | Boolean row predicate. |
| `scope` | property | false | Coordination scope for this transform stage. |

## `Transform.fold`

Turn a named set of source expressions into key/value rows.

| Name | Role | Required | Description |
|---|---|---:|---|
| `as_key` | property | false | Generated key column name. |
| `as_value` | property | false | Generated value column name. |
| `fields` | property | true | A map from emitted key labels to source expressions. |
| `index` | property | false | Optional generated source-order column. |
| `scope` | property | false | Coordination scope for this transform stage. |

## `Transform.impute`

Insert missing key rows and fill a value expression within groups.

| Name | Role | Required | Description |
|---|---|---:|---|
| `as_value` | property | false | Generated value column name. |
| `field` | property | true | The value expression to impute. |
| `fill_value` | property | false | Fill expression required by the `value` method. |
| `flag` | property | false | Optional generated imputation flag column. |
| `group_by` | property | false | Expressions defining independent imputation groups. |
| `key` | property | true | The key expression whose domain is completed. |
| `method` | property | true | The fill strategy. |
| `scope` | property | false | Coordination scope for this transform stage. |

## `Transform.join_aggregate`

Compute grouped aggregate measures and join them back onto every input row.

| Name | Role | Required | Description |
|---|---|---:|---|
| `expressions` | property | true | Named aggregate expressions written as `expression AS output`. |
| `group_by` | property | false | One grouping expression or an array of grouping expressions. |
| `scope` | property | false | Coordination scope for this transform stage. |

Dynamic transform outputs:

- ProjectionAliases { property: "expressions" }: Each projection alias exposes a same-named field handle.

## `Transform.kde`

Estimate a one-dimensional kernel density, optionally by group.

| Name | Role | Required | Description |
|---|---|---:|---|
| `as_fields` | property | false | Two names for the generated value and density columns. |
| `bandwidth` | property | false | Kernel bandwidth; zero selects an automatic value. |
| `counts` | property | false | Scale density by group sample count. |
| `cumulative` | property | false | Emit a cumulative density estimate. |
| `extent` | property | false | Two expressions defining the evaluation interval. |
| `field` | property | true | The quantitative sample expression. |
| `group_by` | property | false | Simple columns defining independent density groups. |
| `resolve` | property | false | Whether groups use independent or shared evaluation domains. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `steps` | property | false | Number of evaluation samples. |

## `Transform.lump`

Keep the highest-ranked categories and combine or drop the remainder.

| Name | Role | Required | Description |
|---|---|---:|---|
| `drop_other` | property | false | Drop categories outside the retained set. |
| `field` | property | true | The categorical value to rank. |
| `keep` | property | false | Predicate identifying retained ranks. |
| `name` | property | false | Base name for generated columns. |
| `order` | property | false | Ranking direction. |
| `order_by` | property | false | Aggregate ranking expression. |
| `other` | property | false | Replacement value for combined categories. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `top_n` | property | true | Scalar number of categories to retain. |
| `window` | property | false | Window ranking expression. |

## `Transform.pipeline`

Run ordered child transforms behind one native parent stage and expose only declared public outputs.

| Name | Role | Required | Description |
|---|---|---:|---|
| `scope` | property | false | Coordination scope for this transform stage. |

## `Transform.rasterize_2d`

Bin two quantitative expressions into a dense two-dimensional raster.

| Name | Role | Required | Description |
|---|---|---:|---|
| `agg` | property | false | Cell reducer applied to the optional value expression. |
| `by` | property | false | Categorical expression producing an additional raster plane dimension. |
| `frame` | property | false | Coordinate reference system asserted for input extents and output geometry. |
| `name` | property | false | Physical raster output column name. |
| `partition_by` | property | false | Expressions producing independent raster rows. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `value` | property | false | Value expression reduced into each cell; required except for count. |
| `x` | property | true | Horizontal input expression. |
| `x_dim` | property | false | Horizontal raster dimension settings. |
| `y` | property | true | Vertical input expression. |
| `y_dim` | property | false | Vertical raster dimension settings. |

## `Transform.scalar_aggregate`

Publish whole-input aggregate measures as derived scalar expressions without changing rows.

| Name | Role | Required | Description |
|---|---|---:|---|
| `evaluation` | property | false | Whether to materialize literal scalars eagerly or publish scalar subqueries lazily. |
| `expressions` | property | true | Named aggregate expressions written as `expression AS output`. |
| `scope` | property | false | Coordination scope for this transform stage. |

Dynamic transform outputs:

- ProjectionAliases { property: "expressions" }: Each projection alias exposes a same-named derived scalar handle.

## `Transform.select`

Project an ordered set of source columns and explicitly aliased expressions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `expressions` | property | true | Ordered direct columns and explicitly aliased computed expressions. |
| `scope` | property | false | Coordination scope for this transform stage. |

Dynamic transform outputs:

- ProjectionAliases { property: "expressions" }: Each explicitly aliased projection exposes a same-named field handle.

## `Transform.sql`

Run one DataFusion SQL query against the reserved `input` relation.

| Name | Role | Required | Description |
|---|---|---:|---|
| `query` | property | true | The SQL query. |
| `scope` | property | false | Coordination scope for this transform stage. |

## `Transform.stack`

Compute stacked start and end positions for a quantitative field.

| Name | Role | Required | Description |
|---|---|---:|---|
| `field` | property | true | The quantitative value to stack. |
| `group_by` | property | false | Expressions that partition independent stacks. |
| `name` | property | false | Base name for generated boundary columns. |
| `offset` | property | false | Stack baseline and normalization mode. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `sort_by` | property | false | Expressions that order rows within each stack. |
| `value_name` | property | false | Optional copied value output column name. |

## `Transform.time_fill`

Complete missing calendar hierarchy rows and fill their values.

| Name | Role | Required | Description |
|---|---|---:|---|
| `as_value` | property | false | Generated value column name. |
| `extent` | property | false | Optional explicit hierarchy-aligned extent. |
| `field` | property | true | The value expression to fill. |
| `fill_value` | property | true | Value used for generated rows. |
| `flag` | property | false | Optional generated-row flag column. |
| `group_by` | property | false | Expressions defining independent completion groups. |
| `levels` | property | true | A `time_levels.levels` metadata handle. |
| `scope` | property | false | Coordination scope for this transform stage. |

## `Transform.time_levels`

Derive an ordered categorical calendar hierarchy from timestamps.

| Name | Role | Required | Description |
|---|---|---:|---|
| `field` | property | true | The temporal input expression. |
| `levels` | property | true | Ordered calendar levels, either atoms or configured level objects. |
| `name` | property | false | Base name for generated key columns. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `time_context` | property | false | Timezone and week-start overrides. |

Dynamic transform outputs:

- ArrayValueNames { property: "levels" }: Each atom level exposes its generated key as a same-named handle.

- ArrayObjectField { property: "levels", field: "level" }: Each configured level exposes its generated key by level name.

## `Transform.time_unit`

Discretize timestamps into calendar-aware interval boundaries.

| Name | Role | Required | Description |
|---|---|---:|---|
| `field` | property | true | The temporal input expression. |
| `interval` | property | false | Whether to emit interval end boundaries. |
| `maxbins` | property | false | Requested maximum interval count. |
| `name` | property | false | Base name for generated columns and state. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `time_context` | property | false | Timezone and week-start overrides. |
| `units` | property | false | One calendar unit or an ordered array of units; overrides `maxbins`. |

## `Transform.window`

Append user-named SQL window expressions over optional partitions and ordering.

| Name | Role | Required | Description |
|---|---|---:|---|
| `expressions` | property | true | Named SQL window expressions written as `expression AS output`. |
| `order_by` | property | false | Expressions defining ascending nulls-last order. |
| `partition_by` | property | false | Expressions defining independent window partitions. |
| `scope` | property | false | Coordination scope for this transform stage. |

Dynamic transform outputs:

- ProjectionAliases { property: "expressions" }: Each user-named window expression exposes a same-named field handle.

## `Tool.box_selection`

Rectangular interval selection with a visible Cartesian overlay.

| Name | Role | Required | Description |
|---|---|---:|---|
| `channels` | property | false | Exactly two channel names, horizontal then vertical. |
| `double_click_clear` | property | false | Clear on double click. |
| `drag_button` | property | false | Pointer button used for dragging. |
| `enabled_by_default` | property | false | Initial enabled state. |
| `facet_scope` | property | false | Facet scope of generated clauses. |
| `repeat_cell_chrome` | property | false | Draw one overlay per repeat cell. |
| `resolve` | property | false | Multi-box selection resolution. |
| `selection` | property | true | Selection state updated by this tool. |
| `unit_aspect_box` | property | false | Optional aspect constraint for the interaction box. |
| `x_channel` | property | false | Horizontal scale channel. |
| `x_dimension` | property | false | Horizontal selection expression. |
| `y_channel` | property | false | Vertical scale channel. |
| `y_dimension` | property | false | Vertical selection expression. |

Exports:

- `enabled` (`param<boolean>`): Whether this tool currently handles input.

- `selection` (`selection`): Selection state owned and updated by this tool.

- `store` (`store<struct(field(utf8,'id'),field(utf8,'cell_id'),field(utf8,'row_id'),field(utf8,'column_id'),field(float64,'x_min'),field(float64,'x_max'),field(float64,'y_min'),field(float64,'y_max'))>`): Hidden interval-row backing store.

## `Tool.box_zoom`

Rectangular drag zoom for Cartesian scale domains.

| Name | Role | Required | Description |
|---|---|---:|---|
| `channels` | property | false | Exactly two channel names, horizontal then vertical. |
| `drag_button` | property | false | Pointer button used for dragging. |
| `enabled_by_default` | property | false | Initial enabled state. |
| `min_size_px` | property | false | Minimum accepted drag-box size. |
| `unit_aspect_box` | property | false | Optional aspect constraint for the interaction box. |
| `x_channel` | property | false | Horizontal scale channel. |
| `x_domain_param` | property | false | Existing x-domain parameter. |
| `x_sharing` | property | false | Explicit x-domain sharing scope. |
| `y_channel` | property | false | Vertical scale channel. |
| `y_domain_param` | property | false | Existing y-domain parameter. |
| `y_sharing` | property | false | Explicit y-domain sharing scope. |

Exports:

- `active` (`param<boolean>`): Whether a box gesture is active.

- `box_x0` (`param<float64>`): Overlay starting x coordinate.

- `box_x1` (`param<float64>`): Overlay ending x coordinate.

- `box_y0` (`param<float64>`): Overlay starting y coordinate.

- `box_y1` (`param<float64>`): Overlay ending y coordinate.

- `enabled` (`param<boolean>`): Whether this tool currently handles input.

- `x_domain` (`param<fixed_size_list(float64,2)>`): Current x domain.

- `y_domain` (`param<fixed_size_list(float64,2)>`): Current y domain.

## `Tool.geo_pan_zoom`

Projected-plane pan, wheel zoom, box zoom, and reset behavior for geo plots.

| Name | Role | Required | Description |
|---|---|---:|---|
| `box_zoom` | property | false | Enable drag-box zoom. |
| `box_zoom_min_size_px` | property | false | Minimum accepted box size in pixels. |
| `box_zoom_requires_shift` | property | false | Require Shift for drag-box zoom. |
| `consume_wheel` | property | false | Consume handled wheel events. |
| `drag_button` | property | false | Pointer button used for dragging. |
| `enabled_by_default` | property | false | Initial enabled state. |
| `scroll_zoom` | property | false | Enable wheel zoom. |
| `settle_exact` | property | false | Run exact evaluation after previews. |
| `sharing` | property | false | Sharing scope for viewport parameters. |
| `viewport_id` | property | false | Geo viewport state id prefix. |
| `zoom_base` | property | false | Multiplicative wheel-zoom base. |

Exports:

- `box_active` (`param<boolean>`): Whether box zoom is active.

- `box_x0` (`param<float64>`): Box starting x coordinate.

- `box_x1` (`param<float64>`): Box ending x coordinate.

- `box_y0` (`param<float64>`): Box starting y coordinate.

- `box_y1` (`param<float64>`): Box ending y coordinate.

- `center_x` (`param<float64>`): Projected viewport center x.

- `center_y` (`param<float64>`): Projected viewport center y.

- `enabled` (`param<boolean>`): Whether the tool handles input.

- `focus_x` (`param<float64>`): Most recent zoom focus x.

- `focus_y` (`param<float64>`): Most recent zoom focus y.

- `units_per_pixel` (`param<float64>`): Projected units per display pixel.

## `Tool.lasso_selection`

Freehand polygon selection over rendered mark geometry.

| Name | Role | Required | Description |
|---|---|---:|---|
| `double_click_clear` | property | false | Clear on double click. |
| `drag_button` | property | false | Pointer button used for lassoing. |
| `enabled_by_default` | property | false | Initial enabled state. |
| `event_path_min_distance_px` | property | false | Minimum sampled distance between event-path points. |
| `facet_scope` | property | false | Facet scope of generated clauses. |
| `fields` | property | true | Selection field names evaluated against same-named datum fields. |
| `marks` | property | false | Optional target mark ids. |
| `selection` | property | true | Selection state updated by this tool. |

Exports:

- `enabled` (`param<boolean>`): Whether this tool currently handles input.

- `selection` (`selection`): Selection state owned and updated by this tool.

## `Tool.pan_scroll_zoom`

Pointer-drag panning and wheel zoom for Cartesian scale domains.

| Name | Role | Required | Description |
|---|---|---:|---|
| `consume_wheel` | property | false | Consume handled wheel events. |
| `drag_button` | property | false | Pointer button used for panning. |
| `enabled_by_default` | property | false | Initial enabled state. |
| `scroll_zoom` | property | false | Enable wheel zoom. |
| `settle_exact` | property | false | Run an exact evaluation after previews. |
| `x_channel` | property | false | Horizontal scale channel name. |
| `x_domain_param` | property | false | Existing x-domain parameter. |
| `x_sharing` | property | false | Explicit x-domain sharing scope. |
| `y_channel` | property | false | Vertical scale channel name. |
| `y_domain_param` | property | false | Existing y-domain parameter. |
| `y_sharing` | property | false | Explicit y-domain sharing scope. |
| `zoom_base` | property | false | Multiplicative wheel-zoom base. |

Exports:

- `enabled` (`param<boolean>`): Whether this tool currently handles input.

- `x_domain` (`param<fixed_size_list(float64,2)>`): Current x-domain override.

- `y_domain` (`param<fixed_size_list(float64,2)>`): Current y-domain override.

## `Tool.point_selection`

Click-driven equality selection over one or more datum fields.

| Name | Role | Required | Description |
|---|---|---:|---|
| `clause_id` | property | false | Clause identity expression required for multiple dimensions. |
| `double_click_clear` | property | false | Clear on double click. |
| `enabled_by_default` | property | false | Initial enabled state. |
| `facet_scope` | property | false | Facet scope of generated clauses. |
| `fields` | property | true | Selection field names evaluated against same-named datum fields. |
| `selection` | property | true | Selection state updated by this tool. |
| `shift_toggle` | property | false | Enable shift-click clause toggling. |

Exports:

- `enabled` (`param<boolean>`): Whether this tool currently handles input.

- `selection` (`selection`): Selection state owned and updated by this tool.

## `Widget.button`

A momentary button with a monotonic activation count.

| Name | Role | Required | Description |
|---|---|---:|---|
| `action` | property | false | Ordered shared-state mutations run atomically after each activation. |
| `activation_param` | property | false | Existing UInt64 parameter bound to the activation count. |
| `label` | property | true | Nonempty visible label. |
| `position` | property | true | Containing chart guide-slot edge. |
| `variant` | property | false | Semantic visual treatment; defaults to neutral. |

Exports:

- `activations` (`param<uint64>`): Number of completed activations.

## `Widget.checkbox`

A scalar boolean checkbox control.

| Name | Role | Required | Description |
|---|---|---:|---|
| `checked_param` | property | false | Existing boolean parameter bound to the checked state. |
| `default` | property | true | Initial checked state. |
| `label` | property | true | Nonempty visible label. |
| `position` | property | true | Containing chart guide-slot edge. |

Exports:

- `checked` (`param<boolean>`): The current checked state.

## `Widget.checkbox_list`

An ordered list of independently toggleable selection values.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | true | Ordered widget item relation. |
| `label` | property | false | Item label expression; defaults to the `label` column. |
| `order_by` | property | false | Nonempty total order required for non-inline data. |
| `position` | property | true | Containing chart guide-slot edge. |
| `selection` | property | false | Existing equality selection managed by the widget. |
| `value` | property | false | Item value expression; defaults to the `value` column. |

Exports:

- `selection` (`selection`): The equality selection managed by the list.

## `Widget.radio_button_list`

An ordered list that selects exactly one scalar value.

| Name | Role | Required | Description |
|---|---|---:|---|
| `data` | property | true | Ordered widget item relation. |
| `default` | property | false | Initial selected scalar; inferred only for compatible inline data. |
| `label` | property | false | Item label expression; defaults to the `label` column. |
| `order_by` | property | false | Nonempty total order required for non-inline data. |
| `position` | property | true | Containing chart guide-slot edge. |
| `value` | property | false | Item value expression; defaults to the `value` column. |
| `value_param` | property | false | Existing item-typed parameter bound to the selected value. |

Exports:

- `value` (`param<item_scalar>`): The currently selected item value.

## `Widget.slider`

A bounded Float64 slider control.

| Name | Role | Required | Description |
|---|---|---:|---|
| `default` | property | false | Initial value; defaults to min. |
| `format` | property | false | d3-compatible number format. |
| `max` | property | true | Finite upper bound greater than min. |
| `min` | property | true | Finite lower bound. |
| `position` | property | true | Containing chart guide-slot edge. |
| `step` | property | false | Positive quantization step; defaults to 1. |
| `throttle_ms` | property | false | Optional drag-update throttle interval. |
| `title` | property | false | Visible caption above the track. |
| `value_param` | property | false | Existing Float64 parameter bound to the value. |

Exports:

- `value` (`param<float64>`): The current slider value.

## `Widget.text_input`

A native single-line UTF-8 text input.

| Name | Role | Required | Description |
|---|---|---:|---|
| `commit` | property | false | Commit policy; defaults to on_change. |
| `debounce_ms` | property | false | On-change quiet period; defaults to 150 ms. |
| `default` | property | false | Initial committed value; defaults to empty. |
| `placeholder` | property | false | Hint shown while the value is empty. |
| `position` | property | true | Containing chart guide-slot edge. |
| `value_param` | property | false | Existing UTF-8 parameter bound to the committed value. |

Exports:

- `cursor_position` (`param<uint64>`): Lazy committed-text cursor position in grapheme units.

- `selected_text` (`param<utf8>`): Lazy selected committed text.

- `value` (`param<utf8>`): The committed text value.

## `Scale.band`

A discrete band-position scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `align` | property | false | Band alignment within the range, from zero through one. |
| `domain` | property | false | Explicit scale domain values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `padding_inner` | property | false | Fractional padding between adjacent bands. |
| `padding_outer` | property | false | Fractional padding outside the first and last bands. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |
| `round` | property | false | Round band positions and widths to whole pixels. |

## `Scale.linear`

A continuous linear scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `clamp` | property | false | Clamp outputs to the configured range. |
| `domain` | property | false | Explicit scale domain values. |
| `nice` | property | false | Round the domain to pleasant values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `padding` | property | false | Add pixel padding around the inferred domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |
| `zero` | property | false | Include zero in the inferred domain. |

## `Scale.log`

A logarithmic continuous scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `base` | property | false | Positive logarithm base. |
| `clamp` | property | false | Clamp outputs to the configured range. |
| `domain` | property | false | Explicit scale domain values. |
| `nice` | property | false | Round the domain to pleasant powers. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |

## `Scale.nested_band`

A hierarchical discrete band-position scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `domain` | property | false | Explicit scale domain values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |

## `Scale.ordinal`

A discrete ordinal scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `domain` | property | false | Explicit scale domain values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |
| `unknown` | property | false | Output value used for inputs absent from the domain. |

## `Scale.point`

A discrete point-position scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `align` | property | false | Point alignment within the range, from zero through one. |
| `domain` | property | false | Explicit scale domain values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `padding` | property | false | Fractional outer padding. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |
| `round` | property | false | Round point positions to whole pixels. |

## `Scale.pow`

A power-transformed continuous scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `clamp` | property | false | Clamp outputs to the configured range. |
| `domain` | property | false | Explicit scale domain values. |
| `exponent` | property | false | Power exponent. |
| `nice` | property | false | Round the domain to pleasant values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |
| `zero` | property | false | Include zero in the inferred domain. |

## `Scale.quantile`

A quantile scale derived from a sample domain.

| Name | Role | Required | Description |
|---|---|---:|---|
| `domain` | property | false | Explicit scale domain values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |

## `Scale.quantize`

A uniformly quantized continuous-domain scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `domain` | property | false | Explicit scale domain values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |

## `Scale.sqrt`

A square-root continuous scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `clamp` | property | false | Clamp outputs to the configured range. |
| `domain` | property | false | Explicit scale domain values. |
| `nice` | property | false | Round the domain to pleasant values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |
| `zero` | property | false | Include zero in the inferred domain. |

## `Scale.symlog`

A symmetric-log continuous scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `clamp` | property | false | Clamp outputs to the configured range. |
| `constant` | property | false | Positive linear-region constant around zero. |
| `domain` | property | false | Explicit scale domain values. |
| `nice` | property | false | Round the domain to pleasant values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |

## `Scale.threshold`

A discrete threshold scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `domain` | property | false | Explicit scale domain values. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |

## `Scale.time`

A temporal continuous scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `clamp` | property | false | Clamp outputs to the configured range. |
| `domain` | property | false | Explicit scale domain values. |
| `nice` | property | false | Round the temporal domain to pleasant boundaries. |
| `order` | property | false | Direction for inferred categorical-domain ordering. |
| `order_by` | property | false | Expression used to order an inferred categorical domain. |
| `range` | property | false | Explicit scalar scale range values. |
| `raw_domain` | property | false | Runtime interval-domain override, with the regular domain retained as fallback. |

## `Axis.cartesian`

A Cartesian position-axis configuration.

| Name | Role | Required | Description |
|---|---|---:|---|
| `datetime_format` | property | false | Date/time-format pattern. |
| `format` | property | false | Number-format pattern. |
| `grid` | property | false | Whether to draw grid lines. |
| `label_angle` | property | false | Tick-label rotation angle in degrees. |
| `label_font_family` | property | false | Tick-label font family. |
| `number_locale` | property | false | Number-format locale identifier. |
| `position` | property | false | Axis side or crossing position. |
| `show_title` | property | false | Whether to render the title while retaining its configuration. |
| `tick_count` | property | false | Requested number of ticks. |
| `tick_label` | property | false | Expression producing tick-label text. |
| `tick_spacing` | property | false | Structured start/step tick-spacing expression. |
| `title` | property | false | Axis title text. |
| `title_font_family` | property | false | Axis-title font family. |
| `title_syntax` | property | false | Axis-title text syntax. |
| `visible` | property | false | Whether the axis is visible. |

## `Axis.polar`

A radial or angular polar-axis configuration.

| Name | Role | Required | Description |
|---|---|---:|---|
| `axis_type` | property | false | Whether this is a radial or angular axis. |
| `direction` | property | false | Angular-axis direction. |
| `format` | property | false | Number-format pattern. |
| `grid` | property | false | Whether to draw grid lines. |
| `grid_levels` | property | false | Radial or angular values at which grid levels are drawn. |
| `start_angle` | property | false | Angular-axis starting angle. |
| `tick_count` | property | false | Requested number of ticks. |
| `title` | property | false | Axis title text. |
| `visible` | property | false | Whether the axis is visible. |

## `Legend.standard`

A standard discrete or continuous chart legend configuration.

| Name | Role | Required | Description |
|---|---|---:|---|
| `background_corner_radius` | property | false | Legend background corner radius. |
| `background_fill` | property | false | Legend background fill color. |
| `background_padding` | property | false | Padding between the background and content. |
| `background_stroke` | property | false | Legend background stroke color. |
| `background_stroke_width` | property | false | Legend background stroke width. |
| `columns` | property | false | Number of discrete legend columns. |
| `format_number` | property | false | Numeric label format pattern. |
| `gradient_thickness` | property | false | Continuous colorbar thickness. |
| `label_color` | property | false | Discrete item-label color. |
| `label_font_family` | property | false | Discrete item-label font family. |
| `label_font_size` | property | false | Discrete item-label font size. |
| `label_font_weight` | property | false | Discrete item-label font weight. |
| `label_limit` | property | false | Maximum item-label width in pixels. |
| `label_syntax` | property | false | Legend-label text syntax. |
| `order` | property | false | Expression controlling discrete item order. |
| `orientation` | property | false | Legend item-flow orientation. |
| `overlay` | property | false | Marks rendered on a continuous colorbar surface using injected Cartesian value and cross-axis scales. |
| `position` | property | false | Legend chrome-slot position. |
| `symbol_size` | property | false | Discrete legend symbol area. |
| `tick_color` | property | false | Continuous colorbar tick-label color. |
| `tick_font_family` | property | false | Continuous tick-label font family. |
| `tick_font_size` | property | false | Continuous tick-label font size. |
| `tick_font_weight` | property | false | Continuous tick-label font weight. |
| `title` | property | false | Legend title text. |
| `title_color` | property | false | Legend-title color. |
| `title_font_family` | property | false | Legend-title font family. |
| `title_font_size` | property | false | Legend-title font size. |
| `title_font_weight` | property | false | Legend-title font weight. |
| `title_syntax` | property | false | Legend-title text syntax. |
| `visible` | property | false | Whether the legend is visible. |

## `Layout.chart`

The default chart frame layout.

| Name | Role | Required | Description |
|---|---|---:|---|
| `canvas` | property | false | Canvas width/height constraints or `auto`. |
| `margins` | property | false | Fixed chart margins. |
| `plot` | property | false | Plot-area width/height constraints or `auto`. |

## `View.cartesian`

A Cartesian inline view whose domains drive view-local transforms.

| Name | Role | Required | Description |
|---|---|---:|---|
| `debounce_ms` | property | false | Quiet interval before evaluation in milliseconds. |
| `stale_policy` | property | false | Behavior while a newer view-local result is pending. |
| `throttle_ms` | property | false | Minimum interval between evaluations in milliseconds. |
| `x_domain` | property | true | Expression whose values define the horizontal view domain. |
| `y_domain` | property | true | Expression whose values define the vertical view domain. |

## `View.pixel_frame`

A scale-free logical-pixel inline view.

| Name | Role | Required | Description |
|---|---|---:|---|
| `debounce_ms` | property | false | Quiet interval before evaluation in milliseconds. |
| `stale_policy` | property | false | Behavior while a newer view-local result is pending. |
| `throttle_ms` | property | false | Minimum interval between evaluations in milliseconds. |

## `Resource.tiles`

A reusable XYZ raster tile source for geographic charts.

| Name | Role | Required | Description |
|---|---|---:|---|
| `attribution` | property | false | Required source attribution text. |
| `kind` | property | true | Tile pyramid kind; v1 supports XYZ tiles. |
| `loading_policy` | property | false | Tile loading and fallback behavior. |
| `max_zoom` | property | false | Maximum available tile zoom. |
| `min_zoom` | property | false | Minimum available tile zoom. |
| `subdomains` | property | false | Subdomains substituted for `{s}` in deterministic order. |
| `tile_size` | property | false | Tile edge length in pixels. |
| `url` | property | true | URL template containing `{z}`, `{x}`, and `{y}` placeholders. |
| `zindex` | property | false | Default layer z-index. |
