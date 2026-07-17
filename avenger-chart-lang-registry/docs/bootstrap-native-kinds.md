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
| `group_by` | property | false | Grouping expressions. |
| `measures` | property | true | Named aggregate measures. |

## `Transform.filter`

Retain rows for which a predicate is true.

| Name | Role | Required | Description |
|---|---|---:|---|
| `predicate` | property | true | Boolean row predicate. |

## `Transform.sql`

Run one DataFusion SQL query against the reserved `input` relation.

| Name | Role | Required | Description |
|---|---|---:|---|
| `query` | property | true | The SQL query. |

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

## `Axis.cartesian`

A Cartesian axis configuration.

## `Legend.standard`

A standard chart legend configuration.

## `Layout.chart`

The default chart frame layout.
