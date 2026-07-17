# Avenger native schema: bootstrap-vertical-slice

Language schema 1.0.

## `Coordinate.cartesian`

A two-dimensional Cartesian coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
| `unit_aspect` | property | false | Optional positive ratio between x and y data units. |

## `Coordinate.geo`

A geographic map projection with an optional authored viewport.

| Name | Role | Required | Description |
|---|---|---:|---|
| `center_lon_lat` | property | false | Viewport center as `[longitude, latitude]` in degrees. |
| `precision` | property | false | Adaptive projection resampling precision in pixels; zero disables it. |
| `projection` | property | false | Map projection; defaults to Equal Earth. |
| `rotate` | property | false | Three-axis spherical rotation in degrees. |
| `viewport_id` | property | false | Runtime viewport state id prefix. |
| `zoom` | property | false | Initial slippy-style zoom level. |

## `Coordinate.parallel`

A wide-form parallel-coordinate frame with user-named dimensions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `order` | property | false | Static left-to-right dimension order; undeclared dimensions follow declaration order. |

## `Coordinate.polar`

A radial and angular two-dimensional coordinate system.

## `Coordinate.treemap`

A hierarchical treemap layout coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
| `display_levels` | property | false | Maximum number of hierarchy levels displayed below the current root. |
| `path` | property | true | Ordered hierarchy-level expressions from root to leaf. |
| `root_path_id` | property | false | Initial visible hierarchy root path id. |
| `value` | property | true | Non-negative leaf weight expression used for area allocation. |

## `Coordinate.zerod`

A zero-dimensional coordinate system that places marks at the plot center.

## `Mark.cartesian.area`

A filled Cartesian area mark.

| Name | Role | Required | Description |
|---|---|---:|---|
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

## `Mark.geo.geo_shape`

A GeoJSON or WKB geometry projected through the geo coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
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
| `dimensions` | property | true | User-named dimension ids mapped to configured encoding channels. |
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
| `dimensions` | property | true | User-named dimension ids mapped to configured encoding channels. |
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
| `opacity` | channel | false | The `opacity` encoding channel. |
| `stroke` | channel | false | The `stroke` encoding channel. |
| `stroke_cap` | channel | false | The `stroke_cap` encoding channel. |
| `stroke_dash` | channel | false | The `stroke_dash` encoding channel. |
| `stroke_width` | channel | false | The `stroke_width` encoding channel. |
| `x` | channel | false | The `x` encoding channel. |
| `x2` | channel | false | The `x2` encoding channel. |
| `y` | channel | false | The `y` encoding channel. |
| `y2` | channel | false | The `y2` encoding channel. |

## `Mark.cartesian.symbol`

A point symbol in Cartesian coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
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
| `lon_lat` | property | false | Convenience pair `[longitude, latitude]`; do not also author `lon` or `lat`. |
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
| `syntax` | property | false | Text syntax mode; defaults to `plain`. |
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
| `syntax` | property | false | Text syntax mode; defaults to `plain`. |
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
| `syntax` | property | false | Text syntax mode; defaults to `plain`. |
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
| `max_depth` | property | false | Maximum relative hierarchy depth. |
| `min_depth` | property | false | Minimum relative hierarchy depth. |
| `padding_px` | property | false | Header text padding in pixels. |
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
| `fit` | property | false | Overflow behavior for labels. |
| `min_height_px` | property | false | Minimum node height for a label. |
| `min_width_px` | property | false | Minimum node width for a label. |
| `node_mode` | property | false | Node set to render; an integer selects one relative depth. |
| `padding_px` | property | false | Inner label padding in pixels. |
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
| `node_mode` | property | false | Node set to render; an integer selects one relative depth. |
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
| `fill_by` | property | false | Categorical raster plane dimension that drives the fill scale. |
| `raster` | property | true | Raster struct expression, usually a rasterize_2d output handle. |
| `smooth` | property | false | Enable smooth image sampling. |
| `x` | property | false | Configured raster x-dimension handle. |
| `y` | property | false | Configured raster y-dimension handle. |
| `fill` | channel | false | The `fill` encoding channel. |
| `non_finite_color` | channel | false | The `non_finite_color` encoding channel. |
| `null_color` | channel | false | The `null_color` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `opacity_by_total` | channel | false | Configuration for the internal per-pixel total-to-opacity channel. |

## `Mark.geo.uniform_raster_2d`

A uniformly binned two-dimensional raster image.

| Name | Role | Required | Description |
|---|---|---:|---|
| `fill_by` | property | false | Categorical raster plane dimension that drives the fill scale. |
| `raster` | property | true | Raster struct expression, usually a rasterize_2d output handle. |
| `smooth` | property | false | Enable smooth image sampling. |
| `x` | property | false | Configured raster x-dimension handle. |
| `y` | property | false | Configured raster y-dimension handle. |
| `fill` | channel | false | The `fill` encoding channel. |
| `non_finite_color` | channel | false | The `non_finite_color` encoding channel. |
| `null_color` | channel | false | The `null_color` encoding channel. |
| `opacity` | channel | false | The `opacity` encoding channel. |
| `opacity_by_total` | channel | false | Configuration for the internal per-pixel total-to-opacity channel. |

