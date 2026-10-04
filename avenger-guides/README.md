# avenger-guides

Visualization guide generation for Avenger (axes, legends, colorbars).

## Purpose

This crate generates the visual guides that help users interpret data visualizations. It creates scene graph elements for axes, legends, and colorbars using scale information to produce properly positioned and formatted guide components.

## Integration

- **Input**: Uses `avenger-scales::ConfiguredScale` for scale information and data mappings
- **Output**: Generates `avenger-scenegraph` mark hierarchies for rendering
- **Labels**: Axes and colorbars require an `Arc<dyn avenger_format::PreparedNumberFormatter>` in their `format` setting. Numeric axes label ticks together with `format_ticks`: log, threshold, and quantile scales use `TickSpacing::Varying`, and other scales use `TickSpacing::Uniform`. Band and point axes label numeric categories with the formatter, each by its own digits, and show other categories as the scale formats them.
