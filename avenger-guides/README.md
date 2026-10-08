# avenger-guides

Visualization guide generation for Avenger (axes, legends, colorbars).

## Purpose

This crate generates the visual guides that help users interpret data visualizations. It creates scene graph elements for axes, legends, and colorbars using scale information to produce properly positioned and formatted guide components.

## Integration

- **Input**: Uses `avenger-scales::ConfiguredScale` for scale information and data mappings
- **Output**: Generates `avenger-scenegraph` mark hierarchies for rendering
- **Labels**: Axes and colorbars require an `avenger_format::PreparedFormatter` in their `tick_format` setting, whose kind must match the ticks: numbers for numeric scales, dates for `Date32` and `Date64` time scales, naive datetimes for timestamps without a timezone, and zoned datetimes for timestamps with one, whose timezone must match the scale's. Numeric ticks are labeled together with `format_ticks`, with the scale's `tick_spacing`: `TickSpacing::Varying` for log, threshold, and quantile scales, and `TickSpacing::Uniform` for other scales. Axes label the ticks that the scale's `label_ticks` chooses, so log axes label only the minor ticks that fit the requested density. For automatic time labels, prepare a provider's `CalendarPatterns`. Band and point axes label numeric and temporal categories with the formatter, each on its own, and show other categories as text.
- **Label markup**: `tick_label` is Typst markup for each label, where `#tick` is the tick as `tick_format` formats it, as in `#tick m/s` or `*#tick*`. Numeric ticks with exponent typesetting are Typst math, with or without it.
- **Styling**: `AxisConfig::style` holds colors, fonts, tick count and placement, and title markup. Symbol, line, and colorbar legends share a `LegendStyle` for their background, title, and labels, and `ColorbarStyle` adds the colorbar's thickness, length, and axis gap. Each style's `Default` is the default look.
- **Legends**: `make_symbol_legend` and `make_line_legend` return a `GuideLegendOutput`: the rendered group and the scene paths of its entries. `make_colorbar_legend` returns a `ColorbarOutput`: the rendered group and the scene path and bounds of its gradient. Callers attach interactions through these paths and place a legend through its group's origin.
- **Label angles**: Rotated labels align to the end nearest their tick, so that they extend away from the axis, as Vega-Lite aligns them. `label_align` and `label_baseline` set the alignment instead.
