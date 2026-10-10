# avenger-guides

Visualization guide generation for Avenger (axes, legends, colorbars).

## Purpose

This crate generates the visual guides that help users interpret data visualizations. It creates scene graph elements for axes, legends, and colorbars using scale information to produce properly positioned and formatted guide components.

## Integration

- **Input**: Uses `avenger-scales::ConfiguredScale` for scale information and data mappings
- **Output**: Generates `avenger-scenegraph` mark hierarchies for rendering
- **Labels**: Axes and colorbars require an `avenger_format::PreparedFormatter` in their `format` setting, whose kind must match the ticks: numbers for numeric scales, dates for `Date32` and `Date64` time scales, naive datetimes for timestamps without a timezone, and zoned datetimes for timestamps with one, whose timezone must match the scale's. Numeric ticks are labeled together with `format_ticks`: log, threshold, and quantile scales use `TickSpacing::Varying`, and other scales use `TickSpacing::Uniform`. For automatic time labels, prepare a provider's `CalendarPatterns`. Band and point axes label numeric and temporal categories with the formatter, each on its own, and show other categories as text.
- **Runtime value types**: When a guide's value type is known only once data arrives, `axis::guide_format` prepares a formatter for it from a number pattern and the providers, using the datetime provider's calendar patterns for dates and timestamps. `axis::label_values` labels values as axes show them.
- **Styling**: `AxisConfig::style` holds optional colors, fonts, title formatting, and tick placement, and `ColorbarConfig::style` holds the colorbar's size, background, and text styling. `AxisStyle::default()` and `ColorbarStyle::default()` keep the defaults.
- **Label templates**: On continuous axes, `AxisStyle::tick_label` wraps each label in Typst markup, where `#label` stands for the formatted tick, as in `#label m/s`. Numeric labels with exponent typesetting render as Typst math, with or without a template.