## `Transform.aggregate`

Group rows and compute named aggregate measures.

| Name | Role | Required | Description |
|---|---|---:|---|
| `group_by` | property | false | One grouping expression or an array of grouping expressions. |
| `measures` | property | false | Legacy structured named measures; user-named expression properties are preferred. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `*` | user-named property | false | A user-named aggregate expression whose property name becomes the output handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {"group_by", "measures", "scope"} }: Each user-named aggregate expression exposes a same-named field handle.

- ArrayObjectField { property: "measures", field: "name" }: Each structured measure exposes the field named by its `name` member.

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
| `scope` | property | false | Coordination scope for this transform stage. |
| `*` | user-named property | false | A user-named row expression whose property name becomes the output column and handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {"scope"} }: Each expression exposes a same-named output field handle.

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
| `group_by` | property | false | One grouping expression or an array of grouping expressions. |
| `measures` | property | false | Legacy structured named measures; user-named expression properties are preferred. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `*` | user-named property | false | A user-named aggregate expression whose property name becomes the output handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {"group_by", "measures", "scope"} }: Each user-named aggregate expression exposes a same-named field handle.

- ArrayObjectField { property: "measures", field: "name" }: Each structured measure exposes the field named by its `name` member.

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
| `measures` | property | false | Legacy structured named measures; user-named expression properties are preferred. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `*` | user-named property | false | A user-named aggregate expression whose property name becomes the output handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {"evaluation", "measures", "scope"} }: Each user-named aggregate expression exposes a same-named derived scalar handle.

- ArrayObjectField { property: "measures", field: "name" }: Each structured measure exposes the derived scalar named by its `name` member.

## `Transform.select`

Project an ordered set of source columns and explicitly aliased expressions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `expressions` | property | true | One projection expression or an ordered array of projection expressions. |
| `scope` | property | false | Coordination scope for this transform stage. |

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
| `order_by` | property | false | Expressions defining ascending nulls-last order. |
| `partition_by` | property | false | Expressions defining independent window partitions. |
| `scope` | property | false | Coordination scope for this transform stage. |
| `*` | user-named property | false | A user-named SQL window expression whose property name becomes the output handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {"order_by", "partition_by", "scope"} }: Each user-named window expression exposes a same-named field handle.

## `Tool.pan_scroll_zoom`

Pointer-drag panning and wheel zoom for Cartesian domains.

Exports:

- `x_domain` (`param<fixed_size_list(float64,2)>`): The tool-owned current x domain.

- `y_domain` (`param<fixed_size_list(float64,2)>`): The tool-owned current y domain.

## `Widget.radio_button_list`

A list that selects exactly one scalar value.

| Name | Role | Required | Description |
|---|---|---:|---|
| `default` | property | false | Initial selected scalar value. |
| `id` | property | true | Widget source id. |
| `items` | property | true | Static list items. |
| `position` | property | false | Chart chrome placement edge. |
| `value_param` | property | false | Optional existing typed parameter bound to the value state slot. |

Exports:

- `value` (`param<item_scalar>`): The currently selected item value.

## `Scale.linear`

A continuous linear scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `domain` | property | false | Explicit scale domain values. |
| `nice` | property | false | Round the domain to pleasant values. |
| `range` | property | false | Explicit scale range values. |
| `zero` | property | false | Include zero in the inferred domain. |

## `Scale.ordinal`

A discrete ordinal scale.

| Name | Role | Required | Description |
|---|---|---:|---|
| `domain` | property | false | Explicit scale domain values. |
| `range` | property | false | Explicit scale range values. |

## `Axis.cartesian`

A Cartesian axis configuration.

| Name | Role | Required | Description |
|---|---|---:|---|
| `grid` | property | false | Whether to draw grid lines. |
| `position` | property | false | Axis side position. |
| `tick_count` | property | false | Requested number of ticks. |
| `title` | property | false | Axis title. |
| `visible` | property | false | Whether the axis is visible. |

## `Legend.standard`

A standard chart legend configuration.

| Name | Role | Required | Description |
|---|---|---:|---|
| `columns` | property | false | Number of legend columns. |
| `orientation` | property | false | Legend orientation. |
| `position` | property | false | Legend chrome position. |
| `title` | property | false | Legend title. |
| `visible` | property | false | Whether the legend is visible. |

## `Layout.chart`

The default chart frame layout.

| Name | Role | Required | Description |
|---|---|---:|---|
| `canvas` | property | false | Canvas width/height constraints or `auto`. |
| `margins` | property | false | Fixed chart margins. |
| `plot` | property | false | Plot-area width/height constraints or `auto`. |
