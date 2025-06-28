# avenger-guides

Visualization guide generation for Avenger (axes, legends, colorbars).

## Purpose

This crate generates the visual guides that help users interpret data visualizations. It creates scene graph elements for axes, legends, and colorbars using scale information to produce properly positioned and formatted guide components.

## Integration

- **Input**: Uses `avenger-scales::ConfiguredScale` for scale information and data mappings
- **Output**: Generates `avenger-scenegraph` mark hierarchies for rendering