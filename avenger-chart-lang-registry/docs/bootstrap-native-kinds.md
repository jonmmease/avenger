# Avenger native schema: bootstrap-vertical-slice

Language schema 1.0.

## `Coordinate.cartesian`

A two-dimensional Cartesian coordinate system.

| Name | Role | Required | Description |
|---|---|---:|---|
| `unit_aspect` | property | false | Optional positive ratio between x and y data units. |

## `Mark.cartesian.symbol`

A point symbol positioned in Cartesian coordinates.

| Name | Role | Required | Description |
|---|---|---:|---|
| `angle` | channel | false | The symbol `angle` encoding expression. |
| `fill` | channel | false | The symbol `fill` encoding expression. |
| `fill_pattern` | channel | false | The symbol `fill_pattern` encoding expression. |
| `opacity` | channel | false | The symbol `opacity` encoding expression. |
| `shape` | channel | false | The symbol `shape` encoding expression. |
| `size` | channel | false | The symbol `size` encoding expression. |
| `stroke` | channel | false | The symbol `stroke` encoding expression. |
| `stroke_width` | channel | false | The symbol `stroke_width` encoding expression. |
| `x` | channel | true | The symbol `x` encoding expression. |
| `y` | channel | true | The symbol `y` encoding expression. |

## `Transform.aggregate`

Group rows and compute named aggregate measures.

| Name | Role | Required | Description |
|---|---|---:|---|
| `group_by` | property | false | One grouping expression or an array of grouping expressions. |
| `measures` | property | false | Legacy structured named measures; user-named expression properties are preferred. |
| `*` | user-named property | false | A user-named aggregate expression whose property name becomes the output handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {"group_by", "measures"} }: Each user-named aggregate expression exposes a same-named field handle.

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
| `span` | property | false | Optional extent span override. |
| `step` | property | false | Exact requested step. |
| `steps` | property | false | Explicit positive candidate steps. |

## `Transform.calculate`

Append user-named columns computed from row expressions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `*` | user-named property | false | A user-named row expression whose property name becomes the output column and handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {} }: Each expression exposes a same-named output field handle.

## `Transform.filter`

Retain rows for which a predicate is true.

| Name | Role | Required | Description |
|---|---|---:|---|
| `predicate` | property | true | Boolean row predicate. |

## `Transform.fold`

Turn a named set of source expressions into key/value rows.

| Name | Role | Required | Description |
|---|---|---:|---|
| `as_key` | property | false | Generated key column name. |
| `as_value` | property | false | Generated value column name. |
| `fields` | property | true | A map from emitted key labels to source expressions. |
| `index` | property | false | Optional generated source-order column. |

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

## `Transform.join_aggregate`

Compute grouped aggregate measures and join them back onto every input row.

| Name | Role | Required | Description |
|---|---|---:|---|
| `group_by` | property | false | One grouping expression or an array of grouping expressions. |
| `measures` | property | false | Legacy structured named measures; user-named expression properties are preferred. |
| `*` | user-named property | false | A user-named aggregate expression whose property name becomes the output handle. |

Dynamic transform outputs:

- PropertyNames { exclude: {"group_by", "measures"} }: Each user-named aggregate expression exposes a same-named field handle.

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
| `top_n` | property | true | Scalar number of categories to retain. |
| `window` | property | false | Window ranking expression. |

## `Transform.select`

Project an ordered set of source columns and explicitly aliased expressions.

| Name | Role | Required | Description |
|---|---|---:|---|
| `expressions` | property | true | One projection expression or an ordered array of projection expressions. |

## `Transform.sql`

Run one DataFusion SQL query against the reserved `input` relation.

| Name | Role | Required | Description |
|---|---|---:|---|
| `query` | property | true | The SQL query. |

## `Transform.stack`

Compute stacked start and end positions for a quantitative field.

| Name | Role | Required | Description |
|---|---|---:|---|
| `field` | property | true | The quantitative value to stack. |
| `group_by` | property | false | Expressions that partition independent stacks. |
| `name` | property | false | Base name for generated boundary columns. |
| `offset` | property | false | Stack baseline and normalization mode. |
| `sort_by` | property | false | Expressions that order rows within each stack. |
| `value_name` | property | false | Optional copied value output column name. |

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
